#![forbid(unsafe_code)]

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("R2 certificate protection disabled")]
    Disabled,
    #[error("invalid CAA binding or Mac proof")]
    Invalid,
    #[error("DNS transaction failed: {0}")]
    Dns(String),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaaRecord {
    pub flags: u8,
    pub tag: String,
    pub value: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub host: String,
    pub account_uri: String,
    pub mac_key: [u8; 32],
    pub recovery_key: [u8; 32],
    pub revision: u64,
}
impl Binding {
    pub fn records(&self) -> [CaaRecord; 2] {
        [
            CaaRecord {
                flags: 0,
                tag: "issue".into(),
                value: format!(
                    "letsencrypt.org; accounturi={}; validationmethods=dns-01",
                    self.account_uri
                ),
            },
            CaaRecord {
                flags: 0,
                tag: "issuewild".into(),
                value: ";".into(),
            },
        ]
    }
    fn validate(&self) -> Result<()> {
        let uri = url::Url::parse(&self.account_uri).map_err(|_| Error::Invalid)?;
        if uri.scheme() != "https"
            || uri.host_str().is_none()
            || !uri.username().is_empty()
            || uri.password().is_some()
            || uri.fragment().is_some()
            || uri.path() == "/"
        {
            return Err(Error::Invalid);
        }
        if self.host.len() > 253
            || self.host.split('.').count() < 2
            || self.host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
            || !self.account_uri.starts_with("https://")
            || self.account_uri.len() > 512
            || self
                .account_uri
                .bytes()
                .any(|b| b <= b' ' || b >= 127 || matches!(b, b';' | b'"' | b'\\' | b'#'))
            || self.account_uri[8..].is_empty()
            || VerifyingKey::from_bytes(&self.mac_key).is_err()
            || VerifyingKey::from_bytes(&self.recovery_key).is_err()
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Operation {
    Enroll,
    Rotate,
    Recover,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub operation: Operation,
    pub binding: Binding,
    pub expires: u64,
}
impl Change {
    pub fn transcript(&self) -> Result<Vec<u8>> {
        let mut bytes = b"apple-bridge-caa/2\0".to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(|_| Error::Invalid)?);
        Ok(bytes)
    }
}

pub trait DnsAdapter: std::fmt::Debug {
    fn binding(&self, host: &str) -> Result<Option<Binding>>;
    fn commit(
        &mut self,
        previous: Option<&Binding>,
        next: &Binding,
        records: &[CaaRecord],
    ) -> Result<()>;
}

#[derive(Debug)]
pub struct Control<D> {
    dns: D,
    enabled: bool,
}
impl<D: DnsAdapter> Control<D> {
    pub fn new(dns: D, enabled: bool) -> Self {
        Self { dns, enabled }
    }
    pub fn adapter(&self) -> &D {
        &self.dns
    }
    pub fn apply(
        &mut self,
        change: &Change,
        signature: &[u8],
        enrolled_mac: Option<[u8; 32]>,
        now: u64,
    ) -> Result<()> {
        if !cfg!(feature = "r2-certificates") || !self.enabled {
            return Err(Error::Disabled);
        }
        change.binding.validate()?;
        if change.expires < now || change.expires.saturating_sub(now) > 300 {
            return Err(Error::Invalid);
        }
        let previous = self.dns.binding(&change.binding.host)?;
        let key = match (&previous, change.operation) {
            (None, Operation::Enroll)
                if change.binding.revision == 0 && enrolled_mac == Some(change.binding.mac_key) =>
            {
                change.binding.mac_key
            }
            (Some(old), Operation::Rotate)
                if old.revision.checked_add(1) == Some(change.binding.revision) =>
            {
                old.mac_key
            }
            (Some(old), Operation::Recover)
                if old.revision.checked_add(1) == Some(change.binding.revision) =>
            {
                old.recovery_key
            }
            _ => return Err(Error::Invalid),
        };
        VerifyingKey::from_bytes(&key)
            .map_err(|_| Error::Invalid)?
            .verify_strict(
                &change.transcript()?,
                &Signature::from_slice(signature).map_err(|_| Error::Invalid)?,
            )
            .map_err(|_| Error::Invalid)?;
        self.dns.commit(
            previous.as_ref(),
            &change.binding,
            &change.binding.records(),
        )
    }
}
