#![forbid(unsafe_code)]

#[cfg(feature = "rust-effects")]
pub mod guard;
pub mod ownership;
pub mod proxy;

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
    #[error("session worker panicked")]
    WorkerPanicked,
}
