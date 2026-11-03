mod auth;
pub mod caa;
mod http;
mod ids;
mod policy;
mod store;
pub use policy::policy_router;

pub use auth::{Github, IdentityProvider, ProviderFuture};
pub use http::router;
pub use store::{Control, Error, Result};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

fn hash(value: impl AsRef<[u8]>) -> String {
    encode(Sha256::digest(value.as_ref()))
}

fn encode(value: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(value)
}

fn decode(value: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(value).map_err(|_| Error::Invalid)
}

fn random(bytes: usize) -> Result<String> {
    let mut value = vec![0; bytes];
    getrandom::fill(&mut value).map_err(|_| Error::Unavailable)?;
    Ok(encode(value))
}

pub fn device_message(id: &str, nonce: &str, key: &str, slot: u8) -> Vec<u8> {
    format!("ab-relay-control/device/1\napp.lexaai.tech\n{id}\n{nonce}\n{key}\n{slot}").into_bytes()
}

pub fn rotation_message(id: &str, nonce: &str, key: &str) -> Vec<u8> {
    format!("ab-relay-control/rotate/1\napp.lexaai.tech\n{id}\n{nonce}\n{key}").into_bytes()
}
