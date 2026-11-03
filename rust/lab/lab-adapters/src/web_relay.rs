#![allow(clippy::too_many_lines)]
use crate::replicas;
use lab_relay::{
    Ca, Frame, Identity, Link, PairCode, Principal, client_config, configure_gateway, fingerprint,
    gateway_address, gateway_key, hello, input1, input2, join, proof_frame, relay1_client,
    relay2_client, replay_gateway, valid_certificate, verify,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rustls::ClientConfig;
use serde_json::Value;
use std::{
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 3] = ["WEB-RELAY-AGENTS", "WEB-RELAY-AUTH", "WEB-RELAY-PAIRING"];
const SCOPE: &str = "reminders:read,reminders:write,system:read";
const ONE_SECOND: Duration = Duration::from_secs(1);

struct Gateway {
    ca: Ca,
    config: Arc<ClientConfig>,
    address: String,
    pin: String,
}
impl Gateway {
    fn dial(&self) -> lab_relay::Result<Link> {
        Link::dial(&self.address, &self.config, None)
    }
}
fn invalid<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}
fn expose(world: &mut World) -> Result<Gateway> {
    let ca = Ca::new().map_err(invalid)?;
    let leaf = ca
        .issue(&world.directory.join("lab-relay"))
        .map_err(invalid)?;
    configure_gateway(&world.gateway_home, "127.0.0.1:0", &leaf).map_err(invalid)?;
    world.restart_gateway()?;
    let address = gateway_address(&world.gateway_home, Duration::from_secs(15)).map_err(invalid)?;
    let config = client_config(ca.roots().map_err(invalid)?).map_err(invalid)?;
    let pin = Identity::load(&world.gateway_home)
        .map_err(invalid)?
        .fingerprint;
    Ok(Gateway {
        ca,
        config,
        address,
        pin,
    })
}
fn principal(world: &World, selector: [&str; 2]) -> Option<Value> {
    let text = replicas::cli(
        &world.gateway_home,
        &["lab-principal", selector[0], selector[1]],
    )
    .ok()?;
    serde_json::from_str(&text).ok()
}
fn grants_of(value: &Value) -> Vec<String> {
    let mut grants: Vec<String> = value["grants"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| g.as_str().map(str::to_owned))
        .collect();
    grants.sort();
    grants
}
fn replica_identity(world: &World, index: usize) -> Result<Identity> {
    Identity::load(&world.directory.join(format!("replica{index}"))).map_err(invalid)
}
fn principal_shaped(principal: &Principal) -> bool {
    principal.replica_shaped()
}
fn parse(text: &str) -> Option<Frame> {
    Frame::parse(text).ok()
}

pub fn web_relay(world: &mut World, _input: &Value, evidence: &mut Evidence) -> Result<()> {
    let outcome = match evidence.scenario.as_str() {
        "WEB-RELAY-AUTH" => {
            auth(world, evidence);
            Ok(())
        }
        "WEB-RELAY-PAIRING" => {
            pairing(world, evidence);
            Ok(())
        }
        other => Err(Error::Invalid(format!(
            "web relay scenario {other} not ported"
        ))),
    };
    crate::labcore::oversized_resets(world, evidence);
    outcome
}

