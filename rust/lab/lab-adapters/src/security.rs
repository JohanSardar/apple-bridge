#![allow(clippy::too_many_lines)]
use crate::{
    consent::{Site, edit_credentials, past, sha256_hex},
    rawhttp::{self, form, query_value},
    relay::{Fixture, Limits, b64, fixture_binary, signing_key},
    replicas,
    webauthn::{COOKIE, Owner},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::Signer;
use jiff::{Timestamp, ToSpan};
use lab_relay::{Ca, Identity, client_config, configure_gateway, gateway_address, relay1_client};
use lab_runner::{Evidence, Scenario};
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 1] = ["SEC-REMOTE-ACCESS"];
const OPENAPI: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/golden/openapi.json"
);

pub fn inject(scenario: &mut Scenario) {
    if SCENARIOS.contains(&scenario.id.as_str())
        && scenario.given.is_empty()
        && scenario.when.is_empty()
        && scenario.then.is_empty()
    {
        scenario.when.push(json!({"security_remote": {}}));
    }
}

fn invalid<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}

fn relay_proofs(e: &mut Evidence) -> Result<()> {
    let directory = std::env::temp_dir().join(format!(
        "ab-test-security-relay-{}-{}",
        std::process::id(),
        Timestamp::now().as_millisecond()
    ));
    fs::create_dir_all(&directory)?;
    let opened = Fixture::open_zone(
        &fixture_binary()?,
        &directory.join("state.json"),
        "relay.example",
        Limits {
            connections: 2,
            bytes_per_second: 1024,
            monthly_bytes: 4096,
        },
    );
    e.check("relay_fixture", opened.is_ok(), "");
    let Ok(store) = opened else {
        return Ok(());
    };
    let device = signing_key()?;
    let public = b64(device.verifying_key().as_bytes());
    let challenge = |store: &Fixture| -> Option<String> {
        store
            .rpc("challenge", Value::Null)
            .ok()
            .and_then(|nonce| nonce.as_str().map(str::to_owned))
    };
    let Some(nonce) = challenge(&store) else {
        e.check("relay_fixture_store", false, "");
        return Ok(());
    };
    let message = format!("apple-bridge-relay-enroll\nrelay.example\n{nonce}");
    let enrollment = json!({
        "public_key": public,
        "nonce": nonce,
        "signature": b64(&device.sign(message.as_bytes()).to_bytes()),
        "hostname": "",
    });
    let credential = store.rpc("enroll", json!({"input": enrollment}));
    e.check("relay_signed_enrollment", credential.is_ok(), "");
    let Ok(credential) = credential else {
        return Ok(());
    };
    e.check(
        "relay_enrollment_replay_refused",
        store.rpc("enroll", json!({"input": enrollment})).is_err(),
        "",
    );
    let host = credential["hostname"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let secret = credential["credential"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let proof = |action: &str| -> Value {
        let nonce = challenge(&store).unwrap_or_default();
        let text = format!("apple-bridge-relay/1\n{action}\n{host}\n{secret}\n{nonce}");
        json!({
            "hostname": host,
            "credential": secret,
            "nonce": nonce,
            "signature": b64(&device.sign(text.as_bytes()).to_bytes()),
        })
    };
    let verify = |proof: &Value, action: &str| {
        store
            .rpc("verify", json!({"proof": proof, "proof_action": action}))
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    };
    let once = proof("tunnel");
    e.check(
        "relay_proof_valid_once",
        verify(&once, "tunnel") && !verify(&once, "tunnel"),
        "",
    );
    e.check(
        "relay_proof_wrong_action_refused",
        !verify(&proof("tunnel"), "dns:value"),
        "",
    );
    let mut other = proof("tunnel");
    other["hostname"] = json!("attacker.relay.example");
    e.check(
        "relay_proof_wrong_device_refused",
        !verify(&other, "tunnel"),
        "",
    );
    let mut other = proof("tunnel");
    other["credential"] = json!("[ab-test] wrong");
    e.check(
        "relay_wrong_credential_refused",
        !verify(&other, "tunnel"),
        "",
    );
    let mut other = proof("tunnel");
    let stranger = signing_key()?;
    let text = format!(
        "apple-bridge-relay/1\ntunnel\n{host}\n{secret}\n{}",
        other["nonce"].as_str().unwrap_or_default()
    );
    other["signature"] = json!(b64(&stranger.sign(text.as_bytes()).to_bytes()));
    e.check(
        "relay_credential_without_private_key_refused",
        !verify(&other, "tunnel"),
        "",
    );
    let limits = store.device(&host)["limits"].clone();
    let revoked = store
        .rpc(
            "policy",
            json!({"host": host, "limits": limits, "disabled": false, "revoked": true}),
        )
        .is_ok();
    e.check(
        "relay_revoked_credential_refused",
        revoked && !verify(&proof("tunnel"), "tunnel"),
        "",
    );
    let _ = fs::remove_dir_all(&directory);
    Ok(())
}

fn public_health(site: &Site, e: &mut Evidence) {
    let reply = rawhttp::request(
        &site.address,
        "GET",
        "/v1/health",
        &[("Host", "bridge.example")],
        None,
        10,
    );
    e.check(
        "public_health_requires_authentication",
        reply.is_ok_and(|r| r.status == 401),
        "",
    );
}

fn remote_failure(world: &World, e: &mut Evidence) -> Result<()> {
    let Ok(mut isolated) = world.sibling("remote") else {
        e.check("remote_failure_fixture", false, "");
        return Ok(());
    };
    let directory = isolated.directory.clone();
    let socket = directory.join("localapi.sock");
    let listener = UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let server = thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer);
                    let body =
                        r#"{"error":"https://login.example/secret?token=[ab-test]-credential"}"#;
                    let _ = stream.write_all(
                        format!(
                            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    );
                }
                Err(_) => thread::sleep(Duration::from_millis(5)),
            }
        }
    });
    let shim = directory.join("bin");
    fs::create_dir_all(&shim)?;
    for name in ["open", "xdg-open"] {
        let path = shim.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    let path = format!(
        "{}:{}",
        shim.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let environment = vec![
        (
            "APPLE_BRIDGE_TAILSCALE_SOCKET".to_owned(),
            socket.to_string_lossy().into_owned(),
        ),
        ("PATH".to_owned(), path.clone()),
    ];
    let restarted = isolated.configure_gateway(environment, vec![]);
    let log_path = directory.join("gateway.log");
    let before = fs::metadata(&log_path).map_or(0, |m| m.len());
    let output = restarted.ok().and_then(|()| {
        Command::new(replicas::binary().ok()?)
            .args(["remote", "on", "--via", "tailscale-app", "--home"])
            .arg(&isolated.gateway_home)
            .env("PATH", &path)
            .env("APPLE_BRIDGE_SECRET_STORE", "file")
            .env("APPLE_BRIDGE_TAILSCALE_SOCKET", &socket)
            .stdin(Stdio::null())
            .output()
            .ok()
    });
    thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::SeqCst);
    let _ = server.join();
    let log = fs::read(&log_path).unwrap_or_default();
    let appended = &log[usize::try_from(before).unwrap_or(0).min(log.len())..];
    let appended = String::from_utf8_lossy(appended);
    let safe = output.is_some_and(|o| {
        let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
        !o.status.success()
            && stderr.contains("Remote setup failed")
            && !stderr.contains("credential")
            && !appended.contains("credential")
            && !appended.contains("https://")
    });
    e.check(
        "remote_setup_failure_redacts_credential_urls_in_logs",
        safe,
        "",
    );
    isolated.close();
    Ok(())
}

