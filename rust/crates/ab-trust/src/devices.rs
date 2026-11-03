use crate::{
    Authority, Error, Grant, Principal, effective_grants, fingerprint,
    oauth::{expiry, live, read, token, write},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub device_id: String,
    pub kind: String,
    pub expires_at: String,
    pub cache: String,
    pub created_at: String,
    pub last_seen: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_hash: String,
}
pub(crate) fn valid(s: &Session) -> bool {
    live(&s.expires_at)
        && s.created_at.parse::<jiff::Timestamp>().is_ok_and(|t| {
            t.as_second()
                + if s.kind == "public" {
                    43200
                } else {
                    90 * 86400
                }
                > jiff::Timestamp::now().as_second()
        })
}
impl Authority {
    pub fn create_device_session(&self, kind: &str) -> Result<(String, Session), Error> {
        if !["trusted", "public"].contains(&kind) {
            return Err(Error::Invalid);
        }
        let raw = token(32);
        let now = jiff::Timestamp::now().to_string();
        let mut s = Session {
            device_id: format!("device:{}", ulid::Ulid::new()),
            kind: kind.into(),
            expires_at: expiry(if kind == "public" { 1800 } else { 30 * 86400 }),
            cache: if kind == "public" {
                "memory"
            } else {
                "persistent"
            }
            .into(),
            created_at: now.clone(),
            last_seen: now,
            token_hash: fingerprint(raw.as_bytes()),
        };
        self.update(|d| {
            let mut sessions: Vec<Session> = read(d, "devices")?;
            sessions.push(s.clone());
            d.principals.push(Principal {
                id: s.device_id.clone(),
                kind: "device".into(),
                grants: d.enrollment.as_ref().map_or_else(
                    || Grant::parse("admin").map(|g| vec![g]),
                    |e| Ok(e.grants.clone()),
                )?,
                revision: 1,
                ..Principal::default()
            });
            write(d, "devices", &sessions)
        })?;
        s.token_hash.clear();
        Ok((raw, s))
    }
    pub fn authenticate_device(&self, raw: &str) -> Result<(Principal, Session), Error> {
        self.update(|d| {
            let mut sessions: Vec<Session> = read(d, "devices")?;
            let s = sessions
                .iter_mut()
                .find(|s| s.token_hash == fingerprint(raw.as_bytes()) && valid(s))
                .ok_or(Error::Unauthorized)?;
            let mut p = d
                .principals
                .iter()
                .find(|p| p.id == s.device_id && p.active())
                .cloned()
                .ok_or(Error::Unauthorized)?;
            if let Some(e) = &d.enrollment {
                p.grants = effective_grants(&p.grants, &e.grants);
            }
            let created = s
                .created_at
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Invalid)?
                .as_second();
            let public = s.kind == "public";
            let until = (jiff::Timestamp::now().as_second()
                + if public { 1800 } else { 30 * 86400 })
            .min(created + if public { 43200 } else { 90 * 86400 });
            s.expires_at = jiff::Timestamp::new(until, 0)
                .map_err(|_| Error::Invalid)?
                .to_string();
            s.last_seen = jiff::Timestamp::now().to_string();
            let mut out = s.clone();
            out.token_hash.clear();
            write(d, "devices", &sessions)?;
            Ok((p, out))
        })
    }
    pub fn create_web_enroll(&self) -> Result<String, Error> {
        let raw = token(20);
        self.update(|d| {
            let mut tokens = d
                .extra
                .get("web_enroll")
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();
            tokens.retain(|_, v| v.as_str().is_some_and(live));
            tokens.insert(fingerprint(raw.as_bytes()), expiry(600).into());
            d.extra.insert("web_enroll".into(), tokens.into());
            Ok(())
        })?;
        Ok(raw)
    }
    pub fn check_web_enroll(&self, raw: &str) -> Result<(), Error> {
        if self
            .load()?
            .extra
            .get("web_enroll")
            .and_then(|v| v.get(fingerprint(raw.as_bytes())))
            .and_then(serde_json::Value::as_str)
            .is_some_and(live)
        {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }
}
