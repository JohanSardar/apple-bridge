mod http;
mod proxy;
mod remote;

pub use http::{Client, Reply};
pub use proxy::{Route, serve as serve_proxy};
pub use remote::{Input, checks};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    #[error("invalid {0}")]
    Invalid(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;