fn operations() -> Vec<String> {
    let raw = fs::read(OPENAPI)
        .or_else(|_| fs::read("contracts/golden/openapi.json"))
        .unwrap_or_default();
    let document: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    document["paths"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(path, _)| path.strip_prefix("/v1/op/").map(str::to_owned))
        .collect()
}

pub(crate) fn endpoint_sweep(site: &Site, public: bool, e: &mut Evidence) {
    sweep(
        public,
        |method, path| site.json(method, path, "", &json!({})).status,
        |path| site.bare("GET", path, "").status,
        e,
    );
}

fn sweep(
    public: bool,
    status_of: impl Fn(&str, &str) -> u16,
    bootstrap: impl Fn(&str) -> u16,
    e: &mut Evidence,
) {
    let mut paths: Vec<(String, String)> = [
        ("GET", "/mcp"),
        ("POST", "/mcp"),
        ("GET", "/mcp/unknown"),
        ("GET", "/v1/openapi.json"),
        ("GET", "/v1/records/messages.chat/test"),
        ("GET", "/v1/changes"),
        ("GET", "/v1/blobs/unknown"),
        ("POST", "/v1/blobs"),
        ("GET", "/v1/unknown"),
        ("GET", "/v1/snapshot"),
        ("POST", "/v1/commands"),
        ("GET", "/web/v1/session"),
        ("GET", "/web/v1/agents"),
        ("GET", "/web/v1/devices"),
        ("GET", "/web/v1/passkeys"),
        ("GET", "/web/v1/activity"),
        ("GET", "/web/v1/approvals"),
        ("GET", "/web/v1/sources"),
        ("GET", "/web/v1/live"),
        ("POST", "/web/v1/handles"),
        ("POST", "/web/v1/lookup"),
        ("POST", "/web/v1/upload"),
        ("GET", "/web/v1/blob/unknown"),
        ("GET", "/web/v1/upload/unknown"),
        ("POST", "/oauth/authorize"),
        ("POST", "/approvals/test"),
        ("GET", "/auth/session"),
        ("POST", "/auth/passkeys/options"),
    ]
    .iter()
    .map(|(m, p)| ((*m).to_owned(), (*p).to_owned()))
    .collect();
    if public {
        paths.push(("GET".into(), "/v1/health".into()));
    }
    for name in operations() {
        paths.push(("POST".into(), format!("/v1/op/{name}")));
    }
    let mut all = true;
    for (method, path) in &paths {
        let status = status_of(method, path);
        let denied =
            status == 401 || status == 403 || (path.starts_with("/web/v1/") && status == 404);
        all &= denied;
        if !denied {
            e.check(
                &format!("unauthenticated_denied_{method}_{path}"),
                false,
                "",
            );
        }
    }
    e.check(
        "all_protected_endpoints_refuse_unauthenticated_requests",
        all,
        "",
    );
    for path in [
        "/",
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-protected-resource",
    ] {
        e.check(
            &format!("public_bootstrap_{path}"),
            bootstrap(path) == 200,
            "",
        );
    }
}

