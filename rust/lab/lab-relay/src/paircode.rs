use crate::{Error, Result, identity::Identity};
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PairCode {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dial: String,
    pub id: String,
    pub address: String,
    pub fingerprint: String,
    pub secret: String,
    pub expires_at: String,
}
impl PairCode {
    pub fn decode(text: &str) -> Result<Self> {
        let body = text.trim().strip_prefix("ab1-").ok_or(Error::Invalid)?;
        let raw = data_encoding::BASE32_NOPAD
            .decode(body.as_bytes())
            .map_err(|_| Error::Invalid)?;
        let code: Self = serde_json::from_slice(&raw)?;
        if code.id.is_empty()
            || code.address.is_empty()
            || code.fingerprint.len() != 64
            || code.secret_bytes()?.len() != 20
        {
            return Err(Error::Invalid);
        }
        Ok(code)
    }
    pub fn encode(&self) -> Result<String> {
        Ok(format!(
            "ab1-{}",
            data_encoding::BASE32_NOPAD.encode(&serde_json::to_vec(self)?)
        ))
    }
    pub fn secret_bytes(&self) -> Result<Vec<u8>> {
        STANDARD.decode(&self.secret).map_err(|_| Error::Invalid)
    }
    pub fn flip_secret_bit(&mut self) -> Result<()> {
        let mut secret = self.secret_bytes()?;
        secret[0] ^= 1;
        self.secret = STANDARD.encode(secret);
        Ok(())
    }
    pub fn expire(&mut self) {
        self.expires_at = "2000-01-01T00:00:00Z".into();
    }
    pub fn expired(&self) -> bool {
        self.expires_at
            .parse::<jiff::Timestamp>()
            .is_ok_and(|at| jiff::Timestamp::now() > at)
    }
    pub fn proof(&self, identity: &Identity) -> Result<Vec<u8>> {
        let key = Sha256::digest(self.secret_bytes()?);
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| Error::Invalid)?;
        mac.update(b"apple-bridge-pair/1\0");
        mac.update(&identity.certificate);
        Ok(mac.finalize().into_bytes().to_vec())
    }
}
