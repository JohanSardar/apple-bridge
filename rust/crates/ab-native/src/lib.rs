#![forbid(unsafe_code)]

pub mod infocus;
pub mod typedstream;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("malformed native data")]
    Malformed,
    #[error("unsupported native encoding")]
    Unsupported,
    #[error("native decoder limit exceeded")]
    Limit,
}

pub fn body_digest(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}
pub mod capture;
pub mod messages;
pub mod native_json;
pub mod notes;
pub mod plist;
pub mod rich;
pub mod safari;
pub mod summary;
pub mod usage;

pub mod telephony;
