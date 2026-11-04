use crate::{Error, invalid};
use lab_wire::{ipc_proof, receive, send, verify_ipc_proof};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

type Result<T> = std::result::Result<T, Error>;

const NATIVE_MAX: usize = 262_144;
const CREDIT: u64 = 4_194_304;

fn read_native(stream: &mut UnixStream) -> Result<Value> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > NATIVE_MAX {
        return Err(invalid("native frame size"));
    }
    let mut bytes = vec![0_u8; size];
    stream.read_exact(&mut bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    if !value.is_object() || value["t"].as_str().is_none() {
        return Err(invalid("native frame"));
    }
    Ok(value)
}

fn write_native(stream: &Mutex<UnixStream>, value: &Value) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.is_empty() || body.len() > NATIVE_MAX {
        return Err(invalid("native frame size"));
    }
    let mut bytes = u32::try_from(body.len())
        .map_err(|_| invalid("native frame size"))?
        .to_be_bytes()
        .to_vec();
    bytes.extend(body);
    stream
        .lock()
        .map_err(|_| invalid("native writer"))?
        .write_all(&bytes)?;
    Ok(())
}

#[derive(Default)]
struct Book {
    sequence: u64,
    scans: BTreeMap<String, (String, usize)>,
    origins: BTreeMap<String, Value>,
    tickets: BTreeMap<String, Value>,
    dispatching: BTreeSet<String>,
    resolving: BTreeMap<String, u32>,
    fence: u64,
}

struct Shared {
    book: Mutex<Book>,
    home: PathBuf,
    refuse: bool,
}

