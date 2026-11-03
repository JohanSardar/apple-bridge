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
pub struct Gateway {
    pub address: SocketAddr,
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
