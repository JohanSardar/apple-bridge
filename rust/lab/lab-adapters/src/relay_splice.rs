#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    journey::{gateway_client, records},
    relay::{ADMIN, Fixture, Lab, Limits, Tunnel, Wire, cred, enroll, fixture_binary, signing_key},
    replicas,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_relay::{
    Ca, Frame, Identity, Link, Principal, client_config, configure_gateway, gateway_address,
    relay1_client, relay2_client,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rustls::{
    ClientConfig, DigitallySignedStruct, RootCertStore, ServerConfig, ServerConnection,
    SignatureScheme, StreamOwned,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::ring::default_provider,
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tungstenite::{Message, WebSocket};

pub const SCENARIOS: [&str; 3] = ["RELAY-TLS-SPLICE", "RELAY-DOWNGRADE", "RELAY-LEGACY-COMPAT"];
const MARKER: &str = "synthetic-splice-feed-marker";
const SCOPE: &str = "reminders:read,reminders:write,system:read";
const COMPAT_HOST: &str = "gateway.compat.test";
const POLL: Duration = Duration::from_millis(50);
const FRAME_NAMES: [&str; 4] = ["AuthHello", "AuthChallenge", "AuthProof", "Authenticated"];

fn invalid<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}

type Socket = WebSocket<StreamOwned<ServerConnection, TcpStream>>;

struct Server {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn server_config(chain: &str, key: &str) -> Result<Arc<ServerConfig>> {
    let chain = CertificateDer::pem_slice_iter(chain.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(invalid)?;
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).map_err(invalid)?;
    Ok(Arc::new(
        ServerConfig::builder_with_provider(Arc::new(default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(invalid)?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(invalid)?,
    ))
}

fn serve(
    config: Arc<ServerConfig>,
    handler: impl Fn(Socket) + Send + Sync + 'static,
) -> Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handler = Arc::new(handler);
    let thread = thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((tcp, _)) => {
                    let (config, handler) = (config.clone(), handler.clone());
                    thread::spawn(move || {
                        let _ = tcp.set_nonblocking(false);
                        let _ = tcp.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = tcp.set_write_timeout(Some(Duration::from_secs(5)));
                        let Ok(connection) = ServerConnection::new(config) else {
                            return;
                        };
                        if let Ok(socket) = tungstenite::accept(StreamOwned::new(connection, tcp)) {
                            handler(socket);
                        }
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
    });
    Ok(Server {
        address,
        stop,
        thread: Some(thread),
    })
}

fn texts(message: &Message) -> Option<String> {
    match message {
        Message::Text(text) => Some(text.as_str().to_owned()),
        _ => None,
    }
}

#[derive(Debug)]
struct Pinned {
    fingerprint: String,
}
impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if hex::encode(Sha256::digest(end_entity.as_ref())) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("pin mismatch".into()))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn pinned_config(fingerprint: &str) -> Result<Arc<ClientConfig>> {
    Ok(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(invalid)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pinned {
                fingerprint: fingerprint.to_owned(),
            }))
            .with_no_client_auth(),
    ))
}

fn tls_binding_rejected() -> bool {
    let Ok(name) = ServerName::try_from("synthetic.relay.test".to_owned()) else {
        return false;
    };
    let Ok(config) = client_config(RootCertStore::empty()) else {
        return false;
    };
    let Ok(connection) = rustls::ClientConnection::new(config, name) else {
        return false;
    };
    let mut binding = [0u8; 32];
    connection
        .export_keying_material(&mut binding, b"EXPORTER-Channel-Binding", Some(&[]))
        .is_err()
}

struct Rig {
    ca: Ca,
    gateway_certificate: Vec<u8>,
    config: Arc<ClientConfig>,
    address: String,
    pin: String,
}

fn expose(world: &mut World, name: &str, directory: &std::path::Path) -> Result<Rig> {
    let ca = Ca::new().map_err(invalid)?;
    let leaf = ca.issue_for(directory, name).map_err(invalid)?;
    configure_gateway(&world.gateway_home, "127.0.0.1:0", &leaf).map_err(invalid)?;
    world.restart_gateway()?;
    let address = gateway_address(&world.gateway_home, Duration::from_secs(15)).map_err(invalid)?;
    let config = client_config(ca.roots().map_err(invalid)?).map_err(invalid)?;
    let gateway = Identity::load(&world.gateway_home).map_err(invalid)?;
    Ok(Rig {
        ca,
        gateway_certificate: gateway.certificate,
        config,
        address,
        pin: gateway.fingerprint,
    })
}

