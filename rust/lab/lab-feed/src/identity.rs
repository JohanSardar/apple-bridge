use crate::{Error, Result};
use ed25519_dalek::{
    Signer, SigningKey,
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
};
use rustls::pki_types::{CertificateDer, pem::PemObject};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn fingerprint(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
pub struct Identity {
    pub certificate: Vec<u8>,
    pub fingerprint: String,
    key: SigningKey,
}
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}
impl Identity {
    pub fn load(home: &Path) -> Result<Self> {
        let pem = std::fs::read(home.join("identity.pem"))?;
        let certificate = CertificateDer::from_pem_slice(&pem)
            .map_err(|e| Error::Certificate(e.to_string()))?
            .to_vec();
        let secret = home.join("secrets").join(fingerprint(b"identity-key"));
        let key = SigningKey::from_pkcs8_pem(
            &String::from_utf8(std::fs::read(secret)?).map_err(|_| Error::Invalid)?,
        )
        .map_err(|e| Error::Certificate(e.to_string()))?;
        Ok(Self {
            fingerprint: fingerprint(&certificate),
            certificate,
            key,
        })
    }
    pub fn pkcs8(&self) -> Result<Vec<u8>> {
        Ok(self
            .key
            .to_pkcs8_der()
            .map_err(|e| Error::Certificate(e.to_string()))?
            .as_bytes()
            .to_vec())
    }
    pub fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        self.key.sign(bytes).to_bytes().to_vec()
    }
}