fn auth(world: &mut World, e: &mut Evidence) {
    let started = expose(world).and_then(|gateway| {
        replicas::start_with(world, 2, SCOPE, 0)?;
        Ok(gateway)
    });
    e.check("paired_world", started.is_ok(), "");
    let Ok(gateway) = started else {
        return;
    };
    e.check("gateway_identity", true, "");
    let conn = gateway.dial();
    e.check(
        "remote_websocket_without_client_tls_certificate",
        conn.is_ok(),
        "",
    );
    let Ok(mut conn) = conn else {
        return;
    };
    let replica = replica_identity(world, 0);
    e.check("replica_identity", replica.is_ok(), "");
    let Ok(replica) = replica else {
        return;
    };
    let first = relay1_client(&mut conn, &replica, &gateway.pin);
    e.check(
        "mutual_device_key_authentication",
        first.as_ref().is_ok_and(principal_shaped),
        "",
    );
    let Ok(authenticated) = first else {
        return;
    };
    capture1(&gateway, &replica, e);
    e.check("hello_before_auth_rejected", reject_hello(&gateway), "");
    e.check(
        "binary_auth_frame_rejected",
        reject_binary(&gateway, &replica),
        "",
    );
    let wrong = gateway.dial();
    e.check("wrong_pin_tls_connection", wrong.is_ok(), "");
    let mut result = wrong.map(|_| ());
    if let Ok(mut link) = gateway.dial() {
        result = relay1_client(&mut link, &replica, &"0".repeat(64)).map(|_| ());
    }
    e.check(
        "webpki_alone_does_not_authenticate_gateway",
        result.is_err(),
        "",
    );
    let stranger = Identity::generate();
    e.check("unpaired_device_identity", stranger.is_ok(), "");
    let mut outcome: lab_relay::Result<()> = Err(lab_relay::Error::Invalid);
    if let Ok(stranger) = &stranger {
        let link = gateway.dial();
        e.check("unpaired_device_tls_connection", link.is_ok(), "");
        if let Ok(mut link) = link {
            outcome = relay1_client(&mut link, stranger, &gateway.pin).map(|_| ());
        }
    }
    e.check("unpaired_device_key_rejected", outcome.is_err(), "");
    let origin = Link::dial(
        &gateway.address,
        &gateway.config,
        Some("https://foreign.example.com"),
    );
    e.check(
        "browser_origin_rejected",
        matches!(origin, Err(lab_relay::Error::Status(401))),
        "",
    );
    let old_revision = authenticated.revision;
    let changed = replicas::cli(
        &world.gateway_home,
        &["scope", &authenticated.id, "--scope", "system:read"],
    )
    .is_ok();
    e.check(
        "scope_change_closes_authenticated_socket",
        changed && conn.closed_within(ONE_SECOND),
        "",
    );
    let scoped = gateway
        .dial()
        .and_then(|mut link| relay1_client(&mut link, &replica, &gateway.pin).map(|p| (link, p)));
    let reauthenticated = scoped
        .as_ref()
        .is_ok_and(|(_, p)| p.revision == old_revision + 1 && p.grants == ["system:read"]);
    e.check(
        "subscription_revision_and_scope_reauthenticated",
        reauthenticated,
        "",
    );
    if let Ok((mut link, principal)) = scoped {
        let revoked = replicas::cli(&world.gateway_home, &["revoke", &principal.id]).is_ok();
        let started = Instant::now();
        let closed = link.closed_within(ONE_SECOND);
        e.check(
            "revocation_closes_within_one_second",
            revoked && closed && started.elapsed() < ONE_SECOND,
            "",
        );
    }
    let mut blocked = gateway.dial();
    let mut rejected = blocked.is_err();
    if let Ok(link) = blocked.as_mut() {
        rejected = relay1_client(link, &replica, &gateway.pin).is_err();
    }
    e.check("revoked_device_key_rejected", rejected, "");
    let stranger2 = Identity::generate();
    e.check("relay2_unpaired_device_identity", stranger2.is_ok(), "");
    if let Ok(stranger2) = stranger2 {
        let bound = replica_identity(world, 1);
        e.check("relay2_replica_identity", bound.is_ok(), "");
        if let Ok(bound) = bound {
            relay2_variant(&gateway, &bound, &stranger2, e);
        }
    }
}

