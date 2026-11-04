use lab_relay::{FeedLink, Identity};
use lab_world::{Error, Result};
use rustls::{
    DigitallySignedStruct, DistinguishedName, ServerConfig, ServerConnection, SignatureScheme,
    StreamOwned,
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use serde_json::Value;
use std::{
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use tungstenite::{Message, WebSocket};

const POLL: Duration = Duration::from_millis(10);

#[derive(Debug)]
struct AnyClient(Arc<CryptoProvider>);
impl ClientCertVerifier for AnyClient {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("tls 1.2 is not allowed".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureProbe {
    pub atomic_offers: u64,
    pub legacy_offers: u64,
    pub atomic_welcomes: u64,
    pub legacy_welcomes: u64,
    pub partial_fields: u64,
    pub partial_frames: u64,
    pub partial_bytes: u64,
    pub partial_ack_without_cursor: u64,
    pub partial_ack_with_cursor: u64,
    pub reset_frames: u64,
    pub hello_sequences: Vec<u64>,
}

#[derive(Debug, Default)]
struct Counters {
    probe: Mutex<CaptureProbe>,
    capture_reset: AtomicBool,
    force_snapshot: AtomicBool,
    suppress: AtomicBool,
    stop: AtomicBool,
    connections: AtomicU64,
    dropped: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct FrameProxy {
    address: String,
    counters: Arc<Counters>,
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

fn poll(
    socket: &mut WebSocket<StreamOwned<ServerConnection, TcpStream>>,
) -> Result<Option<String>> {
    socket
        .get_mut()
        .sock
        .set_read_timeout(Some(POLL))
        .map_err(invalid)?;
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => return Ok(Some(text.as_str().to_owned())),
            Ok(Message::Close(_)) => return Err(invalid("replica closed")),
            Ok(Message::Binary(_)) => return Err(invalid("binary feed frame")),
            Ok(_) => {}
            Err(tungstenite::Error::Io(io))
                if matches!(
                    io.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(invalid(error)),
        }
    }
}

fn probe(counters: &Counters) -> std::sync::MutexGuard<'_, CaptureProbe> {
    counters
        .probe
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn offers_atomic(frame: &Value) -> bool {
    frame["features"]
        .as_array()
        .is_some_and(|features| features.iter().any(|f| f == "capture_atomic"))
}

fn from_replica(text: &str, receipts: &mut Vec<bool>, counters: &Counters) -> Option<String> {
    let Ok(mut frame) = serde_json::from_str::<Value>(text) else {
        return Some(text.to_owned());
    };
    let kind = frame["t"].as_str().unwrap_or_default().to_owned();
    let mut rewritten = false;
    if kind == "Hello" {
        let mut seen = probe(counters);
        if offers_atomic(&frame) {
            seen.atomic_offers += 1;
        } else {
            seen.legacy_offers += 1;
        }
        if let Some(sequence) = frame["cursor"]["sequence"].as_u64() {
            seen.hello_sequences.push(sequence);
        }
        drop(seen);
        if counters.force_snapshot.swap(false, Ordering::SeqCst)
            && let Some(object) = frame.as_object_mut()
        {
            object.remove("cursor");
            rewritten = true;
        }
    }
    if kind == "Ack" {
        if counters.suppress.load(Ordering::SeqCst) {
            counters.dropped.fetch_add(1, Ordering::SeqCst);
            return None;
        }
        if !receipts.is_empty() && receipts.remove(0) {
            let mut seen = probe(counters);
            if frame["cursor"].is_null() {
                seen.partial_ack_without_cursor += 1;
            } else {
                seen.partial_ack_with_cursor += 1;
            }
        }
    }
    if rewritten {
        serde_json::to_string(&frame).ok()
    } else {
        Some(text.to_owned())
    }
}

fn from_gateway(text: &str, receipts: &mut Vec<bool>, counters: &Counters) -> bool {
    let Ok(frame) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let kind = frame["t"].as_str().unwrap_or_default();
    let partial = kind == "Batch" && frame["partial"] == true;
    if matches!(
        kind,
        "Welcome" | "Reset" | "SnapshotBegin" | "SnapshotPage" | "SnapshotEnd" | "Batch"
    ) {
        receipts.push(partial);
    }
    let mut seen = probe(counters);
    if kind == "Batch" && frame.get("partial").is_some() {
        seen.partial_fields += 1;
    }
    if kind == "Welcome" {
        if offers_atomic(&frame) {
            seen.atomic_welcomes += 1;
        }
        if frame["features"].as_array().is_none_or(Vec::is_empty) {
            seen.legacy_welcomes += 1;
        }
    }
    if partial {
        seen.partial_frames += 1;
        seen.partial_bytes += text.len() as u64;
        if counters.capture_reset.swap(false, Ordering::SeqCst) {
            counters.force_snapshot.store(true, Ordering::SeqCst);
            seen.reset_frames += 1;
            return true;
        }
    }
    false
}

fn relay(
    tcp: TcpStream,
    config: Arc<ServerConfig>,
    replica: &Identity,
    gateway_pin: &str,
    gateway_peer: &str,
    counters: &Counters,
) -> Result<()> {
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    let connection = ServerConnection::new(config).map_err(invalid)?;
    let mut downstream = tungstenite::accept(StreamOwned::new(connection, tcp)).map_err(invalid)?;
    let mut upstream = FeedLink::dial(gateway_peer, replica, gateway_pin, Duration::from_secs(5))
        .map_err(invalid)?;
    counters.connections.fetch_add(1, Ordering::SeqCst);
    let mut receipts: Vec<bool> = Vec::new();
    while !counters.stop.load(Ordering::SeqCst) {
        if let Some(text) = poll(&mut downstream)? {
            let Some(forward) = from_replica(&text, &mut receipts, counters) else {
                continue;
            };
            upstream.send_text(&forward).map_err(invalid)?;
        }
        if let Some(text) = upstream.try_recv_text(POLL).map_err(invalid)? {
            let reset = from_gateway(&text, &mut receipts, counters);
            downstream.send(Message::text(text)).map_err(invalid)?;
            if reset {
                downstream
                    .send(Message::text(r#"{"t":"Reset","reason":"capture_fixture"}"#))
                    .map_err(invalid)?;
                return Ok(());
            }
        }
    }
    Ok(())
}

impl FrameProxy {
    pub fn start(gateway_home: &Path, replica_home: &Path, gateway_peer: &str) -> Result<Self> {
        let gateway = Identity::load(gateway_home).map_err(invalid)?;
        let replica = Arc::new(Identity::load(replica_home).map_err(invalid)?);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = Arc::new(
            ServerConfig::builder_with_provider(provider.clone())
                .with_protocol_versions(&[&rustls::version::TLS13])
                .map_err(invalid)?
                .with_client_cert_verifier(Arc::new(AnyClient(provider)))
                .with_single_cert(
                    vec![CertificateDer::from(gateway.certificate.clone())],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                        gateway.pkcs8().map_err(invalid)?,
                    )),
                )
                .map_err(invalid)?,
        );
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let counters = Arc::new(Counters::default());
        let accepting = counters.clone();
        let pin = gateway.fingerprint.clone();
        let peer = gateway_peer.to_owned();
        thread::spawn(move || {
            for tcp in listener.incoming() {
                if accepting.stop.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(tcp) = tcp else { continue };
                let (config, replica, pin, peer, counters) = (
                    config.clone(),
                    replica.clone(),
                    pin.clone(),
                    peer.clone(),
                    accepting.clone(),
                );
                thread::spawn(move || {
                    let _ = relay(tcp, config, &replica, &pin, &peer, &counters);
                });
            }
        });
        Ok(Self { address, counters })
    }
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn capture_reset(&self, enabled: bool) {
        self.counters.capture_reset.store(enabled, Ordering::SeqCst);
    }
    #[must_use]
    pub fn capture_probe(&self) -> CaptureProbe {
        probe(&self.counters).clone()
    }
    pub fn suppress_acks(&self, enabled: bool) {
        self.counters.suppress.store(enabled, Ordering::SeqCst);
    }
    #[must_use]
    pub fn connections(&self) -> u64 {
        self.counters.connections.load(Ordering::SeqCst)
    }
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.counters.dropped.load(Ordering::SeqCst)
    }
    pub fn close(&self) {
        self.counters.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.address);
    }
}