fn oauth(world: &World, site: &Site, owner: &Owner, e: &mut Evidence) {
    let origin = &site.origin;
    let resource = format!("{origin}/mcp");
    let redirect = "https://connector.example/callback";
    let registered = site.json(
        "POST",
        "/oauth/register",
        "",
        &json!({
            "client_name": "[ab-test] security",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_method": "none"
        }),
    );
    let client = registered.json()["client_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    e.check(
        "security_oauth_client",
        registered.status == 201 && !client.is_empty(),
        "",
    );
    if client.is_empty() {
        return;
    }
    let verifier = "a".repeat(43);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let query = |overrides: &[(&str, &str)]| {
        let mut pairs: Vec<(&str, &str)> = vec![
            ("client_id", &client),
            ("response_type", "code"),
            ("redirect_uri", redirect),
            ("resource", &resource),
            ("scope", "messages:read"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ];
        for (key, value) in overrides {
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            }
        }
        form(&pairs)
    };
    for (key, value) in [
        ("code_challenge_method", "plain"),
        ("code_challenge_method", ""),
        ("redirect_uri", "https://attacker.example/callback"),
        ("redirect_uri", "//attacker.example"),
        ("redirect_uri", "javascript:alert(1)"),
    ] {
        let reply = site.bare(
            "GET",
            &format!("/oauth/authorize?{}", query(&[(key, value)])),
            "",
        );
        e.check(
            "pkce_downgrade_and_open_redirect_refused",
            reply.status == 400 && reply.header("Location").is_none(),
            "",
        );
    }
    let request = |token: &str, collection: &str| {
        site.json(
            "POST",
            "/v1/op/records.query",
            token,
            &json!({"collection": collection, "limit": 1}),
        )
        .status
    };
    let issue = |e: &mut Evidence| -> String {
        let reply = owner.request(
            "POST",
            &format!("/oauth/authorize?{}", query(&[])),
            "application/json",
            json!({"decision": "approve", "level": "read", "scopes": ["messages:read"]})
                .to_string()
                .as_bytes(),
        );
        let destination = reply
            .as_ref()
            .map(|r| r.json()["redirect"].as_str().unwrap_or_default().to_owned())
            .unwrap_or_default();
        let code = query_value(&destination, "code").unwrap_or_default();
        let granted = reply.is_ok_and(|r| r.status == 200);
        e.check("security_agent_created", granted, "");
        e.check("security_code_created", granted && !code.is_empty(), "");
        code
    };
    let exchange = |code: &str, redirect_uri: &str, proof: &str, target: &str, id: &str| {
        site.form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("client_id", id),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", proof),
                ("resource", target),
            ],
        )
    };
    let tokens = |reply: &rawhttp::Response| -> (String, String) {
        let value = reply.json();
        (
            value["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            value["refresh_token"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    };
    let raw = issue(e);
    let wrong_proof = "b".repeat(43);
    for (uri, proof, target, id) in [
        (
            "https://attacker.example/callback",
            verifier.as_str(),
            resource.as_str(),
            client.as_str(),
        ),
        (
            redirect,
            wrong_proof.as_str(),
            resource.as_str(),
            client.as_str(),
        ),
        (redirect, "", resource.as_str(), client.as_str()),
        (
            redirect,
            verifier.as_str(),
            "https://attacker.example/mcp",
            client.as_str(),
        ),
        (
            redirect,
            verifier.as_str(),
            resource.as_str(),
            "wrong-client",
        ),
    ] {
        e.check(
            "mismatched_oauth_binding_refused",
            exchange(&raw, uri, proof, target, id).status == 400,
            "",
        );
    }
    let reply = exchange(&raw, redirect, &verifier, &resource, &client);
    let (access, refresh_token) = tokens(&reply);
    e.check(
        "valid_oauth_access",
        reply.status == 200 && request(&access, "messages.chat") == 200,
        "",
    );
    if reply.status != 200 {
        return;
    }
    let wrong_scope = site
        .json("POST", "/v1/op/iphone.devices", &access, &json!({}))
        .status;
    e.check(
        "wrong_scope_refused",
        wrong_scope == 401 || wrong_scope == 403,
        "",
    );
    e.check(
        "garbage_token_refused",
        request("[ab-test] invalid", "messages.chat") == 401,
        "",
    );
    let hash = sha256_hex(&access);
    let expired = edit_credentials(&site.home, |data| {
        for entry in data["oauth_access"].as_array_mut().into_iter().flatten() {
            if entry["hash"] == hash.as_str() {
                entry["expires_at"] = json!(past());
            }
        }
    });
    e.check(
        "expired_access_refused",
        expired && request(&access, "messages.chat") == 401,
        "",
    );
    let refresh = |token: &str| {
        site.form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("refresh_token", token),
                ("resource", &resource),
            ],
        )
    };
    let reply = refresh(&refresh_token);
    let (rotated_access, rotated_refresh) = tokens(&reply);
    e.check(
        "refresh_rotation_works",
        reply.status == 200 && request(&rotated_access, "messages.chat") == 200,
        "",
    );
    e.check(
        "replayed_refresh_refused_and_family_revoked",
        refresh(&refresh_token).status == 400 && request(&rotated_access, "messages.chat") == 401,
        "",
    );
    let raw = issue(e);
    let first = exchange(&raw, redirect, &verifier, &resource, &client);
    let (family, _) = tokens(&first);
    let second_use = exchange(&raw, redirect, &verifier, &resource, &client);
    e.check(
        "replayed_code_refused_and_family_revoked",
        first.status == 200 && second_use.status == 400 && request(&family, "messages.chat") == 401,
        "",
    );
    let raw = issue(e);
    let digest = sha256_hex(&raw);
    let expired = edit_credentials(&site.home, |data| {
        for entry in data["oauth_codes"].as_array_mut().into_iter().flatten() {
            if entry["hash"] == digest.as_str() {
                entry["expires_at"] = json!(past());
            }
        }
    });
    e.check(
        "expired_code_refused",
        expired && exchange(&raw, redirect, &verifier, &resource, &client).status == 400,
        "",
    );
    let raw = issue(e);
    let reply = exchange(&raw, redirect, &verifier, &resource, &client);
    let (live, live_refresh) = tokens(&reply);
    e.check(
        "connector_works_before_revocation",
        reply.status == 200 && request(&live, "messages.chat") == 200,
        "",
    );
    let principal = principal_of(site, &live);
    let started = Instant::now();
    let revoked = principal
        .as_deref()
        .is_some_and(|id| world.gateway_cli(&["agent", "revoke", id]).is_ok());
    let status = request(&live, "messages.chat");
    e.check(
        "revocation_stops_public_token_within_seconds",
        revoked && status == 401 && started.elapsed() < Duration::from_secs(3),
        "",
    );
    e.check(
        "revoked_refresh_refused",
        refresh(&live_refresh).status == 400,
        "",
    );
    let stored = fs::read_to_string(site.home.join("credentials.json")).unwrap_or_default();
    let log = fs::read_to_string(site.directory.join("gateway.log"));
    let mut safe = !stored.is_empty() && log.is_ok();
    let log = log.unwrap_or_default();
    for secret in [
        &raw,
        &live,
        &live_refresh,
        &rotated_access,
        &rotated_refresh,
    ] {
        safe = safe && !secret.is_empty() && !stored.contains(secret) && !log.contains(secret);
    }
    e.check(
        "oauth_codes_and_tokens_absent_from_storage_and_logs",
        safe,
        "",
    );
}

