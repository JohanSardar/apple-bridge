#![forbid(unsafe_code)]
mod accounts;
pub mod caa;
mod http;
mod pages;
mod relay_admin;
mod store;

pub use accounts::{Accounts, BetterAuth, Session, SessionFuture};
pub use http::{router, sync_policy};
pub use relay_admin::{Admitted, RelayAdmin, RelayApi, RelayFuture};
pub use store::{Control, DeviceAction, Error, PolicyChange, Result, VerifiedGrant};

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