impl Shared {
    fn book(&self) -> Result<std::sync::MutexGuard<'_, Book>> {
        self.book.lock().map_err(|_| invalid("bridge book"))
    }

    fn log(&self, line: &str) {
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.home.join("native-session.log"))
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn object(value: &Value) -> Value {
    if value.is_object() {
        value.clone()
    } else {
        json!({})
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub fn connect(path: &str, secret: &[u8], home: &Path) -> Result<UnixStream> {
    let mut native = UnixStream::connect(path)?;
    native.set_read_timeout(Some(Duration::from_secs(60)))?;
    let hello = read_native(&mut native)?;
    if hello["t"] != "Hello" || hello["protocol"] != "native/1" || hello["version"] != 1 {
        return Err(invalid("native hello"));
    }
    let fence = hello["owner_fence"].as_u64().unwrap_or(0);
    if fence == 0 {
        return Err(invalid("native fence"));
    }
    let native_writer = Arc::new(Mutex::new(native.try_clone()?));
    write_native(
        &native_writer,
        &json!({"t":"Welcome","protocol":"native/1","version":1}),
    )?;
    let session = read_native(&mut native)?;
    if session["t"] != "Session" {
        return Err(invalid("native session"));
    }
    native.set_read_timeout(None)?;
    let shared = Arc::new(Shared {
        book: Mutex::new(Book {
            fence,
            ..Book::default()
        }),
        home: home.to_owned(),
        refuse: std::env::var("AB_LAB_SESSION_CONTROL").as_deref() == Ok("refuse"),
    });
    let (simulator, bridge) = UnixStream::pair()?;
    let secret = secret.to_vec();
    let mut open_checkpoints = serde_json::Map::new();
    if let Some(offered) = session["checkpoints"].as_object() {
        for (source, checkpoint) in offered {
            open_checkpoints.insert(source.clone(), checkpoint.clone());
        }
    }
    thread::spawn(move || {
        let result = run(
            &shared,
            bridge,
            native,
            &native_writer,
            &secret,
            fence,
            &open_checkpoints,
        );
        if let Err(error) = result {
            eprintln!("native bridge: {error}");
        }
        if let Ok(stream) = native_writer.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
    Ok(simulator)
}

fn run(
    shared: &Arc<Shared>,
    mut bridge: UnixStream,
    native: UnixStream,
    native_writer: &Arc<Mutex<UnixStream>>,
    secret: &[u8],
    fence: u64,
    checkpoints: &serde_json::Map<String, Value>,
) -> Result<()> {
    bridge.set_read_timeout(Some(Duration::from_secs(10)))?;
    let a = lab_nonce()?;
    send(&mut bridge, &json!({"t":"Challenge","nonce":a}))?;
    let (answer, _) = receive(&mut bridge)?;
    let b = text(&answer, "nonce");
    if answer["t"] != "Authenticate" || b.len() != 64 {
        return Err(invalid("authenticate"));
    }
    verify_ipc_proof(secret, "helper", &a, &b, &text(&answer, "proof"))?;
    send(
        &mut bridge,
        &json!({"t":"Authenticated","proof":ipc_proof(secret,"gateway",&a,&b)?}),
    )?;
    send(
        &mut bridge,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":fence,"byte_credit":CREDIT,"checkpoints":checkpoints}),
    )?;
    bridge.set_read_timeout(None)?;
    let to_simulator = Arc::new(Mutex::new(bridge.try_clone()?));
    let inbound_shared = Arc::clone(shared);
    let inbound_simulator = Arc::clone(&to_simulator);
    let mut native_reader = native;
    let inbound = thread::spawn(move || -> Result<()> {
        let result = (|| -> Result<()> {
            loop {
                let frame = read_native(&mut native_reader)?;
                if frame["t"] == "Close" {
                    return Ok(());
                }
                let Some(translated) = to_ipc(&inbound_shared, &frame)? else {
                    continue;
                };
                let mut stream = inbound_simulator
                    .lock()
                    .map_err(|_| invalid("simulator writer"))?;
                send(&mut *stream, &translated)?;
            }
        })();
        if let Ok(stream) = inbound_simulator.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        result
    });
    let outbound = (|| -> Result<()> {
        loop {
            let (frame, size) = receive(&mut bridge)?;
            if let Some(translated) = to_native(shared, &frame, size)? {
                write_native(native_writer, &translated)?;
            }
        }
    })();
    if let Ok(stream) = to_simulator.lock() {
        let _ = stream.shutdown(Shutdown::Both);
    }
    let _ = native_writer
        .lock()
        .map(|stream| stream.shutdown(Shutdown::Both));
    let inbound = inbound.join().map_err(|_| invalid("bridge thread"))?;
    outbound.and(inbound)
}

fn lab_nonce() -> Result<String> {
    let mut bytes = [0_u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(hex::encode(bytes))
}

fn to_native(shared: &Shared, frame: &Value, size: usize) -> Result<Option<Value>> {
    let kind = frame["t"].as_str().unwrap_or_default();
    Ok(Some(match kind {
        "Ready" => {
            json!({"t":"Ready","coverage":object(&frame["coverage"]),"capabilities":object(&frame["capabilities"]),"helper_build":text(frame,"helper_build"),"os_build":text(frame,"os_build")})
        }
        "SourceState" => {
            json!({"t":"SourceState","source":frame["source"],"state":frame["state"],"coverage":object(&frame["coverage"]),"capabilities":object(&frame["capabilities"]),"reason":text(frame,"reason")})
        }
        "Capture" => {
            if shared.refuse {
                return Ok(None);
            }
            let mut book = shared.book()?;
            book.sequence += 1;
            let id = format!("scan-{}", book.sequence);
            book.scans.insert(id.clone(), (text(frame, "source"), size));
            let handles: serde_json::Map<String, Value> = frame["blob_handles"]
                .as_object()
                .map(|map| {
                    map.iter()
                        .filter(|(_, value)| value.is_string())
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let mut scan = json!({"t":"Scan","id":id,"source":frame["source"],"generation":frame["generation"],"observations":frame["observations"].as_array().cloned().unwrap_or_default(),"scope":strings(&frame["scope"]),"completion_evidence":text(frame,"completion_evidence"),"blob_handles":handles,"live":frame["live"].as_bool().unwrap_or(false),"whole":true});
            if !text(frame, "scan_id").is_empty() {
                scan["scan_id"] = frame["scan_id"].clone();
            }
            if !text(frame, "scan_started_at").is_empty() {
                scan["scan_started_at"] = frame["scan_started_at"].clone();
            }
            scan
        }
        "Prepared" => {
            let command = text(frame, "command_id");
            shared
                .book()?
                .tickets
                .insert(command.clone(), frame["ticket"].clone());
            json!({"t":"Prepared","command_id":command,"native_state":frame["native_state"],"route":text(frame,"route"),"candidate_ids":frame["candidate_ids"].as_array().cloned().unwrap_or_default(),"fingerprint":text(frame,"fingerprint")})
        }
        "Rejected" => {
            let mut rejected = json!({"t":"Rejected","command_id":frame["command_id"],"code":text(&frame["error"],"code"),"message":text(&frame["error"],"message")});
            if frame["evidence"].is_object() {
                rejected["evidence"] = frame["evidence"].clone();
            }
            rejected
        }
        "Outcome" => {
            let command = text(frame, "command_id");
            let mut book = shared.book()?;
            let resolved = match book.resolving.get_mut(&command) {
                Some(count) if *count > 1 => {
                    *count -= 1;
                    true
                }
                Some(_) => book.resolving.remove(&command).is_some(),
                None => false,
            };
            let live = book.dispatching.remove(&command) | resolved;
            if !live {
                return Ok(None);
            }
            json!({"t":"Outcome","command_id":command,"disposition":frame["disposition"],"evidence":frame["evidence"]})
        }
        "QueryResult" => {
            json!({"t":"QueryResult","id":frame["id"],"result_json":frame["result_json"],"blobs":frame["blobs"].as_array().cloned().unwrap_or_default()})
        }
        "QueryError" => json!({"t":"QueryError","id":frame["id"],"code":frame["code"]}),
        "PrimitiveResult" => {
            json!({"t":"PrimitiveResult","id":frame["id"],"result":frame["result"]})
        }
        "PrimitiveError" => json!({"t":"PrimitiveError","id":frame["id"],"code":frame["code"]}),
        "BlobChunk" => {
            json!({"t":"BlobChunk","handle":frame["handle"],"offset":frame["offset"],"bytes":frame["bytes"],"digest":frame["digest"],"eof":frame["eof"].as_bool().unwrap_or(false)})
        }
        "BlobUnavailable" => {
            json!({"t":"BlobUnavailable","handle":frame["handle"],"reason":text(frame,"reason")})
        }
        _ => return Ok(None),
    }))
}

fn to_ipc(shared: &Shared, frame: &Value) -> Result<Option<Value>> {
    let kind = frame["t"].as_str().unwrap_or_default();
    Ok(Some(match kind {
        "ScanCommitted" => {
            let id = text(frame, "id");
            let (source, size) = shared
                .book()?
                .scans
                .remove(&id)
                .ok_or_else(|| invalid("unknown scan"))?;
            let session = text(frame, "checkpoint");
            shared.log(&format!("committed {source} {session}"));
            json!({"t":"Committed","source":source,"checkpoint":session,"credit":size})
        }
        "ScanFailed" => return Err(invalid("scan failed")),
        "Prepare" => {
            let command = text(frame, "command_id");
            shared
                .book()?
                .origins
                .insert(command.clone(), frame["origin"].clone());
            let mut prepare = json!({"t":"Prepare","command_id":command,"principal":frame["principal"],"request_hash":frame["request_hash"],"origin":frame["origin"],"expires_at":frame["expires_at"],"action":frame["action"]});
            if frame["expected_state"].is_object() {
                prepare["expected_state"] = frame["expected_state"].clone();
            }
            prepare
        }
        "Dispatch" => {
            let command = text(frame, "command_id");
            let mut book = shared.book()?;
            book.dispatching.insert(command.clone());
            json!({"t":"Dispatch","command_id":command,"ticket":book.tickets.remove(&command).unwrap_or(Value::Null),"origin":book.origins.get(&command).cloned().unwrap_or(Value::Null),"owner_fence":book.fence})
        }
        "Resolve" => {
            let command = text(frame, "command_id");
            *shared.book()?.resolving.entry(command.clone()).or_default() += 1;
            json!({"t":"Resolve","command_id":command})
        }
        "Query" => {
            json!({"t":"Query","id":frame["id"],"op":frame["op"],"args":frame["args"],"deadline_ms":frame["deadline_ms"]})
        }
        "QueryCancel" => json!({"t":"QueryCancel","id":frame["id"]}),
        "Primitive" => {
            json!({"t":"Primitive","id":frame["id"],"op":frame["op"],"args":frame["args"]})
        }
        "BlobRead" => {
            json!({"t":"BlobRead","handle":frame["handle"],"offset":frame["offset"],"length":frame["length"]})
        }
        _ => return Err(invalid("native frame")),
    }))
}