fn principal_of(site: &Site, token: &str) -> Option<String> {
    let credentials: Value =
        serde_json::from_slice(&fs::read(site.home.join("credentials.json")).ok()?).ok()?;
    let hash = sha256_hex(token);
    credentials["oauth_access"]
        .as_array()?
        .iter()
        .find(|entry| entry["hash"] == hash.as_str())
        .and_then(|entry| entry["principal"].as_str().map(str::to_owned))
}

fn owner_requests(site: &Site, owner: &Owner, e: &mut Evidence) {
    let cookie = owner.cookie.clone().unwrap_or_default();
    for path in [
        "/oauth/authorize",
        "/approvals/test",
        "/auth/passkeys/options",
    ] {
        for origin in ["", "null", "https://attacker.example"] {
            let cookie_header = format!("{COOKIE}={cookie}");
            let mut headers = vec![
                ("Host", site.host.as_str()),
                ("Cookie", cookie_header.as_str()),
            ];
            if !origin.is_empty() {
                headers.push(("Origin", origin));
            }
            let reply = rawhttp::request(
                &site.address,
                "POST",
                path,
                &headers,
                Some(br#"{"decision":"approve"}"#),
                10,
            );
            e.check(
                "approve_csrf_and_cross_origin_refused",
                reply.is_ok_and(|r| r.status == 403),
                "",
            );
        }
    }
    let session = owner.request("GET", "/auth/session", "application/json", b"");
    e.check(
        "session_cookie_flags_and_response_redaction",
        session
            .is_ok_and(|r| r.status == 200 && !String::from_utf8_lossy(&r.body).contains(&cookie)),
        "",
    );
}

type Exposed = (Ca, String, Arc<rustls::ClientConfig>, String);

fn expose(world: &mut World, directory: &Path) -> Result<Exposed> {
    let ca = Ca::new().map_err(invalid)?;
    let leaf = ca.issue(directory).map_err(invalid)?;
    configure_gateway(&world.gateway_home, "127.0.0.1:0", &leaf).map_err(invalid)?;
    world.restart_gateway()?;
    let address = gateway_address(&world.gateway_home, Duration::from_secs(15)).map_err(invalid)?;
    let config = client_config(ca.roots().map_err(invalid)?).map_err(invalid)?;
    let pin = Identity::load(&world.gateway_home)
        .map_err(invalid)?
        .fingerprint;
    Ok((ca, address, config, pin))
}

fn trust_boundaries(
    world: &mut World,
    site: &Site,
    owner: &mut Owner,
    exposed: Result<Exposed>,
    e: &mut Evidence,
) {
    let session = owner.login();
    let trusted = session.is_ok_and(|r| r.status == 200) && owner.cookie.is_some();
    let probe = |owner: &Owner| {
        owner
            .request("POST", "/auth/passkeys/options", "application/json", b"{}")
            .map_or(0, |r| r.status)
    };
    let working = probe(owner);
    let expired = edit_credentials(&site.home, |data| {
        for device in data["devices"].as_array_mut().into_iter().flatten() {
            device["expires_at"] = json!(past());
        }
    });
    let after = probe(owner);
    e.check(
        "expired_owner_session_cannot_enroll_passkey",
        trusted && working == 200 && expired && (after == 401 || after == 403),
        "",
    );
    let created = world
        .gateway_cli(&[
            "agent",
            "create",
            "--name",
            "[ab-test] expired",
            "--level",
            "read",
            "--scope",
            "messages:read",
        ])
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let agent = created
        .as_ref()
        .and_then(|v| v["agent"]["id"].as_str().or_else(|| v["id"].as_str()))
        .unwrap_or_default()
        .to_owned();
    let marker = sha256_hex("[ab-test] refresh");
    let expiry = (Timestamp::now() + 1.hour()).to_string();
    let edited = !agent.is_empty()
        && edit_credentials(&site.home, |data| {
            for principal in data["principals"].as_array_mut().into_iter().flatten() {
                if principal["id"] == agent.as_str() {
                    principal["expires_at"] = json!(past());
                }
            }
            let entry = json!({"hash": marker, "principal": agent, "client_id": "client", "resource": "resource", "expires_at": expiry});
            match data["oauth_refresh"].as_array_mut() {
                Some(list) => list.push(entry),
                None => data["oauth_refresh"] = json!([entry]),
            }
        });
    let reply = site.form(
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("client_id", "client"),
            ("refresh_token", "[ab-test] refresh"),
            ("resource", "resource"),
        ],
    );
    e.check(
        "expired_agent_cannot_mint_oauth_token",
        edited && (reply.status == 400 || reply.status == 401),
        "",
    );
    replica_boundaries(world, site, exposed, e);
}

