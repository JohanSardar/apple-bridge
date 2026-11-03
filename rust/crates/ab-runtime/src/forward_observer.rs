use ab_operations::Error;
use data_encoding::{BASE64, BASE64URL_NOPAD};
use serde_json::{Map, Value, json, value::RawValue};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Mutex;

static WRITER: Mutex<()> = Mutex::new(());

pub(crate) struct Observation {
    path: String,
    start: String,
    header: String,
    body: Vec<u8>,
}

fn stamp() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S.%9fZ")
        .to_string()
}

pub(crate) fn begin(header: &str, raw: &RawValue) -> Option<Observation> {
    let path = std::env::var("APPLE_BRIDGE_LAB_FEED_LOG").ok()?;
    if path.is_empty() {
        return None;
    }
    Some(Observation {
        path,
        start: stamp(),
        header: header.to_owned(),
        body: raw.get().as_bytes().to_vec(),
    })
}

impl Observation {
    pub(crate) fn finish(self, result: &Result<Box<RawValue>, Error>) {
        let _ = self.write(result);
    }
    fn write(self, result: &Result<Box<RawValue>, Error>) -> Option<()> {
        let request: Value = serde_json::from_slice(&self.body).ok()?;
        let method = request["method"].as_str()?.to_owned();
        if !matches!(method.as_str(), "submit" | "status" | "list") {
            return None;
        }
        let outcome = match result {
            Err(error) => {
                let value = serde_json::to_value(error).ok()?;
                let code = value["code"].as_str()?;
                if code == "unauthorized" {
                    return None;
                }
                json!({"status": 400, "error": code})
            }
            Ok(raw) => {
                let value: Value = serde_json::from_str(raw.get()).ok()?;
                if method == "list" {
                    json!({"status": 200, "count": value.as_array()?.len()})
                } else {
                    json!({
                        "status": 200,
                        "state": value["state"].as_str().unwrap_or(""),
                        "disposition": value["disposition"].as_str().unwrap_or(""),
                    })
                }
            }
        };
        let payload = BASE64URL_NOPAD
            .decode(self.header.split('.').next()?.as_bytes())
            .ok()?;
        let delegation: Map<String, Value> = serde_json::from_slice(&payload).ok()?;
        let kept: Map<String, Value> = [
            "local_level",
            "local_principal",
            "local_grants",
            "command_id",
            "request_hash",
        ]
        .iter()
        .filter_map(|key| {
            delegation
                .get(*key)
                .map(|value| ((*key).to_owned(), value.clone()))
        })
        .collect();
        let mut line = serde_json::to_vec(&json!({
            "method": method,
            "start": self.start,
            "end": stamp(),
            "body": BASE64.encode(&self.body),
            "delegation": kept,
            "outcome": outcome,
        }))
        .ok()?;
        line.push(b'\n');
        let _guard = WRITER.lock().ok()?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.path)
            .ok()?;
        file.write_all(&line).ok()
    }
}
