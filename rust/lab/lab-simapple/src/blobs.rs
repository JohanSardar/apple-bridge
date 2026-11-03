use super::{
    Request, Result, Shared, State, Value, enqueue, invalid, json, lock, mpsc, now, observation,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_FIXTURE: u64 = 32 << 20;
const MAX_CHUNK: u64 = 128 << 10;
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Debug, Default)]
pub struct Fixture {
    bytes: BTreeMap<String, Vec<u8>>,
    state: String,
    delay_ms: u64,
    drops: u64,
    busy: u64,
    refresh: bool,
    fault_digest: String,
}

#[derive(Debug)]
pub enum Read {
    Foreign,
    Drop,
    Reply {
        frame: Value,
        delay_ms: u64,
        refresh: Option<String>,
    },
}

fn base64(bytes: &[u8]) -> String {
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

pub fn handles(fixture: &Fixture) -> Value {
    Value::Object(
        fixture
            .bytes
            .keys()
            .map(|digest| (digest.clone(), json!(digest)))
            .collect(),
    )
}

fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref":{"collection":collection,"native_key":key}})
}

fn fixture_bytes(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| u8::try_from((i * 31 + i / 251) % 256).unwrap_or_default())
        .collect()
}

pub fn fixture(state: &Shared, tx: &mpsc::Sender<Request>, input: &Value) -> Result<Value> {
    let size = input["size"].as_u64().unwrap_or_default();
    if size > MAX_FIXTURE {
        return Err(invalid("blob fixture size"));
    }
    let mut s = lock(state)?;
    s.blob.refresh = input["refresh_before_unavailable"] == true;
    s.blob.busy = input["unavailable_reads"].as_u64().unwrap_or_default();
    s.blob.drops = input["drop_reads"].as_u64().unwrap_or_default();
    s.blob.state = input["state"].as_str().unwrap_or_default().to_owned();
    s.blob.delay_ms = input["slow_ms"].as_u64().unwrap_or_default();
    if input["delete"] == true {
        let keys: Vec<String> = s
            .records()
            .iter()
            .filter(|r| super::source(r["collection"].as_str().unwrap_or_default()) == "messages")
            .map(|r| super::string(r, "native_key"))
            .collect();
        for key in keys {
            let mut record = s.data["records"][&key].clone();
            record["kind"] = json!("absent");
            record["revalidated"] = json!(true);
            record["observed_at"] = json!(now());
            s.put(record);
        }
    } else {
        let bytes = if input["relay_fixture"] == true {
            b"[ab-test] relay blob\n".repeat(8192)
        } else {
            fixture_bytes(usize::try_from(size).map_err(|_| invalid("size"))?)
        };
        let digest = hex::encode(Sha256::digest(&bytes));
        let length = bytes.len();
        s.blob.bytes.insert(digest.clone(), bytes);
        s.blob.fault_digest.clone_from(&digest);
        let blob_state = if s.blob.state.is_empty() {
            "present".to_owned()
        } else {
            s.blob.state.clone()
        };
        s.put(observation(
            "messages.message",
            "blob-message",
            json!({"title":"[ab-test] blob-message","text":"[ab-test] attachment","kind":"message","decode_status":"decoded","parts":[{"index":0,"kind":"attachment","attachment_id":reference("messages.attachment","blob-attachment")}]}),
        ));
        s.put(observation(
            "messages.attachment",
            "blob-attachment",
            json!({"title":"[ab-test] blob-attachment","message_id":reference("messages.message","blob-message"),"filename":"fixture.bin","mime_type":"application/octet-stream","uti":"public.data","size":length,"blob":{"digest":digest,"state":blob_state}}),
        ));
    }
    s.save()?;
    let frame = s.scan("messages")?;
    drop(s);
    super::enqueue(tx, frame, true)?;
    Ok(json!({"effect_done":true}))
}

pub fn faults(state: &Shared) -> Result<Value> {
    Ok(json!({"unavailable_remaining":lock(state)?.blob.busy}))
}

pub fn history(state: &Shared, tx: &mpsc::Sender<Request>, input: &Value) -> Result<Value> {
    let rows = input["rows"]
        .as_u64()
        .filter(|rows| (1..=20000).contains(rows))
        .ok_or_else(|| invalid("rows"))?;
    let mut s = lock(state)?;
    s.put(observation(
        "reminders.list",
        "list",
        json!({"title":"Bridge Test","writable":true}),
    ));
    for i in 0..rows {
        let key = format!("blob-history-{i:05}");
        let mut data = json!({"title":format!("[ab-test] {key}"),"completed":false,"list_id":reference("reminders.list","list")});
        if i % 5 == 0 {
            data["blob"] = json!({"digest":hex::encode(Sha256::digest(key.as_bytes())),"size":128,"state":"offloaded"});
        }
        s.put(observation("reminders.reminder", &key, data));
    }
    s.save()?;
    let frame = s.scan("reminders")?;
    drop(s);
    let start = std::time::Instant::now();
    enqueue(tx, frame, true)?;
    let elapsed = start.elapsed().as_secs_f64();
    Ok(
        json!({"commit_ms":elapsed * 1000.0,"rows_per_second":rows as f64 / elapsed.max(f64::EPSILON)}),
    )
}

pub fn read(state: &Shared, frame: &Value) -> Result<Read> {
    let handle = super::string(frame, "handle");
    let digest = handle
        .split_once(':')
        .map_or(handle.as_str(), |(_, digest)| digest)
        .to_owned();
    let mut s = lock(state)?;
    let blob = &mut s.blob;
    let Some(data) = blob.bytes.get(&digest).cloned() else {
        return Ok(Read::Foreign);
    };
    let faulted = digest == blob.fault_digest;
    let busy = faulted && blob.busy > 0;
    if busy {
        blob.busy -= 1;
    }
    if faulted && blob.drops > 0 {
        blob.drops -= 1;
        return Ok(Read::Drop);
    }
    let unavailable = |reason: &str| json!({"t":"BlobUnavailable","handle":handle,"reason":reason});
    if busy {
        return Ok(Read::Reply {
            frame: unavailable("unavailable"),
            delay_ms: 0,
            refresh: blob.refresh.then(|| digest.clone()),
        });
    }
    if blob.state == "offloaded" || blob.state == "missing" {
        let reason = blob.state.clone();
        return Ok(Read::Reply {
            frame: unavailable(&reason),
            delay_ms: 0,
            refresh: None,
        });
    }
    let offset = frame["offset"].as_u64().unwrap_or_default();
    let length = frame["length"].as_u64().unwrap_or_default();
    if offset > data.len() as u64 || length == 0 || length > MAX_CHUNK {
        return Err(invalid("blob range"));
    }
    let end = (offset + length).min(data.len() as u64);
    let slice = &data[usize::try_from(offset).map_err(|_| invalid("offset"))?
        ..usize::try_from(end).map_err(|_| invalid("end"))?];
    Ok(Read::Reply {
        frame: json!({"t":"BlobChunk","handle":handle,"offset":offset,"bytes":base64(slice),"digest":digest,"eof":end == data.len() as u64}),
        delay_ms: blob.delay_ms,
        refresh: None,
    })
}

pub fn refresh_frame(digest: &str) -> Value {
    json!({"t":"Capture","source":"messages","observations":[],"blob_handles":{digest:digest}})
}

impl State {
    pub fn attach_handles(&self, frame: &mut Value) {
        if frame["source"] == "messages" && !self.blob.bytes.is_empty() {
            frame["blob_handles"] = handles(&self.blob);
        }
    }
}
