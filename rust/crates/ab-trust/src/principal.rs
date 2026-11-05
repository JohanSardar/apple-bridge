use crate::Grant;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
    pub fn relay2(&self) -> bool {
        self.extra
            .get("relay2")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
    pub fn active(&self) -> bool {
        if self
            .extra
            .get("revoked_at")
            .is_some_and(|value| !value.is_null())
        {
            return false;
        }
        self.extra
            .get("expires_at")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|value| {
                value == "0001-01-01T00:00:00Z"
                    || value
                        .parse::<jiff::Timestamp>()
                        .is_ok_and(|timestamp| timestamp > jiff::Timestamp::now())
            })
    }
    pub fn visibility(&self) -> Visibility {
        let mut visibility = Visibility::default();
        visibility.principals.insert(self.id.clone());
        visibility.commands_all = self.kind == "replica";
        for grant in &self.grants {
            if grant.0 == "admin" {
                visibility.all = true;
                visibility.commands_all = true;
            }
            if let Some((source, _)) = grant.0.split_once(':') {
                visibility.sources.insert(source.into());
            }
        }
        visibility
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
            .and_then(|value| value.as_str().map(str::to_owned));
        if !self.all && !source.is_some_and(|source| self.sources.contains(&source)) {
            return false;
        }
        if record.collection != ab_domain::Collection::SystemCommand || self.commands_all {
            return true;
        }
        serde_json::from_str::<serde_json::Value>(record.data.get())
            .ok()
            .and_then(|value| {
                value
                    .get("principal")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .is_some_and(|principal| {
                self.principals.contains(&principal)
                    || principal
                        .split_once('/')
                        .is_some_and(|(_, local)| self.principals.contains(local))
            })
    }
}
impl Visibility {
    pub fn to_map(&self) -> std::collections::HashMap<String, bool> {
        let mut map: std::collections::HashMap<String, bool> = self
            .sources
            .iter()
            .map(|source| (source.clone(), true))
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoked_and_malformed_expiry_principals_are_inactive() {
        let mut principal = Principal::default();
        assert!(principal.active());
        principal
            .extra
            .insert("expires_at".into(), serde_json::json!("malformed"));
        assert!(!principal.active());
        principal.extra.insert(
            "expires_at".into(),
            serde_json::json!("0001-01-01T00:00:00Z"),
        );
        assert!(principal.active());
        principal.extra.insert(
            "revoked_at".into(),
            serde_json::json!("2020-01-01T00:00:00Z"),
        );
        assert!(!principal.active());
    }

    #[test]
    fn replica_command_visibility_does_not_grant_all_sources() {
        let principal = Principal {
            id: "replica:test".into(),
            kind: "replica".into(),
            grants: vec![Grant("messages:read".into())],
            ..Principal::default()
        };
        let visibility = principal.visibility();
        assert!(visibility.commands_all);
        assert!(!visibility.all);
        assert!(visibility.sources.contains("messages"));
        assert!(!visibility.sources.contains("system"));
        assert!(visibility.principals.contains("replica:test"));
    }
}
