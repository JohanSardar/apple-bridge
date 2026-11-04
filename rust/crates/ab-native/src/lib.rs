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

pub mod agenda;
pub mod assets;
pub mod calendar_store;
pub mod contacts;
pub mod reminders_enrichment;
pub mod reminders_projection;
pub mod reminders_store;
pub mod voice_memo;

pub mod mail;
pub mod mail_body;
pub mod mail_url;

mod notes_store;

mod notes_changes;
mod notes_projection;
pub mod notes_reader;