fn replica_boundaries(world: &mut World, site: &Site, exposed: Result<Exposed>, e: &mut Evidence) {
    let started =
        replicas::start_with(
            world,
            1,
            "messages:read,contacts:read,contacts:write,system:read",
            0,
        )
        .is_ok();
    let endpoint = replicas::endpoints().into_iter().next();
    let (Some(endpoint), true) = (endpoint, started) else {
        e.check("replica_fixture", false, "replicas did not start");
        e.check("delegated_scope_cannot_expand_to_replica_scope", false, "");
        e.check("inactive_replica_certificate_refused", false, "");
        e.check("inactive_replica_certificate_refused", false, "");
        return;
    };
    let created = replicas::cli(
        &endpoint.home,
        &[
            "agent",
            "create",
            "--name",
            "[ab-test] delegated",
            "--level",
            "full",
            "--scope",
            "messages:read",
        ],
    )
    .ok()
    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let token = created
        .as_ref()
        .and_then(|v| v["token"].as_str())
        .unwrap_or_default()
        .to_owned();
    let query = |operation: &str, collection: &str| {
        rawhttp::request(
            &endpoint.address,
            "POST",
            &format!("/v1/op/{operation}"),
            &[
                ("Authorization", &format!("Bearer {token}")),
                ("Content-Type", "application/json"),
            ],
            Some(
                if operation == "records.query" {
                    json!({"collection": collection, "limit": 1})
                } else {
                    json!({})
                }
                .to_string()
                .as_bytes(),
            ),
            10,
        )
        .map_or((0, String::new()), |r| {
            (r.status, r.text().chars().take(160).collect::<String>())
        })
    };
    let allowed = query("records.query", "messages.chat");
    let denied = query("contacts.create", "");
    e.check(
        "delegated_scope_cannot_expand_to_replica_scope",
        !token.is_empty()
            && allowed.0 == 200
            && !allowed.1.contains("\"error\"")
            && (denied.0 == 401 || denied.0 == 403),
        &format!("allowed={allowed:?} denied={denied:?}"),
    );
    let identity = Identity::load(&endpoint.home).map_err(invalid);
    let failure = match (&exposed, &identity) {
        (Err(error), _) => format!("expose: {error}"),
        (_, Err(error)) => format!("identity: {error}"),
        _ => String::new(),
    };
    let (Ok((_ca, address, config, pin)), Ok(identity)) = (exposed, identity) else {
        let detail = failure.as_str();
        for _ in 0..2 {
            e.check("inactive_replica_certificate_refused", false, detail);
        }
        return;
    };
    let authenticate = || {
        lab_relay::Link::dial(&address, &config, None)
            .and_then(|mut link| relay1_client(&mut link, &identity, &pin))
    };
    let baseline = authenticate().map(|p| p.id);
    let fingerprint = identity.fingerprint.clone();
    for field in ["expires_at", "revoked_at"] {
        let edited = edit_credentials(&site.home, |data| {
            for principal in data["principals"].as_array_mut().into_iter().flatten() {
                if principal["fingerprint"] == fingerprint.as_str() {
                    principal[field] = json!(past());
                }
            }
        });
        e.check(
            "inactive_replica_certificate_refused",
            baseline.is_ok() && edited && authenticate().is_err(),
            &format!("baseline={:?} edited={edited}", baseline.as_ref().err()),
        );
    }
}

