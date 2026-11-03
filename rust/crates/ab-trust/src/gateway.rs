use crate::{Authority, Error, Grant, PairCode, Principal, fingerprint, subset};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signature, VerifyingKey};
use hmac::Mac;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize, Deserialize)]
struct Pairing {
    #[serde(default)]
    purpose: String,
    #[serde(default)]
    dial: String,
    key: Option<String>,
    expires_at: String,
    attempts: u64,
    used: bool,
    grants: Vec<Grant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Principal>,
    #[serde(default)]
    complete: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedDelegation {
    #[serde(default)]
    pub local_level: String,
    #[serde(default)]
    pub challenge: String,
    pub replica_id: String,
    pub local_principal: String,
    pub local_grants: Vec<Grant>,
    pub command_id: String,
    pub request_hash: String,
    pub iat: i64,
}

fn certificate(bytes: &[u8]) -> Result<x509_parser::certificate::X509Certificate<'_>, Error> {
    let (remaining, cert) =
        x509_parser::parse_x509_certificate(bytes).map_err(|_| Error::Unauthorized)?;
    if !remaining.is_empty() || !cert.validity().is_valid() {
        return Err(Error::Unauthorized);
    }
    Ok(cert)
}

fn random_id(prefix: &str) -> String {
    let mut bytes = [0; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", hex::encode(bytes))
}

pub fn relay_url(address: &str) -> Result<String, Error> {
    let url = url::Url::parse(address).map_err(|_| Error::Invalid)?;
    if url.scheme() != "https"
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !["", "/"].contains(&url.path())
        || url.port() == Some(0)
    {
        return Err(Error::Invalid);
    }
    Ok(url[url::Position::BeforeScheme..url::Position::AfterPort].to_owned())
}
impl Authority {
    pub fn pair_remote(
        &self,
        address: &str,
        grants: Vec<Grant>,
        ttl: std::time::Duration,
    ) -> Result<String, Error> {
        let raw = self.pair_with_ttl(&relay_url(address)?, "", grants, ttl)?;
        let mut code = PairCode::decode(&raw)?;
        code.remote = true;
        Ok(format!(
            "ab1-{}",
            data_encoding::BASE32_NOPAD.encode(&serde_json::to_vec(&code)?)
        ))
    }
    #[cfg(feature = "lab-harness")]
    pub fn principal_by_fingerprint(&self, fingerprint: &str) -> Result<Principal, Error> {
        self.load()?
            .principals
            .into_iter()
            .find(|p| p.fingerprint == fingerprint && p.active())
            .ok_or(Error::Unauthorized)
    }
    pub fn pair(
        &self,
        address: &str,
        dial: &str,
        grants: Vec<Grant>,
        ttl_seconds: i64,
    ) -> Result<String, Error> {
        self.pair_with_ttl(
            address,
            dial,
            grants,
            std::time::Duration::from_secs(u64::try_from(ttl_seconds).map_err(|_| Error::Invalid)?),
        )
    }

    pub fn pair_with_ttl(
        &self,
        address: &str,
        dial: &str,
        grants: Vec<Grant>,
        ttl: std::time::Duration,
    ) -> Result<String, Error> {
        if address.is_empty()
            || ttl.is_zero()
            || ttl > std::time::Duration::from_secs(600)
            || !dial.is_empty() && dial.parse::<std::net::SocketAddr>().is_err()
        {
            return Err(Error::Invalid);
        }
        let mut secret = [0; 20];
        rand::thread_rng().fill_bytes(&mut secret);
        let code = PairCode {
            purpose: String::new(),
            remote: false,
            dial: dial.into(),
            id: random_id(""),
            address: address.into(),
            fingerprint: self.fingerprint.clone(),
            secret: STANDARD.encode(secret),
            expires_at: (jiff::Timestamp::now()
                .checked_add(jiff::SignedDuration::try_from(ttl).map_err(|_| Error::Invalid)?)
                .map_err(|_| Error::Invalid)?)
            .to_string(),
        };
        let pairing = Pairing {
            purpose: String::new(),
            dial: dial.into(),
            key: Some(STANDARD.encode(Sha256::digest(secret))),
            expires_at: code.expires_at.clone(),
            attempts: 0,
            used: false,
            grants,
            result: None,
            complete: false,
        };
        self.update(|d| {
            d.pairings
                .insert(code.id.clone(), serde_json::to_value(pairing)?);
            Ok(())
        })?;
        Ok(format!(
            "ab1-{}",
            data_encoding::BASE32_NOPAD.encode(&serde_json::to_vec(&code)?)
        ))
    }

    pub fn accept_pair(
        &self,
        id: &str,
        proof: &[u8],
        cert: &[u8],
        resume: bool,
    ) -> Result<Principal, Error> {
        let valid_certificate = certificate(cert).is_ok();
        self.update(|d| {
            let Some(value) = d.pairings.get(id) else {
                return Ok(Err(Error::Unauthorized));
            };
            let mut pair: Pairing = serde_json::from_value(value.clone())?;
            if !pair.purpose.is_empty()
                || pair.used && !resume
                || !pair.used && pair.attempts >= 5
                || pair
                    .expires_at
                    .parse::<jiff::Timestamp>()
                    .map_err(|_| Error::Invalid)?
                    < jiff::Timestamp::now()
            {
                return Ok(Err(Error::Unauthorized));
            }
            if !pair.used {
                pair.attempts += 1;
            }
            let key = STANDARD
                .decode(pair.key.as_deref().unwrap_or(""))
                .map_err(|_| Error::Invalid)?;
            let mut mac = hmac::Hmac::<Sha256>::new_from_slice(&key).map_err(|_| Error::Invalid)?;
            mac.update(b"apple-bridge-pair/1\0");
            mac.update(cert);
            let result = if mac.verify_slice(proof).is_err() || !valid_certificate {
                Err(Error::Unauthorized)
            } else if pair.used {
                pair.result
                    .as_ref()
                    .filter(|p| p.fingerprint == fingerprint(cert))
                    .and_then(|p| {
                        d.principals
                            .iter()
                            .find(|current| current.id == p.id && current.active())
                    })
                    .cloned()
                    .ok_or(Error::Unauthorized)
            } else {
                let p = Principal {
                    id: random_id("replica:"),
                    kind: "replica".into(),
                    grants: pair.grants.clone(),
                    revision: 1,
                    fingerprint: fingerprint(cert),
                    dial: pair.dial.clone(),
                    ..Principal::default()
                };
                d.principals.push(p.clone());
                pair.used = true;
                if pair.dial.is_empty() {
                    pair.key = None;
                } else {
                    pair.result = Some(p.clone());
                }
                Ok(p)
            };
            d.pairings.insert(id.into(), serde_json::to_value(pair)?);
            Ok(result)
        })?
    }

    pub fn authenticate_certificate(&self, cert: &[u8]) -> Result<Principal, Error> {
        certificate(cert)?;
        let hash = fingerprint(cert);
        self.load()?
            .principals
            .into_iter()
            .find(|p| p.kind == "replica" && p.active() && p.fingerprint == hash)
            .ok_or(Error::Unauthorized)
    }

    pub fn revoke(&self, id: &str) -> Result<(), Error> {
        self.update(|d| {
            let index = d
                .principals
                .iter()
                .position(|p| p.id == id)
                .ok_or(Error::Unauthorized)?;
            if matches!(d.principals[index].kind.as_str(), "agent" | "replica") {
                let p = &mut d.principals[index];
                p.extra.insert(
                    "revoked_at".into(),
                    serde_json::Value::String(jiff::Timestamp::now().to_string()),
                );
                p.token_hash.clear();
                p.revision += 1;
            } else {
                d.principals.remove(index);
            }
            Ok(())
        })
    }

    pub fn mark_relay2(&self, id: &str) -> Result<Principal, Error> {
        self.update(|d| {
            let p = d
                .principals
                .iter_mut()
                .find(|p| p.id == id && p.kind == "replica")
                .ok_or(Error::Unauthorized)?;
            p.extra
                .insert("relay2".into(), serde_json::Value::Bool(true));
            Ok(p.clone())
        })
    }

    pub fn change_scope(&self, id: &str, grants: Vec<Grant>) -> Result<(), Error> {
        self.update(|d| {
            let p = d
                .principals
                .iter_mut()
                .find(|p| p.id == id && p.kind == "replica")
                .ok_or(Error::Unauthorized)?;
            p.grants = grants;
            p.revision += 1;
            Ok(())
        })
    }

    pub fn session_valid(&self, principal: &Principal) -> Result<bool, Error> {
        Ok(self.load()?.principals.iter().any(|p| {
            p.id == principal.id
                && p.active()
                && p.revision == principal.revision
                && p.fingerprint == principal.fingerprint
        }))
    }

    pub fn verify_delegation(
        &self,
        cert: &[u8],
        header: &str,
        id: &str,
        hash: &str,
        challenge: &str,
    ) -> Result<Principal, Error> {
        let replica = self.authenticate_certificate(cert)?;
        let (body, signature) = header.split_once('.').ok_or(Error::Unauthorized)?;
        let body = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| Error::Unauthorized)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| Error::Unauthorized)?;
        let cert = certificate(cert)?;
        if cert.public_key().algorithm.algorithm.to_id_string() != "1.3.101.112" {
            return Err(Error::Unauthorized);
        }
        let bytes: [u8; 32] = cert
            .public_key()
            .subject_public_key
            .data
            .as_ref()
            .try_into()
            .map_err(|_| Error::Unauthorized)?;
        let key = VerifyingKey::from_bytes(&bytes).map_err(|_| Error::Unauthorized)?;
        key.verify_strict(
            &body,
            &Signature::from_slice(&signature).map_err(|_| Error::Unauthorized)?,
        )
        .map_err(|_| Error::Unauthorized)?;
        let d: VerifiedDelegation =
            serde_json::from_slice(&body).map_err(|_| Error::Unauthorized)?;
        let agent = d.local_principal.starts_with("agent:")
            && d.local_principal.len() > 6
            && !d.local_principal.contains(['/', '\r', '\n'])
            && ["read", "approve", "full"].contains(&d.local_level.as_str());
        let ordinary = (d.local_principal.starts_with("token:")
            || d.local_principal.starts_with("device:"))
            && d.local_level.is_empty();
        let now = jiff::Timestamp::now().as_second();
        if d.replica_id != replica.id
            || !(agent || ordinary)
            || d.challenge != challenge
            || d.command_id != id
            || d.request_hash != hash
            || d.iat < now - 60
            || d.iat > now + 5
            || !subset(&d.local_grants, &replica.grants)
        {
            return Err(Error::Unauthorized);
        }
        Ok(Principal {
            id: format!("{}/{}", replica.id, d.local_principal),
            kind: "delegated".into(),
            level: d.local_level,
            grants: d.local_grants,
            fingerprint: replica.fingerprint,
            ..Principal::default()
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLevel {
    Read,
    Approve,
    Full,
}
impl AgentLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Approve => "approve",
            Self::Full => "full",
        }
    }
}
impl Authority {
    pub fn create_agent(
        &self,
        name: &str,
        level: AgentLevel,
        scopes: Vec<Grant>,
    ) -> Result<(Principal, String), Error> {
        let name = name.trim();
        if name.is_empty() || name.len() > 120 || scopes.is_empty() {
            return Err(Error::Invalid);
        }
        let mut grants = Vec::new();
        for scope in scopes {
            let (source, _) = scope.as_str().split_once(':').ok_or(Error::Invalid)?;
            if ![
                #[cfg(feature = "lab-harness")]
                "labdummy",
                "messages",
                "calendar",
                "reminders",
                "contacts",
                "notes",
                "usage",
                "phone",
                "location",
                "audio",
                "capture",
                "files",
                "focus",
                "health",
                "iphone",
                "mac",
                "mail",
                "maps",
                "pdf",
                "print",
                "safari",
                "shortcuts",
                "speech",
                "vision",
                "voice_memos",
                "voicemail",
            ]
            .contains(&source)
            {
                return Err(Error::Invalid);
            }
            grants.push(if level == AgentLevel::Read {
                Grant::parse(&format!("{source}:read"))?
            } else {
                scope
            });
        }
        let mut token_bytes = [0; 32];
        rand::thread_rng().fill_bytes(&mut token_bytes);
        let token = format!("aba-{}", URL_SAFE_NO_PAD.encode(token_bytes));
        let mut p = Principal {
            id: format!("agent:{}", ulid::Ulid::new()),
            kind: "agent".into(),
            grants,
            level: level.as_str().into(),
            token_hash: fingerprint(token.as_bytes()),
            revision: 1,
            ..Principal::default()
        };
        p.extra.insert("name".into(), serde_json::json!(name));
        p.extra.insert(
            "created_at".into(),
            serde_json::json!(jiff::Timestamp::now().to_string()),
        );
        self.update(|d| {
            if d.enrollment
                .as_ref()
                .is_some_and(|e| !subset(&p.grants, &e.grants))
            {
                return Err(Error::Unauthorized);
            }
            d.principals.push(p.clone());
            Ok(())
        })?;
        Ok((p, token))
    }
    pub fn agents(&self) -> Result<Vec<Principal>, Error> {
        Ok(self
            .load()?
            .principals
            .into_iter()
            .filter(|p| p.kind == "agent")
            .map(|mut p| {
                p.token_hash.clear();
                p
            })
            .collect())
    }
}

