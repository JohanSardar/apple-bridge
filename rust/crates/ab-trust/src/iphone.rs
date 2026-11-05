use crate::{Authority, Error, Principal};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

fn byte_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairRequest {
    pub id: String,
    #[serde(default, deserialize_with = "byte_string")]
    pub signing_key: String,
    #[serde(default, deserialize_with = "byte_string")]
    pub encryption_key: String,
    #[serde(default, deserialize_with = "byte_string")]
    pub proof: String,
    #[serde(default, deserialize_with = "byte_string")]
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub device_id: String,
    pub at: i64,
    #[serde(default, deserialize_with = "byte_string")]
    pub payload: String,
    #[serde(default, deserialize_with = "byte_string")]
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gateway_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pair_proof: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadAuthorization {
    pub length: u64,
    pub digest: [u8; 32],
}
#[derive(Debug, Deserialize)]
struct Pairing {
    purpose: String,
    key: Option<String>,
    #[serde(default)]
    address: String,
    expires_at: String,
    #[serde(default)]
    used: bool,
    #[serde(default)]
    attempts: u64,
}
pub fn pair_message(id: &str, signing: &[u8], encryption: &[u8]) -> Vec<u8> {
    let mut message = format!("apple-bridge-iphone-pair/1\0{id}\0").into_bytes();
    message.extend_from_slice(signing);
    message.extend_from_slice(encryption);
    message
}
pub fn pair_response_message(
    pairing_id: &str,
    address: &str,
    device_id: &str,
    signing: &[u8],
    encryption: &[u8],
    gateway_key: &[u8],
) -> Vec<u8> {
    let mut message =
        format!("apple-bridge-iphone-pair-response/1\0{pairing_id}\0{address}\0{device_id}\0")
            .into_bytes();
    message.extend_from_slice(signing);
    message.extend_from_slice(encryption);
    message.extend_from_slice(gateway_key);
    message
}
pub fn upload_message(path: &str, device: &str, at: i64, length: u64, digest: &[u8]) -> Vec<u8> {
    let mut message =
        format!("apple-bridge-iphone-upload/1\0{path}\0{device}\0{at}\0{length}\0").into_bytes();
    message.extend_from_slice(digest);
    message
}
pub fn request_message(path: &str, at: i64, payload: &[u8]) -> Vec<u8> {
    let mut message = format!("apple-bridge-iphone/1\0{path}\0{at}\0").into_bytes();
    message.extend_from_slice(payload);
    message
}
fn decode(raw: &str) -> Result<Vec<u8>, Error> {
    STANDARD.decode(raw).map_err(|_| Error::Invalid)
}
fn encryption_key(phone: &Principal) -> Result<[u8; 32], Error> {
    if phone.kind != "iphone" || !phone.active() {
        return Err(Error::Unauthorized);
    }
    decode(
        phone
            .extra
            .get("iphone_encryption_key")
            .and_then(serde_json::Value::as_str)
            .ok_or(Error::Invalid)?,
    )?
    .try_into()
    .map_err(|_| Error::Invalid)
}
fn verify(key: &[u8], message: &[u8], signature: &str) -> Result<(), Error> {
    let key = VerifyingKey::from_bytes(key.try_into().map_err(|_| Error::Unauthorized)?)
        .map_err(|_| Error::Unauthorized)?;
    key.verify_strict(
        message,
        &Signature::from_slice(&decode(signature)?).map_err(|_| Error::Unauthorized)?,
    )
    .map_err(|_| Error::Unauthorized)
}
impl Authority {
    pub fn seal_iphone(
        &self,
        phone: &Principal,
        plaintext: &[u8],
    ) -> Result<ab_wire::crypto::Packet, Error> {
        let current = self.principal(&phone.id)?;
        let public = encryption_key(&current)?;
        let mut ephemeral = [0; 32];
        let mut nonce = [0; 12];
        rand::thread_rng().fill_bytes(&mut ephemeral);
        rand::thread_rng().fill_bytes(&mut nonce);
        ab_wire::crypto::seal(
            public,
            &current.id,
            plaintext,
            ephemeral,
            nonce,
            &self.iphone_signing_key()?,
        )
        .map_err(|_| Error::Invalid)
    }
    pub fn iphone_gateway_key(&self) -> Result<String, Error> {
        Ok(STANDARD.encode(self.iphone_signing_key()?.verifying_key().to_bytes()))
    }
    pub fn set_iphone_live_token(&self, id: &str, task: &str, token: &str) -> Result<(), Error> {
        if !(task.is_empty() && token.is_empty())
            && (task.is_empty()
                || task.len() > 128
                || !token.is_empty()
                    && (!(32..=512).contains(&token.len())
                        || !token
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))))
        {
            return Err(Error::Invalid);
        }
        self.update(|d| {
            let p = d
                .principals
                .iter_mut()
                .find(|p| p.id == id && p.kind == "iphone")
                .ok_or(Error::Unauthorized)?;
            if task.is_empty() {
                p.extra.remove("iphone_live_tokens");
                return Ok(());
            }
            let raw = p
                .extra
                .entry("iphone_live_tokens".into())
                .or_insert_with(|| serde_json::json!({}));
            if raw.is_null() {
                *raw = serde_json::json!({});
            }
            let tokens = raw.as_object_mut().ok_or(Error::Invalid)?;
            if token.is_empty() {
                tokens.remove(task);
            } else {
                if tokens.len() >= 128 && !tokens.contains_key(task) {
                    return Err(Error::Invalid);
                }
                tokens.insert(task.into(), token.into());
            }
            Ok(())
        })
    }

    pub fn iphone_status(&self, request: &Request) -> Result<Device, Error> {
        let (phone, _) = self.verify_iphone("/v1/iphone/status", request)?;
        Ok(Device {
            id: phone.id,
            gateway_key: STANDARD.encode(self.iphone_signing_key()?.verifying_key().to_bytes()),
            pair_proof: String::new(),
        })
    }
    pub fn pair_iphone(&self, req: &PairRequest) -> Result<Device, Error> {
        let signing = decode(&req.signing_key)?;
        let encryption = decode(&req.encryption_key)?;
        if signing.len() != 32 || encryption.len() != 32 {
            return Err(Error::Invalid);
        }
        let message = pair_message(&req.id, &signing, &encryption);
        verify(&signing, &message, &req.signature)?;
        let gateway_bytes = self.iphone_signing_key()?.verifying_key().to_bytes();
        let gateway_key = STANDARD.encode(gateway_bytes);
        self.update(|d| {
            let raw = d.pairings.get_mut(&req.id).ok_or(Error::Unauthorized)?;
            let pair: Pairing = serde_json::from_value(raw.clone())?;
            if pair.purpose != "iphone"
                || pair.used
                || pair.attempts >= 5
                || pair
                    .expires_at
                    .parse::<jiff::Timestamp>()
                    .map_err(|_| Error::Invalid)?
                    < jiff::Timestamp::now()
            {
                return Err(Error::Unauthorized);
            }
            let key = decode(pair.key.as_deref().ok_or(Error::Unauthorized)?)?;
            let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| Error::Invalid)?;
            mac.update(&message);
            mac.verify_slice(&decode(&req.proof)?)
                .map_err(|_| Error::Unauthorized)?;
            let id = format!("iphone:{}", ulid::Ulid::new());
            let mut response = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| Error::Invalid)?;
            response.update(&pair_response_message(
                &req.id,
                &pair.address,
                &id,
                &signing,
                &encryption,
                &gateway_bytes,
            ));
            let pair_proof = STANDARD.encode(response.finalize().into_bytes());
            raw["used"] = true.into();
            raw["key"] = serde_json::Value::Null;
            let mut p = Principal {
                id: id.clone(),
                kind: "iphone".into(),
                grants: vec![],
                revision: 1,
                ..Principal::default()
            };
            p.extra
                .insert("iphone_signing_key".into(), req.signing_key.clone().into());
            p.extra.insert(
                "iphone_encryption_key".into(),
                req.encryption_key.clone().into(),
            );
            p.extra.insert(
                "created_at".into(),
                jiff::Timestamp::now().to_string().into(),
            );
            d.principals.push(p);
            Ok(Device {
                id,
                gateway_key,
                pair_proof,
            })
        })
    }
    pub fn verify_iphone(&self, path: &str, req: &Request) -> Result<(Principal, Vec<u8>), Error> {
        let p = self.principal(&req.device_id)?;
        let now = crate::decision_now();
        let at = jiff::Timestamp::from_second(req.at).map_err(|_| Error::Unauthorized)?;
        if p.kind != "iphone" || (now.as_nanosecond() - at.as_nanosecond()).abs() > 60_000_000_000 {
            return Err(Error::Unauthorized);
        }
        let key = decode(
            p.extra
                .get("iphone_signing_key")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Unauthorized)?,
        )?;
        let payload = decode(&req.payload)?;
        verify(
            &key,
            &request_message(path, req.at, &payload),
            &req.signature,
        )?;
        Ok((p, payload))
    }
    pub fn verify_iphone_upload(
        &self,
        path: &str,
        header: &str,
    ) -> Result<UploadAuthorization, Error> {
        let mut fields = header.split('.');
        let (Some(device), Some(at), Some(length), Some(digest), Some(signature), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(Error::Unauthorized);
        };
        let at = at.parse::<i64>().map_err(|_| Error::Unauthorized)?;
        let length = length.parse::<u64>().map_err(|_| Error::Unauthorized)?;
        let digest: [u8; 32] = decode(digest)
            .map_err(|_| Error::Unauthorized)?
            .try_into()
            .map_err(|_| Error::Unauthorized)?;
        if (crate::decision_now().as_second() - at).abs() > 60 {
            return Err(Error::Unauthorized);
        }
        let phone = self.principal(device)?;
        if phone.kind != "iphone" {
            return Err(Error::Unauthorized);
        }
        let key = decode(
            phone
                .extra
                .get("iphone_signing_key")
                .and_then(serde_json::Value::as_str)
                .ok_or(Error::Unauthorized)?,
        )?;
        verify(
            &key,
            &upload_message(path, &phone.id, at, length, &digest),
            signature,
        )?;
        Ok(UploadAuthorization { length, digest })
    }
    pub fn iphones(&self) -> Result<Vec<Device>, Error> {
        Ok(self
            .load()?
            .principals
            .into_iter()
            .filter(|p| p.kind == "iphone")
            .map(|p| Device {
                id: p.id,
                gateway_key: String::new(),
                pair_proof: String::new(),
            })
            .collect())
    }
    pub fn pair_iphone_code(
        &self,
        address: &str,
        remote: bool,
        ttl: std::time::Duration,
    ) -> Result<String, Error> {
        let url = reqwest::Url::parse(address).map_err(|_| Error::Invalid)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || url.port() == Some(0)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || ttl.is_zero()
            || ttl > std::time::Duration::from_secs(600)
        {
            return Err(Error::Invalid);
        }
        let mut secret = [0; 20];
        rand::thread_rng().fill_bytes(&mut secret);
        let id = ulid::Ulid::new().to_string();
        let duration = jiff::SignedDuration::try_from(ttl).map_err(|_| Error::Invalid)?;
        let expires = (jiff::Timestamp::now() + duration).to_string();
        let address = url.as_str().trim_end_matches('/');
        let code = serde_json::json!({"purpose":"iphone", "remote":remote, "id":id,
            "address":address, "fingerprint":self.fingerprint, "gateway_key":self.iphone_gateway_key()?, "secret":STANDARD.encode(secret), "expires_at":expires});
        self.update(|d| {
            d.pairings.insert(id, serde_json::json!({"purpose":"iphone", "key":STANDARD.encode(Sha256::digest(secret)), "address":address,
                "expires_at":expires, "attempts":0, "used":false, "grants":null}));
            Ok(())
        })?;
        Ok(format!(
            "ab1-{}",
            data_encoding::BASE32_NOPAD.encode(&serde_json::to_vec(&code)?)
        ))
    }
    pub fn set_iphone_push_token(&self, id: &str, token: &str) -> Result<(), Error> {
        if !(32..=512).contains(&token.len())
            || !token
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid);
        }
        self.update(|d| {
            let p = d
                .principals
                .iter_mut()
                .find(|p| p.id == id && p.kind == "iphone")
                .ok_or(Error::Unauthorized)?;
            p.extra.insert("iphone_push_token".into(), token.into());
            Ok(())
        })
    }
}