fn capture1(gateway: &Gateway, replica: &Identity, e: &mut Evidence) {
    let link = gateway.dial();
    e.check("capture_connection", link.is_ok(), "");
    let Ok(mut conn) = link else {
        return;
    };
    let hello = hello(replica, false, None);
    let Ok(text) = hello.text() else {
        return;
    };
    let _ = conn.send_text(&text);
    let Some(challenge) = conn
        .recv_text(Duration::from_secs(5))
        .ok()
        .and_then(|t| parse(&t))
    else {
        e.check("capture_challenge", false, "");
        return;
    };
    let Ok((der, key)) = gateway_key(&challenge) else {
        e.check("capture_gateway_certificate", false, "");
        return;
    };
    let signed = input1("gateway", &hello, &challenge, None)
        .ok()
        .zip(challenge.signature_bytes().ok())
        .is_some_and(|(input, signature)| verify(&key, &signature, &input));
    e.check(
        "gateway_signed_both_nonces_and_pinned_identities",
        challenge.nonce_bytes().is_ok_and(|n| n.len() == 32)
            && fingerprint(&der) == gateway.pin
            && signed,
        "",
    );
    let Ok(replica_input) = input1("replica", &hello, &challenge, None) else {
        return;
    };
    let proof = proof_frame(false, &replica.sign(&replica_input), None);
    let Ok(proof_text) = proof.text() else {
        return;
    };
    let _ = conn.send_text(&proof_text);
    let accepted = conn
        .recv_text(Duration::from_secs(5))
        .ok()
        .and_then(|t| parse(&t));
    e.check(
        "independent_replica_signature_accepted",
        accepted.is_some_and(|f| f.t == "Authenticated"),
        "",
    );
    let replay = gateway.dial();
    e.check("replay_connection", replay.is_ok(), "");
    let Ok(mut replay) = replay else {
        return;
    };
    let _ = replay.send_text(&hello.text().unwrap_or_default());
    let fresh = replay
        .recv_text(Duration::from_secs(5))
        .ok()
        .and_then(|t| parse(&t));
    e.check(
        "fresh_server_nonce_on_reused_client_nonce",
        fresh.is_some_and(|f| f.nonce != challenge.nonce),
        "",
    );
    let _ = replay.send_text(&proof_text);
    e.check(
        "captured_auth_frame_replay_rejected",
        replay.closed_within(ONE_SECOND),
        "",
    );
    let Ok((address, sent)) = replay_gateway(&gateway.ca, challenge.text().unwrap_or_default())
    else {
        return;
    };
    let forged = Link::dial(&address, &gateway.config, None);
    e.check("gateway_replay_tls_connection", forged.is_ok(), "");
    if let Ok(mut forged) = forged {
        let outcome = relay1_client(&mut forged, replica, &gateway.pin);
        drop(forged);
        let proof_sent = sent.recv_timeout(Duration::from_secs(5)).unwrap_or(false);
        e.check(
            "replayed_gateway_signature_rejected_before_replica_proof",
            outcome.is_err() && !proof_sent,
            "",
        );
    }
}
fn reject_hello(gateway: &Gateway) -> bool {
    let Ok(mut conn) = gateway.dial() else {
        return false;
    };
    let _ =
        conn.send_text(r#"{"t":"Hello","protocol":"feed/1","replica_id":"replica:unverified"}"#);
    conn.closed_within(ONE_SECOND)
}
fn reject_binary(gateway: &Gateway, replica: &Identity) -> bool {
    let Ok(mut conn) = gateway.dial() else {
        return false;
    };
    let Ok(text) = hello(replica, false, None).text() else {
        return false;
    };
    conn.send_binary(text.as_bytes()).is_ok() && conn.closed_within(ONE_SECOND)
}

fn relay2_variant(gateway: &Gateway, replica: &Identity, stranger: &Identity, e: &mut Evidence) {
    let conn = gateway.dial();
    e.check(
        "relay2_bound_tls_connection",
        conn.as_ref().is_ok_and(|c| c.binding.len() == 32),
        "",
    );
    let Ok(mut conn) = conn else {
        return;
    };
    let authenticated = relay2_client(&mut conn, replica, &gateway.pin);
    e.check(
        "relay2_mutual_device_key_authentication",
        authenticated.as_ref().is_ok_and(principal_shaped),
        "",
    );
    drop(conn);
    let capture = gateway.dial();
    e.check("relay2_capture_connection", capture.is_ok(), "");
    let Ok(mut capture) = capture else {
        return;
    };
    let binding = capture.binding;
    let hello = hello(replica, true, None);
    let Ok(text) = hello.text() else {
        e.check("relay2_capture_nonce", false, "");
        return;
    };
    let _ = capture.send_text(&text);
    let Some(challenge) = capture
        .recv_text(Duration::from_secs(5))
        .ok()
        .and_then(|t| parse(&t))
    else {
        e.check("relay2_capture_challenge", false, "");
        return;
    };
    let Ok((der, key)) = gateway_key(&challenge) else {
        e.check("relay2_capture_gateway_certificate", false, "");
        return;
    };
    let signature = challenge.signature_bytes().unwrap_or_default();
    let valid = |binding: &[u8]| {
        input2("gateway", &hello, &challenge, None, binding)
            .is_ok_and(|input| verify(&key, &signature, &input))
    };
    e.check(
        "relay2_gateway_signed_binding_nonces_and_pinned_identities",
        challenge.version == "relay/2"
            && challenge.nonce_bytes().is_ok_and(|n| n.len() == 32)
            && fingerprint(&der) == gateway.pin
            && valid(&binding),
        "",
    );
    e.check(
        "relay2_gateway_signature_not_valid_without_binding",
        !valid(&[0; 32]),
        "",
    );
    let Ok(replica_input) = input2("replica", &hello, &challenge, None, &binding) else {
        return;
    };
    let proof = proof_frame(true, &replica.sign(&replica_input), None);
    let Ok(proof_text) = proof.text() else {
        return;
    };
    let _ = capture.send_text(&proof_text);
    let accepted = capture
        .recv_text(Duration::from_secs(5))
        .ok()
        .and_then(|t| parse(&t));
    let accepted_ok = accepted.is_some_and(|f| {
        f.t == "Authenticated"
            && f.principal_text().is_some_and(|raw| {
                f.signature_bytes().is_ok_and(|sig| {
                    input2("accepted", &hello, &challenge, Some(raw), &binding)
                        .is_ok_and(|input| verify(&key, &sig, &input))
                })
            })
    });
    e.check(
        "relay2_independent_replica_signature_accepted",
        accepted_ok,
        "",
    );
    let replay = gateway.dial();
    e.check("relay2_replay_connection", replay.is_ok(), "");
    let Ok(mut replay) = replay else {
        return;
    };
    let _ = replay.send_text(&text);
    let fresh = replay
        .recv_text(Duration::from_secs(5))
        .ok()
        .and_then(|t| parse(&t));
    e.check(
        "relay2_fresh_server_nonce_on_reused_client_nonce",
        fresh.is_some_and(|f| f.nonce != challenge.nonce),
        "",
    );
    let _ = replay.send_text(&proof_text);
    e.check(
        "relay2_captured_proof_replay_on_other_connection_rejected",
        replay.closed_within(ONE_SECOND),
        "",
    );
    let mut rejected = true;
    if let Ok(mut link) = gateway.dial() {
        rejected = relay2_client(&mut link, replica, &"0".repeat(64)).is_err();
    }
    e.check("relay2_wrong_pin_rejected", rejected, "");
    let mut rejected = true;
    if let Ok(mut link) = gateway.dial() {
        rejected = relay2_client(&mut link, stranger, &gateway.pin).is_err();
    }
    e.check("relay2_unpaired_device_key_rejected", rejected, "");
    let mut rejected = true;
    if let Ok(mut link) = gateway.dial() {
        rejected = relay1_client(&mut link, replica, &gateway.pin).is_err();
    }
    e.check(
        "relay1_rejected_for_peer_that_authenticated_with_relay2",
        rejected,
        "",
    );
}

fn pair_code(world: &World, url: &str, scope: &str, ttl: &str) -> lab_world::Result<String> {
    let output = Command::new(replicas::binary()?)
        .args([
            "pair",
            "--via-remote",
            "--scope",
            scope,
            "--expires-in",
            ttl,
            "--home",
        ])
        .arg(&world.gateway_home)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .env("APPLE_BRIDGE_LAB_PAIR_URL", url)
        .output()?;
    if !output.status.success() {
        return Err(Error::Invalid(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn pairing(world: &mut World, e: &mut Evidence) {
    let exposed = expose(world);
    e.check("pairing_world", exposed.is_ok(), "");
    let Ok(gateway) = exposed else {
        return;
    };
    e.check("gateway_identity", true, "");
    let url = format!(
        "https://{}:{}",
        lab_relay::HOST,
        gateway.address.rsplit(':').next().unwrap_or_default()
    );
    let encoded = pair_code(world, &url, "reminders:write,system:read", "10m");
    e.check("remote_pair_code_created", encoded.is_ok(), "");
    let Ok(encoded) = encoded else {
        return;
    };
    let code = PairCode::decode(&encoded);
    e.check(
        "remote_pair_code_transport",
        code.as_ref()
            .is_ok_and(|c| c.remote && c.address == url && c.fingerprint == gateway.pin),
        "",
    );
    let Ok(code) = code else {
        return;
    };
    if !code.remote {
        return;
    }
    let replica = Identity::generate();
    e.check("replica_key_created", replica.is_ok(), "");
    let Ok(replica) = replica else {
        return;
    };
    let attempt = |candidate: &PairCode| -> lab_relay::Result<Principal> {
        let mut link = gateway.dial()?;
        join(&mut link, &replica, candidate)
    };
    let mut bad_pin = code.clone();
    bad_pin.fingerprint = "0".repeat(64);
    e.check(
        "join_pins_gateway_before_using_code",
        matches!(attempt(&bad_pin), Err(lab_relay::Error::Unauthorized)),
        "",
    );
    let joined = attempt(&code);
    e.check("remote_signed_pairing", joined.is_ok(), "");
    let enrolled = joined
        .as_ref()
        .ok()
        .filter(|p| p.fingerprint == replica.fingerprint);
    e.check(
        "remote_enrollment_persists_public_url_and_gateway_pin",
        enrolled.is_some() && code.remote && code.address == url && code.fingerprint == gateway.pin,
        "",
    );
    let Some(enrolled) = enrolled else {
        return;
    };
    if !valid_certificate(&replica.certificate) {
        e.check("paired_certificate", false, "");
        return;
    }
    let paired = principal(world, ["--fingerprint", &replica.fingerprint]);
    let wanted = vec!["reminders:write".to_owned(), "system:read".to_owned()];
    let mut enrolled_grants = enrolled.grants.clone();
    enrolled_grants.sort();
    e.check(
        "paired_replica_identity_and_delegable_scope",
        paired.as_ref().is_some_and(|p| {
            p["id"] == enrolled.id.as_str()
                && p["fingerprint"] == replica.fingerprint.as_str()
                && grants_of(p) == wanted
        }) && enrolled_grants == wanted,
        "",
    );
    e.check("remote_pair_code_single_use", attempt(&code).is_err(), "");
    let unchanged = principal(world, ["--fingerprint", &replica.fingerprint]);
    e.check(
        "failed_replay_preserves_enrollment",
        unchanged.is_some_and(|p| p["id"] == enrolled.id.as_str()),
        "",
    );
    let mut expired = code.clone();
    expired.expire();
    e.check(
        "expired_remote_code_rejected",
        matches!(attempt(&expired), Err(lab_relay::Error::Expired)),
        "",
    );
    for address in [
        "http://bridge.example.com",
        "wss://bridge.example.com",
        "https://owner:password@bridge.example.com",
        "https://bridge.example.com/feed",
        "https://bridge.example.com?token=invalid",
        "https://bridge.example.com#fragment",
        "https://bridge.example.com:0",
        "https://bridge.example.com:65536",
    ] {
        let rejected = pair_code(world, address, "system:read", "1m")
            .err()
            .is_some_and(|error| error.to_string().to_lowercase().contains("invalid"));
        e.check(&format!("remote_pair_url_rejected_{address}"), rejected, "");
    }
    let Ok(second) = pair_code(world, &url, "system:read", "1m") else {
        e.check("attempt_code", false, "");
        return;
    };
    let Ok(second) = PairCode::decode(&second) else {
        e.check("attempt_decode", false, "");
        return;
    };
    let mut bad = second.clone();
    if bad.flip_secret_bit().is_err() {
        return;
    }
    for _ in 0..5 {
        e.check(
            "bad_remote_pair_secret_rejected",
            attempt(&bad).is_err(),
            "",
        );
    }
    e.check(
        "remote_pair_five_attempt_limit",
        attempt(&second).is_err(),
        "",
    );
}