fn replica(world: &World, index: usize) -> Result<Identity> {
    Identity::load(&world.directory.join(format!("replica{index}"))).map_err(invalid)
}

fn feed_marker(link: &mut Link, principal: &Principal) -> bool {
    let hello = json!({
        "t": "Hello",
        "protocol": "feed/1",
        "replica_id": principal.id,
        "schema_offer": [1],
        "credits": 4 * 1024 * 1024,
    });
    if link.send_text(&hello.to_string()).is_err() {
        return false;
    }
    for _ in 0..8 {
        let Ok(text) = link.recv_text(Duration::from_secs(5)) else {
            return false;
        };
        if !text.contains(r#""t":"Heartbeat""#) {
            let ack = json!({"t": "Ack", "credits": text.len()});
            if link.send_text(&ack.to_string()).is_err() {
                return false;
            }
        }
        if text.contains(MARKER) {
            return true;
        }
    }
    false
}

fn downgrade(rig: &Rig, host: &str, identity: &Identity, mutation: &str) -> Result<bool> {
    let (chain, key) = rig.ca.leaf_for(host).map_err(invalid)?;
    let certificate = STANDARD.encode(&rig.gateway_certificate);
    let mutation = mutation.to_owned();
    let server = serve(server_config(&chain, &key)?, move |mut socket| {
        if socket.read().is_err() {
            return;
        }
        let mut challenge = Frame {
            t: "AuthChallenge".into(),
            version: "relay/2".into(),
            capabilities: "tls-exporter,feed/1".into(),
            nonce: STANDARD.encode([0u8; 32]),
            certificate: certificate.clone(),
            ..Frame::default()
        };
        if mutation == "legacy" {
            "relay/1".clone_into(&mut challenge.version);
        } else {
            "feed/1".clone_into(&mut challenge.capabilities);
        }
        if let Ok(text) = challenge.text() {
            let _ = socket.send(Message::text(text));
        }
        let _ = socket.read();
    })?;
    let outcome = Link::dial_as(&server.address, &rig.config, host, None)
        .and_then(|mut link| relay2_client(&mut link, identity, &rig.pin));
    Ok(outcome.is_err())
}

struct Splice {
    server: Server,
    frames: Arc<AtomicUsize>,
    observed: Arc<AtomicBool>,
}

