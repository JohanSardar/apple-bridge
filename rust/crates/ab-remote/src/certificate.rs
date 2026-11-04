use crate::{Error, Result, relay::Identity, state};
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, NewOrder, RetryPolicy,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct Certificates {
    pub home: PathBuf,
    pub directory: String,
    pub root: Option<PathBuf>,
    current: RwLock<Option<Arc<rustls::ServerConfig>>>,
    renewal: Mutex<()>,
}
impl Certificates {
    pub fn new(home: PathBuf, directory: String) -> Self {
        Self {
            home,
            directory,
            root: None,
            current: RwLock::new(None),
            renewal: Mutex::new(()),
        }
    }
    pub fn config(&self) -> Result<Arc<rustls::ServerConfig>> {
        self.current
            .read()
            .map_err(|_| Error::Contract("certificate lock"))?
            .clone()
            .ok_or(Error::Contract("certificate unavailable"))
    }
    fn install(&self, pem: &str, key: &str, host: &str) -> Result<Vec<u8>> {
        let chain = CertificateDer::pem_slice_iter(pem.as_bytes())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let leaf = chain.first().ok_or(Error::Contract("empty certificate"))?;
        let (_, parsed) = x509_parser::parse_x509_certificate(leaf.as_ref())
            .map_err(|_| Error::Contract("invalid certificate"))?;
        if !parsed.validity().is_valid() {
            return Err(Error::Contract("certificate expired"));
        }
        let names = parsed
            .subject_alternative_name()
            .map_err(|_| Error::Contract("invalid SAN"))?
            .ok_or(Error::Contract("missing SAN"))?;
        if !names.value.general_names.iter().any(
            |name| matches!(name, x509_parser::extensions::GeneralName::DNSName(n) if *n == host),
        ) {
            return Err(Error::Contract("certificate hostname mismatch"));
        }
        let fingerprint = Sha256::digest(leaf.as_ref()).to_vec();
        let key = PrivateKeyDer::from_pem_slice(key.as_bytes())?;
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
        if crate::ct::enabled(&self.home) {
            let value = crate::ct::hex(&fingerprint);
            crate::ct::remember(&self.home, &value)?;
        }
        *self
            .current
            .write()
            .map_err(|_| Error::Contract("certificate lock"))? = Some(Arc::new(config));
        Ok(fingerprint)
    }
    pub async fn ensure(
        &self,
        identity: &Identity,
        host: &str,
        client: &reqwest::Client,
    ) -> Result<()> {
        let _guard = self.renewal.lock().await;
        let dir = self.home.join("remote");
        let key_path = dir.join("relay-key.pem");
        let cert_path = dir.join("relay-cert.pem");
        if let (Ok(cert), Ok(key)) = (
            std::fs::read_to_string(&cert_path),
            std::fs::read_to_string(&key_path),
        ) {
            let chain = CertificateDer::pem_slice_iter(cert.as_bytes())
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if let Some(der) = chain.first()
                && let Ok((_, parsed)) = x509_parser::parse_x509_certificate(der.as_ref())
                && parsed.validity().not_after.timestamp()
                    - x509_parser::time::ASN1Time::now().timestamp()
                    > 30 * 86400
                && let Ok(fingerprint) = self.install(&cert, &key, host)
            {
                return report(identity, client, &fingerprint).await;
            }
        }
        let account_path = dir.join("acme-account.pem");
        let account_key = load_key(&account_path)?;
        let private = PrivateKeyDer::from_pem_slice(account_key.as_bytes())?;
        let PrivateKeyDer::Pkcs8(ref der) = private else {
            return Err(Error::Contract("invalid ACME PKCS8 key"));
        };
        let key = instant_acme::Key::from_pkcs8_der(der.clone_key())?;
        let builder = match &self.root {
            Some(root) => Account::builder_with_root(root)?,
            None => Account::builder()?,
        };
        let (account, _) = builder
            .create_from_key((key, private), self.directory.clone())
            .await?;
        let identifiers = [Identifier::Dns(host.to_owned())];
        let mut order = account.new_order(&NewOrder::new(&identifiers)).await?;
        let result = async {
            let mut auths = order.authorizations();
            while let Some(auth) = auths.next().await {
                let mut auth = auth?;
                if auth.status == AuthorizationStatus::Valid {
                    continue;
                }
                let mut challenge = auth
                    .challenge(ChallengeType::Dns01)
                    .ok_or(Error::Contract("DNS-01 unavailable"))?;
                let value = challenge.key_authorization().dns_value();
                dns(identity, client, &value).await?;
                challenge.set_ready().await?;
            }
            order.poll_ready(&RetryPolicy::default()).await?;
            let key_pem = load_key(&key_path)?;
            let key = rcgen::KeyPair::from_pem(&key_pem)?;
            let csr = certificate_request(host, &key)?;
            order.finalize_csr(csr.der()).await?;
            let pem = order.poll_certificate(&RetryPolicy::default()).await?;
            let fingerprint = self.install(&pem, &key_pem, host)?;
            report(identity, client, &fingerprint).await?;
            state::atomic(&cert_path, pem.as_bytes())?;
            Ok(())
        }
        .await;
        let cleanup = dns(identity, client, "").await;
        result.and(cleanup)
    }
}

