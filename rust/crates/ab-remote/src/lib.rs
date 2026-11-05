#![forbid(unsafe_code)]

pub mod certificate;
pub mod ct;
pub mod dns;
pub mod localapi;
pub mod onboarding;
pub mod private;
pub mod relay;
mod smux;
pub mod state;
pub mod tailscale;

use futures_util::StreamExt;
use std::{future::Future, net::SocketAddr, pin::Pin};
use tokio::sync::watch;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("remote IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("remote JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("remote HTTP: {0}")]
    Http(#[from] reqwest::Error),
    #[error("remote PEM: {0}")]
    Pem(#[from] rustls::pki_types::pem::Error),
    #[error("remote TLS: {0}")]
    Tls(#[from] rustls::Error),
    #[error("remote WebSocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("remote ACME: {0}")]
    Acme(#[from] instant_acme::Error),
    #[error("remote certificate: {0}")]
    Certificate(#[from] rcgen::Error),
    #[error("remote contract: {0}")]
    Contract(&'static str),
    #[error("{0}")]
    Remote(String),
}

pub type Result<T> = std::result::Result<T, Error>;
pub type IngressFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub state: &'static str,
    pub message: String,
    pub pending_link: String,
    pub last_error: String,
    pub link: String,
}
impl Progress {
    #[must_use]
    pub fn failed() -> Self {
        Self {
            state: "error",
            message: String::new(),
            pending_link: String::new(),
            last_error: "Remote setup failed. Turn Remote off and try again.".into(),
            link: String::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct IngressStatus {
    pub host: String,
    pub active: bool,
    pub certificate_expiry: Option<String>,
    pub progress: Option<Progress>,
}
pub type IngressStatuses = std::collections::BTreeMap<String, IngressStatus>;
#[derive(Debug)]
pub struct ActiveIngress {
    statuses: Option<watch::Sender<IngressStatuses>>,
    name: String,
}
impl Drop for ActiveIngress {
    fn drop(&mut self) {
        if let Some(statuses) = &self.statuses {
            statuses.send_modify(|entries| {
                if let Some(entry) = entries.get_mut(&self.name) {
                    entry.active = false;
                }
            });
        }
    }
}
const INGRESS_MAGIC: &[u8] = b"ab-ingress/1\0";
const INGRESS_HAS_BINDING: u8 = 1;
const INGRESS_HAS_SOURCE: u8 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IngressPreamble {
    pub binding: Option<[u8; 32]>,
    pub source: Option<std::net::IpAddr>,
}

pub async fn read_ingress_preamble<R>(socket: &mut R, proof: &[u8; 32]) -> Result<IngressPreamble>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use subtle::ConstantTimeEq;
    use tokio::io::AsyncReadExt;
    let mut magic = vec![0; INGRESS_MAGIC.len()];
    socket.read_exact(&mut magic).await?;
    let mut received = [0; 32];
    socket.read_exact(&mut received).await?;
    if magic != INGRESS_MAGIC || !bool::from(proof.ct_eq(&received)) {
        return Err(Error::Contract("ingress proof"));
    }
    let flags = socket.read_u8().await?;
    if flags & !(INGRESS_HAS_BINDING | INGRESS_HAS_SOURCE) != 0 {
        return Err(Error::Contract("ingress flags"));
    }
    let mut preamble = IngressPreamble::default();
    if flags & INGRESS_HAS_BINDING != 0 {
        let mut binding = [0; 32];
        socket.read_exact(&mut binding).await?;
        preamble.binding = Some(binding);
    }
    if flags & INGRESS_HAS_SOURCE != 0 {
        let length = usize::from(socket.read_u8().await?);
        if length != 4 && length != 16 {
            return Err(Error::Contract("ingress source"));
        }
        let mut address = vec![0; length];
        socket.read_exact(&mut address).await?;
        preamble.source = ab_wire::framing::decode_tunnel_source(&address);
    }
    Ok(preamble)
}
#[derive(Clone)]
pub struct Gateway {
    pub address: SocketAddr,
    proof: Option<std::sync::Arc<[u8; 32]>>,
    statuses: Option<watch::Sender<IngressStatuses>>,
}
impl std::fmt::Debug for Gateway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Gateway")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}
impl Gateway {
    pub fn new(address: SocketAddr) -> Self {
        Self {
            address,
            proof: None,
            statuses: None,
        }
    }
    pub fn authenticated(address: SocketAddr, proof: [u8; 32]) -> Self {
        Self {
            address,
            proof: Some(std::sync::Arc::new(proof)),
            statuses: None,
        }
    }
    #[must_use]
    pub fn with_status(mut self, statuses: watch::Sender<IngressStatuses>) -> Self {
        self.statuses = Some(statuses);
        self
    }
    pub(crate) fn ready(&self, name: &str, host: &str) -> ActiveIngress {
        if let Some(statuses) = &self.statuses {
            statuses.send_modify(|entries| {
                entries.insert(
                    name.into(),
                    IngressStatus {
                        host: host.into(),
                        active: true,
                        certificate_expiry: None,
                        progress: None,
                    },
                );
            });
        }
        ActiveIngress {
            statuses: self.statuses.clone(),
            name: name.into(),
        }
    }
    pub fn progress(&self, name: &str, progress: Progress) {
        if let Some(statuses) = &self.statuses {
            statuses.send_if_modified(|entries| {
                let entry = entries.entry(name.into()).or_insert_with(|| IngressStatus {
                    host: String::new(),
                    active: false,
                    certificate_expiry: None,
                    progress: None,
                });
                if entry.active || entry.progress.as_ref() == Some(&progress) {
                    return false;
                }
                entry.progress = Some(progress);
                true
            });
        }
    }
    pub fn clear_progress(&self, name: &str) {
        if let Some(statuses) = &self.statuses {
            statuses.send_if_modified(|entries| {
                entries
                    .get_mut(name)
                    .is_some_and(|entry| entry.progress.take().is_some())
            });
        }
    }
    pub(crate) fn refresh_status(&self) {
        if let Some(statuses) = &self.statuses {
            statuses.send_modify(|_| {});
        }
    }
    pub(crate) fn certificate_expiry(&self, name: &str, expiry: &str) {
        if let Some(statuses) = &self.statuses {
            statuses.send_if_modified(|entries| {
                entries.get_mut(name).is_some_and(|entry| {
                    let changed = entry.certificate_expiry.as_deref() != Some(expiry);
                    entry.certificate_expiry = Some(expiry.into());
                    changed
                })
            });
        }
    }
    pub(crate) async fn connect(&self) -> Result<tokio::net::TcpStream> {
        self.connect_stream(None, None).await
    }
    pub(crate) async fn connect_stream(
        &self,
        binding: Option<[u8; 32]>,
        source: Option<std::net::IpAddr>,
    ) -> Result<tokio::net::TcpStream> {
        use tokio::io::AsyncWriteExt;
        let mut socket = tokio::net::TcpStream::connect(self.address).await?;
        if let Some(proof) = &self.proof {
            let mut preamble = INGRESS_MAGIC.to_vec();
            preamble.extend_from_slice(proof.as_ref());
            preamble.push(
                u8::from(binding.is_some()) * INGRESS_HAS_BINDING
                    + u8::from(source.is_some()) * INGRESS_HAS_SOURCE,
            );
            if let Some(binding) = binding {
                preamble.extend_from_slice(&binding);
            }
            if let Some(source) = source {
                let address = ab_wire::framing::encode_tunnel_source(source);
                preamble.push(u8::try_from(address.len()).map_err(|_| Error::Contract("source"))?);
                preamble.extend_from_slice(&address);
            }
            socket.write_all(&preamble).await?;
        }
        Ok(socket)
    }
}

pub trait Ingress: std::fmt::Debug + Send + Sync {
    fn run(&self, gateway: Gateway, stop: watch::Receiver<bool>) -> IngressFuture<'_>;
}

pub async fn stopped(stop: &mut watch::Receiver<bool>) {
    while !*stop.borrow_and_update() {
        if stop.changed().await.is_err() {
            break;
        }
    }
}

pub async fn run_all(
    adapters: &[Box<dyn Ingress>],
    gateway: Gateway,
    stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut runs = futures_util::stream::FuturesUnordered::new();
    for adapter in adapters {
        runs.push(adapter.run(gateway.clone(), stop.clone()));
    }

    while let Some(result) = runs.next().await {
        result?;
    }
    Ok(())
}
