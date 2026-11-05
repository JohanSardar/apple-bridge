use ab_trust::{Authority, iphone};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

type Check = Result<(), Box<dyn std::error::Error>>;
const ADDRESS: &str = "https://mac.example.test";

fn open_code(code: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let raw = data_encoding::BASE32_NOPAD
        .decode(code.strip_prefix("ab1-").ok_or("prefix")?.as_bytes())?;
    Ok(serde_json::from_slice(&raw)?)
}

struct Paired {
    code: Value,
    signing: SigningKey,
    encryption_private: [u8; 32],
    encryption: [u8; 32],
    device: Value,
}

fn pair(authority: &Authority) -> Result<Paired, Box<dyn std::error::Error>> {
    let code = open_code(&authority.pair_iphone_code(ADDRESS, true, Duration::from_secs(60))?)?;
    let id = code["id"].as_str().ok_or("id")?;
    let secret = STANDARD.decode(code["secret"].as_str().ok_or("secret")?)?;
    let signing = SigningKey::from_bytes(&[31; 32]);
    let encryption_private = [17; 32];
    let encryption = ab_wire::crypto::public_key(encryption_private).to_bytes();
    let public = signing.verifying_key().to_bytes();
    let message = iphone::pair_message(id, &public, &encryption);
    let mut mac = Hmac::<Sha256>::new_from_slice(&Sha256::digest(&secret))?;
    mac.update(&message);
    let request: iphone::PairRequest = serde_json::from_value(json!({
        "id": id,
        "signing_key": STANDARD.encode(public),
        "encryption_key": STANDARD.encode(encryption),
        "proof": STANDARD.encode(mac.finalize().into_bytes()),
        "signature": STANDARD.encode(signing.sign(&message).to_bytes()),
    }))?;
    let device = serde_json::to_value(authority.pair_iphone(&request)?)?;
    Ok(Paired {
        code,
        signing,
        encryption_private,
        encryption,
        device,
    })
}

fn transcript(paired: &Paired, gateway_key: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut message = format!(
        "apple-bridge-iphone-pair-response/1\0{}\0{}\0{}\0",
        paired.code["id"].as_str().ok_or("id")?,
        paired.code["address"].as_str().ok_or("address")?,
        paired.device["id"].as_str().ok_or("device")?,
    )
    .into_bytes();
    message.extend_from_slice(&paired.signing.verifying_key().to_bytes());
    message.extend_from_slice(&paired.encryption);
    message.extend_from_slice(gateway_key);
    Ok(message)
}

fn response_proof_is_valid(
    paired: &Paired,
    gateway_key: &[u8],
) -> Result<bool, Box<dyn std::error::Error>> {
    let secret = STANDARD.decode(paired.code["secret"].as_str().ok_or("secret")?)?;
    let proof = STANDARD.decode(paired.device["pair_proof"].as_str().unwrap_or_default())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&Sha256::digest(&secret))?;
    mac.update(&transcript(paired, gateway_key)?);
    Ok(mac.verify_slice(&proof).is_ok())
}

#[test]
fn the_pairing_code_commits_the_gateway_signing_key() -> Check {
    let home = tempfile::tempdir()?;
    let authority = Authority::open(home.path())?;
    let code = open_code(&authority.pair_iphone_code(ADDRESS, true, Duration::from_secs(60))?)?;
    assert_eq!(
        code["gateway_key"],
        authority.iphone_gateway_key()?.as_str()
    );
    Ok(())
}

#[test]
fn the_pair_response_is_authenticated_by_the_pairing_secret() -> Check {
    let home = tempfile::tempdir()?;
    let authority = Authority::open(home.path())?;
    let paired = pair(&authority)?;
    let gateway_key = STANDARD.decode(paired.device["gateway_key"].as_str().ok_or("key")?)?;
    assert_eq!(paired.device["gateway_key"], paired.code["gateway_key"]);
    assert!(response_proof_is_valid(&paired, &gateway_key)?);
    let substituted = SigningKey::from_bytes(&[66; 32]).verifying_key().to_bytes();
    assert!(!response_proof_is_valid(&paired, &substituted)?);
    Ok(())
}

