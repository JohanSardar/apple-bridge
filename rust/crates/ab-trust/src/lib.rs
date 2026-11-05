#![forbid(unsafe_code)]
pub mod activity;
pub mod devices;
mod gateway;
mod identity;
pub mod iphone;
pub mod oauth;
pub use gateway::{AgentLevel, VerifiedDelegation};
mod pairing;
pub mod passkeys;
mod store;
pub mod tls;
pub use identity::Authority;
pub use pairing::{Enrollment, PairCode, pair_proof};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub use store::{
    CredentialStore, FileStore, KeychainStore, LibsecretStore, StoreEnvironment, atomic_write,
    backend_name, select,
};

pub fn decision_now() -> jiff::Timestamp {
    #[cfg(feature = "lab-harness")]
    if let Some(path) = std::env::var_os("APPLE_BRIDGE_LAB_REPLAY") {
        let raw =
            std::fs::read(path).unwrap_or_else(|_| panic!("lab replay decisions unavailable"));
        let value: serde_json::Value =
            serde_json::from_slice(&raw).unwrap_or_else(|_| panic!("invalid lab replay decisions"));
        return value["now"]
            .as_str()
            .and_then(|now| now.parse().ok())
            .unwrap_or_else(|| panic!("invalid lab replay clock"));
    }
    jiff::Timestamp::now()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unauthorized")]
    Unauthorized,
    #[error(
        "unsupported stored passkey algorithm {0}; re-enrol this passkey using ES256 or Ed25519 before starting the Rust gateway"
    )]
    UnsupportedPasskey(i128),
    #[error("invalid input")]
    Invalid,
    #[error("credential store unavailable")]
    Unavailable,
    #[error(transparent)]
    CredentialStore(#[from] CredentialStoreError),
    #[error("invalid identity: {0}")]
    Identity(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialStoreError {
    pub code: String,
    pub store: String,
    pub remedy: String,
    pub retryable: bool,
}
impl std::fmt::Display for CredentialStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} credential store unavailable; {}",
            self.code, self.store, self.remedy
        )
    }
}
impl std::error::Error for CredentialStoreError {}
pub fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrincipalId(pub String);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Grant(String);
impl Grant {
    const SOURCES: &'static [&'static str] = &[
        "location",
        "audio",
        "calendar",
        "capture",
        "contacts",
        "files",
        "focus",
        "health",
        "iphone",
        "mac",
        "mail",
        "maps",
        "messages",
        "notes",
        "pdf",
        "phone",
        "print",
        "reminders",
        "safari",
        "shortcuts",
        "speech",
        "system",
        "usage",
        "vision",
        "voice_memos",
        "voicemail",
        #[cfg(feature = "lab-harness")]
        "labdummy",
    ];
    pub fn agent_scopes() -> Vec<String> {
        Self::SOURCES
            .iter()
            .filter(|source| **source != "system")
            .flat_map(|source| [format!("{source}:read"), format!("{source}:write")])
            .collect()
    }
    pub fn parse(value: &str) -> Result<Self, Error> {
        if value == "admin"
            || value
                .split_once(':')
                .is_some_and(|(s, a)| Self::SOURCES.contains(&s) && ["read", "write"].contains(&a))
        {
            Ok(Self(value.into()))
        } else {
            Err(Error::Invalid)
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for Grant {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        Self::parse(&value)
    }
}
impl From<Grant> for String {
    fn from(value: Grant) -> Self {
        value.0
    }
}
pub fn subset(grants: &[Grant], ceiling: &[Grant]) -> bool {
    grants.iter().all(|g| {
        ceiling.iter().any(|c| {
            c.0 == "admin" || c == g || format!("{}:write", g.0.trim_end_matches(":read")) == c.0
        })
    })
}
pub fn effective_grants(grants: &[Grant], ceiling: &[Grant]) -> Vec<Grant> {
    grants
        .iter()
        .filter_map(|g| {
            if subset(std::slice::from_ref(g), ceiling) {
                Some(g.clone())
            } else if let Some(source) = g.0.strip_suffix(":write") {
                let read = Grant(format!("{source}:read"));
                subset(std::slice::from_ref(&read), ceiling).then_some(read)
            } else {
                None
            }
        })
        .collect()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default, deserialize_with = "null_default")]
    pub principals: Vec<Principal>,
    #[serde(default)]
    pub pairings: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrollment: Option<Enrollment>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
pub fn parse_grants(scope: &str) -> Result<Vec<Grant>, Error> {
    if scope.is_empty() {
        return Err(Error::Invalid);
    }
    scope.split(',').map(Grant::parse).collect()
}

pub use ab_domain as domain;
pub use ab_wire as wire;

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

pub type Reauthorization<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Principal, Error>> + Send + 'a>>;

pub trait ApprovalReauthorizer: std::fmt::Debug + Send + Sync {
    fn reauthorize<'a>(
        &'a self,
        principal: &'a str,
        command: &'a str,
        hash: &'a str,
    ) -> Reauthorization<'a>;
}

mod principal;
pub use principal::{Principal, Visibility};