impl Authority {
    pub fn revoke_iphone(&self, id: &str) -> Result<(), Error> {
        self.update(|d| {
            let index = d
                .principals
                .iter()
                .position(|p| p.id == id && p.kind == "iphone")
                .ok_or(Error::Invalid)?;
            d.principals.remove(index);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    type Check = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn a_pairing_issued_before_this_version_still_completes() -> Check {
        let home = tempfile::tempdir()?;
        let authority = Authority::open(home.path())?;
        let secret = [9; 20];
        let key = Sha256::digest(secret);
        let expires = (jiff::Timestamp::now() + jiff::SignedDuration::from_secs(60)).to_string();
        authority.update(|d| {
            d.pairings.insert(
                "old".into(),
                serde_json::json!({"purpose":"iphone", "key":STANDARD.encode(key),
                "expires_at":expires, "attempts":0, "used":false, "grants":null}),
            );
            Ok(())
        })?;
        let signing = SigningKey::from_bytes(&[31; 32]);
        let encryption = [5; 32];
        let public = signing.verifying_key().to_bytes();
        let message = pair_message("old", &public, &encryption);
        let mut mac = Hmac::<Sha256>::new_from_slice(&key)?;
        mac.update(&message);
        let device = authority.pair_iphone(&PairRequest {
            id: "old".into(),
            signing_key: STANDARD.encode(public),
            encryption_key: STANDARD.encode(encryption),
            proof: STANDARD.encode(mac.finalize().into_bytes()),
            signature: STANDARD.encode(signing.sign(&message).to_bytes()),
        })?;
        assert!(authority.principal(&device.id).is_ok());
        Ok(())
    }
}
