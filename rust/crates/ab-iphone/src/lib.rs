#![forbid(unsafe_code)]
pub mod delivery;
mod http;
pub use ab_wire::crypto::Packet;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Trust(#[from] ab_trust::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid input")]
    Invalid,
    #[error("authentication failed")]
    Authentication,
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("conflict")]
    Conflict,
    #[error("iPhone delivery unavailable")]
    Unavailable,
}
