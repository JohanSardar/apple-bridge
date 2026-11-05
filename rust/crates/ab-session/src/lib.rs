#![forbid(unsafe_code)]

pub mod flight;
mod frame;
pub mod guard;
pub mod media_facts;
pub mod messages_media;
pub mod messages_policy;
pub mod native_facts;
pub mod production;
pub mod queries;
pub mod readers;
pub mod session;
pub mod staged;
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
    #[error("native/1 facts link lost")]
    LinkLost,
    #[error("dispatch already started")]
    AlreadyStarted,
    #[error("source reader: {0}")]
    Reader(String),
    #[error("session worker panicked")]
    WorkerPanicked,
}
