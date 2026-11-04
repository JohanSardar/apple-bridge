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
        Arc,
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

#[derive(Debug, Default)]
struct Counters {
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
    while !counters.stop.load(Ordering::SeqCst) {
        if let Some(text) = poll(&mut downstream)? {
            let ack = serde_json::from_str::<Value>(&text).is_ok_and(|frame| frame["t"] == "Ack");
            if ack && counters.suppress.load(Ordering::SeqCst) {
                counters.dropped.fetch_add(1, Ordering::SeqCst);
            } else {
                upstream.send_text(&text).map_err(invalid)?;
            }
        }
        if let Some(text) = upstream.try_recv_text(POLL).map_err(invalid)? {
            downstream.send(Message::text(text)).map_err(invalid)?;
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
