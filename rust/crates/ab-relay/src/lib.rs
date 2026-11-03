#![forbid(unsafe_code)]

pub mod hello;
pub mod mux;
pub mod server;
pub mod store;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("relay access denied")]
    Denied,
    #[error("invalid relay configuration")]
    Configuration,
    #[error("storage: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("random source: {0}")]
    Random(String),
}
pub type Result<T> = std::result::Result<T, Error>;

pub mod http;

pub mod cloud_dns;
pub mod ct;

pub mod acme;

pub mod replay;

mod client;

pub mod push;
