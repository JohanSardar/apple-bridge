use crate::{Control, Error, Result, decode, encode, hash, random};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fmt, future::Future, pin::Pin};

pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
pub trait IdentityProvider: Send + Sync {
    fn authorize(&self, state: &str, challenge: &str) -> Result<String>;
    fn subject<'a>(&'a self, code: &'a str, verifier: &'a str) -> ProviderFuture<'a>;
}
pub struct Github {
    client: reqwest::Client,
    id: String,
    secret: String,
}
impl fmt::Debug for Github {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Github([redacted])")
    }
}
impl Github {
    pub fn new(id: String, secret: String) -> Result<Self> {
        if id.is_empty() || secret.is_empty() {
            return Err(Error::Invalid);
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|_| Error::Unavailable)?,
            id,
            secret,
        })
    }
}
impl IdentityProvider for Github {
    fn authorize(&self, state: &str, challenge: &str) -> Result<String> {
        let mut url = url::Url::parse("https://github.com/login/oauth/authorize")
            .map_err(|_| Error::Invalid)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", self.id.as_str()),
            ("redirect_uri", "https://app.lexaai.tech/v1/oauth/callback"),
            ("scope", ""),
            ("state", state),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ]);
        Ok(url.into())
    }
    fn subject<'a>(&'a self, code: &'a str, verifier: &'a str) -> ProviderFuture<'a> {
        Box::pin(async move {
            let token: Value = self
                .client
                .post("https://github.com/login/oauth/access_token")
                .header("Accept", "application/json")
                .form(&[
                    ("client_id", self.id.as_str()),
                    ("client_secret", self.secret.as_str()),
                    ("code", code),
                    ("code_verifier", verifier),
                    ("redirect_uri", "https://app.lexaai.tech/v1/oauth/callback"),
                ])
                .send()
                .await
                .map_err(|_| Error::Unavailable)?
                .error_for_status()
                .map_err(|_| Error::Unauthorized)?
                .json()
                .await
                .map_err(|_| Error::Unauthorized)?;
            if token.get("scope").and_then(Value::as_str) != Some("") {
                return Err(Error::Forbidden);
            }
            let access = token
                .get("access_token")
                .and_then(Value::as_str)
                .ok_or(Error::Unauthorized)?;
            let user: Value = self
                .client
                .get("https://api.github.com/user")
                .header("User-Agent", "ab-relay-control")
                .bearer_auth(access)
                .send()
                .await
                .map_err(|_| Error::Unavailable)?
                .error_for_status()
                .map_err(|_| Error::Unauthorized)?
                .json()
                .await
                .map_err(|_| Error::Unauthorized)?;
            let id = user
                .get("id")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .ok_or(Error::Unauthorized)?;
            Ok(format!("github:{id}"))
        })
    }
}

