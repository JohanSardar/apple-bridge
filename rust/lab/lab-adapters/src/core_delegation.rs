use crate::{
    core_frames,
    journey::effect_count,
    replicas::{self, Endpoint},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use lab_world::{Error, Result, World};
use rustls::pki_types::{PrivateKeyDer, pem::PemObject};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn document(home: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(
        home.join("credentials.json"),
    )?)?)
}
fn effective(grants: &[String], ceiling: &[String]) -> Vec<String> {
    let allowed = |grant: &str| {
        ceiling.iter().any(|limit| {
            limit == "admin"
                || limit == grant
                || format!("{}:write", grant.trim_end_matches(":read")) == *limit
        })
    };
    let mut result = vec![];
    for grant in grants {
        if allowed(grant) {
            result.push(grant.clone());
        } else if let Some(base) = grant.strip_suffix(":write") {
            let read = format!("{base}:read");
            if allowed(&read) {
                result.push(read);
            }
        }
    }
    result
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}
fn signing_key(home: &Path) -> Result<SigningKey> {
    let name = hex::encode(Sha256::digest(b"identity-key"));
    let pem = fs::read(home.join("secrets").join(name))?;
    let der = PrivateKeyDer::from_pem_slice(&pem).map_err(|e| Error::Invalid(e.to_string()))?;
    let bytes = der.secret_der();
    let seed: [u8; 32] = bytes
        .get(bytes.len().saturating_sub(32)..)
        .and_then(|tail| tail.try_into().ok())
        .ok_or_else(|| Error::Invalid("identity key shape".into()))?;
    Ok(SigningKey::from_bytes(&seed))
}
fn post(address: &str, home: &Path, pin: &str, header: &str, body: &[u8]) -> Result<Value> {
    let mut stream = core_frames::pinned_client(address, home, pin)?;
    stream
        .sock
        .set_read_timeout(Some(std::time::Duration::from_secs(15)))?;
    stream.write_all(
        format!(
            "POST /v1/commands HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nX-Apple-Bridge-Delegation: {header}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(body)?;
    let mut raw = vec![];
    let _ = stream.read_to_end(&mut raw);
    let start = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(raw.len(), |at| at + 4);
    Ok(serde_json::from_slice(&raw[start..])?)
}

pub fn forged(world: &World, endpoint: &Endpoint, list: &str) -> Result<bool> {
    let credentials = document(&endpoint.home)?;
    let token_hash = hex::encode(Sha256::digest(endpoint.token.as_bytes()));
    let principal = credentials["principals"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["token_hash"] == token_hash.as_str())
        .ok_or_else(|| Error::Invalid("world principal unavailable".into()))?;
    let enrollment = &credentials["enrollment"];
    let mut grants = effective(
        &strings(&principal["grants"]),
        &strings(&enrollment["grants"]),
    );
    grants.push("notes:write".into());
    let args = json!({"list_id": list, "title": "[ab-test] forged"});
    let id = crate::http::random_id("cmd")?;
    let canonical = serde_json::to_string(&args)?;
    let hash = hex::encode(Sha256::digest(
        format!(
            r#"{{"Op":"reminders.create","Args":{canonical},"Version":0,"Expires":"0001-01-01T00:00:00Z","Era":""}}"#
        )
        .as_bytes(),
    ));
    let iat = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let raw = serde_json::to_vec(&json!({
        "replica_id": enrollment["replica_id"],
        "local_principal": principal["id"],
        "local_grants": grants,
        "command_id": id,
        "request_hash": hash,
        "iat": iat,
    }))?;
    let signature = signing_key(&endpoint.home)?.sign(&raw);
    let header = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&raw),
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    );
    let body = serde_json::to_vec(&json!({"method": "submit", "command": {
        "command_id": id, "op": "reminders.create", "args": args,
        "expires_at": "0001-01-01T00:00:00Z"}}))?;
    let address = enrollment["address"].as_str().unwrap_or_default();
    let pin = enrollment["fingerprint"].as_str().unwrap_or_default();
    let reply = post(address, &endpoint.home, pin, &header, &body)?;
    Ok(reply["error"]["code"] == "unauthorized" && effect_count(world, &id) == Some(0))
}

pub fn run(world: &World, list: &str) -> bool {
    replicas::endpoints()
        .first()
        .is_some_and(|endpoint| forged(world, endpoint, list).unwrap_or(false))
}
