#![forbid(unsafe_code)]

#[cfg(feature = "rust-effects")]
pub mod effects;
#[cfg(any(feature = "rust-effects", feature = "rust-capture"))]
mod frame;
#[cfg(feature = "rust-effects")]
pub mod guard;
pub mod ownership;
#[cfg(feature = "rust-capture")]
pub mod production;
pub mod proxy;
#[cfg(feature = "rust-capture")]
pub mod readers;
#[cfg(feature = "rust-capture")]
pub mod routed;
#[cfg(feature = "rust-capture")]
pub mod store;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("session IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("session JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid session configuration")]
    Invalid,
    #[error("ownership conflict")]
    Conflict,
    #[error("dispatch already started")]
    AlreadyStarted,
    #[error("source reader: {0}")]
    Reader(String),
    #[error("session worker panicked")]
    WorkerPanicked,
}
