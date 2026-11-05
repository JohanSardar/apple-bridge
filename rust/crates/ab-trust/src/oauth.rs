use crate::{Authority, Credentials, Error, Grant, Principal, effective_grants, fingerprint};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Client {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: String,
    #[serde(default, deserialize_with = "nullable_strings")]
    pub grant_types: Vec<String>,
    #[serde(default, deserialize_with = "nullable_strings")]
    pub response_types: Vec<String>,
    #[serde(default)]
    pub client_id_issued_at: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub used: bool,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
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
    #[serde(default)]
    pub rotated_at: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub retry_salt: String,
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
            || u.scheme() == "http" && u.host_str().is_some_and(loopback_host))
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
    Sha256::digest(a.as_bytes())
        .ct_eq(&Sha256::digest(b.as_bytes()))
        .into()
}
fn pkce_verified(challenge: &str, verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        && equal(
            challenge,
            oauth2::PkceCodeChallenge::from_code_verifier_sha256(&oauth2::PkceCodeVerifier::new(
                verifier.to_owned(),
            ))
            .as_str(),
        )
}
fn loopback_host(host: &str) -> bool {
    host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
pub fn redirect_matches(registered: &str, presented: &str) -> bool {
    if registered == presented {
        return true;
    }
    let (Ok(registered), Ok(presented)) = (url::Url::parse(registered), url::Url::parse(presented))
    else {
        return false;
    };
    registered.scheme() == "http"
        && presented.scheme() == "http"
        && registered.host_str() == presented.host_str()
        && registered.host_str().is_some_and(loopback_host)
        && registered.path() == presented.path()
        && registered.query() == presented.query()
}
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{61c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e0000}'..='\u{e007f}'
    )
}
pub fn normalized_name(raw: &str) -> String {
    let visible: String = raw
        .chars()
        .filter(|c| !invisible(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    let bounded: String = words.chars().take(MAX_DISPLAY_NAME_CHARS).collect();
    bounded.trim_end().to_owned()
}
pub fn callback_label(authority: &str) -> String {
    let loopback = url::Url::parse(&format!("http://{authority}"))
        .ok()
        .and_then(|u| u.host_str().map(loopback_host))
        .unwrap_or(false);
    if loopback {
        "Returns to an app on this computer. Allow it only if you just started this.".to_owned()
    } else {
        format!("Returns to {authority}")
    }
}
pub fn display_name(client: &Client) -> String {
    let name = normalized_name(&client.client_name);
    if !name.is_empty() {
        return name;
    }
    client
        .redirect_uris
        .first()
        .and_then(|uri| url::Url::parse(uri).ok())
        .and_then(|uri| uri.host_str().map(str::to_owned))
        .unwrap_or_else(|| "Unnamed app".to_owned())
}
pub const OFFLINE_ACCESS: &str = "offline_access";
const REFRESH_RETRY_SECONDS: i64 = 60;
const MAX_CLIENTS: usize = 1000;
const MAX_PENDING_CLIENTS: usize = 64;
const PENDING_CLIENT_SECONDS: i64 = 3600;
const MAX_CLIENT_METADATA_BYTES: usize = 4096;
const MAX_DISPLAY_NAME_CHARS: usize = 64;
const LAST_USED_RESOLUTION_SECONDS: i64 = 300;
struct Spent {
    principal: String,
    reused: bool,
    retry_salt: Option<String>,
    refresh_expires: String,
}
fn successor(spent: &str, salt: &str, label: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"apple-bridge oauth successor\0");
    hash.update(label.as_bytes());
    hash.update(b"\0");
    hash.update(salt.as_bytes());
    hash.update(b"\0");
    hash.update(spent.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.finalize())
}
pub(crate) fn recently_used(p: &Principal) -> bool {
    p.extra
        .get("last_used")
        .and_then(serde_json::Value::as_str)
        .and_then(|used| used.parse::<jiff::Timestamp>().ok())
        .is_some_and(|used| {
            let age = jiff::Timestamp::now().as_second() - used.as_second();
            (0..LAST_USED_RESOLUTION_SECONDS).contains(&age)
        })
}
fn spend_code(codes: &mut [Code], e: &Exchange) -> Result<Spent, Error> {
    let c = codes
        .iter_mut()
        .find(|c| c.hash == fingerprint(e.code.as_bytes()))
        .ok_or(Error::Unauthorized)?;
    if c.client_id != e.client_id
        || c.redirect_uri != e.redirect_uri
        || c.resource != e.resource
        || !live(&c.expires_at)
        || !pkce_verified(&c.challenge, &e.code_verifier)
    {
        return Err(Error::Unauthorized);
    }
    let reused = c.used;
    c.used = true;
    Ok(Spent {
        principal: c.principal.clone(),
        reused,
        retry_salt: None,
        refresh_expires: expiry(90 * 86400),
    })
}
fn spend_refresh(refreshes: &mut [Refresh], e: &Exchange, salt: &str) -> Result<Spent, Error> {
    let r = refreshes
        .iter_mut()
        .find(|r| r.hash == fingerprint(e.refresh_token.as_bytes()))
        .ok_or(Error::Unauthorized)?;
    if r.client_id != e.client_id || r.resource != e.resource || !live(&r.expires_at) {
        return Err(Error::Unauthorized);
    }
    let now = jiff::Timestamp::now().as_second();
    let reused = r.used && now - r.rotated_at > REFRESH_RETRY_SECONDS;
    let retry_salt = if r.used && !reused {
        if r.retry_salt.is_empty() {
            return Err(Error::Unauthorized);
        }
        Some(r.retry_salt.clone())
    } else {
        None
    };
    if !r.used {
        r.rotated_at = now;
        salt.clone_into(&mut r.retry_salt);
    }
    r.used = true;
    Ok(Spent {
        principal: r.principal.clone(),
        reused,
        retry_salt,
        refresh_expires: r.expires_at.clone(),
    })
}
fn saved_successor(
    accesses: &[Access],
    refreshes: &[Refresh],
    e: &Exchange,
    salt: &str,
    scope: String,
) -> Result<Tokens, Error> {
    let access = successor(&e.refresh_token, salt, "access");
    let next = successor(&e.refresh_token, salt, "refresh");
    let expires = accesses
        .iter()
        .find(|a| a.hash == fingerprint(access.as_bytes()) && live(&a.expires_at))
        .and_then(|a| a.expires_at.parse::<jiff::Timestamp>().ok())
        .ok_or(Error::Unauthorized)?;
    if !refreshes
        .iter()
        .any(|r| r.hash == fingerprint(next.as_bytes()) && live(&r.expires_at))
    {
        return Err(Error::Unauthorized);
    }
    Ok(Tokens {
        access_token: access,
        token_type: "Bearer".into(),
        expires_in: u32::try_from(expires.as_second() - jiff::Timestamp::now().as_second())
            .unwrap_or(0),
        refresh_token: next,
        scope,
    })
}
impl Authority {
    pub fn register_oauth(&self, mut c: Client) -> Result<Client, Error> {
        if c.client_name.len() > 512
            || c.redirect_uris.is_empty()
            || c.redirect_uris.len() > 10
            || !c.redirect_uris.iter().all(|s| valid_redirect(s))
            || c.client_name.len() + c.redirect_uris.iter().map(String::len).sum::<usize>()
                > MAX_CLIENT_METADATA_BYTES
        {
            return Err(Error::Invalid);
        }
        c.client_name = display_name(&c);
        c.client_id = format!("client_{}", ulid::Ulid::new());
        c.client_id_issued_at = jiff::Timestamp::now().as_second();
        c.used = false;
        c.token_endpoint_auth_method = "none".into();
        c.grant_types = vec!["authorization_code".into(), "refresh_token".into()];
        c.response_types = vec!["code".into()];
        self.update(|d| {
            let mut clients: Vec<Client> = read(d, "oauth_clients")?;
            let stale = c.client_id_issued_at - PENDING_CLIENT_SECONDS;
            clients.retain(|existing| existing.used || existing.client_id_issued_at > stale);
            let pending = clients.iter().filter(|existing| !existing.used).count();
            let mut evict = (pending + 1).saturating_sub(MAX_PENDING_CLIENTS);
            clients.retain(|existing| {
                if evict > 0 && !existing.used {
                    evict -= 1;
                    return false;
                }
                true
            });
            if clients.len() >= MAX_CLIENTS {
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
            .find(|c| {
                c.client_id == id
                    && c.redirect_uris
                        .iter()
                        .any(|u| redirect_matches(u, redirect))
            })
            .map(|mut c| {
                c.client_name = display_name(&c);
                c
            })
            .ok_or(Error::Invalid)
    }
    pub fn create_oauth_agent(
        &self,
        name: &str,
        level: &str,
        scopes: &[Grant],
        over_public_host: bool,
    ) -> Result<(Principal, String), Error> {
        let (agent, secret) = self.create_agent(
            name,
            match level {
                "read" => crate::AgentLevel::Read,
                "approve" => crate::AgentLevel::Approve,
                "full" => crate::AgentLevel::Full,
                _ => return Err(Error::Invalid),
            },
            agent_grants(level, scopes)?,
        )?;
        if !over_public_host {
            return Ok((agent, secret));
        }
        self.update(|d| {
            let stored = d
                .principals
                .iter_mut()
                .find(|p| p.id == agent.id)
                .ok_or(Error::Unavailable)?;
            stored.remote = true;
            Ok((stored.clone(), secret.clone()))
        })
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
        let salt = token(32);
        let refreshing = e.grant_type == "refresh_token";
        let (access, next) = if refreshing {
            (
                successor(&e.refresh_token, &salt, "access"),
                successor(&e.refresh_token, &salt, "refresh"),
            )
        } else {
            (token(32), token(32))
        };
        self.update(|d| {
            let mut codes: Vec<Code> = read(d, "oauth_codes")?;
            let mut refreshes: Vec<Refresh> = read(d, "oauth_refresh")?;
            let spent = match e.grant_type.as_str() {
                "authorization_code" => spend_code(&mut codes, e)?,
                "refresh_token" => spend_refresh(&mut refreshes, e, &salt)?,
                _ => return Err(Error::Invalid),
            };
            if spent.reused {
                if let Some(p) = d.principals.iter_mut().find(|p| p.id == spent.principal) {
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
                .find(|p| p.id == spent.principal && p.kind == "agent" && p.active())
                .ok_or(Error::Unauthorized)?;
            let scopes = scope(p);
            let granted: Vec<&str> = scopes.split_whitespace().collect();
            if !e
                .scope
                .split_whitespace()
                .all(|s| s == OFFLINE_ACCESS || granted.contains(&s))
            {
                return Err(Error::Invalid);
            }
            if let Some(saved) = &spent.retry_salt {
                let accesses: Vec<Access> = read(d, "oauth_access")?;
                return Ok(saved_successor(&accesses, &refreshes, e, saved, scopes));
            }
            p.token_hash.clear();
            p.extra.insert("resource".into(), e.resource.clone().into());
            let mut accesses: Vec<Access> = read(d, "oauth_access")?;
            accesses.retain(|a| live(&a.expires_at));
            accesses.push(Access {
                hash: fingerprint(access.as_bytes()),
                principal: spent.principal.clone(),
                resource: e.resource.clone(),
                expires_at: expiry(3600),
            });
            let window_start = jiff::Timestamp::now().as_second() - REFRESH_RETRY_SECONDS;
            refreshes.retain(|r| live(&r.expires_at));
            for r in refreshes
                .iter_mut()
                .filter(|r| r.used && r.rotated_at < window_start)
            {
                r.retry_salt.clear();
            }
            refreshes.push(Refresh {
                hash: fingerprint(next.as_bytes()),
                client_id: e.client_id.clone(),
                resource: e.resource.clone(),
                principal: spent.principal,
                expires_at: spent.refresh_expires,
                used: false,
                rotated_at: 0,
                retry_salt: String::new(),
            });
            let mut clients: Vec<Client> = read(d, "oauth_clients")?;
            if let Some(client) = clients
                .iter_mut()
                .find(|c| c.client_id == e.client_id && !c.used)
            {
                client.used = true;
                write(d, "oauth_clients", &clients)?;
            }
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
        if recently_used(&p) {
            return Ok(p);
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
#[path = "oauth_security_tests.rs"]
mod security_tests;

#[cfg(test)]
mod remote_agent_tests {
    #![allow(clippy::unwrap_used)]
    use crate::{Authority, Grant};

    #[test]
    fn a_connector_approved_on_a_public_host_may_use_that_host() {
        let home = tempfile::tempdir().unwrap();
        let authority = Authority::open(home.path()).unwrap();
        let scopes = [Grant::parse("calendar:read").unwrap()];
        let (public, _) = authority
            .create_oauth_agent("Claude", "approve", &scopes, true)
            .unwrap();
        let (local, _) = authority
            .create_oauth_agent("Local", "approve", &scopes, false)
            .unwrap();
        assert!(public.remote);
        assert!(!local.remote);
        assert!(authority.principal(&public.id).unwrap().remote, "persisted");
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::redirect_matches;

    #[test]
    fn loopback_redirects_ignore_only_the_port() {
        for (registered, presented, expected) in [
            (
                "http://localhost/callback",
                "http://localhost:3118/callback",
                true,
            ),
            (
                "http://127.0.0.1/callback",
                "http://127.0.0.1:49152/callback",
                true,
            ),
            ("http://[::1]/callback", "http://[::1]:49152/callback", true),
            (
                "https://claude.ai/api/mcp/auth_callback",
                "https://claude.ai/api/mcp/auth_callback",
                true,
            ),
            ("https://claude.ai/cb", "https://claude.ai:8443/cb", false),
            (
                "https://localhost/callback",
                "https://localhost:3118/callback",
                false,
            ),
            (
                "http://localhost/callback",
                "http://localhost:3118/other",
                false,
            ),
            (
                "http://localhost/callback",
                "http://127.0.0.1:3118/callback",
                false,
            ),
            (
                "http://localhost/callback",
                "http://localhost.evil.test:3118/callback",
                false,
            ),
            (
                "http://localhost/callback",
                "http://localhost:3118/callback?x=1",
                false,
            ),
        ] {
            assert_eq!(
                redirect_matches(registered, presented),
                expected,
                "{registered} {presented}"
            );
        }
    }

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
