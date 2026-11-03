#![forbid(unsafe_code)]

pub mod certificate;
pub mod ct;
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
}

pub type Result<T> = std::result::Result<T, Error>;
pub type IngressFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct IngressStatus {
    pub host: String,
    pub active: bool,
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
pub const INGRESS_MAGIC: &[u8] = b"ab-ingress/1\0";
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
                    },
                );
            });
        }
        ActiveIngress {
            statuses: self.statuses.clone(),
            name: name.into(),
        }
    }
    pub(crate) async fn connect(&self) -> Result<tokio::net::TcpStream> {
        use tokio::io::AsyncWriteExt;
        let mut socket = tokio::net::TcpStream::connect(self.address).await?;
        if let Some(proof) = &self.proof {
            socket.write_all(INGRESS_MAGIC).await?;
            socket.write_all(proof.as_ref()).await?;
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
