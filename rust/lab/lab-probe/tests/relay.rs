#![allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cloned_ref_to_slice_refs
)]
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_probe::{Route, Session, run_with_roots, serve_proxy};
use lab_relay::{
    Ca, Frame, HOST, Identity, fingerprint, gateway_key, input1, random_nonce, verify,
};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use serde_json::{Value, json, value::RawValue};
use std::{
    io::{Cursor, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

fn pem(label: &str, der: &[u8]) -> String {
    let body = STANDARD.encode(der);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        lines.join("\n")
    )
}
fn write_identity(home: &Path, identity: &Identity) {
    use sha2::{Digest, Sha256};
    std::fs::create_dir_all(home.join("secrets")).unwrap();
    std::fs::write(
        home.join("identity.pem"),
        pem("CERTIFICATE", &identity.certificate),
    )
    .unwrap();
    let name = hex::encode(Sha256::digest(b"identity-key"));
    std::fs::write(
        home.join("secrets").join(name),
        pem("PRIVATE KEY", &identity.pkcs8().unwrap()),
    )
    .unwrap();
}

struct Gateway {
    port: u16,
    sessions: Arc<Mutex<Vec<String>>>,
    revoke: mpsc::Sender<()>,
    feed_acks: Arc<Mutex<Vec<Value>>>,
}
fn authenticated(
    socket: &mut tungstenite::WebSocket<StreamOwned<ServerConnection, std::net::TcpStream>>,
    gateway: &Identity,
    replica_fingerprint: &str,
    seen_nonces: &Mutex<Vec<String>>,
) -> bool {
    let read = |socket: &mut tungstenite::WebSocket<_>| -> Option<Frame> {
        match socket.read().ok()? {
            tungstenite::Message::Text(text) => Frame::parse(text.as_str()).ok(),
            _ => None,
        }
    };
    let Some(hello) = read(socket) else {
        return false;
    };
    if hello.t != "AuthHello" || !hello.version.is_empty() {
        return false;
    }
    seen_nonces.lock().unwrap().push(hello.nonce.clone());
    let mut challenge = Frame {
        t: "AuthChallenge".into(),
        nonce: random_nonce(),
        certificate: STANDARD.encode(&gateway.certificate),
        ..Frame::default()
    };
    let signed = input1("gateway", &hello, &challenge, None).unwrap();
    challenge.signature = STANDARD.encode(gateway.sign(&signed));
    if socket
        .send(tungstenite::Message::text(challenge.text().unwrap()))
        .is_err()
    {
        return false;
    }
    let Some(proof) = read(socket) else {
        return false;
    };
    let Ok((der, key)) = gateway_key(&hello) else {
        return false;
    };
    if proof.t != "AuthProof"
        || fingerprint(&der) != replica_fingerprint
        || !verify(
            &key,
            &proof.signature_bytes().unwrap(),
            &input1("replica", &hello, &challenge, None).unwrap(),
        )
    {
        return false;
    }
    let principal = json!({"id": "replica:one", "kind": "replica", "fingerprint": replica_fingerprint, "grants": ["reminders:write"], "revision": 1}).to_string();
    let accepted_input = input1("accepted", &hello, &challenge, Some(&principal)).unwrap();
    let accepted = Frame {
        t: "Authenticated".into(),
        signature: STANDARD.encode(gateway.sign(&accepted_input)),
        principal: Some(RawValue::from_string(principal).unwrap()),
        ..Frame::default()
    };
    socket
        .send(tungstenite::Message::text(accepted.text().unwrap()))
        .is_ok()
}
fn start_gateway(ca: &Ca, gateway: Identity, replica_fingerprint: String) -> Gateway {
    let (chain_pem, key_pem) = ca.leaf().unwrap();
    let chain = CertificateDer::pem_slice_iter(chain_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).unwrap();
    let config = Arc::new(
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let sessions = Arc::new(Mutex::new(Vec::new()));
    let feed_acks = Arc::new(Mutex::new(Vec::new()));
    let (revoke, revoked) = mpsc::channel();
    let revoked = Arc::new(Mutex::new(revoked));
    let (nonces, acks, gateway) = (sessions.clone(), feed_acks.clone(), Arc::new(gateway));
    thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (config, nonces, acks, revoked, gateway, replica) = (
                config.clone(),
                nonces.clone(),
                acks.clone(),
                revoked.clone(),
                gateway.clone(),
                replica_fingerprint.clone(),
            );
            thread::spawn(move || {
                tcp.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let stream = StreamOwned::new(ServerConnection::new(config).unwrap(), tcp);
                let Ok(mut socket) = tungstenite::accept(stream) else {
                    return;
                };
                if !authenticated(&mut socket, &gateway, &replica, &nonces) {
                    let _ = socket.close(None);
                    return;
                }
                let Ok(tungstenite::Message::Text(text)) = socket.read() else {
                    return;
                };
                let hello: Value = serde_json::from_str(text.as_str()).unwrap();
                if hello["t"] != "Hello" {
                    return;
                }
                let welcome = json!({"t": "Welcome", "protocol": "feed/1"});
                socket
                    .send(tungstenite::Message::text(welcome.to_string()))
                    .unwrap();
                let batch = json!({"t": "Batch", "batch": {"epoch": "epoch-1", "subscription_revision": 1, "after": 0, "through": 4, "changes": []}});
                socket
                    .send(tungstenite::Message::text(batch.to_string()))
                    .unwrap();
                if let Ok(tungstenite::Message::Text(ack)) = socket.read() {
                    acks.lock()
                        .unwrap()
                        .push(serde_json::from_str(ack.as_str()).unwrap());
                }
                let _ = revoked
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(20));
                let _ = socket.close(None);
                let _ = socket.flush();
            });
        }
    });
    Gateway {
        port,
        sessions,
        revoke,
        feed_acks,
    }
}

