use crate::{Error, Result, http::DnsProvider, store::random_token};
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, Key, NewOrder, OrderStatus,
    RetryPolicy,
};
use rustls::pki_types::pem::PemObject;

use rustls::{
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct Certificates {
    current: RwLock<Option<Arc<CertifiedKey>>>,
}
impl ResolvesServerCert for Certificates {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.current.read().ok()?.clone()
    }
}
fn failed(reason: impl std::fmt::Display) -> Error {
    eprintln!("certificate request failed: {reason}");
    Error::Configuration
}
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or(Error::Configuration)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let tmp = dir.join(format!(".certificate-{}", random_token()?));
    let result = (|| {
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)?;
        File::open(dir)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result
}
fn private_key(path: &Path) -> Result<Vec<u8>> {
    use pkcs8::der::{Decode, Encode, EncodePem, asn1::ObjectIdentifier};
    let curve = ObjectIdentifier::new("1.2.840.10045.3.1.7").map_err(failed)?;
    let pem = match fs::read(path) {
        Ok(pem) => pem,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = rcgen::KeyPair::generate().map_err(failed)?;
            let der = key.serialize_der();
            let info = pkcs8::PrivateKeyInfo::from_der(&der).map_err(failed)?;
            let mut sec1 = sec1::EcPrivateKey::from_der(info.private_key).map_err(failed)?;
            sec1.parameters = Some(sec1::EcParameters::NamedCurve(curve));
            write_private(
                path,
                sec1.to_pem(sec1::LineEnding::LF)
                    .map_err(failed)?
                    .as_bytes(),
            )?;
            return Ok(der);
        }
        Err(e) => return Err(e.into()),
    };
    match rustls::pki_types::PrivateKeyDer::from_pem_slice(&pem).map_err(failed)? {
        rustls::pki_types::PrivateKeyDer::Pkcs8(key) => Ok(key.secret_pkcs8_der().to_vec()),
        rustls::pki_types::PrivateKeyDer::Sec1(key) => {
            let sec1 = sec1::EcPrivateKey::from_der(key.secret_sec1_der()).map_err(failed)?;
            if sec1.parameters != Some(sec1::EcParameters::NamedCurve(curve)) {
                return Err(Error::Configuration);
            }
            let algorithm = pkcs8::AlgorithmIdentifierRef {
                oid: ObjectIdentifier::new("1.2.840.10045.2.1").map_err(failed)?,
                parameters: Some((&curve).into()),
            };
            pkcs8::PrivateKeyInfo::new(algorithm, key.secret_sec1_der())
                .to_der()
                .map_err(failed)
        }
        _ => Err(Error::Configuration),
    }
}
fn load(path: &Path, host: &str, renewal: bool) -> Result<Arc<CertifiedKey>> {
    let cert = fs::read(path.join("relay-cert.pem"))?;
    let certs = rustls::pki_types::CertificateDer::pem_slice_iter(&cert)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(failed)?;
    let leaf = certs.first().ok_or(Error::Configuration)?;
    let (_, parsed) = x509_parser::parse_x509_certificate(leaf.as_ref()).map_err(failed)?;
    let now = jiff::Timestamp::now().as_second();
    if parsed.validity().not_before.timestamp() > now
        || parsed.validity().not_after.timestamp() <= now
        || (renewal && parsed.validity().not_after.timestamp() - now <= 30 * 86400)
    {
        return Err(Error::Configuration);
    }
    let names = parsed
        .subject_alternative_name()
        .map_err(failed)?
        .ok_or(Error::Configuration)?;
    if !names.value.general_names.iter().any(|n| matches!(n,x509_parser::extensions::GeneralName::DNSName(name) if name.eq_ignore_ascii_case(host))) { return Err(Error::Configuration) }
    let key =
        rustls::pki_types::PrivatePkcs8KeyDer::from(private_key(&path.join("relay-key.pem"))?);
    let signing = rustls::crypto::ring::sign::any_supported_type(&key.into()).map_err(failed)?;
    let cert = CertifiedKey::new(certs, signing);
    cert.keys_match().map_err(failed)?;
    Ok(Arc::new(cert))
}
async fn issue(path: &Path, host: &str, dns: &dyn DnsProvider, options: &Options) -> Result<()> {
    let account_key =
        rustls::pki_types::PrivatePkcs8KeyDer::from(private_key(&path.join("acme-account.pem"))?);
    let key = Key::from_pkcs8_der(account_key.clone_key()).map_err(failed)?;
    let builder = match &options.root {
        Some(path) => Account::builder_with_root(path),
        None => Account::builder(),
    }
    .map_err(failed)?;
    let (account, _) = builder
        .create_from_key((key, account_key.into()), options.directory.clone())
        .await
        .map_err(failed)?;
    let identifiers = [Identifier::Dns(host.to_owned())];
    let mut order = account
        .new_order(&NewOrder::new(&identifiers))
        .await
        .map_err(failed)?;
    let name = format!("_acme-challenge.{host}");
    let operation = async {
        let mut authorizations = order.authorizations();
        while let Some(auth) = authorizations.next().await {
            let mut auth = auth.map_err(failed)?;
            if auth.status == AuthorizationStatus::Valid {
                continue;
            }
            if auth.status != AuthorizationStatus::Pending {
                eprintln!("certificate authorization for {host} is {:?}", auth.status);
                return Err(Error::Denied);
            }
            let mut challenge = auth.challenge(ChallengeType::Dns01).ok_or(Error::Denied)?;
            dns.txt(&name, &challenge.key_authorization().dns_value())
                .await
                .inspect_err(|error| eprintln!("certificate DNS record for {host}: {error}"))?;
            challenge.set_ready().await.map_err(failed)?;
        }
        let status = order
            .poll_ready(&RetryPolicy::default())
            .await
            .map_err(failed)?;
        if status != OrderStatus::Ready {
            eprintln!("certificate order for {host} is {status:?}");
            return Err(Error::Denied);
        }
        let key =
            rustls::pki_types::PrivatePkcs8KeyDer::from(private_key(&path.join("relay-key.pem"))?);
        let key = rcgen::KeyPair::try_from(&key).map_err(failed)?;
        let request = certificate_request(host, &key).map_err(failed)?;
        order.finalize_csr(request.der()).await.map_err(failed)?;
        let chain = order
            .poll_certificate(&RetryPolicy::default())
            .await
            .map_err(failed)?;
        write_private(&path.join("relay-cert.pem"), chain.as_bytes())
    };
    let result = tokio::time::timeout(Duration::from_secs(300), operation)
        .await
        .map_err(|_| {
            eprintln!("certificate request for {host} timed out");
            Error::Denied
        });
    let _ = tokio::time::timeout(Duration::from_secs(15), dns.txt(&name, "")).await;
    result?
}
fn certificate_request(
    host: &str,
    key: &rcgen::KeyPair,
) -> std::result::Result<rcgen::CertificateSigningRequest, rcgen::Error> {
    let mut params = rcgen::CertificateParams::new(vec![host.to_owned()])?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.serialize_request(key)
}
#[derive(Debug, Clone)]
pub struct Options {
    pub directory: String,
    pub root: Option<PathBuf>,
}
pub async fn start(
    path: PathBuf,
    host: String,
    dns: Arc<dyn DnsProvider>,
    stop: CancellationToken,
) -> Result<Arc<Certificates>> {
    let options = Options {
        directory: std::env::var("APPLE_BRIDGE_ACME_DIRECTORY")
            .unwrap_or_else(|_| "https://acme-v02.api.letsencrypt.org/directory".into()),
        root: None,
    };
    start_with_options(path, host, dns, stop, options).await
}
pub async fn start_with_options(
    path: PathBuf,
    host: String,
    dns: Arc<dyn DnsProvider>,
    stop: CancellationToken,
    options: Options,
) -> Result<Arc<Certificates>> {
    if !crate::hello::valid_host(&host) {
        return Err(Error::Configuration);
    }
    if load(&path, &host, true).is_err() {
        issue(&path, &host, dns.as_ref(), &options).await?;
    }
    let certificates = Arc::new(Certificates {
        current: RwLock::new(Some(load(&path, &host, false)?)),
    });
    let current = certificates.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! { () = stop.cancelled() => return, () = tokio::time::sleep(Duration::from_hours(12)) => {} }
            if load(&path, &host, true).is_err() {
                let result = tokio::select! { () = stop.cancelled() => return, result = issue(&path,&host,dns.as_ref(),&options) => result };
                if result.is_ok()
                    && let Ok(cert) = load(&path, &host, false)
                    && let Ok(mut guard) = current.current.write()
                {
                    *guard = Some(cert);
                }
            }
        }
    });
    Ok(certificates)
}

#[cfg(test)]
mod request_tests {
    #![allow(clippy::unwrap_used)]

    #[test]
    fn a_certificate_request_names_only_the_host() {
        let key = rcgen::KeyPair::generate().unwrap();
        let request = super::certificate_request("ab.example.com", &key).unwrap();
        let der = request.der().as_ref();
        let contains = |text: &[u8]| der.windows(text.len()).any(|window| window == text);
        assert!(contains(b"ab.example.com"));
        assert!(
            !contains(b"rcgen"),
            "Let's Encrypt refuses rcgen's default name"
        );
    }
}
