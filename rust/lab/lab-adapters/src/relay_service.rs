use crate::{
    relay::{ADMIN, Fixture, Lab, Limits, Tunnel, Wire, b64, cred, enroll, post, signing_key},
    relay_tls::{self, Origin},
};
use lab_runner::Evidence;
use lab_world::Result;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    net::TcpStream,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn policy(lab: &Lab, host: &str, limits: Limits, disabled: bool) -> bool {
    post(
        &lab.api,
        "/policy",
        &json!({"hostname": host, "limits": limits.json(), "disabled": disabled}),
        Some(ADMIN),
    )
    .0 == 204
}

fn dial(address: &str) -> Option<TcpStream> {
    let socket = TcpStream::connect(address).ok()?;
    socket.set_nonblocking(false).ok()?;
    Some(socket)
}

struct Context<'a> {
    binary: &'a Path,
    state: &'a Path,
    limits: Limits,
}

fn enrollment(binary: &Path, state: &Path, limits: Limits) -> Result<(Arc<Fixture>, Lab)> {
    let fixture = Arc::new(Fixture::open(binary, state, limits)?);
    let lab = Lab::start(&fixture)?;
    Ok((fixture, lab))
}

pub fn run(id: &str, binary: &Path, directory: &Path, evidence: &mut Evidence) -> Result<()> {
    let state = directory.join("relay.json");
    let limits = Limits::standard();
    let (mut store, mut lab) = enrollment(binary, &state, limits)?;
    let key1 = signing_key()?;
    let key2 = signing_key()?;
    let (a, code1) = enroll(&lab.api, &key1, "");
    let (b, code2) = enroll(&lab.api, &key2, "");
    evidence.check(
        "two_distinct_device_enrollments",
        code1 == 200 && code2 == 200 && cred(&a, "hostname") != cred(&b, "hostname"),
        "",
    );
    if code1 != 200 || code2 != 200 {
        return Ok(());
    }
    if id == "RELAY-ENROLL" {
        return enroll_flow(
            binary,
            &state,
            limits,
            (&mut store, &mut lab),
            (&key1, &key2),
            &a,
            evidence,
        );
    }
    let context = Context {
        binary,
        state: &state,
        limits,
    };
    passthrough(id, &context, &store, &lab, (&a, &b), evidence)
}

fn enroll_flow(
    binary: &Path,
    state: &Path,
    limits: Limits,
    current: (&mut Arc<Fixture>, &mut Lab),
    keys: (&ed25519_dalek::SigningKey, &ed25519_dalek::SigningKey),
    a: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let (store, lab) = current;
    lab.close();
    let (reopened, restarted) = enrollment(binary, state, limits)?;
    *store = reopened;
    *lab = restarted;
    let host = cred(a, "hostname");
    let (rotated, status) = enroll(&lab.api, keys.0, "");
    evidence.check(
        "stable_hostname_after_restart",
        status == 200
            && cred(&rotated, "hostname") == host
            && cred(&rotated, "credential") != cred(a, "credential"),
        "",
    );
    let wire = Wire::new(&[]);
    let tunnel = Tunnel::Dial("127.0.0.1:1".into());
    let old = lab.attach_kept(&host, &cred(a, "credential"), tunnel.clone(), &wire);
    evidence.check("old_credential_refused", old.is_err(), "");
    let fresh = lab.attach_kept(&host, &cred(&rotated, "credential"), tunnel.clone(), &wire);
    evidence.check("rotated_credential_accepted", fresh.is_ok(), "");
    let (_, status) = enroll(&lab.api, keys.1, &host);
    evidence.check("hostname_claim_by_other_key_refused", status == 403, "");
    let (status, _) = post(
        &lab.api,
        "/enroll",
        &json!({"public_key": b64(keys.0.verifying_key().as_bytes())}),
        None,
    );
    evidence.check("key_possession_required", status == 403, "");
    let (status, _) = post(
        &lab.api,
        "/revoke",
        &json!({"hostname": host}),
        Some(&cred(&rotated, "credential")),
    );
    evidence.check("revoke", status == 204, "");
    let blocked = lab.attach_kept(&host, &cred(&rotated, "credential"), tunnel, &wire);
    let (_, status) = enroll(&lab.api, keys.0, "");
    evidence.check(
        "revocation_blocks_tunnel_and_reenrollment",
        blocked.is_err() && status == 403,
        "",
    );
    let persisted = Fixture::open(binary, state, limits);
    evidence.check("revocation_persists", persisted.is_ok(), "");
    if let Ok(persisted) = persisted {
        evidence.check(
            "revoked_state",
            persisted.device(&host)["revoked"].as_bool() == Some(true),
            "",
        );
    }
    let mode = fs::metadata(state).map(|m| m.permissions().mode() & 0o777);
    let data = fs::read(state).unwrap_or_default();
    let secret = cred(&rotated, "credential");
    evidence.check(
        "private_state_no_bearer_credentials",
        mode.is_ok_and(|m| m == 0o600)
            && !data
                .windows(secret.len().max(1))
                .any(|w| w == secret.as_bytes()),
        "",
    );
    Ok(())
}

