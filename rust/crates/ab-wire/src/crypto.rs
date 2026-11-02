use ab_domain::ErrorCode;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::{PublicKey, X25519_BASEPOINT_BYTES, x25519};

pub fn ipc_proof(
    secret: &[u8],
    role: &str,
    nonce: &str,
    peer_nonce: &str,
) -> Result<Vec<u8>, ErrorCode> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).map_err(|_| ErrorCode::Invalid)?;
    mac.update(format!("apple-bridge-ipc/1\0{role}\0{nonce}\0{peer_nonce}").as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

pub fn sign_delegation(raw: &[u8], key: &SigningKey) -> String {
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(raw),
        URL_SAFE_NO_PAD.encode(key.sign(raw).to_bytes())
    )
}
pub fn verify_delegation(header: &str, key: &VerifyingKey) -> Result<Vec<u8>, ErrorCode> {
    let (payload, signature) = header.split_once('.').ok_or(ErrorCode::Invalid)?;
    let raw = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| ErrorCode::Invalid)?;
    let sig = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| ErrorCode::Invalid)?;
    let sig = Signature::from_slice(&sig).map_err(|_| ErrorCode::Unauthorized)?;
    key.verify_strict(&raw, &sig)
        .map_err(|_| ErrorCode::Unauthorized)?;
    Ok(raw)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Packet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_activity: Option<Box<serde_json::value::RawValue>>,
    pub ephemeral_key: String,
    pub r#box: String,
    pub signature: String,
}
impl Packet {
    pub fn signature_input(&self) -> Result<Vec<u8>, ErrorCode> {
        let mut raw = b"apple-bridge-iphone-command/1\0".to_vec();
        raw.extend(
            STANDARD
                .decode(&self.ephemeral_key)
                .map_err(|_| ErrorCode::Invalid)?,
        );
        raw.extend(
            STANDARD
                .decode(&self.r#box)
                .map_err(|_| ErrorCode::Invalid)?,
        );
        Ok(raw)
    }
}
fn cipher(private: [u8; 32], public: [u8; 32], device_id: &str) -> Result<Aes256Gcm, ErrorCode> {
    let shared = x25519(private, public);
    if shared == [0; 32] {
        return Err(ErrorCode::Invalid);
    }
    let mut key = [0; 32];
    Hkdf::<Sha256>::new(Some(b"apple-bridge-iphone/1"), &shared)
        .expand(device_id.as_bytes(), &mut key)
        .map_err(|_| ErrorCode::Invalid)?;
    Aes256Gcm::new_from_slice(&key).map_err(|_| ErrorCode::Invalid)
}
pub fn seal(
    recipient: [u8; 32],
    device_id: &str,
    plain: &[u8],
    ephemeral_private: [u8; 32],
    nonce: [u8; 12],
    signer: &SigningKey,
) -> Result<Packet, ErrorCode> {
    let cipher = cipher(ephemeral_private, recipient, device_id)?;
    let mut sealed = nonce.to_vec();
    sealed.extend(
        cipher
            .encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|_| ErrorCode::Invalid)?,
    );
    let mut packet = Packet {
        live_activity: None,
        ephemeral_key: STANDARD.encode(x25519(ephemeral_private, X25519_BASEPOINT_BYTES)),
        r#box: STANDARD.encode(sealed),
        signature: String::new(),
    };
    packet.signature = STANDARD.encode(signer.sign(&packet.signature_input()?).to_bytes());
    Ok(packet)
}
pub fn open(
    private: [u8; 32],
    device_id: &str,
    packet: &Packet,
    gateway: &VerifyingKey,
) -> Result<Vec<u8>, ErrorCode> {
    let signature = STANDARD
        .decode(&packet.signature)
        .map_err(|_| ErrorCode::Unauthorized)?;
    gateway
        .verify_strict(
            &packet.signature_input()?,
            &Signature::from_slice(&signature).map_err(|_| ErrorCode::Unauthorized)?,
        )
        .map_err(|_| ErrorCode::Unauthorized)?;
    let public: [u8; 32] = STANDARD
        .decode(&packet.ephemeral_key)
        .map_err(|_| ErrorCode::Invalid)?
        .try_into()
        .map_err(|_| ErrorCode::Invalid)?;
    let cipher = cipher(private, public, device_id)?;
    let sealed = STANDARD
        .decode(&packet.r#box)
        .map_err(|_| ErrorCode::Invalid)?;
    if sealed.len() < 12 {
        return Err(ErrorCode::Invalid);
    }
    cipher
        .decrypt(Nonce::from_slice(&sealed[..12]), &sealed[12..])
        .map_err(|_| ErrorCode::Corrupt)
}
pub fn public_key(private: [u8; 32]) -> PublicKey {
    PublicKey::from(x25519(private, X25519_BASEPOINT_BYTES))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delegation {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub local_level: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub challenge: String,
    pub replica_id: String,
    pub local_principal: String,
    pub local_grants: Vec<String>,
    pub command_id: String,
    pub request_hash: String,
    pub iat: i64,
}
impl Delegation {
    pub fn sign(&self, key: &SigningKey) -> Result<String, ErrorCode> {
        let raw = crate::gojson::legacy_json(self, crate::gojson::EscapeMode::Html)
            .map_err(|_| ErrorCode::Invalid)?;
        Ok(sign_delegation(&raw, key))
    }
}