fn load_key(path: &std::path::Path) -> Result<String> {
    use pkcs8::der::{Decode, EncodePem, asn1::ObjectIdentifier};
    let curve = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
    let key = match std::fs::read_to_string(path) {
        Ok(key) => key,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = rcgen::KeyPair::generate()?;
            let der = key.serialize_der();
            let info = pkcs8::PrivateKeyInfo::from_der(&der)
                .map_err(|_| Error::Contract("generated PKCS8"))?;
            let mut sec1 = sec1::EcPrivateKey::from_der(info.private_key)
                .map_err(|_| Error::Contract("generated SEC1"))?;
            sec1.parameters = Some(sec1::EcParameters::NamedCurve(curve));
            let pem = sec1
                .to_pem(sec1::LineEnding::LF)
                .map_err(|_| Error::Contract("SEC1 encode"))?;
            state::atomic(path, pem.as_bytes())?;
            return Ok(key.serialize_pem());
        }
        Err(e) => return Err(e.into()),
    };
    match PrivateKeyDer::from_pem_slice(key.as_bytes())? {
        PrivateKeyDer::Pkcs8(_) => Ok(rcgen::KeyPair::from_pem(&key)?.serialize_pem()),
        PrivateKeyDer::Sec1(der) => {
            let key = sec1::EcPrivateKey::from_der(der.secret_sec1_der())
                .map_err(|_| Error::Contract("invalid SEC1"))?;
            if key.parameters != Some(sec1::EcParameters::NamedCurve(curve)) {
                return Err(Error::Contract("key must use P256"));
            }
            let algorithm = pkcs8::AlgorithmIdentifierRef {
                oid: ObjectIdentifier::new_unwrap("1.2.840.10045.2.1"),
                parameters: Some((&curve).into()),
            };
            let info = pkcs8::PrivateKeyInfo::new(algorithm, der.secret_sec1_der());
            let pem = info
                .to_pem(sec1::LineEnding::LF)
                .map_err(|_| Error::Contract("PKCS8 encode"))?;
            Ok(rcgen::KeyPair::from_pem(&pem)?.serialize_pem())
        }
        _ => Err(Error::Contract("key must use P256")),
    }
}
async fn dns(id: &Identity, client: &reqwest::Client, value: &str) -> Result<()> {
    id.control(
        client,
        "/dns",
        &format!("dns:{value}"),
        serde_json::json!({"value": value}),
    )
    .await
}
async fn report(id: &Identity, client: &reqwest::Client, bytes: &[u8]) -> Result<()> {
    let fingerprint = bytes.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
        out
    });
    id.control(
        client,
        "/certificate",
        &format!("certificate:{fingerprint}"),
        serde_json::json!({"fingerprint":fingerprint}),
    )
    .await
}

fn certificate_request(
    host: &str,
    key: &rcgen::KeyPair,
) -> std::result::Result<rcgen::CertificateSigningRequest, rcgen::Error> {
    let mut params = rcgen::CertificateParams::new(vec![host.to_owned()])?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.serialize_request(key)
}

#[cfg(test)]
mod request_tests {
    #![allow(clippy::unwrap_used)]

    #[test]
    fn a_certificate_request_names_only_the_host() {
        let key = rcgen::KeyPair::generate().unwrap();
        let request = super::certificate_request("mac.ab.example.com", &key).unwrap();
        let der = request.der().as_ref();
        let contains = |text: &[u8]| der.windows(text.len()).any(|window| window == text);
        assert!(contains(b"mac.ab.example.com"));
        assert!(
            !contains(b"rcgen"),
            "Let's Encrypt refuses rcgen's default name"
        );
    }
}