struct Rig {
    home: PathBuf,
    input: Value,
    gateway: Gateway,
    roots: rustls::RootCertStore,
    stop: Arc<AtomicBool>,
}
fn rig(name: &str) -> Rig {
    let home = std::env::temp_dir().join(format!("lab-probe-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let ca = Ca::new().unwrap();
    let (gateway_identity, replica) =
        (Identity::generate().unwrap(), Identity::generate().unwrap());
    write_identity(&home, &replica);
    let gateway_fingerprint = gateway_identity.fingerprint.clone();
    let gateway = start_gateway(&ca, gateway_identity, replica.fingerprint.clone());
    std::fs::write(
        home.join("credentials.json"),
        json!({"enrollment": {"remote": true, "address": format!("https://{HOST}:{}", gateway.port), "fingerprint": gateway_fingerprint, "replica_id": "replica:one"}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        home.join("gateway-clock.json"),
        json!({"t0": "2026-01-01T00:00:00.000Z", "t1": "2026-01-01T00:00:00.001Z", "t2": "2026-01-01T00:00:00.002Z", "t3": "2026-01-01T00:00:00.003Z"}).to_string(),
    )
    .unwrap();
    let ready = home.join("ready.json");
    let route = Route::parse(
        &format!("https://{HOST}:{}", gateway.port),
        "127.0.0.1",
        true,
    )
    .unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let (flag, ready_path) = (stop.clone(), ready.clone());
    thread::spawn(move || serve_proxy(&proxy, &route, &ready_path, &flag));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.is_file() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let local = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = local.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for mut stream in local.incoming().flatten() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                if std::io::BufRead::read_line(&mut reader, &mut line).unwrap_or(0) == 0
                    || line == "\r\n"
                {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = std::io::Read::read_exact(&mut reader, &mut body);
            let body = json!({"records": [], "cursor": {"epoch": "epoch-1", "subscription_revision": 1, "sequence": 4}}).to_string();
            let _ = write!(
                stream,
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let input = json!({"URL": format!("http://{address}"), "Host": address, "Token": "t", "Home": home.to_string_lossy(), "ProxyReady": ready.to_string_lossy()});
    Rig {
        home,
        input,
        gateway,
        roots: ca.roots().unwrap(),
        stop,
    }
}
fn lines(values: &[Value]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| format!("{v}\n").into_bytes())
        .collect()
}

#[test]
fn replay_probe_authenticates_once_then_sees_the_replayed_proof_rejected() {
    let rig = rig("replay");
    let mut session = Session::new(Cursor::new(lines(&[rig.input.clone()])), Vec::new());
    run_with_roots("replay", &mut session, &rig.roots).unwrap();
    let output: Value = serde_json::from_slice(&session.into_output()).unwrap();
    assert_eq!(
        output,
        json!({"capture_authenticated": true, "fresh_nonce": true, "replay_rejected": true})
    );
    let nonces = rig.gateway.sessions.lock().unwrap().clone();
    assert_eq!(nonces.len(), 2);
    assert_eq!(nonces[0], nonces[1]);
    rig.stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&rig.home);
}

#[test]
fn replay_probe_refuses_a_gateway_with_the_wrong_pin() {
    let rig = rig("pin");
    let mut credentials: Value =
        serde_json::from_slice(&std::fs::read(rig.home.join("credentials.json")).unwrap()).unwrap();
    credentials["enrollment"]["fingerprint"] = json!("0".repeat(64));
    std::fs::write(rig.home.join("credentials.json"), credentials.to_string()).unwrap();
    let mut session = Session::new(Cursor::new(lines(&[rig.input.clone()])), Vec::new());
    let error = run_with_roots("replay", &mut session, &rig.roots).unwrap_err();
    assert_eq!(error.to_string(), "gateway identity rejected");
    rig.stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&rig.home);
}

struct Staged {
    first: Cursor<Vec<u8>>,
    later: Cursor<Vec<u8>>,
    gate: mpsc::Receiver<()>,
    waited: bool,
}
impl std::io::Read for Staged {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.first.read(buffer)?;
        if count > 0 {
            return Ok(count);
        }
        if !self.waited {
            self.waited = true;
            let _ = self.gate.recv_timeout(Duration::from_secs(20));
        }
        self.later.read(buffer)
    }
}

#[test]
fn revocation_probe_acks_the_feed_and_reports_when_the_gateway_closes_it() {
    let rig = rig("revocation");
    let (open, gate) = mpsc::channel();
    let staged = Staged {
        first: Cursor::new(lines(&[rig.input.clone()])),
        later: Cursor::new(lines(&[json!({"revoked": true})])),
        gate,
        waited: false,
    };
    let roots = rig.roots.clone();
    let worker = thread::spawn(move || {
        let mut session = Session::new(staged, Vec::new());
        let result = run_with_roots("revocation", &mut session, &roots).map_err(|e| e.to_string());
        (result, session.into_output())
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while rig.gateway.feed_acks.lock().unwrap().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let _ = open.send(());
    thread::sleep(Duration::from_millis(100));
    let _ = rig.gateway.revoke.send(());
    let (result, bytes) = worker.join().unwrap();
    result.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let outputs: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(outputs[0], json!({"ready": true}));
    assert!(outputs[1]["closed_gateway_wall_ns"].as_i64().unwrap() > 0);
    assert!((outputs[1]["clock_uncertainty_ms"].as_f64().unwrap() - 1.0).abs() < 1e-6);
    let acks = rig.gateway.feed_acks.lock().unwrap().clone();
    assert_eq!(acks[0]["t"], "Ack");
    assert_eq!(acks[0]["cursor"]["sequence"], 4);
    rig.stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&rig.home);
}
