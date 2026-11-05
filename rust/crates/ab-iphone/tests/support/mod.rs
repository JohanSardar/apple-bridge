#![allow(dead_code)]

use ab_iphone::{
    Error,
    delivery::{Delivery, Gateway, HealthBatch, LocationRecord, QueuedCommand},
};
use ab_trust::{
    Authority,
    domain::{CommandState, Disposition},
    iphone,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use hmac::{Hmac, Mac};
use serde_json::{Value, json, value::RawValue};
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

pub type Check = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Default)]
pub struct Rows(pub Arc<Mutex<Vec<QueuedCommand>>>);

impl Gateway for Rows {
    fn commands(&mut self) -> Result<Vec<QueuedCommand>, Error> {
        Ok(self.0.lock().map_err(|_| Error::Unavailable)?.clone())
    }
    fn command(&mut self, id: &str) -> Result<QueuedCommand, Error> {
        self.0
            .lock()
            .map_err(|_| Error::Unavailable)?
            .iter()
            .find(|row| {
                serde_json::from_str::<Value>(row.command.get())
                    .is_ok_and(|fields| fields["command_id"] == id)
            })
            .cloned()
            .ok_or(Error::NotFound)
    }
    fn transition(
        &mut self,
        prior: &QueuedCommand,
        state: CommandState,
        _disposition: Option<Disposition>,
        _evidence: Option<&RawValue>,
    ) -> Result<QueuedCommand, Error> {
        let mut rows = self.0.lock().map_err(|_| Error::Unavailable)?;
        let row = rows
            .iter_mut()
            .find(|row| row.command.get() == prior.command.get())
            .ok_or(Error::NotFound)?;
        row.state = state;
        Ok(row.clone())
    }
    fn authorize(&mut self, _command: &QueuedCommand) -> Result<(), Error> {
        Ok(())
    }
    fn decide(&mut self, _id: &str, _hash: &str, _approve: bool) -> Result<(), Error> {
        Ok(())
    }
    fn ingest_health(
        &mut self,
        _device: &str,
        _batch: &HealthBatch,
        _digest: &str,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn ingest_locations(
        &mut self,
        _device: &str,
        _records: &[LocationRecord],
    ) -> Result<(), Error> {
        Ok(())
    }
}

pub struct Phone {
    pub id: String,
    pub signing: SigningKey,
    pub encryption_private: [u8; 32],
    pub encryption_public: [u8; 32],
    pub gateway_key: ed25519_dalek::VerifyingKey,
}

pub struct World {
    pub home: tempfile::TempDir,
    pub rows: Rows,
    pub delivery: Arc<Delivery<Rows>>,
    pub phone: Phone,
}

pub fn world() -> Result<World, Box<dyn std::error::Error>> {
    let home = tempfile::tempdir()?;
    let authority = Arc::new(Authority::open(home.path())?);
    let code =
        authority.pair_iphone_code("https://mac.example.test", true, Duration::from_secs(60))?;
    let raw = data_encoding::BASE32_NOPAD
        .decode(code.strip_prefix("ab1-").ok_or("prefix")?.as_bytes())?;
    let code: Value = serde_json::from_slice(&raw)?;
    let id = code["id"].as_str().ok_or("id")?;
    let secret = STANDARD.decode(code["secret"].as_str().ok_or("secret")?)?;
    let signing = SigningKey::from_bytes(&[31; 32]);
    let encryption_private = [17; 32];
    let encryption_public = ab_wire::crypto::public_key(encryption_private).to_bytes();
    let public = signing.verifying_key().to_bytes();
    let message = iphone::pair_message(id, &public, &encryption_public);
    let mut mac = Hmac::<Sha256>::new_from_slice(&Sha256::digest(&secret))?;
    mac.update(&message);
    let device = authority.pair_iphone(&serde_json::from_value(json!({
        "id": id,
        "signing_key": STANDARD.encode(public),
        "encryption_key": STANDARD.encode(encryption_public),
        "proof": STANDARD.encode(mac.finalize().into_bytes()),
        "signature": STANDARD.encode(signing.sign(&message).to_bytes()),
    }))?)?;
    let key: [u8; 32] = STANDARD
        .decode(authority.iphone_gateway_key()?)?
        .try_into()
        .map_err(|_| "key length")?;
    let rows = Rows::default();
    let delivery = Arc::new(Delivery::new(authority, rows.clone()));
    Ok(World {
        home,
        rows,
        delivery,
        phone: Phone {
            id: device.id,
            signing,
            encryption_private,
            encryption_public,
            gateway_key: ed25519_dalek::VerifyingKey::from_bytes(&key)?,
        },
    })
}

impl Phone {
    pub fn body(&self, path: &str, payload: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let at = ab_trust::decision_now().as_second();
        let message = iphone::request_message(path, at, payload);
        Ok(serde_json::to_vec(&json!({
            "device_id": self.id,
            "at": at,
            "payload": STANDARD.encode(payload),
            "signature": STANDARD.encode(self.signing.sign(&message).to_bytes()),
        }))?)
    }

    pub fn upload_header(&self, path: &str, body: &[u8]) -> String {
        self.upload_header_for(
            path,
            ab_trust::decision_now().as_second(),
            body.len() as u64,
            &Sha256::digest(body),
        )
    }

    pub fn upload_header_for(&self, path: &str, at: i64, length: u64, digest: &[u8]) -> String {
        let mut message = format!(
            "apple-bridge-iphone-upload/1\0{path}\0{}\0{at}\0{length}\0",
            self.id
        )
        .into_bytes();
        message.extend_from_slice(digest);
        format!(
            "{}.{at}.{length}.{}.{}",
            self.id,
            STANDARD.encode(digest),
            STANDARD.encode(self.signing.sign(&message).to_bytes()),
        )
    }
}

pub fn live_row(id: &str, phone: &str, op: &str, state: CommandState) -> QueuedCommand {
    let command = json!({
        "command_id": id,
        "op": op,
        "args": {
            "device_id": phone,
            "task_id": "private-task-marker",
            "title": "private-title-marker",
            "status": "private-status-marker",
            "progress": 0.5
        }
    });
    QueuedCommand {
        command: serde_json::value::to_raw_value(&command).unwrap_or_else(|_| {
            RawValue::from_string("null".into()).unwrap_or_else(|_| unreachable!())
        }),
        principal: "agent:fixture".into(),
        source: "iphone".into(),
        request_hash: format!("hash-{id}"),
        state,
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
    }
}
