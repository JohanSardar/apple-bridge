use lab_world::{Error, Result, World};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, DistinguishedName, ServerConfig,
    ServerConnection, SignatureScheme, StreamOwned,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring::default_provider, verify_tls13_signature},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::ErrorKind,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tungstenite::{Message, WebSocket};

pub const BYTE_CREDIT: usize = 4 * 1024 * 1024;
pub const MAX_FRAME: usize = 256 * 1024;
static PROXY: Mutex<Option<FrameProxy>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn failed<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}

#[derive(Debug, Default)]
struct Faults {
    drop_batch: bool,
    drop_acks: bool,
    dropped: usize,
    connections: usize,
    high_water: usize,
}
#[derive(Debug)]
struct State {
    upstream: String,
    gateway_home: PathBuf,
    replica_home: PathBuf,
    faults: Mutex<Faults>,
    framed: AtomicBool,
    closed: AtomicBool,
}
#[derive(Debug, Clone)]
pub struct FrameProxy {
    address: String,
    state: Arc<State>,
}

#[derive(Debug)]
struct AnyPeer(Option<String>);
impl ClientCertVerifier for AnyPeer {
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
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 refused".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
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
impl ServerCertVerifier for AnyPeer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        match &self.0 {
            Some(pin) if hex::encode(Sha256::digest(end_entity.as_ref())) != *pin => {
                Err(rustls::Error::General("peer pin mismatch".into()))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.2 refused".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
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

fn identity(home: &Path) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let certificate = CertificateDer::from_pem_slice(&std::fs::read(home.join("identity.pem"))?)
        .map_err(failed)?;
    let name = hex::encode(Sha256::digest(b"identity-key"));
    let key = PrivateKeyDer::from_pem_slice(&std::fs::read(home.join("secrets").join(name))?)
        .map_err(failed)?;
    Ok((vec![certificate], key))
}

type Socket<C> = WebSocket<StreamOwned<C, TcpStream>>;

fn idle(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(e)
        if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

struct Frames {
    outstanding: usize,
}

fn from_replica(state: &State, frames: &mut Frames, text: &str) -> bool {
    let frame: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    if frame["t"] != "Ack" {
        return true;
    }
    if lock(&state.faults).drop_acks {
        return false;
    }
    let credits = usize::try_from(frame["credits"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
    frames.outstanding = frames.outstanding.saturating_sub(credits);
    true
}

fn from_gateway(state: &State, frames: &mut Frames, text: &str) -> Option<Value> {
    let frame: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let kind = frame["t"].as_str().unwrap_or_default();
    let mut faults = lock(&state.faults);
    if matches!(
        kind,
        "Welcome" | "Reset" | "SnapshotBegin" | "SnapshotPage" | "SnapshotEnd" | "Batch"
    ) {
        frames.outstanding += text.len();
        faults.high_water = faults.high_water.max(frames.outstanding);
    }
    if kind == "Batch" && faults.drop_batch {
        faults.drop_batch = false;
        faults.dropped += 1;
        frames.outstanding = frames.outstanding.saturating_sub(text.len());
        return Some(json!({"t": "Ack", "credits": text.len(), "cursor": {
            "epoch": frame["batch"]["epoch"],
            "subscription_revision": frame["batch"]["subscription_revision"],
            "sequence": frame["batch"]["through"]}}));
    }
    None
}

fn relay(state: &Arc<State>, downstream: TcpStream) -> Result<()> {
    let (chain, key) = identity(&state.gateway_home)?;
    let server = ServerConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(failed)?
        .with_client_cert_verifier(Arc::new(AnyPeer(None)))
        .with_single_cert(chain, key)
        .map_err(failed)?;
    let (chain, key) = identity(&state.replica_home)?;
    let client = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(failed)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyPeer(None)))
        .with_client_auth_cert(chain, key)
        .map_err(failed)?;
    downstream.set_nonblocking(false)?;
    downstream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let accepted = StreamOwned::new(
        ServerConnection::new(Arc::new(server)).map_err(failed)?,
        downstream,
    );
    let mut down: Socket<ServerConnection> = tungstenite::accept(accepted).map_err(failed)?;
    let upstream = TcpStream::connect(&state.upstream)?;
    upstream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let dialed = StreamOwned::new(
        ClientConnection::new(
            Arc::new(client),
            ServerName::try_from("localhost").map_err(failed)?,
        )
        .map_err(failed)?,
        upstream,
    );
    let (mut up, _): (Socket<ClientConnection>, _) =
        tungstenite::client::client(format!("wss://{}/v1/feed", state.upstream), dialed)
            .map_err(failed)?;
    lock(&state.faults).connections += 1;
    let tick = Some(Duration::from_millis(10));
    down.get_mut().sock.set_read_timeout(tick)?;
    up.get_mut().sock.set_read_timeout(tick)?;
    let mut frames = Frames { outstanding: 0 };
    while !state.closed.load(Ordering::SeqCst) {
        match down.read() {
            Ok(Message::Text(text)) => {
                if from_replica(state, &mut frames, &text) {
                    up.send(Message::Text(text)).map_err(failed)?;
                }
            }
            Ok(Message::Binary(bytes)) => up.send(Message::Binary(bytes)).map_err(failed)?,
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(error) if idle(&error) => {}
            Err(_) => return Ok(()),
        }
        match up.read() {
            Ok(Message::Text(text)) => {
                if let Some(ack) = from_gateway(state, &mut frames, &text) {
                    up.send(Message::text(ack.to_string())).map_err(failed)?;
                } else {
                    down.send(Message::Text(text)).map_err(failed)?;
                }
            }
            Ok(Message::Binary(bytes)) => down.send(Message::Binary(bytes)).map_err(failed)?,
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(error) if idle(&error) => {}
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

fn pass(state: &Arc<State>, downstream: TcpStream) -> Result<()> {
    let upstream = TcpStream::connect(&state.upstream)?;
    let (mut from_down, mut to_up) = (downstream.try_clone()?, upstream.try_clone()?);
    let forward = thread::spawn(move || {
        let _ = std::io::copy(&mut from_down, &mut to_up);
        let _ = to_up.shutdown(std::net::Shutdown::Both);
    });
    let (mut from_up, mut to_down) = (upstream, downstream);
    let _ = std::io::copy(&mut from_up, &mut to_down);
    let _ = to_down.shutdown(std::net::Shutdown::Both);
    let _ = forward.join();
    Ok(())
}

impl FrameProxy {
    pub fn arm(&self) {
        self.state.framed.store(true, Ordering::SeqCst);
    }
    pub fn start(upstream: &str, gateway_home: &Path, replica_home: &Path) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?.to_string();
        let state = Arc::new(State {
            upstream: upstream.to_owned(),
            gateway_home: gateway_home.to_owned(),
            replica_home: replica_home.to_owned(),
            faults: Mutex::new(Faults::default()),
            framed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        let accepting = state.clone();
        thread::spawn(move || {
            while !accepting.closed.load(Ordering::SeqCst) {
                if let Ok((stream, _)) = listener.accept() {
                    let state = accepting.clone();
                    thread::spawn(move || {
                        if state.framed.load(Ordering::SeqCst) {
                            let _ = relay(&state, stream);
                        } else {
                            let _ = pass(&state, stream);
                        }
                    });
                } else {
                    thread::sleep(Duration::from_millis(10));
                }
            }
        });
        Ok(Self { address, state })
    }
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn reset_connections(&self) {
        lock(&self.state.faults).connections = 0;
    }
    pub fn drop_next_batch(&self) {
        lock(&self.state.faults).drop_batch = true;
    }
    pub fn suppress_acks(&self, enabled: bool) {
        lock(&self.state.faults).drop_acks = enabled;
    }
    #[must_use]
    pub fn dropped(&self) -> usize {
        lock(&self.state.faults).dropped
    }
    #[must_use]
    pub fn connections(&self) -> usize {
        lock(&self.state.faults).connections
    }
    #[must_use]
    pub fn credit_high_water(&self) -> usize {
        lock(&self.state.faults).high_water
    }
}

pub fn install(world: &mut World, replica_home: &Path) -> Result<()> {
    world.install_proxy()?;
    let proxy = FrameProxy::start(&world.proxy_address()?, &world.gateway_home, replica_home)?;
    crate::replicas::pair_through(Some(proxy.address().to_owned()));
    *lock(&PROXY) = Some(proxy);
    Ok(())
}
pub fn current() -> Result<FrameProxy> {
    lock(&PROXY)
        .clone()
        .ok_or_else(|| Error::Invalid("frame proxy not installed".into()))
}
pub fn shutdown() {
    if let Some(proxy) = lock(&PROXY).take() {
        proxy.state.closed.store(true, Ordering::SeqCst);
    }
}

pub(crate) type Tls = StreamOwned<ClientConnection, TcpStream>;

pub(crate) fn pinned_client(address: &str, home: &Path, pin: &str) -> Result<Tls> {
    let (chain, key) = identity(home)?;
    let config = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(failed)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyPeer(Some(pin.to_owned()))))
        .with_client_auth_cert(chain, key)
        .map_err(failed)?;
    let socket = TcpStream::connect(address)?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(StreamOwned::new(
        ClientConnection::new(
            Arc::new(config),
            ServerName::try_from("localhost").map_err(failed)?,
        )
        .map_err(failed)?,
        socket,
    ))
}

pub(crate) fn fingerprint(home: &Path) -> Result<String> {
    let (chain, _) = identity(home)?;
    Ok(hex::encode(Sha256::digest(chain[0].as_ref())))
}