#[test]
fn a_phone_that_ignores_the_new_fields_still_pairs_and_receives_commands() -> Check {
    #[derive(Deserialize)]
    struct OldPairCode {
        id: String,
        purpose: String,
        address: String,
        fingerprint: String,
        secret: String,
        expires_at: String,
        remote: Option<bool>,
    }
    #[derive(Deserialize)]
    struct OldPairedDevice {
        id: String,
        gateway_key: Option<String>,
    }
    let home = tempfile::tempdir()?;
    let authority = Authority::open(home.path())?;
    let paired = pair(&authority)?;
    let old_code: OldPairCode = serde_json::from_value(paired.code.clone())?;
    assert_eq!(old_code.purpose, "iphone");
    assert_eq!(old_code.address, ADDRESS);
    assert_eq!(old_code.fingerprint.len(), 64);
    assert!(!old_code.id.is_empty() && !old_code.secret.is_empty());
    assert!(!old_code.expires_at.is_empty() && old_code.remote == Some(true));
    let old: OldPairedDevice = serde_json::from_value(paired.device.clone())?;
    let phone = authority.principal(&old.id)?;
    let packet = authority.seal_iphone(&phone, b"synthetic command")?;
    let key: [u8; 32] = STANDARD
        .decode(old.gateway_key.ok_or("key")?)?
        .try_into()
        .map_err(|_| "key length")?;
    let gateway = ed25519_dalek::VerifyingKey::from_bytes(&key)?;
    assert_eq!(
        ab_wire::crypto::open(paired.encryption_private, &old.id, &packet, &gateway)?,
        b"synthetic command"
    );
    let at = ab_trust::decision_now().as_second();
    let message = iphone::request_message("/v1/iphone/pending", at, b"");
    let request: iphone::Request = serde_json::from_value(json!({
        "device_id": old.id,
        "at": at,
        "payload": "",
        "signature": STANDARD.encode(paired.signing.sign(&message).to_bytes()),
    }))?;
    assert!(
        authority
            .verify_iphone("/v1/iphone/pending", &request)
            .is_ok()
    );
    Ok(())
}

#[test]
fn the_status_endpoint_does_not_carry_a_pair_proof() -> Check {
    let home = tempfile::tempdir()?;
    let authority = Authority::open(home.path())?;
    let paired = pair(&authority)?;
    let id = paired.device["id"].as_str().ok_or("id")?;
    let at = ab_trust::decision_now().as_second();
    let message = iphone::request_message("/v1/iphone/status", at, b"");
    let request: iphone::Request = serde_json::from_value(json!({
        "device_id": id,
        "at": at,
        "payload": "",
        "signature": STANDARD.encode(paired.signing.sign(&message).to_bytes()),
    }))?;
    let status = serde_json::to_value(authority.iphone_status(&request)?)?;
    assert!(status.get("pair_proof").is_none());
    assert_eq!(status["gateway_key"], paired.code["gateway_key"]);
    Ok(())
}

#[test]
fn the_shared_vector_matches_this_implementation() -> Check {
    let golden: Value = serde_json::from_str(include_str!(
        "../../../../contracts/golden/iphone-pairing.json"
    ))?;
    let field = |name: &str| golden[name].as_str().ok_or("field");
    let bytes = |name: &str| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(STANDARD.decode(field(name)?)?)
    };
    let message = iphone::pair_response_message(
        field("pairing_id")?,
        field("address")?,
        field("device_id")?,
        &bytes("signing_key")?,
        &bytes("encryption_key")?,
        &bytes("gateway_key")?,
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(&Sha256::digest(bytes("secret")?))?;
    mac.update(&message);
    assert_eq!(mac.finalize().into_bytes().as_slice(), bytes("pair_proof")?);
    assert_eq!(
        iphone::upload_message(
            field("upload_path")?,
            field("device_id")?,
            golden["upload_at"].as_i64().ok_or("at")?,
            golden["upload_length"].as_u64().ok_or("length")?,
            &bytes("upload_digest")?,
        ),
        bytes("upload_message")?
    );
    Ok(())
}
