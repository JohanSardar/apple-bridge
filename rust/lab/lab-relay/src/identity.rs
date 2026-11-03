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
    pub fn generate() -> Result<Self> {
        let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
            .map_err(|e| Error::Certificate(e.to_string()))?;
        let mut params = rcgen::CertificateParams::new(vec!["localhost".into()])
            .map_err(|e| Error::Certificate(e.to_string()))?;
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        ];
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::minutes(1);
        params.not_after = now + time::Duration::days(365);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Apple Bridge");
        let certificate = params
            .self_signed(&pair)
            .map_err(|e| Error::Certificate(e.to_string()))?
            .der()
            .to_vec();
        let key = SigningKey::from_pkcs8_pem(&pair.serialize_pem())
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
pub fn valid_certificate(der: &[u8]) -> bool {
    x509_parser::parse_x509_certificate(der).is_ok_and(|(_, cert)| cert.validity().is_valid())
}
