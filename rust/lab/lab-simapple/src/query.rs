use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

pub type Cancel = Arc<(Mutex<bool>, Condvar)>;

pub fn cancel(token: &Cancel) {
    if let Ok(mut flag) = token.0.lock() {
        *flag = true;
        token.1.notify_all();
    }
}

pub fn sleep_request(frame: &Value) -> Option<u64> {
    if frame["op"] != "test.sleep" {
        return None;
    }
    let args = frame.get("args").cloned().unwrap_or_else(|| json!({}));
    number(&args, "ms").filter(|ms| *ms <= 60000)
}

pub fn sleep(
    id: Value,
    ms: u64,
    deadline: u64,
    token: Cancel,
    done: mpsc::Sender<(String, Value)>,
) {
    thread::spawn(move || {
        let wait = ms.min(deadline);
        let end = Instant::now() + Duration::from_millis(wait);
        let mut cancelled = token.0.lock().is_ok_and(|g| *g);
        while !cancelled {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let Ok(guard) = token.0.lock() else { break };
            let Ok((guard, _)) = token.1.wait_timeout(guard, left) else {
                break;
            };
            cancelled = *guard;
        }
        let reply = if cancelled || ms > deadline {
            failure(&id, "deadline")
        } else {
            json!({"t":"QueryResult","id":id,"result_json":{"ms":ms},"blobs":[]})
        };
        let _ = done.send((id.as_str().unwrap_or_default().to_owned(), reply));
    });
}

const MAX_BLOB: u64 = 64 << 20;
const MAX_CHUNK: u64 = 128 << 10;
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let n = group
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= group.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn number(args: &Value, field: &str) -> Option<u64> {
    let map = args.as_object().filter(|m| m.len() == 1)?;
    map.get(field)?.as_u64()
}

fn failure(id: &Value, code: &str) -> Value {
    json!({"t":"QueryError","id":id,"code":code})
}

pub fn query(blobs: &mut BTreeMap<String, Vec<u8>>, frame: &Value) -> Value {
    let id = &frame["id"];
    let args = frame.get("args").cloned().unwrap_or_else(|| json!({}));
    match frame["op"].as_str() {
        Some("test.echo") => json!({"t":"QueryResult","id":id,"result_json":args,"blobs":[]}),
        Some("helper.version") => {
            json!({"t":"QueryResult","id":id,"result_json":{"version":"sim","protocol":"ipc/1"},"blobs":[]})
        }
        Some("shortcuts.actions") => {
            if args.as_object().is_some_and(serde_json::Map::is_empty) {
                json!({"t":"QueryResult","id":id,"result_json":{"items":[{"app":"com.example.synthetic","identifier":"Export","title":"Export"}],"complete":true,"schema_guaranteed":false},"blobs":[]})
            } else {
                failure(id, "invalid")
            }
        }
        Some("test.sleep") => failure(id, "invalid"),
        Some("test.blob") => {
            let Some(size) = number(&args, "size").filter(|size| *size <= MAX_BLOB) else {
                return failure(id, "invalid");
            };
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let digest = hex::encode(Sha256::digest(&data));
            blobs.insert(digest.clone(), data);
            json!({"t":"QueryResult","id":id,"result_json":{},"blobs":[{"digest":digest,"size":size,"handle":digest}]})
        }
        _ => failure(id, "unsupported"),
    }
}

pub fn blob(blobs: &BTreeMap<String, Vec<u8>>, frame: &Value) -> Option<Value> {
    let handle = frame["handle"].as_str()?;
    let Some(data) = blobs.get(handle) else {
        return Some(json!({"t":"BlobUnavailable","handle":handle,"reason":"missing"}));
    };
    let offset = frame
        .get("offset")
        .map_or(Some(0), Value::as_u64)
        .filter(|o| *o <= data.len() as u64)?;
    let length = frame["length"]
        .as_u64()
        .filter(|l| (1..=MAX_CHUNK).contains(l))?;
    let end = (offset + length).min(data.len() as u64);
    let slice = &data[usize::try_from(offset).ok()?..usize::try_from(end).ok()?];
    Some(
        json!({"t":"BlobChunk","handle":handle,"offset":offset,"bytes":base64(slice),"digest":handle,"eof":end == data.len() as u64}),
    )
}