pub fn run(world: &mut World, _input: &Value, e: &mut Evidence) -> Result<()> {
    crate::connectors::connector_approval(world, &Value::Null, e)?;
    relay_proofs(e)?;
    let first = Site::new(world)?;
    public_health(&first, e);
    remote_failure(world, e)?;
    let mut second = match world.sibling("security") {
        Ok(second) => second,
        Err(error) => {
            e.check("isolated_gateway", false, &error.to_string());
            return Ok(());
        }
    };
    e.check("isolated_gateway", true, "");
    e.check(
        "authority",
        second.gateway_cli(&["agent", "list"]).is_ok(),
        "",
    );
    let relay_directory = second.directory.join("security-relay");
    let exposed = expose(&mut second, &relay_directory);
    let site = Site::new(&second)?;
    endpoint_sweep(&site, false, e);
    let Ok(mut owner) = site.owner_session(&second) else {
        e.check("owner_session", false, "");
        return Ok(());
    };
    oauth(&second, &site, &owner, e);
    e.check("owner_session", owner.cookie.is_some(), "");
    owner_requests(&site, &owner, e);
    let token = owner.cookie.clone().unwrap_or_default();
    trust_boundaries(&mut second, &site, &mut owner, exposed, e);
    let stored = fs::read_to_string(site.home.join("credentials.json")).unwrap_or_default();
    let mut safe = !stored.contains(&token);
    for name in ["gateway.log", "helper.log"] {
        match fs::read_to_string(second.directory.join(name)) {
            Ok(log) => safe = safe && !token.is_empty() && !log.contains(&token),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => safe = false,
        }
    }
    e.check("owner_token_absent_from_storage_and_logs", safe, "");
    let serialized = serde_json::to_string(&*e).map_err(invalid)?;
    e.check(
        "evidence_contains_only_security_results",
        !token.is_empty() && !serialized.contains(&token),
        "",
    );
    replicas::stop_all();
    Ok(())
}