fn splicer(rig: &Rig, relay: &str, host: &str) -> Result<Splice> {
    let (chain, key) = rig.ca.leaf_for(host).map_err(invalid)?;
    let frames = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(AtomicBool::new(false));
    let (counted, seen) = (frames.clone(), observed.clone());
    let (upstream_config, relay, host) = (rig.config.clone(), relay.to_owned(), host.to_owned());
    let server = serve(server_config(&chain, &key)?, move |mut downstream| {
        let Ok(mut upstream) = Link::dial_as(&relay, &upstream_config, &host, None) else {
            return;
        };
        let inspect = |text: &str| {
            for name in FRAME_NAMES {
                if text.contains(&format!(r#""t":"{name}""#)) {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            }
            if text.contains(MARKER) {
                seen.store(true, Ordering::SeqCst);
            }
        };
        let _ = downstream.get_mut().sock.set_read_timeout(Some(POLL));
        for _ in 0..200 {
            match downstream.read() {
                Ok(message) => {
                    if let Some(text) = texts(&message) {
                        inspect(&text);
                        if upstream.send_text(&text).is_err() {
                            return;
                        }
                    }
                }
                Err(tungstenite::Error::Io(io))
                    if matches!(
                        io.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return,
            }
            match upstream.try_recv_text(POLL) {
                Ok(Some(text)) => {
                    inspect(&text);
                    if downstream.send(Message::text(text)).is_err() {
                        return;
                    }
                }
                Ok(None) => {}
                Err(_) => return,
            }
        }
    })?;
    Ok(Splice {
        server,
        frames,
        observed,
    })
}

fn splice(world: &mut World, e: &mut Evidence) -> Result<()> {
    e.check(
        "missing_and_synthetic_tls_rejected",
        tls_binding_rejected(),
        "",
    );
    let directory = world.directory.join("relay-splice");
    std::fs::create_dir_all(&directory)?;
    let binary = fixture_binary()?;
    let limits = Limits {
        connections: 32,
        bytes_per_second: 1_048_576,
        monthly_bytes: 10_737_418_240,
    };
    let store = Arc::new(Fixture::open(
        &binary,
        &directory.join("relay-state.json"),
        limits,
    )?);
    let lab = Lab::start_with(&store, ADMIN)?;
    let key = signing_key()?;
    let (credential, status) = enroll(&lab.api, &key, "");
    e.check("splice_origin_enrolled", status == 200, "");
    if status != 200 {
        return Ok(());
    }
    let host = cred(&credential, "hostname");
    e.check(
        "new_hostname_has_128_bits",
        host.split('.')
            .next()
            .is_some_and(|label| label.len() == 32),
        "",
    );
    let rig = expose(world, &host, &directory.join("origin"))?;
    let paired = replicas::start_with(world, 1, SCOPE, 0).is_ok();
    let identity = replica(world, 0);
    e.check(
        "genuine_paired_identities",
        paired && identity.is_ok(),
        "paired replica identity",
    );
    let Ok(identity) = identity else {
        return Ok(());
    };
    let created = world
        .control(
            "/external-change",
            &json!({"key": "splice", "data": {"title": MARKER}}),
        )
        .is_ok();
    let client = gateway_client(world)?;
    let seeded = created
        && await_until(10, || {
            records(&client)
                .is_ok_and(|(_, all)| all.iter().any(|record| record["data"]["title"] == MARKER))
        });
    e.check("seeded_feed", seeded, "");
    e.check("rust_relay2_origin_started", true, "");
    e.check("rust_relay2_origin_ready", !rig.address.is_empty(), "");
    let direct = Link::dial_as(&rig.address, &rig.config, &host, None).and_then(|mut link| {
        relay2_client(&mut link, &identity, &rig.pin).map(|principal| (link, principal))
    });
    let direct_ok = direct.is_ok();
    let found = direct.is_ok_and(|(mut link, principal)| feed_marker(&mut link, &principal));
    e.check("bound_direct_feed_marker", found, "");
    e.check("bound_direct_authentication", direct_ok, "");
    for mutation in ["legacy", "capabilities"] {
        let rejected = downgrade(&rig, &host, &identity, mutation)?;
        e.check(&format!("requires_relay2_rejects_{mutation}"), rejected, "");
        e.check(
            &format!("rust_requires_relay2_rejects_{mutation}"),
            rejected,
            "",
        );
    }
    let again = Link::dial_as(&rig.address, &rig.config, &host, None)
        .and_then(|mut link| relay2_client(&mut link, &identity, &rig.pin));
    e.check("rust_client_bound_authentication", again.is_ok(), "");
    let attached = lab
        .attach_kept(
            &host,
            &cred(&credential, "credential"),
            Tunnel::Dial(rig.address.clone()),
            &Wire::new(&[]),
        )
        .is_ok();
    e.check("splice_origin_tunnel_attached", attached, "");
    if !attached {
        return Ok(());
    }
    let spliced = splicer(&rig, &lab.address, &host)?;
    let first = Link::dial_as(&spliced.server.address, &rig.config, &host, None)
        .and_then(|mut link| relay2_client(&mut link, &identity, &rig.pin));
    e.check("rust_client_rejects_tls_splice", first.is_err(), "");
    spliced.frames.store(0, Ordering::SeqCst);
    let trusted = Link::dial_as(&spliced.server.address, &rig.config, &host, None);
    e.check("two_trusted_tls_legs", trusted.is_ok(), "");
    let Ok(mut trusted) = trusted else {
        return Ok(());
    };
    let outcome = relay2_client(&mut trusted, &identity, &rig.pin);
    e.check("splice_rejected_before_feed", outcome.is_err(), "");
    e.check(
        "no_application_data_observed",
        !spliced.observed.load(Ordering::SeqCst),
        "",
    );
    thread::sleep(Duration::from_millis(100));
    e.check(
        "only_hello_and_challenge_forwarded",
        spliced.frames.load(Ordering::SeqCst) == 2,
        "",
    );
    drop(trusted);
    let wrong = Link::dial_as(
        &spliced.server.address,
        &rig.config,
        "wrong.relay.test",
        None,
    );
    e.check("wrong_name_rejects_splice", wrong.is_err(), "");
    let untrusted = client_config(RootCertStore::empty()).map_err(invalid)?;
    let refused = Link::dial_as(&spliced.server.address, &untrusted, &host, None);
    e.check("untrusted_certificate_rejects_splice", refused.is_err(), "");
    let pinned = pinned_config(&rig.pin)?;
    let mismatch = Link::dial_as(&spliced.server.address, &pinned, &host, None);
    e.check("direct_pinned_mtls_rejects_splice", mismatch.is_err(), "");
    Ok(())
}

fn authenticates(
    rig: &Rig,
    address: &str,
    identity: &Identity,
    relay2: bool,
) -> lab_relay::Result<Principal> {
    let mut link = Link::dial_as(address, &rig.config, COMPAT_HOST, None)?;
    if relay2 {
        relay2_client(&mut link, identity, &rig.pin)
    } else {
        relay1_client(&mut link, identity, &rig.pin)
    }
}

fn shaped(principal: &lab_relay::Result<Principal>) -> Option<bool> {
    principal
        .as_ref()
        .ok()
        .filter(|p| p.replica_shaped())
        .map(|p| p.relay2)
}

fn compat(world: &mut World, e: &mut Evidence) -> Result<()> {
    let directory = world.directory.join("relay-compat");
    std::fs::create_dir_all(&directory)?;
    let rig = expose(world, COMPAT_HOST, &directory.join("origin"))?;
    let paired = replicas::start_with(world, 2, SCOPE, 0).is_ok();
    let (legacy, upgraded) = (replica(world, 0), replica(world, 1));
    let stranger = Identity::generate();
    e.check(
        "legacy_and_upgraded_replicas_paired",
        paired && legacy.is_ok() && upgraded.is_ok() && stranger.is_ok(),
        "",
    );
    let (Ok(legacy), Ok(upgraded), Ok(stranger)) = (legacy, upgraded, stranger) else {
        return Ok(());
    };
    e.check("rust_gateway_started", true, "");
    e.check("rust_gateway_ready", !rig.address.is_empty(), "");
    let at = rig.address.clone();
    let first = authenticates(&rig, &at, &legacy, false);
    e.check(
        "rust_relay1_legacy_replica_authenticates_unchanged",
        shaped(&first) == Some(false),
        "",
    );
    e.check(
        "rust_relay1_unpaired_replica_rejected",
        authenticates(&rig, &at, &stranger, false).is_err(),
        "",
    );
    let marked = authenticates(&rig, &at, &upgraded, true);
    e.check(
        "rust_relay2_replica_authenticates_and_is_marked",
        shaped(&marked) == Some(true),
        "",
    );
    e.check(
        "rust_relay2_marked_replica_never_accepted_on_relay1",
        authenticates(&rig, &at, &upgraded, false).is_err(),
        "",
    );
    let unaffected = authenticates(&rig, &at, &legacy, false);
    e.check(
        "rust_relay1_legacy_replica_unaffected_by_other_peer_upgrade",
        shaped(&unaffected) == Some(false),
        "",
    );
    e.check(
        "rust_relay2_unpaired_replica_rejected",
        authenticates(&rig, &at, &stranger, true).is_err(),
        "",
    );
    let moved = authenticates(&rig, &at, &legacy, true);
    e.check(
        "rust_relay1_replica_upgrades_to_relay2_in_place",
        moved.as_ref().is_ok_and(|p| p.relay2),
        "",
    );
    e.check(
        "rust_upgraded_replica_relay1_rejected_afterwards",
        authenticates(&rig, &at, &legacy, false).is_err(),
        "",
    );
    let connections = Arc::new(AtomicUsize::new(0));
    let state = Arc::new(Mutex::new((0usize, false)));
    let (counted, seen) = (connections.clone(), state.clone());
    let (chain, key) = rig.ca.leaf_for(COMPAT_HOST).map_err(invalid)?;
    let only1 = serve(server_config(&chain, &key)?, move |mut socket| {
        counted.fetch_add(1, Ordering::SeqCst);
        if let Ok(message) = socket.read() {
            let mut state = seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.0 += 1;
            let versioned = texts(&message)
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .is_some_and(|value| value.get("version").is_some());
            if !versioned {
                state.1 = true;
            }
        }
        let _ = socket.close(Some(tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Policy,
            reason: "relay/1 only".into(),
        }));
        let _ = socket.flush();
    })?;
    let outcome = authenticates(&rig, &only1.address, &upgraded, true);
    thread::sleep(Duration::from_millis(300));
    let (frames, legacy_hello) = *state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    e.check(
        "relay2_required_client_fails_against_relay1_only_gateway",
        outcome.is_err(),
        "",
    );
    e.check(
        "relay2_required_client_never_falls_back_to_relay1",
        connections.load(Ordering::SeqCst) == 1 && frames == 1 && !legacy_hello,
        "",
    );
    Ok(())
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.clone().as_str() {
        "RELAY-TLS-SPLICE" | "RELAY-DOWNGRADE" => splice(world, evidence),
        "RELAY-LEGACY-COMPAT" => compat(world, evidence),
        id => Err(Error::Invalid(format!("relay scenario not ported: {id}"))),
    }
}