struct Wiring {
    config: Arc<rustls::ClientConfig>,
    wire: Arc<Wire>,
    marker: Vec<u8>,
}
impl Wiring {
    fn request(&self, lab: &Lab, host: &str) -> Result<String> {
        relay_tls::request(&self.config, &lab.address, host, &self.marker)
    }
    fn attach(&self, lab: &Lab, device: &Value, tunnel: Tunnel) -> bool {
        lab.attach_kept(
            &cred(device, "hostname"),
            &cred(device, "credential"),
            tunnel,
            &self.wire,
        )
        .is_ok()
    }
}

fn passthrough(
    id: &str,
    context: &Context,
    store: &Arc<Fixture>,
    lab: &Lab,
    devices: (&Value, &Value),
    evidence: &mut Evidence,
) -> Result<()> {
    let (a, b) = devices;
    let (host_a, host_b) = (cred(a, "hostname"), cred(b, "hostname"));
    let marker = format!("ab-synthetic-secret-marker-{id}").into_bytes();
    let first: Origin = relay_tls::origin(&host_a, "first", marker.clone())?;
    let second: Origin = relay_tls::origin(&host_b, "second", marker.clone())?;
    let wiring = Wiring {
        config: relay_tls::client_config(&[first.authority.clone(), second.authority.clone()])?,
        wire: Wire::new(&marker),
        marker,
    };
    let first_up = wiring.attach(lab, a, Tunnel::Dial(first.address.clone()));
    let second_up = wiring.attach(lab, b, Tunnel::Dial(second.address.clone()));
    evidence.check("authenticated_device_tunnels", first_up && second_up, "");
    let wrong = lab.attach_kept(
        &host_a,
        &cred(b, "credential"),
        Tunnel::Dial(second.address.clone()),
        &wiring.wire,
    );
    evidence.check("cross_device_credential_refused", wrong.is_err(), "");
    let one = wiring.request(lab, &host_a);
    let two = wiring.request(lab, &host_b);
    evidence.check(
        "hosts_reach_only_their_bridge",
        one.as_ref().is_ok_and(|v| v.starts_with("first"))
            && two.as_ref().is_ok_and(|v| v.starts_with("second")),
        "",
    );
    let before = wiring.wire.opens();
    let unknown_hello = relay_tls::hello(&wiring.config, "unknown.relay.test");
    match dial(&lab.address) {
        Some(mut unknown) => {
            let _ = unknown.set_write_timeout(Some(Duration::from_secs(1)));
            let closed = unknown.write_all(&unknown_hello).is_ok() && {
                let (clean, total) = relay_tls::drain(&mut unknown, 1);
                clean && total == 0
            };
            evidence.check("unknown_sni_closed", closed, "");
        }
        None => evidence.check("unknown_sni_closed", false, ""),
    }
    evidence.check(
        "unknown_sni_opens_no_tunnel",
        wiring.wire.opens() == before,
        "",
    );
    if id == "RELAY-LIMITS" {
        limits_flow(context, store, lab, &wiring, devices, &first, evidence);
    }
    if id == "RELAY-PASSTHROUGH" {
        let held = relay_tls::connect(&wiring.config, &lab.address, &host_b, 1);
        evidence.check("listener_exit_active_stream", held.is_ok(), "");
        lab.close_listener();
        evidence.check(
            "listener_exit_returns_promptly",
            lab.serve_exited(Duration::from_secs(1)),
            "",
        );
        if let Ok(mut held) = held {
            evidence.check(
                "listener_exit_closes_stream",
                relay_tls::closed_within(&mut held, 1),
                "",
            );
        }
        evidence.check("listener_exit_drains_streams", wiring.wire.drained(), "");
    }
    evidence.check(
        "relay_tunnel_buffers_contain_ciphertext_only",
        wiring.wire.ciphertext_only(),
        "",
    );
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn limits_flow(
    context: &Context,
    store: &Arc<Fixture>,
    lab: &Lab,
    wiring: &Wiring,
    devices: (&Value, &Value),
    first: &Origin,
    evidence: &mut Evidence,
) {
    let (a, b) = devices;
    let limits = context.limits;
    let host = cred(a, "hostname");
    let host_b = cred(b, "hostname");
    let small = Limits {
        connections: 1,
        ..limits
    };
    let origin = || Tunnel::Dial(first.address.clone());
    evidence.check(
        "connection_policy",
        policy(lab, &host, small, false) && wiring.attach(lab, a, origin()),
        "",
    );
    match dial(&lab.address) {
        Some(mut stalled) => {
            let _ = stalled.set_write_timeout(Some(Duration::from_secs(3)));
            let hello = relay_tls::hello(&wiring.config, &host);
            if stalled.write_all(&hello).is_ok() {
                let (clean, total) = relay_tls::drain(&mut stalled, 3);
                evidence.check(
                    "stalled_tls_releases_slot",
                    clean && total > 0 && wiring.wire.drained(),
                    "",
                );
            } else {
                evidence.check("stalled_tls_releases_slot", false, "");
            }
        }
        None => evidence.check("stalled_tls_releases_slot", false, ""),
    }
    evidence.check(
        "stalled_tls_slot_reusable",
        wiring.request(lab, &host).is_ok(),
        "",
    );
    evidence.check("stalled_tls_probe_drains", wiring.wire.drained(), "");
    lab.disconnect(&host);
    let blocked_up = wiring.attach(lab, a, Tunnel::Blocked);
    let opened = wiring.wire.opens();
    match (blocked_up, dial(&lab.address)) {
        (true, Some(mut socket)) => {
            let hello = relay_tls::hello(&wiring.config, &host);
            let wrote = socket.write_all(&hello).is_ok();
            let (clean, total) = relay_tls::drain(&mut socket, 3);
            let drained = wiring.wire.drained();
            evidence.check(
                "blocked_tunnel_write_releases_slot",
                wrote && clean && total == 0 && wiring.wire.opens() == opened + 1 && drained,
                "",
            );
        }
        _ => evidence.check("blocked_tunnel_write_releases_slot", false, ""),
    }
    lab.disconnect(&host);
    evidence.check(
        "restore_origin_after_idle",
        wiring.attach(lab, a, origin()),
        "",
    );
    evidence.check(
        "slot_reusable_after_idle",
        wiring.request(lab, &host).is_ok(),
        "",
    );
    evidence.check("idle_probe_drains", wiring.wire.drained(), "");
    let held = relay_tls::connect(&wiring.config, &lab.address, &host, 2);
    evidence.check("first_connection_accepted", held.is_ok(), "");
    evidence.check(
        "excess_connection_rejected",
        wiring.request(lab, &host).is_err(),
        "",
    );
    evidence.check("kill_switch", policy(lab, &host, small, true), "");
    if let Ok(mut held) = held {
        evidence.check(
            "kill_closes_active_connection",
            relay_tls::closed_within(&mut held, 1),
            "",
        );
        evidence.check("kill_drains_stream", wiring.wire.drained(), "");
    }
    let reattached = lab.attach_kept(&host, &cred(a, "credential"), origin(), &wiring.wire);
    evidence.check("kill_blocks_reattach", reattached.is_err(), "");
    let slow = Limits {
        bytes_per_second: 32768,
        ..limits
    };
    evidence.check(
        "bandwidth_policy",
        policy(lab, &host, slow, false) && wiring.attach(lab, a, origin()),
        "",
    );
    let started = Instant::now();
    let throttled = wiring.request(lab, &host);
    evidence.check(
        "aggregate_bandwidth_throttled",
        throttled.is_ok() && started.elapsed() >= Duration::from_millis(250),
        "",
    );
    lab.disconnect(&host);
    let used = store.device(&host)["bytes"].as_i64().unwrap_or(0);
    let exhausted = Limits {
        monthly_bytes: used + 1,
        ..limits
    };
    evidence.check(
        "monthly_policy",
        policy(lab, &host, exhausted, false) && wiring.attach(lab, a, origin()),
        "",
    );
    evidence.check(
        "monthly_budget_rejects",
        wiring.request(lab, &host).is_err(),
        "",
    );
    match Fixture::open(context.binary, context.state, limits) {
        Ok(reopened) => evidence.check(
            "monthly_usage_persisted",
            reopened.device(&host)["bytes"].as_i64() == Some(exhausted.monthly_bytes),
            "",
        ),
        Err(_) => evidence.check("monthly_usage_persisted", false, ""),
    }
    evidence.check(
        "other_device_unaffected",
        wiring
            .request(lab, &host_b)
            .is_ok_and(|v| v.starts_with("second")),
        "",
    );
}
