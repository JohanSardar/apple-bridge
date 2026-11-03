use crate::{Authority, Credentials, Error, Grant, Principal, effective_grants, fingerprint};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Client {
    #[serde(default)]
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: String,
    #[serde(default, deserialize_with = "nullable_strings")]
    pub grant_types: Vec<String>,
    #[serde(default, deserialize_with = "nullable_strings")]
    pub response_types: Vec<String>,
}
fn nullable_strings<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Code {
    pub hash: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub challenge: String,
    pub resource: String,
    pub principal: String,
    pub expires_at: String,
    pub used: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Access {
    pub hash: String,
    pub principal: String,
    pub resource: String,
    pub expires_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Refresh {
    pub hash: String,
    pub client_id: String,
    pub resource: String,
    pub principal: String,
    pub expires_at: String,
    pub used: bool,
}
#[derive(Debug, Serialize)]
pub struct Tokens {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u32,
    pub refresh_token: String,
    pub scope: String,
}
#[derive(Debug, Deserialize)]
pub struct Exchange {
    pub grant_type: String,
    pub client_id: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub code_verifier: String,
    pub resource: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub scope: String,
}
pub fn token(size: usize) -> String {
    let mut bytes = vec![0; size];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn expiry(seconds: i64) -> String {
    jiff::Timestamp::new(jiff::Timestamp::now().as_second() + seconds, 0)
        .map_or_else(|_| String::new(), |t| t.to_string())
}
pub fn live(expiry: &str) -> bool {
    expiry
        .parse::<jiff::Timestamp>()
        .is_ok_and(|t| t > jiff::Timestamp::now())
}
pub fn valid_redirect(raw: &str) -> bool {
    let Ok(u) = url::Url::parse(raw) else {
        return false;
    };
    raw.len() <= 2048
        && !raw.contains('#')
        && u.username().is_empty()
        && u.password().is_none()
        && u.host_str().is_some()
        && (u.scheme() == "https"
            || u.scheme() == "http"
                && u.host_str().is_some_and(|h| {
                    h == "localhost"
                        || h.trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }))
}
pub fn agent_grants(level: &str, scopes: &[Grant]) -> Result<Vec<Grant>, Error> {
    if !["read", "approve", "full"].contains(&level) || scopes.is_empty() {
        return Err(Error::Invalid);
    }
    scopes
        .iter()
        .map(|g| {
            let (source, action) = g.as_str().split_once(':').ok_or(Error::Invalid)?;
            if !["read", "write"].contains(&action) || source == "system" {
                return Err(Error::Invalid);
            }
            Grant::parse(&format!(
                "{source}:{}",
                if level == "read" { "read" } else { action }
            ))
        })
        .collect()
}
fn scope(p: &Principal) -> String {
    p.grants
        .iter()
        .map(Grant::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}
pub(crate) fn read<T: serde::de::DeserializeOwned>(
    d: &Credentials,
    key: &str,
) -> Result<Vec<T>, Error> {
    d.extra
        .get(key)
        .cloned()
        .map_or_else(|| Ok(Vec::new()), |v| Ok(serde_json::from_value(v)?))
}
pub(crate) fn write<T: Serialize>(d: &mut Credentials, key: &str, value: &T) -> Result<(), Error> {
    d.extra.insert(key.into(), serde_json::to_value(value)?);
    Ok(())
}
fn equal(a: &str, b: &str) -> bool {
    use hmac::Mac;
    let Ok(mut mac) = hmac::Hmac::<Sha256>::new_from_slice(b"apple-bridge-token-comparison") else {
        return false;
    };
    mac.update(a.as_bytes());
    let tag = mac.finalize().into_bytes();
    let Ok(mut mac) = hmac::Hmac::<Sha256>::new_from_slice(b"apple-bridge-token-comparison") else {
        return false;
    };
    mac.update(b.as_bytes());
    mac.verify_slice(&tag).is_ok()
}
impl Authority {
    pub fn register_oauth(&self, mut c: Client) -> Result<Client, Error> {
        if c.client_name.trim().is_empty()
            || c.client_name.len() > 120
            || c.redirect_uris.is_empty()
            || c.redirect_uris.len() > 10
            || !["", "none"].contains(&c.token_endpoint_auth_method.as_str())
            || !c.redirect_uris.iter().all(|s| valid_redirect(s))
            || !c
                .grant_types
                .iter()
                .all(|s| ["authorization_code", "refresh_token"].contains(&s.as_str()))
            || !c.response_types.iter().all(|s| s == "code")
        {
            return Err(Error::Invalid);
        }
        c.client_id = format!("client_{}", ulid::Ulid::new());
        c.token_endpoint_auth_method = "none".into();
        if c.grant_types.is_empty() {
            c.grant_types.push("authorization_code".into());
        }
        c.response_types = vec!["code".into()];
        self.update(|d| {
            let mut clients: Vec<Client> = read(d, "oauth_clients")?;
            if clients.len() >= 1000 {
                return Err(Error::Unavailable);
            }
            clients.push(c.clone());
            write(d, "oauth_clients", &clients)
        })?;
        Ok(c)
    }
    pub fn oauth_client(&self, id: &str, redirect: &str) -> Result<Client, Error> {
        read::<Client>(&self.load()?, "oauth_clients")?
            .into_iter()
            .find(|c| c.client_id == id && c.redirect_uris.iter().any(|u| u == redirect))
            .ok_or(Error::Invalid)
    }
    pub fn create_oauth_agent(
        &self,
        name: &str,
        level: &str,
        scopes: &[Grant],
    ) -> Result<(Principal, String), Error> {
        self.create_agent(
            name,
            match level {
                "read" => crate::AgentLevel::Read,
                "approve" => crate::AgentLevel::Approve,
                "full" => crate::AgentLevel::Full,
                _ => return Err(Error::Invalid),
            },
            agent_grants(level, scopes)?,
        )
    }
    pub fn issue_oauth_code(&self, mut c: Code) -> Result<String, Error> {
        let raw = token(32);
        c.hash = fingerprint(raw.as_bytes());
        c.expires_at = expiry(300);
        self.update(|d| {
            let mut codes: Vec<Code> = read(d, "oauth_codes")?;
            codes.retain(|c| live(&c.expires_at));
            if codes.len() >= 1000 {
                return Err(Error::Unavailable);
            }
            codes.push(c);
            write(d, "oauth_codes", &codes)
        })?;
        Ok(raw)
    }
    pub fn exchange_oauth(&self, e: &Exchange) -> Result<Tokens, Error> {
        let access = token(32);
        let next = token(32);
        self.update(|d| {
            let mut codes: Vec<Code> = read(d, "oauth_codes")?;
            let mut refreshes: Vec<Refresh> = read(d, "oauth_refresh")?;
            let mut expires = expiry(90 * 86400);
            let (principal, reused) = match e.grant_type.as_str() {
                "authorization_code" => {
                    let c = codes
                        .iter_mut()
                        .find(|c| c.hash == fingerprint(e.code.as_bytes()))
                        .ok_or(Error::Unauthorized)?;
                    let v = &e.code_verifier;
                    if c.client_id != e.client_id
                        || c.redirect_uri != e.redirect_uri
                        || c.resource != e.resource
                        || !live(&c.expires_at)
                        || !(43..=128).contains(&v.len())
                        || !v
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
                        || !equal(
                            &c.challenge,
                            &URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())),
                        )
                    {
                        return Err(Error::Unauthorized);
                    }
                    let used = c.used;
                    c.used = true;
                    (c.principal.clone(), used)
                }
                "refresh_token" => {
                    let r = refreshes
                        .iter_mut()
                        .find(|r| r.hash == fingerprint(e.refresh_token.as_bytes()))
                        .ok_or(Error::Unauthorized)?;
                    if r.client_id != e.client_id
                        || r.resource != e.resource
                        || !live(&r.expires_at)
                    {
                        return Err(Error::Unauthorized);
                    }
                    expires.clone_from(&r.expires_at);
                    let used = r.used;
                    r.used = true;
                    (r.principal.clone(), used)
                }
                _ => return Err(Error::Invalid),
            };
            if reused {
                if let Some(p) = d.principals.iter_mut().find(|p| p.id == principal) {
                    p.extra.insert(
                        "revoked_at".into(),
                        jiff::Timestamp::now().to_string().into(),
                    );
                    p.token_hash.clear();
                    p.revision += 1;
                }
                return Ok(Err(Error::Unauthorized));
            }
            let p = d
                .principals
                .iter_mut()
                .find(|p| p.id == principal && p.kind == "agent" && p.active())
                .ok_or(Error::Unauthorized)?;
            let scopes = scope(p);
            if !e.scope.is_empty() && e.scope != scopes {
                return Err(Error::Invalid);
            }
            p.token_hash.clear();
            p.extra.insert("resource".into(), e.resource.clone().into());
            let mut accesses: Vec<Access> = read(d, "oauth_access")?;
            accesses.retain(|a| live(&a.expires_at));
            accesses.push(Access {
                hash: fingerprint(access.as_bytes()),
                principal: principal.clone(),
                resource: e.resource.clone(),
                expires_at: expiry(3600),
            });
            refreshes.retain(|r| live(&r.expires_at));
            refreshes.push(Refresh {
                hash: fingerprint(next.as_bytes()),
                client_id: e.client_id.clone(),
                resource: e.resource.clone(),
                principal,
                expires_at: expires,
                used: false,
            });
            write(d, "oauth_codes", &codes)?;
            write(d, "oauth_refresh", &refreshes)?;
            write(d, "oauth_access", &accesses)?;
            Ok(Ok(Tokens {
                access_token: access,
                token_type: "Bearer".into(),
                expires_in: 3600,
                refresh_token: next,
                scope: scopes,
            }))
        })?
    }
    pub fn authenticate_oauth(&self, raw: &str) -> Result<Principal, Error> {
        let d = self.load()?;
        let hash = fingerprint(raw.as_bytes());
        let access = read::<Access>(&d, "oauth_access")?
            .into_iter()
            .find(|a| equal(&a.hash, &hash) && live(&a.expires_at));
        let mut p = d
            .principals
            .iter()
            .find(|p| {
                p.kind == "agent"
                    && p.active()
                    && (equal(&p.token_hash, &hash)
                        || access.as_ref().is_some_and(|a| a.principal == p.id))
            })
            .cloned()
            .ok_or(Error::Unauthorized)?;
        if let Some(a) = access {
            p.extra.insert("resource".into(), a.resource.into());
            p.extra.insert("expires_at".into(), a.expires_at.into());
        }
        if let Some(e) = d.enrollment {
            p.grants = effective_grants(&p.grants, &e.grants);
        }
        self.update(|d| {
            let current = d
                .principals
                .iter_mut()
                .find(|current| current.id == p.id && current.active())
                .ok_or(Error::Unauthorized)?;
            let now = serde_json::json!(jiff::Timestamp::now().to_string());
            current.extra.insert("last_used".into(), now.clone());
            p.extra.insert("last_used".into(), now);
            Ok(())
        })?;
        Ok(p)
    }
}

#[cfg(test)]
mod compatibility_tests {
    #[test]
    fn registration_accepts_null_optional_arrays() -> Result<(), serde_json::Error> {
        let client: super::Client = serde_json::from_str(
            r#"{"client_name":"fixture","redirect_uris":["https://connector.example/callback"],"grant_types":null,"response_types":null}"#,
        )?;
        assert_eq!(client.grant_types, Vec::<String>::new());
        assert_eq!(client.response_types, Vec::<String>::new());
        Ok(())
    }
}
