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
use std::collections::{BTreeMap, BTreeSet};
pub use store::{CredentialStore, FileStore, KeychainStore, atomic_write};

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
    #[error("invalid identity: {0}")]
    Identity(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}
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
pub struct Principal {
    pub id: String,
    pub kind: String,
    pub grants: Vec<Grant>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dial: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl Principal {
    pub fn active(&self) -> bool {
        if self.extra.get("revoked_at").is_some_and(|v| !v.is_null()) {
            return false;
        }
        self.extra
            .get("expires_at")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|v| {
                v == "0001-01-01T00:00:00Z"
                    || v.parse::<jiff::Timestamp>()
                        .is_ok_and(|t| t > jiff::Timestamp::now())
            })
    }
    pub fn visibility(&self) -> Visibility {
        let mut v = Visibility::default();
        v.principals.insert(self.id.clone());
        v.commands_all = self.kind == "replica";
        for g in &self.grants {
            if g.0 == "admin" {
                v.all = true;
                v.commands_all = true;
            }
            if let Some((s, _)) = g.0.split_once(':') {
                v.sources.insert(s.into());
            }
        }
        v
    }
}
#[derive(Clone, Debug, Default)]
pub struct Visibility {
    pub sources: BTreeSet<String>,
    pub principals: BTreeSet<String>,
    pub commands_all: bool,
    pub all: bool,
}
impl Visibility {
    pub fn allows(&self, record: &ab_domain::Record) -> bool {
        let source = serde_json::to_value(record.source)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned));
        if !self.all && !source.is_some_and(|s| self.sources.contains(&s)) {
            return false;
        }
        if record.collection != ab_domain::Collection::SystemCommand || self.commands_all {
            return true;
        }
        serde_json::from_str::<serde_json::Value>(record.data.get())
            .ok()
            .and_then(|v| {
                v.get("principal")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|p| {
                self.principals.contains(&p)
                    || p.split_once('/')
                        .is_some_and(|(_, local)| self.principals.contains(local))
            })
    }
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
impl Visibility {
    pub fn to_map(&self) -> std::collections::HashMap<String, bool> {
        let mut map: std::collections::HashMap<String, bool> =
            self.sources.iter().map(|s| (s.clone(), true)).collect();
        if self.all {
            for source in [
                #[cfg(feature = "lab-harness")]
                "labdummy",
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
            ] {
                map.insert(source.into(), true);
            }
        }
        for id in &self.principals {
            map.insert(format!("principal:{id}"), true);
        }
        if self.commands_all {
            map.insert("commands:all".into(), true);
        }
        map
    }
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
