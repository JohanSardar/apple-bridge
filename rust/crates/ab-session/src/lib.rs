#![forbid(unsafe_code)]

#[cfg(feature = "rust-effects")]
pub mod effects;
pub mod flight;
#[cfg(any(feature = "rust-effects", feature = "rust-capture"))]
mod frame;
pub mod guard;
pub mod messages_policy;
#[cfg(feature = "rust-capture")]
pub mod native_facts;
pub mod ownership;
#[cfg(feature = "rust-capture")]
pub mod production;
pub mod proxy;
#[cfg(feature = "rust-capture")]
pub mod queries;
#[cfg(feature = "rust-capture")]
pub mod readers;
#[cfg(feature = "rust-capture")]
pub mod routed;
pub mod session;
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