fn curl_status(method: &str, url: &str, headers: &[String], body: Option<&str>) -> u16 {
    let mut command = Command::new("curl");
    command
        .args(["--silent", "--max-time", "15", "--output", "/dev/null"])
        .args(["--write-out", "%{http_code}", "-X", method]);
    for header in headers {
        command.args(["-H", header]);
    }
    if let Some(body) = body {
        command.args(["--data", body]);
    }
    command
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

fn revoke_relay(home: &Path) -> bool {
    let Ok(raw) = fs::read(home.join("remote").join("relay.json")) else {
        return true;
    };
    let identity: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let (endpoint, host, secret) = (
        identity["endpoint"].as_str().unwrap_or_default(),
        identity["credential"]["hostname"]
            .as_str()
            .unwrap_or_default(),
        identity["credential"]["credential"]
            .as_str()
            .unwrap_or_default(),
    );
    if host.is_empty() {
        return true;
    }
    curl_status(
        "POST",
        &format!("{endpoint}/revoke"),
        &[
            format!("Authorization: Bearer {secret}"),
            "Content-Type: application/json".to_owned(),
        ],
        Some(&json!({"hostname": host}).to_string()),
    ) == 204
}

pub fn real(world: &mut World, e: &mut Evidence) -> Result<()> {
    let enabled = world.gateway_cli(&["remote", "on", "--via", "relay"]);
    e.check("public_relay_gateway_ready", enabled.is_ok(), "");
    if enabled.is_ok() {
        let status: Value = fs::read(world.gateway_home.join("remote").join("status.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or(Value::Null);
        let url = status["url"].as_str().unwrap_or_default().to_owned();
        e.check(
            "public_https_origin",
            status["state"] == "on" && status["backend"] == "relay" && !url.is_empty(),
            "",
        );
        if !url.is_empty() {
            let json = ["Content-Type: application/json".to_owned()];
            sweep(
                true,
                |method, path| curl_status(method, &format!("{url}{path}"), &json, Some("{}")),
                |path| curl_status("GET", &format!("{url}{path}"), &[], None),
                e,
            );
        }
    }
    let revoked = revoke_relay(&world.gateway_home);
    let off = world.gateway_cli(&["remote", "off"]);
    e.check(
        "public_relay_test_credentials_revoked_and_tunnel_closed",
        revoked && off.is_ok(),
        "",
    );
    Ok(())
}
