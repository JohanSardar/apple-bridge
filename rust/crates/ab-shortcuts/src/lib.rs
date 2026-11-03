pub mod cli;
pub mod clock;
#[cfg(feature = "lab-harness")]
pub mod fixtures;
pub mod focus;
pub mod pack;

pub use cli::{Cli, Entry, Input, MAX_BYTES, Output, Runner, parse_inventory};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid shortcut input")]
    InvalidInput,
    #[error("shortcut output exceeds limit")]
    OutputExceeded,
    #[error("invalid shortcut output")]
    InvalidOutput,
    #[error("invalid shortcuts inventory")]
    InvalidInventory,
    #[error("shortcuts inventory unavailable")]
    InventoryUnavailable,
    #[error("shortcut process unavailable")]
    ProcessUnavailable,
    #[error("deadline exceeded")]
    DeadlineExceeded,
    #[error("Clock pack structured readback is not qualified")]
    ClockUnqualified,
    #[error("Focus status readback is unavailable or mismatched")]
    FocusUnqualified,
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    #[must_use]
    pub fn message(text: &str) -> Self {
        Self::Message(text.to_owned())
    }
}