impl Authority {
    pub fn rotate_agent(&self, id: &str) -> Result<String, Error> {
        let mut bytes = [0; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = format!("aba-{}", URL_SAFE_NO_PAD.encode(bytes));
        self.update(|d| {
            let p = d
                .principals
                .iter_mut()
                .find(|p| p.id == id && p.kind == "agent" && p.active())
                .ok_or(Error::Unauthorized)?;
            p.token_hash = fingerprint(token.as_bytes());
            p.revision += 1;
            Ok(())
        })?;
        Ok(token)
    }
}

impl Authority {
    pub fn dial_replicas(&self) -> Result<Vec<Principal>, Error> {
        Ok(self
            .load()?
            .principals
            .into_iter()
            .filter(|p| p.kind == "replica" && p.active() && !p.dial.is_empty())
            .collect())
    }
    pub fn confirm_dial(&self, id: &str) -> Result<(), Error> {
        self.update(|d| {
            for value in d.pairings.values_mut() {
                if value
                    .get("purpose")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|purpose| !purpose.is_empty())
                {
                    continue;
                }
                let mut pair: Pairing = serde_json::from_value(value.clone())?;
                if pair.result.as_ref().is_some_and(|p| p.id == id) && !pair.complete {
                    pair.complete = true;
                    pair.key = None;
                    *value = serde_json::to_value(pair)?;
                }
            }
            Ok(())
        })
    }
    pub async fn dial_pairings_once(&self) -> Result<(), Error> {
        for (id, value) in self.load()?.pairings {
            if value
                .get("purpose")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|purpose| !purpose.is_empty())
            {
                continue;
            }
            let pair: Pairing = serde_json::from_value(value)?;
            if pair.dial.is_empty()
                || pair.complete
                || !pair.purpose.is_empty()
                || (!pair.used && pair.attempts >= 5)
                || pair
                    .expires_at
                    .parse::<jiff::Timestamp>()
                    .map_err(|_| Error::Invalid)?
                    < jiff::Timestamp::now()
            {
                continue;
            }
            let _ = self.dial_pair(&id, &pair.dial).await;
        }
        Ok(())
    }
    async fn dial_pair(&self, id: &str, address: &str) -> Result<(), Error> {
        let mut tls = self.client_tls("")?;
        tls.dangerous()
            .set_certificate_verifier(std::sync::Arc::new(crate::tls::Pin {
                digest: String::new(),
                accept_unpaired: true,
            }));
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(tls)
            .tls_info(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        let response = client
            .post(format!("https://{address}/v1/pair/proof"))
            .json(&serde_json::json!({"id":id}))
            .send()
            .await?
            .error_for_status()?;
        let cert = response
            .extensions()
            .get::<reqwest::tls::TlsInfo>()
            .and_then(reqwest::tls::TlsInfo::peer_certificate)
            .ok_or(Error::Unauthorized)?
            .to_vec();
        if response.content_length().is_some_and(|n| n > 4096) {
            return Err(Error::Invalid);
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 4096 {
                return Err(Error::Invalid);
            }
            bytes.extend_from_slice(&chunk);
        }
        let proof: serde_json::Value = serde_json::from_slice(&bytes)?;
        if proof["id"].as_str() != Some(id) {
            return Err(Error::Unauthorized);
        }
        let proof = STANDARD
            .decode(proof["proof"].as_str().ok_or(Error::Unauthorized)?)
            .map_err(|_| Error::Unauthorized)?;
        let principal = self.accept_pair(id, &proof, &cert, true)?;
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(self.client_tls(&principal.fingerprint)?)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        client
            .post(format!("https://{address}/v1/pair/finish"))
            .json(&principal)
            .send()
            .await?
            .error_for_status()?;
        self.confirm_dial(&principal.id)
    }
}
