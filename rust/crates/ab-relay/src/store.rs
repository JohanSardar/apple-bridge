use crate::{Error, Result, hello::valid_host};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const NONCE_LIFETIME: Duration = Duration::from_secs(60);
const NONCE_CAPACITY: usize = 4096;

#[derive(Debug, Default)]
struct NoncePool {
    expiries: HashMap<String, Instant>,
    issued: VecDeque<String>,
}
#[derive(Debug, Default)]
pub struct Nonces(Mutex<NoncePool>);
impl Nonces {
    pub fn issue(&self) -> Result<String> {
        let nonce = random_token()?;
        let now = Instant::now();
        let mut pool = self.0.lock().map_err(|_| Error::Denied)?;
        let NoncePool { expiries, issued } = &mut *pool;
        while issued
            .front()
            .is_some_and(|front| expiries.get(front).is_none_or(|until| *until < now))
        {
            if let Some(front) = issued.pop_front() {
                expiries.remove(&front);
            }
        }
        while expiries.len() >= NONCE_CAPACITY {
            if let Some(oldest) = issued.pop_front() {
                expiries.remove(&oldest);
            }
        }
        if issued.len() >= 2 * NONCE_CAPACITY {
            issued.retain(|nonce| expiries.contains_key(nonce));
        }
        expiries.insert(nonce.clone(), now + NONCE_LIFETIME);
        issued.push_back(nonce.clone());
        Ok(nonce)
    }
    #[must_use]
    pub fn contains(&self, nonce: &str) -> bool {
        self.0
            .lock()
            .is_ok_and(|pool| pool.expiries.contains_key(nonce))
    }
    fn take(&self, nonce: &str) -> Option<Instant> {
        self.0.lock().ok()?.expiries.remove(nonce)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub connections: i64,
    pub bytes_per_second: i64,
    pub monthly_bytes: i64,
}
impl Limits {
    pub fn valid(self) -> bool {
        self.connections > 0 && self.bytes_per_second > 0 && self.monthly_bytes > 0
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub certificates: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub domain: String,
    pub public_key: String,
    pub hostname: String,
    pub credential_hash: String,
    pub revoked: bool,
    pub disabled: bool,
    pub limits: Limits,
    pub month: String,
    pub bytes: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub policy_version: i64,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &i64) -> bool {
    *value == 0
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Enrollment {
    pub public_key: String,
    pub nonce: String,
    pub signature: String,
    pub hostname: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub hostname: String,
    pub credential: String,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Proof {
    pub hostname: String,
    pub credential: String,
    pub nonce: String,
    pub signature: String,
}
impl Proof {
    pub fn message(&self, action: &str) -> String {
        format!(
            "apple-bridge-relay/1\n{action}\n{}\n{}\n{}",
            self.hostname, self.credential, self.nonce
        )
    }
}
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    pub zone: String,
    defaults: Limits,
    pub devices: BTreeMap<String, Device>,
    pub nonces: Arc<Nonces>,
    owner_keys: Option<PathBuf>,
}
pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub(crate) fn random_token() -> Result<String> {
    let mut b = [0; 32];
    getrandom::fill(&mut b).map_err(|e| Error::Random(e.to_string()))?;
    Ok(B64.encode(b))
}
pub fn month_at(timestamp: jiff::Timestamp) -> String {
    timestamp
        .to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%Y-%m")
        .to_string()
}
pub fn month() -> String {
    month_at(jiff::Timestamp::now())
}
fn verify(key: &str, sig: &str, message: &[u8]) -> bool {
    let Ok(raw) = B64.decode(key) else {
        return false;
    };
    if B64.encode(&raw) != key {
        return false;
    }
    let Ok(bytes) = <[u8; 32]>::try_from(raw.as_slice()) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&bytes) else {
        return false;
    };
    let Ok(raw) = B64.decode(sig) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(&raw) else {
        return false;
    };
    key.verify_strict(message, &sig).is_ok()
}
impl Store {
    pub fn open(path: PathBuf, zone: &str, defaults: Limits) -> Result<Self> {
        let zone = zone.strip_suffix('.').unwrap_or(zone).to_lowercase();
        if path.as_os_str().is_empty()
            || zone.len() > 228
            || !valid_host(&zone)
            || !defaults.valid()
        {
            return Err(Error::Configuration);
        }
        let devices: BTreeMap<String, Device> = match fs::read(&path) {
            Ok(data) => serde_json::from_slice(&data)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        for (host, d) in &devices {
            if !valid_host(host)
                || d.hostname != *host
                || d.bytes < 0
                || !d.limits.valid()
                || !host.ends_with(&format!(".{zone}"))
            {
                return Err(Error::Configuration);
            }
        }
        Ok(Self {
            path,
            zone,
            defaults,
            devices,
            nonces: Arc::default(),
            owner_keys: None,
        })
    }
    pub fn require_owner_keys(&mut self, path: PathBuf) {
        self.owner_keys = Some(path);
    }
    pub fn validate_owner_keys(&self) -> Result<Vec<String>> {
        let path = self.owner_keys.as_ref().ok_or(Error::Denied)?;
        let data = fs::read(path)?;
        if data.len() > 16384 {
            return Err(Error::Denied);
        }
        let text = std::str::from_utf8(&data).map_err(|_| Error::Denied)?;
        let mut keys = Vec::new();
        for key in text.split_whitespace() {
            let raw = B64.decode(key).map_err(|_| Error::Denied)?;
            if raw.len() != 32 || B64.encode(&raw) != key {
                return Err(Error::Denied);
            }
            keys.push(key.to_owned());
        }
        if keys.is_empty() {
            return Err(Error::Denied);
        }
        Ok(keys)
    }
    pub fn save(&self) -> Result<()> {
        let dir = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        let path = dir.join(format!(".relay-{}", random_token()?));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            f.write_all(&serde_json::to_vec(&self.devices)?)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&path, &self.path)?;
            File::open(dir)?.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(path);
        result
    }
    pub fn challenge(&mut self) -> Result<String> {
        self.nonces.issue()
    }
    pub fn enroll(&mut self, input: Enrollment) -> Result<Credential> {
        let expiry = self.nonces.take(&input.nonce).ok_or(Error::Denied)?;
        let message = format!("apple-bridge-relay-enroll\n{}\n{}", self.zone, input.nonce);
        if Instant::now() > expiry
            || !verify(&input.public_key, &input.signature, message.as_bytes())
        {
            return Err(Error::Denied);
        }
        if self.owner_keys.is_some()
            && !self
                .validate_owner_keys()
                .is_ok_and(|keys| keys.contains(&input.public_key))
        {
            return Err(Error::Denied);
        }
        self.issue(input.public_key, &input.hostname)
    }
    pub fn admit(&mut self, public_key: &str) -> Result<Credential> {
        let key = B64.decode(public_key).map_err(|_| Error::Denied)?;
        if key.len() != 32 {
            return Err(Error::Denied);
        }
        self.issue(public_key.to_owned(), "")
    }
    fn issue(&mut self, public_key: String, hostname: &str) -> Result<Credential> {
        let mut host = self
            .devices
            .iter()
            .find(|(_, d)| d.public_key == public_key)
            .map(|(h, _)| h.clone())
            .unwrap_or_default();
        if !hostname.is_empty() && hostname != host {
            return Err(Error::Denied);
        }
        if host.is_empty() {
            loop {
                host = format!("{}.{}", &digest(&random_token()?)[..32], self.zone);
                if !self.devices.contains_key(&host) {
                    break;
                }
            }
        }
        let old = self.devices.get(&host).cloned();
        let mut device = old.clone().unwrap_or(Device {
            public_key,
            hostname: host.clone(),
            limits: self.defaults,
            month: month(),
            ..Device::default()
        });
        if device.revoked {
            return Err(Error::Denied);
        }
        let token = random_token()?;
        device.credential_hash = digest(&token);
        self.commit(host.clone(), device, old)?;
        Ok(Credential {
            hostname: host,
            credential: token,
        })
    }
    fn commit(&mut self, host: String, device: Device, old: Option<Device>) -> Result<()> {
        self.devices.insert(host.clone(), device);
        if let Err(e) = self.save() {
            if let Some(d) = old {
                self.devices.insert(host, d);
            } else {
                self.devices.remove(&host);
            }
            return Err(e);
        }
        Ok(())
    }
    pub fn authenticated(&self, host: &str, token: &str) -> bool {
        self.devices
            .get(host)
            .is_some_and(|d| !d.revoked && !d.disabled && d.credential_hash == digest(token))
    }
    pub fn verify(&mut self, proof: &Proof, action: &str) -> bool {
        let Some(expiry) = self.nonces.take(&proof.nonce) else {
            return false;
        };
        self.devices.get(&proof.hostname).is_some_and(|d| {
            Instant::now() < expiry
                && self.authenticated(&proof.hostname, &proof.credential)
                && verify(
                    &d.public_key,
                    &proof.signature,
                    proof.message(action).as_bytes(),
                )
        })
    }
    pub fn policy(
        &mut self,
        host: &str,
        limits: Limits,
        disabled: bool,
        revoked: bool,
    ) -> Result<()> {
        self.versioned_policy(host, limits, disabled, revoked, 0)
            .map(|_| ())
    }
    pub(crate) fn versioned_policy(
        &mut self,
        host: &str,
        limits: Limits,
        disabled: bool,
        revoked: bool,
        version: i64,
    ) -> Result<bool> {
        let old = self.devices.get(host).cloned().ok_or(Error::Denied)?;
        if version < old.policy_version && version != 0 {
            return Ok(false);
        }
        let limits = if limits == Limits::default() {
            old.limits
        } else {
            limits
        };
        if !limits.valid() {
            return Err(Error::Denied);
        }
        let mut d = old.clone();
        d.limits = limits;
        d.disabled = disabled;
        d.revoked |= revoked;
        d.policy_version = d.policy_version.max(version);
        self.commit(host.to_owned(), d, Some(old))?;
        Ok(true)
    }
    pub(crate) fn charge(&mut self, host: &str, size: usize) -> Result<usize> {
        let old = self.devices.get(host).cloned().ok_or(Error::Denied)?;
        if old.disabled || old.revoked {
            return Err(Error::Denied);
        }
        let mut d = old.clone();
        let month = month();
        if d.month != month {
            d.month = month;
            d.bytes = 0;
        }
        let remaining = d.limits.monthly_bytes - d.bytes;
        if remaining <= 0 {
            return Err(Error::Denied);
        }
        let n = size.min(usize::try_from(remaining).unwrap_or(usize::MAX));
        d.bytes += i64::try_from(n).map_err(|_| Error::Denied)?;
        self.commit(host.to_owned(), d, Some(old))?;
        Ok(n)
    }
    pub fn canonical(&self, host: &str) -> String {
        if self.devices.contains_key(host) {
            return host.to_owned();
        }
        self.devices
            .iter()
            .find(|(_, d)| d.domain == host)
            .map_or_else(|| host.to_owned(), |(h, _)| h.clone())
    }
    pub fn set_domain(&mut self, host: &str, domain: &str) -> Result<()> {
        if self
            .devices
            .iter()
            .any(|(h, d)| (h == domain || d.domain == domain) && h != host)
        {
            return Err(Error::Denied);
        }
        let old = self.devices.get(host).cloned().ok_or(Error::Denied)?;
        let mut d = old.clone();
        domain.clone_into(&mut d.domain);
        self.commit(host.to_owned(), d, Some(old))
    }
    pub fn certificate(&mut self, host: &str, fingerprint: &str) -> Result<bool> {
        let old = self.devices.get(host).cloned().ok_or(Error::Denied)?;
        if old.certificates.iter().any(|f| f == fingerprint) {
            return Ok(true);
        }
        if old.certificates.len() >= 128 {
            return Ok(false);
        }
        let mut d = old.clone();
        d.certificates.push(fingerprint.to_owned());
        self.commit(host.to_owned(), d, Some(old))?;
        Ok(true)
    }
}
