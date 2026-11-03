use crate::{Error, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PKCS_ED25519,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const HOST: &str = "localhost";

pub struct Ca {
    issuer: Issuer<'static, KeyPair>,
    pub root_pem: String,
}
#[derive(Debug, Clone)]
pub struct Leaf {
    pub chain: PathBuf,
    pub key: PathBuf,
}
fn certificate(error: impl std::fmt::Display) -> Error {
    Error::Certificate(error.to_string())
}
impl std::fmt::Debug for Ca {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ca").finish_non_exhaustive()
    }
}
impl Ca {
    pub fn new() -> Result<Self> {
        let key = KeyPair::generate_for(&PKCS_ED25519).map_err(certificate)?;
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(certificate)?;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(DnType::CommonName, "Apple Bridge Lab Relay Test CA");
        let now = time_now();
        params.not_before = now - time::Duration::hours(1);
        params.not_after = now + time::Duration::days(2);
        let root = params.self_signed(&key).map_err(certificate)?;
        Ok(Self {
            root_pem: root.pem(),
            issuer: Issuer::new(params, key),
        })
    }
    pub fn leaf(&self) -> Result<(String, String)> {
        let key = KeyPair::generate_for(&PKCS_ED25519).map_err(certificate)?;
        let mut params = CertificateParams::new(vec![HOST.to_owned()]).map_err(certificate)?;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let now = time_now();
        params.not_before = now - time::Duration::hours(1);
        params.not_after = now + time::Duration::days(1);
        let leaf = params.signed_by(&key, &self.issuer).map_err(certificate)?;
        Ok((
            format!("{}{}", leaf.pem(), self.root_pem),
            key.serialize_pem(),
        ))
    }
    pub fn issue(&self, directory: &Path) -> Result<Leaf> {
        let (chain_pem, key_pem) = self.leaf()?;
        fs::create_dir_all(directory)?;
        let chain = directory.join("leaf-chain.pem");
        let key_path = directory.join("leaf-key.pem");
        write_private(&chain, chain_pem.as_bytes())?;
        write_private(&key_path, key_pem.as_bytes())?;
        Ok(Leaf {
            chain,
            key: key_path,
        })
    }
    pub fn roots(&self) -> Result<rustls::RootCertStore> {
        use rustls::pki_types::{CertificateDer, pem::PemObject};
        let mut store = rustls::RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(self.root_pem.as_bytes()) {
            store.add(cert.map_err(certificate)?).map_err(certificate)?;
        }
        Ok(store)
    }
}
fn time_now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc()
}
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
pub fn configure_gateway(home: &Path, listen: &str, leaf: &Leaf) -> Result<()> {
    fs::create_dir_all(home)?;
    let _ = fs::remove_file(home.join("run").join("lab-relay-addr"));
    write_private(
        &home.join("lab-relay.json"),
        &serde_json::to_vec(&serde_json::json!({
            "listen": listen,
            "cert": leaf.chain,
            "key": leaf.key,
        }))?,
    )
}
pub fn gateway_address(home: &Path, deadline: std::time::Duration) -> Result<String> {
    let started = std::time::Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(home.join("run").join("lab-relay-addr"))
            && !text.trim().is_empty()
        {
            return Ok(text.trim().to_owned());
        }
        if started.elapsed() > deadline {
            return Err(Error::Timeout);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