impl Control {
    pub fn oauth_start(
        &self,
        provider: &dyn IdentityProvider,
        invite: &str,
        browser: &str,
        now: i64,
    ) -> Result<Value> {
        let verifier = random(32)?;
        let state = self.challenge(
            "oauth",
            &json!({"verifier":verifier,"invite":hash(invite),"browser":hash(browser)}),
            now,
        )?;
        Ok(json!({"url":provider.authorize(&state,&hash(verifier))?,"state":state}))
    }
    pub async fn oauth_finish(
        &self,
        provider: &dyn IdentityProvider,
        state: &str,
        code: &str,
        browser: &str,
        now: i64,
    ) -> Result<Value> {
        let data = self.take_challenge(state, "oauth", now)?;
        if data["browser"].as_str() != Some(&hash(browser)) {
            return Err(Error::Forbidden);
        }
        let subject = provider.subject(code, text(&data, "verifier")?).await?;
        self.login(&subject, text(&data, "invite")?, now)
    }
    pub(crate) fn passkey_start(&self, account: &str, register: bool, now: i64) -> Result<Value> {
        let nonce = random(32)?;
        let id = self.challenge(
            if register { "register" } else { "authenticate" },
            &json!({"account":account,"nonce":nonce}),
            now,
        )?;
        let ids = self.passkeys(account)?;
        if register {
            Ok(
                json!({"transaction":id,"publicKey":{"challenge":nonce,"rp":{"id":"app.lexaai.tech","name":"Apple Bridge control"},"user":{"id":hash(account),"name":account,"displayName":account},"pubKeyCredParams":[{"type":"public-key","alg":-7}],"timeout":300_000,"attestation":"none","authenticatorSelection":{"userVerification":"required","residentKey":"preferred"},"excludeCredentials":descriptors(&ids)}}),
            )
        } else {
            Ok(
                json!({"transaction":id,"publicKey":{"challenge":nonce,"rpId":"app.lexaai.tech","userVerification":"required","timeout":300_000,"allowCredentials":descriptors(&ids)}}),
            )
        }
    }
    pub(crate) fn passkey_finish(
        &self,
        account: Option<&str>,
        register: bool,
        input: &Value,
        now: i64,
    ) -> Result<Value> {
        let data = self.take_challenge(
            text(input, "transaction")?,
            if register { "register" } else { "authenticate" },
            now,
        )?;
        let owner = text(&data, "account")?;
        if register && account != Some(owner) {
            return Err(Error::Forbidden);
        }
        let response = &input["response"];
        let client = decode(text(response, "clientDataJSON")?)?;
        let parsed: Value = serde_json::from_slice(&client)?;
        if parsed["type"]
            != if register {
                "webauthn.create"
            } else {
                "webauthn.get"
            }
            || parsed["challenge"] != data["nonce"]
            || parsed["origin"] != "https://app.lexaai.tech"
            || parsed.get("crossOrigin").is_some_and(|v| v != false)
        {
            return Err(Error::Forbidden);
        }
        let id = text(input, "id")?;
        if register {
            let object = decode(text(response, "attestationObject")?)?;
            let att: ciborium::Value =
                ciborium::from_reader(object.as_slice()).map_err(|_| Error::Invalid)?;
            if field(&att, "fmt")?.as_text() != Some("none") {
                return Err(Error::Forbidden);
            }
            let auth = field(&att, "authData")?.as_bytes().ok_or(Error::Invalid)?;
            let counter = auth_data(auth, true)?;
            if auth.len() < 55 {
                return Err(Error::Invalid);
            }
            let size = usize::from(u16::from_be_bytes([auth[53], auth[54]]));
            let end = 55_usize
                .checked_add(size)
                .filter(|e| *e <= auth.len())
                .ok_or(Error::Invalid)?;
            if encode(&auth[55..end]) != id {
                return Err(Error::Forbidden);
            }
            let key: ciborium::Value =
                ciborium::from_reader(&auth[end..]).map_err(|_| Error::Invalid)?;
            for (label, value) in [(1, 2), (3, -7), (-1, 1)] {
                if integer(&key, label)? != value {
                    return Err(Error::Invalid);
                }
            }
            let x = coordinate(&key, -2)?;
            let y = coordinate(&key, -3)?;
            let mut public = vec![4];
            public.extend(x);
            public.extend(y);
            VerifyingKey::from_sec1_bytes(&public).map_err(|_| Error::Invalid)?;
            let codes = self.save_passkey(owner, id, &encode(public), counter)?;
            Ok(json!({"recovery_codes":codes}))
        } else {
            let auth = decode(text(response, "authenticatorData")?)?;
            let counter = auth_data(&auth, false)?;
            let sig = decode(text(response, "signature")?)?;
            let mut message = auth;
            message.extend(Sha256::digest(client));
            self.authenticate_passkey(
                owner,
                id,
                |key, previous| {
                    if (previous != 0 || counter != 0) && counter <= previous {
                        return Err(Error::Forbidden);
                    }
                    let public = decode(key)?;
                    let key = VerifyingKey::from_sec1_bytes(&public).map_err(|_| Error::Invalid)?;
                    let sig = Signature::from_der(&sig).map_err(|_| Error::Invalid)?;
                    key.verify(&message, &sig).map_err(|_| Error::Forbidden)?;
                    Ok(counter)
                },
                now,
            )
        }
    }
}
pub(crate) fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or(Error::Invalid)
}
fn descriptors(ids: &Value) -> Value {
    json!(ids.as_array().map(|ids| {
        ids.iter()
            .map(|id| json!({"id":id,"type":"public-key"}))
            .collect::<Vec<_>>()
    }))
}
fn auth_data(auth: &[u8], register: bool) -> Result<u32> {
    if auth.len() < 37
        || auth[..32] != Sha256::digest(b"app.lexaai.tech")[..]
        || auth[32] & 5 != 5
        || (register && auth[32] & 64 == 0)
    {
        return Err(Error::Forbidden);
    }
    Ok(u32::from_be_bytes(
        auth[33..37].try_into().map_err(|_| Error::Invalid)?,
    ))
}
fn field<'a>(v: &'a ciborium::Value, key: &str) -> Result<&'a ciborium::Value> {
    v.as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_text() == Some(key))
                .map(|(_, v)| v)
        })
        .ok_or(Error::Invalid)
}
fn cose(v: &ciborium::Value, key: i64) -> Result<&ciborium::Value> {
    v.as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| {
                    k.as_integer()
                        .is_some_and(|n| i128::from(n) == i128::from(key))
                })
                .map(|(_, v)| v)
        })
        .ok_or(Error::Invalid)
}
fn integer(v: &ciborium::Value, key: i64) -> Result<i128> {
    cose(v, key)?
        .as_integer()
        .map(i128::from)
        .ok_or(Error::Invalid)
}
fn coordinate(v: &ciborium::Value, key: i64) -> Result<&[u8]> {
    let bytes = cose(v, key)?.as_bytes().ok_or(Error::Invalid)?;
    if bytes.len() != 32 {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
