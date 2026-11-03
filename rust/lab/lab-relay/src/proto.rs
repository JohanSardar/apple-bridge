use crate::{
    Error, Result,
    identity::{Identity, fingerprint},
    link::Link,
    paircode::PairCode,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const VERSION: &str = "relay/2";
pub const CAPABILITIES: &str = "tls-exporter,feed/1";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Frame {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub capabilities: String,
    pub t: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nonce: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub certificate: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pairing_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proof: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<Box<RawValue>>,
}
#[derive(Debug, Clone)]
pub struct Principal {
    pub id: String,
    pub kind: String,
    pub fingerprint: String,
    pub grants: Vec<String>,
    pub revision: u64,
    pub relay2: bool,
}
impl Principal {
    fn parse(raw: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(raw)?;
        let text = |name: &str| {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Ok(Self {
            id: text("id"),
            kind: text("kind"),
            fingerprint: text("fingerprint"),
            grants: value
                .get("grants")
                .and_then(serde_json::Value::as_array)
                .map(|grants| {
                    grants
                        .iter()
                        .filter_map(|g| g.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            revision: value
                .get("revision")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            relay2: value
                .get("relay2")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or_default(),
        })
    }
    pub fn replica_shaped(&self) -> bool {
        self.id.starts_with("replica:")
            && self.kind == "replica"
            && !self.grants.is_empty()
            && self.revision > 0
    }
}
impl Frame {
    pub fn text(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
    pub fn parse(text: &str) -> Result<Self> {
        Ok(serde_json::from_str(text)?)
    }
    pub fn decode(field: &str) -> Result<Vec<u8>> {
        STANDARD.decode(field).map_err(|_| Error::Invalid)
    }
    pub fn nonce_bytes(&self) -> Result<Vec<u8>> {
        Self::decode(&self.nonce)
    }
    pub fn certificate_bytes(&self) -> Result<Vec<u8>> {
        Self::decode(&self.certificate)
    }
    pub fn signature_bytes(&self) -> Result<Vec<u8>> {
        Self::decode(&self.signature)
    }
    pub fn principal_text(&self) -> Option<&str> {
        self.principal.as_deref().map(RawValue::get)
    }
}
pub fn random_nonce() -> String {
    let mut nonce = [0; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    STANDARD.encode(nonce)
}
pub fn hello(identity: &Identity, relay2: bool, pairing: Option<&PairCode>) -> Frame {
    Frame {
        version: if relay2 {
            VERSION.into()
        } else {
            String::new()
        },
        capabilities: if relay2 {
            CAPABILITIES.into()
        } else {
            String::new()
        },
        t: "AuthHello".into(),
        nonce: random_nonce(),
        certificate: STANDARD.encode(&identity.certificate),
        pairing_id: pairing.map(|p| p.id.clone()).unwrap_or_default(),
        ..Frame::default()
    }
}
pub fn proof_frame(relay2: bool, signature: &[u8], pair_proof: Option<&[u8]>) -> Frame {
    Frame {
        version: if relay2 {
            VERSION.into()
        } else {
            String::new()
        },
        capabilities: if relay2 {
            CAPABILITIES.into()
        } else {
            String::new()
        },
        t: "AuthProof".into(),
        signature: STANDARD.encode(signature),
        proof: pair_proof.map(|p| STANDARD.encode(p)).unwrap_or_default(),
        ..Frame::default()
    }
}
fn transcript(
    prefix: Vec<u8>,
    hello: &Frame,
    challenge: &Frame,
    principal: Option<&str>,
) -> Result<Vec<u8>> {
    let mut out = prefix;
    out.extend(hello.nonce_bytes()?);
    out.extend(challenge.nonce_bytes()?);
    for part in [
        hello.certificate_bytes()?,
        challenge.certificate_bytes()?,
        hello.pairing_id.clone().into_bytes(),
        principal.unwrap_or_default().as_bytes().to_vec(),
    ] {
        out.extend(Sha256::digest(part));
    }
    Ok(out)
}
pub fn input1(
    role: &str,
    hello: &Frame,
    challenge: &Frame,
    principal: Option<&str>,
) -> Result<Vec<u8>> {
    transcript(
        format!("apple-bridge-relay/1\0{role}\0").into_bytes(),
        hello,
        challenge,
        principal,
    )
}
pub fn input2(
    role: &str,
    hello: &Frame,
    challenge: &Frame,
    principal: Option<&str>,
    binding: &[u8],
) -> Result<Vec<u8>> {
    let mut prefix =
        format!("apple-bridge-relay/2\0{role}\0{VERSION}\0{CAPABILITIES}\0").into_bytes();
    prefix.extend_from_slice(binding);
    transcript(prefix, hello, challenge, principal)
}
pub fn gateway_key(challenge: &Frame) -> Result<(Vec<u8>, VerifyingKey)> {
    let der = challenge.certificate_bytes()?;
    if challenge.nonce_bytes()?.len() != 32 || der.len() > 4096 {
        return Err(Error::Unauthorized);
    }
    let (_, cert) =
        x509_parser::parse_x509_certificate(&der).map_err(|e| Error::Certificate(e.to_string()))?;
    if !cert.validity().is_valid() {
        return Err(Error::Expired);
    }
    let bytes: [u8; 32] = cert
        .public_key()
        .subject_public_key
        .data
        .as_ref()
        .try_into()
        .map_err(|_| Error::Unauthorized)?;
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| Error::Unauthorized)?;
    Ok((der, key))
}
pub fn verify(key: &VerifyingKey, signature: &[u8], input: &[u8]) -> bool {
    Signature::from_slice(signature).is_ok_and(|s| key.verify(input, &s).is_ok())
}
fn versioned(frame: &Frame, relay2: bool) -> bool {
    if relay2 {
        frame.version == VERSION && frame.capabilities == CAPABILITIES
    } else {
        frame.version.is_empty() && frame.capabilities.is_empty()
    }
}
pub fn authenticate(
    link: &mut Link,
    identity: &Identity,
    relay2: bool,
    pin: &str,
    pairing: Option<&PairCode>,
) -> Result<Principal> {
    let hello = hello(identity, relay2, pairing);
    link.send_text(&hello.text()?)?;
    let challenge = Frame::parse(&link.recv_text(TIMEOUT)?)?;
    if !versioned(&challenge, relay2) {
        return Err(Error::Unauthorized);
    }
    let (der, key) = gateway_key(&challenge)?;
    if challenge.t != "AuthChallenge" || fingerprint(&der) != pin {
        return Err(Error::Unauthorized);
    }
    let binding = link.binding;
    let gateway_input = |role: &str, principal: Option<&str>| {
        if relay2 {
            input2(role, &hello, &challenge, principal, &binding)
        } else {
            input1(role, &hello, &challenge, principal)
        }
    };
    if !verify(
        &key,
        &challenge.signature_bytes()?,
        &gateway_input("gateway", None)?,
    ) {
        return Err(Error::Unauthorized);
    }
    let pair_proof = pairing.map(|code| code.proof(identity)).transpose()?;
    let signature = identity.sign(&gateway_input("replica", None)?);
    link.send_text(&proof_frame(relay2, &signature, pair_proof.as_deref()).text()?)?;
    let accepted = Frame::parse(&link.recv_text(TIMEOUT)?)?;
    if !versioned(&accepted, relay2) || accepted.t != "Authenticated" {
        return Err(Error::Unauthorized);
    }
    let raw = accepted.principal_text().ok_or(Error::Unauthorized)?;
    if !verify(
        &key,
        &accepted.signature_bytes()?,
        &gateway_input("accepted", Some(raw))?,
    ) {
        return Err(Error::Unauthorized);
    }
    let principal = Principal::parse(raw)?;
    if principal.kind != "replica" || principal.fingerprint != identity.fingerprint {
        return Err(Error::Unauthorized);
    }
    Ok(principal)
}
pub fn relay1_client(link: &mut Link, identity: &Identity, pin: &str) -> Result<Principal> {
    authenticate(link, identity, false, pin, None)
}
pub fn relay2_client(link: &mut Link, identity: &Identity, pin: &str) -> Result<Principal> {
    authenticate(link, identity, true, pin, None)
}
pub fn valid_remote_address(address: &str) -> bool {
    let Some(rest) = address.strip_prefix("https://") else {
        return false;
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.contains(['/', '?', '#', '@']) {
        return false;
    }
    match rest.rsplit_once(':') {
        Some((host, port)) => {
            !host.is_empty() && port.parse::<u32>().is_ok_and(|p| (1..=65535).contains(&p))
        }
        None => true,
    }
}
pub fn join(link: &mut Link, identity: &Identity, code: &PairCode) -> Result<Principal> {
    if !code.remote
        || !code.dial.is_empty()
        || code.id.is_empty()
        || code.secret_bytes()?.len() != 20
        || code.fingerprint.len() != 64
        || !valid_remote_address(&code.address)
    {
        return Err(Error::Invalid);
    }
    if code.expired() {
        return Err(Error::Expired);
    }
    authenticate(link, identity, true, &code.fingerprint, Some(code))
}
