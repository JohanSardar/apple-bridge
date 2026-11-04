mod actions;
mod agents;
mod clock;
mod contacts;
mod http;
mod live;
mod local;
mod proxy;
mod remote;
mod replica;
mod session;

pub use actions::{public_roots, run as run_action, run_with_roots};
pub use clock::Estimate;
pub use http::{Client, Reply};
pub use local::{Common, Local, new_id, records_digest};
pub use proxy::{Route, serve as serve_proxy};
pub use remote::{Input, checks};
pub use session::Session;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    #[error("relay: {0}")]
    Relay(#[from] lab_relay::Error),
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("local API {operation} status {status} code {code}")]
    Api {
        operation: String,
        status: u16,
        code: String,
    },
    #[error("{0}")]
    Failed(String),
}
pub type Result<T> = std::result::Result<T, Error>;

pub fn failed<T>(message: &str) -> Result<T> {
    Err(Error::Failed(message.to_owned()))
}
