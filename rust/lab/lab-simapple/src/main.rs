#![allow(clippy::too_many_lines)]
mod blobs;
mod calendar;
mod contacts;
mod mac;
mod media;
mod messages;
mod notes;
mod query;
mod reminders;
mod sources;
mod stale;
use lab_wire::{receive, send};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("wire: {0}")]
    Wire(#[from] lab_wire::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid: {0}")]
    Invalid(String),
}
type Result<T> = std::result::Result<T, Error>;
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
fn now() -> String {
    jiff::Timestamp::now().to_string()
}
fn id(prefix: &str) -> Result<String> {
    let mut bytes = [0; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(format!("{prefix}{}", hex::encode(bytes)))
}
fn string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
fn generation_of(data: &Value, src: &str, default: &str) -> String {
    let key = if src == "messages" {
        "message_generation".to_owned()
    } else {
        format!("{src}_generation")
    };
    data[&key]
        .as_str()
        .or_else(|| data["generation"].as_str())
        .unwrap_or(default)
        .to_owned()
}
fn source(collection: &str) -> &str {
    collection.split('.').next().unwrap_or(collection)
}
#[allow(clippy::needless_pass_by_value)]
fn observation(collection: &str, key: &str, data: Value) -> Value {
    json!({"kind":"upsert", "collection":collection,"native_key":key,"data":data,"observed_at":now()})
}
#[derive(Debug, Default)]
struct Bulk {
    target: String,
    paused: bool,
    blocked: bool,
    count: u64,
    ack_ms: f64,
    bytes: u64,
    credit_waits: u64,
}
#[derive(Debug)]
struct State {
    bulk: Bulk,
    data: Value,
    fault: String,
    pause: u64,
    denied: BTreeMap<String, bool>,
    path: PathBuf,
    socket: String,
    secret: Vec<u8>,
    calendar: calendar::Live,
    blob: blobs::Fixture,
    clipboard: String,
}
impl State {
    fn save(&self) -> Result<()> {
        let temporary = self.path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(&self.data)?)?;
        file.sync_all()?;
        fs::rename(temporary, &self.path)?;
        fs::File::open(self.path.parent().ok_or_else(|| invalid("state parent"))?)?.sync_all()?;
        Ok(())
    }
    fn records(&self) -> Vec<Value> {
        let mut records = self.data["records"]
            .as_object()
            .map(|m| m.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        records.sort_by_key(|v| string(v, "native_key"));
        records
    }
    fn scan(&self, src: &str) -> Result<Value> {
        let records: Vec<_> = self
            .records()
            .into_iter()
            .filter(|v| source(v["collection"].as_str().unwrap_or_default()) == src)
            .collect();
        let records = if src == "calendar" {
            calendar::view(self, records)
        } else {
            records
        };
        let fixed: &[&str] = match src {
            "messages" => &["chat", "handle", "message", "reaction", "attachment"],
            "contacts" => &["contact", "group"],
            "calendar" => &["calendar", "event"],
            _ => &[],
        };
        let scope: std::collections::BTreeSet<_> = if fixed.is_empty() {
            records.iter().map(|v| string(v, "collection")).collect()
        } else {
            fixed.iter().map(|name| format!("{src}.{name}")).collect()
        };
        let mut frame = json!({"t":"Capture","source":src,"observations":records,"scan_id":id("scan")?,"scope":scope,"scan_started_at":now(),"completion_evidence":"complete"});
        self.attach_handles(&mut frame);
        Ok(frame)
    }
    fn put(&mut self, record: Value) {
        let key = string(&record, "native_key");
        self.data["records"][key] = record;
    }
    fn guard(&self, entry: &Value) -> Result<()> {
        let path = self.path.with_file_name("dispatch-guard.log");
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer(&mut file, entry)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }
}
type Shared = Arc<Mutex<State>>;
fn lock(state: &Shared) -> Result<std::sync::MutexGuard<'_, State>> {
    state.lock().map_err(|_| invalid("state poisoned"))
}
#[derive(Debug)]
struct Request {
    frame: Value,
    done: Option<mpsc::Sender<()>>,
    whole: bool,
}
fn enqueue(tx: &mpsc::Sender<Request>, frame: Value, wait: bool) -> Result<()> {
    enqueue_mode(tx, frame, wait, false)
}
fn enqueue_mode(tx: &mpsc::Sender<Request>, frame: Value, wait: bool, whole: bool) -> Result<()> {
    let (done, rx) = mpsc::channel();
    tx.send(Request {
        frame,
        done: if wait { Some(done) } else { None },
        whole,
    })
    .map_err(|_| invalid("session stopped"))?;
    if wait {
        rx.recv_timeout(Duration::from_secs(110))
            .map_err(|_| invalid("capture not committed"))?;
    }
    Ok(())
}
fn control(state: &Shared, tx: &mpsc::Sender<Request>, route: &str, input: Value) -> Result<Value> {
    if route == "/stale-frame" {
        return stale::control_stale(state, &input);
    }
    if route == "/messages" {
        return messages::control_messages(state, tx, &input);
    }
    if route == "/mac" {
        return mac::control_mac(state, tx, &input);
    }
    if route == "/mac-notifications" {
        return mac::control_notifications(state, &input);
    }
    if route == "/calendar" {
        return calendar::control_calendar(state, tx, &input);
    }
    if route == "/contacts" {
        return contacts::control_contacts(state, tx, &input);
    }
    if route == "/readmodels" {
        return sources::readmodels(state, tx, &input);
    }
    if matches!(route, "/phone" | "/recordings" | "/safari" | "/usage") {
        return sources::control_source(state, tx, route, &input);
    }
    let mut s = lock(state)?;
    match route {
        "/messages-status" => Ok(json!({
            "generation":s.data["message_generation"].as_str().unwrap_or_else(|| s.data["generation"].as_str().unwrap_or_default()),
            "watcher_armed":s.data["message_watcher_armed"] != false,
            "poll_pending":s.data["message_poll_pending"] == true,
            "permitted":s.denied.get("messages") != Some(&true),
            "bulk_blocked":s.bulk.blocked,
            "capture_count":s.bulk.count,
            "capture_ack_ms":s.bulk.ack_ms,
            "capture_bytes":s.bulk.bytes,
            "credit_waits":s.bulk.credit_waits
        })),
        "/protocol" => Ok(s.data["protocol"].clone()),
        "/inspect" => Ok(json!(
            s.records()
                .into_iter()
                .filter(|record| calendar::in_window(&s, record))
                .collect::<Vec<_>>()
        )),
        "/effects" => Ok(s.data["effects"].clone()),
        "/command-origins" => Ok(s.data["origins"].clone()),
        "/stats" => Ok(
            json!({"blob_digests":blobs::digests(&s.blob),"scans":s.data["scans"],"source_scans":s.data["source_scans"],"revision":{},"committed":{}}),
        ),
        "/fault" => {
            let name = string(&input, "name");
            if ![
                "",
                "pause",
                "crash-after-effect",
                "crash-before-outcome",
                "ambiguous-outcome",
                "identical-manual-send",
                "ui-interference",
                "ui-interference-before",
                "calendar-rule-mismatch",
                "external-move-before-prepare",
                "external-edit-between-prepare-and-dispatch",
            ]
            .contains(&name.as_str())
            {
                return Err(invalid("fault not implemented in Rust foundation"));
            }
            s.fault = name;
            s.pause = input["ms"].as_u64().unwrap_or_default();
            Ok(json!({"armed":true}))
        }
        "/source-state" => {
            let src = string(&input, "source");
            if input["state"] != "permission_missing" {
                return Err(invalid("source state"));
            }
            s.denied.insert(src.clone(), true);
            s.data["denied"][&src] = json!(true);
            s.save()?;
            drop(s);
            enqueue(
                tx,
                json!({"t":"SourceState","source":src,"state":"permission_missing","coverage":{src:{"fields":"partial","missing":["permission_missing"]}}}),
                false,
            )?;
            Ok(json!({"armed":true}))
        }
        "/blob-fixture" => {
            drop(s);
            blobs::fixture(state, tx, &input)
        }
        "/blob-history" => {
            drop(s);
            blobs::history(state, tx, &input)
        }
        "/blob-faults" => {
            drop(s);
            blobs::faults(state)
        }
        "/seed" => {
            s.put(observation(
                "reminders.list",
                "list",
                json!({"title":"Bridge Test","writable":true}),
            ));
            let collection = input["collection"].as_str().unwrap_or("reminders.reminder");
            let src = source(collection).to_owned();
            for _ in 0..input["count"].as_u64().unwrap_or_default() {
                let key = id("seed")?;
                s.put(observation(collection,&key,json!({"title":format!("[ab-test] {key}"),"completed":false,"opaque_number":9_007_199_254_740_993_u64,"list_id":{"$ref":{"collection":"reminders.list","native_key":"list"}}})));
            }
            s.save()?;
            let frame = s.scan(&src)?;
            drop(s);
            enqueue(tx, frame, false)?;
            Ok(json!({"effect_done":true}))
        }
        "/external-change" => {
            let key = string(&input, "key");
            if key.is_empty() {
                return Err(invalid("key required"));
            }
            let collection = input["collection"].as_str().unwrap_or("reminders.reminder");
            let mut record = if input["delete"] == true {
                let mut record = s.data["records"][&key].clone();
                if record.is_null() {
                    return Err(invalid("not found"));
                }
                record["kind"] = json!("absent");
                record["revalidated"] = json!(true);
                record["observed_at"] = json!(now());
                record
            } else {
                let data = if input["data"].is_null() {
                    json!({"title":format!("[ab-test] {key}"),"completed":false,"opaque_number":9_007_199_254_740_993_u64,"list_id":{"$ref":{"collection":"reminders.list","native_key":"list"}}})
                } else {
                    input["data"].clone()
                };
                observation(collection, &key, data)
            };
            if source(collection) == "contacts" {
                record["scoped_alias"] = json!(format!("contacts/{collection}/{key}"));
            }
            s.put(record.clone());
            s.save()?;
            let src = source(collection).to_owned();
            let denied = s.denied.get(&src) == Some(&true);
            let frame = if input["delete"] == true {
                s.scan(&src)?
            } else {
                json!({"t":"Capture","source":src,"observations":[record]})
            };
            drop(s);
            if input["omit_capture"] != true && !denied {
                enqueue(tx, frame, false)?;
            }
            Ok(json!({"effect_done":true}))
        }
        "/capture" | "/capture-atomic" => {
            drop(s);
            let mut frame = input;
            frame["t"] = json!("Capture");
            if frame["source"].is_null() {
                frame["source"] = json!("reminders");
            }
            enqueue_mode(tx, frame, true, route == "/capture-atomic")?;
            Ok(json!({"committed":true}))
        }
        "/complete-recurring" => {
            let key = string(&input, "key");
            let mut original = s.data["records"][&key].clone();
            if original.is_null()
                || original["data"]["recurrence"].is_null()
                || s.denied.get("reminders") == Some(&true)
            {
                return Err(invalid("unavailable"));
            }
            original["data"]["completed"] = json!(true);
            original["observed_at"] = json!(now());
            let mut next = original.clone();
            let next_key = id("recurring")?;
            next["native_key"] = json!(next_key);
            next["data"]["completed"] = json!(false);
            s.put(original.clone());
            s.put(next.clone());
            s.save()?;
            drop(s);
            enqueue(
                tx,
                json!({"t":"Capture","source":"reminders","observations":[original,next],"scan_id":id("scan")?,"scope":["reminders.reminder"],"scan_started_at":now(),"completion_evidence":"complete"}),
                true,
            )?;
            Ok(json!({"effect_done":true}))
        }
        "/flush" => {
            drop(s);
            for src in ["messages", "reminders"] {
                enqueue(
                    tx,
                    json!({"t":"Capture","source":src,"observations":[]}),
                    true,
                )?;
            }
            Ok(json!({"committed":true}))
        }
        "/close" => {
            drop(s);
            enqueue(tx, json!({"t":"Close","reason":"lab"}), false)?;
            Ok(json!({"closed":true}))
        }
        "/dataset" => {
            let records = input["records"]
                .as_array()
                .ok_or_else(|| invalid("records"))?;
            if let Some(blobs) = input["blobs"].as_object() {
                for (digest, encoded) in blobs {
                    s.data["blobs"][digest] = encoded.clone();
                }
            }
            for record in records {
                let mut record = record.clone();
                let collection = string(&record, "collection");
                let key = string(&record, "native_key");
                if record["observed_at"].is_null() || source(&collection) == "reminders" {
                    record["observed_at"] = json!(now());
                }
                record["scoped_alias"] = json!(if source(&collection) == "contacts" {
                    format!("contacts/{collection}/{key}")
                } else if source(&collection) == "mail" && !record["scoped_alias"].is_null() {
                    string(&record, "scoped_alias")
                } else {
                    format!("{collection}/{key}")
                });
                sources::dataset_alias(&mut record);
                s.put(record);
            }
            s.save()?;
            let sources: std::collections::BTreeSet<_> = records
                .iter()
                .map(|v| source(v["collection"].as_str().unwrap_or_default()))
                .collect();
            let frames = sources
                .into_iter()
                .map(|src| s.scan(src))
                .collect::<Result<Vec<_>>>()?;
            drop(s);
            for frame in frames {
                enqueue(tx, frame, true)?;
            }
            Ok(json!({"committed":true}))
        }
        _ => Err(invalid("unsupported control route")),
    }
}
fn http(mut stream: UnixStream, state: &Shared, tx: &mpsc::Sender<Request>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() > 16384 {
            return Err(invalid("headers too large"));
        }
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    let headers = String::from_utf8_lossy(&bytes);
    let route = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| invalid("request"))?
        .to_owned();
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.trim().parse::<usize>())
        .transpose()
        .map_err(|_| invalid("content length"))?
        .unwrap_or_default();
    if length > 128 * 1024 * 1024 {
        return Err(invalid("body capacity"));
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    let input = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body)?
    };
    let (status, result) = match control(state, tx, &route, input) {
        Ok(value) => (200, value),
        Err(err) => {
            eprintln!("control {route}: {err}");
            (400, json!({"error":err.to_string()}))
        }
    };
    let body = serde_json::to_vec(&result)?;
    write!(
        stream,
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    Ok(())
}
#[derive(Debug)]
struct Ticket {
    prepare: Value,
    before: Value,
    deadline: Instant,
}
#[derive(Debug)]
struct Session<'a> {
    requests: &'a mpsc::Receiver<Request>,
    stream: UnixStream,
    incoming: mpsc::Receiver<Value>,
    pending: VecDeque<Value>,
    checkpoints: BTreeMap<String, String>,
    generation: String,
    generations: BTreeMap<String, String>,
    fence: u64,
    credit: usize,
    tickets: BTreeMap<String, Ticket>,
    blobs: BTreeMap<String, Vec<u8>>,
    sleeping: BTreeMap<String, query::Cancel>,
    finished: mpsc::Sender<(String, Value)>,
    finished_rx: mpsc::Receiver<(String, Value)>,
}
impl Session<'_> {
    fn emit(&mut self, state: &Shared, mut frame: Value) -> Result<()> {
        if let Some(observations) = frame["observations"].as_array_mut() {
            observations.sort_by_key(|record| record["kind"] == "absent");
        }
        let Some(observations) = frame["observations"].as_array().filter(|v| v.len() > 64) else {
            return self.emit_part(state, frame);
        };
        for (index, chunk) in observations.chunks(64).enumerate() {
            self.hold_bulk(state, &frame, chunk)?;
            let mut part = frame.clone();
            part["observations"] = json!(chunk);
            if (index + 1) * 64 < observations.len()
                && chunk
                    .last()
                    .is_some_and(|record| record["kind"] != "absent")
            {
                part.as_object_mut()
                    .ok_or_else(|| invalid("capture"))?
                    .remove("completion_evidence");
            }
            self.emit_part(state, part)?;
        }
        Ok(())
    }
    fn hold_bulk(&mut self, state: &Shared, frame: &Value, chunk: &[Value]) -> Result<()> {
        if frame["source"] != "messages" {
            return Ok(());
        }
        loop {
            self.serve_live(state)?;
            let blocked = {
                let mut s = lock(state)?;
                let blocked = s.bulk.paused
                    && chunk
                        .iter()
                        .any(|record| string(record, "native_key") == s.bulk.target);
                s.bulk.blocked = blocked;
                blocked
            };
            if !blocked {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn serve_live(&mut self, state: &Shared) -> Result<()> {
        loop {
            match self.incoming.try_recv() {
                Ok(frame) if frame["t"] == "Ping" => {
                    send(&mut self.stream, &json!({"t":"Pong"}))?;
                }
                Ok(frame) => self.pending.push_back(frame),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(invalid("gateway disconnected"));
                }
            }
        }
        while let Ok(request) = self.requests.try_recv() {
            self.serve(state, request)?;
        }
        Ok(())
    }
    fn serve(&mut self, state: &Shared, request: Request) -> Result<()> {
        if request.frame["t"] == "Close" {
            send(&mut self.stream, &request.frame)?;
            loop {
                thread::park();
            }
        }
        if request.whole {
            self.emit_part(state, request.frame)?;
        } else {
            self.emit(state, request.frame)?;
        }
        if let Some(done) = request.done {
            let _ = done.send(());
        }
        Ok(())
    }
    fn emit_part(&mut self, state: &Shared, mut frame: Value) -> Result<()> {
        if frame["t"] != "Capture" {
            send(&mut self.stream, &frame)?;
            return Ok(());
        }
        let src = string(&frame, "source");
        if src == "messages" {
            let guard = lock(state)?;
            messages::revalidate(&guard, &mut frame);
            drop(guard);
            messages::aliases(&mut frame);
            if !self.blobs.is_empty() {
                frame["blob_handles"] = json!(
                    self.blobs
                        .keys()
                        .map(|digest| (digest.clone(), json!(digest)))
                        .collect::<serde_json::Map<_, _>>()
                );
            }
        }
        if lock(state)?.denied.get(&src) == Some(&true) {
            return Ok(());
        }
        let generation = generation_of(&lock(state)?.data, &src, &self.generation);
        if self.generations.get(&src).is_some_and(|g| *g != generation) {
            self.checkpoints.remove(&src);
        }
        self.generations.insert(src.clone(), generation.clone());
        frame["generation"] = json!(generation);
        frame["predecessor"] = json!(self.checkpoints.get(&src).cloned().unwrap_or_default());
        let checkpoint = id("cp")?;
        frame["checkpoint"] = json!(checkpoint);
        let size = lab_wire::frame_bytes(&lab_wire::encode_json(&frame)?)?.len();
        if size > self.credit {
            return Err(invalid("capture credit"));
        }
        let started = Instant::now();
        let n = send(&mut self.stream, &frame)?;
        self.credit -= n;
        loop {
            let reply = self
                .incoming
                .recv_timeout(Duration::from_secs(30))
                .map_err(|_| invalid("commit timeout"))?;
            match reply["t"].as_str() {
                Some("Committed") => {
                    if reply["source"] != src
                        || reply["checkpoint"] != checkpoint
                        || reply["credit"].as_u64() != Some(n as u64)
                    {
                        return Err(invalid("commit identity or credit"));
                    }
                    {
                        let mut s = lock(state)?;
                        s.data["protocol"]["acked"][&src] = json!(checkpoint);
                        let log = &mut s.data["protocol"]["commits"];
                        if !log.is_array() {
                            *log = json!([]);
                        }
                        if let Some(entries) = log.as_array_mut() {
                            entries.push(json!({"source": src, "checkpoint": checkpoint}));
                        }
                    }
                    if src == "messages" {
                        let mut s = lock(state)?;
                        s.bulk.count += 1;
                        s.bulk.ack_ms += started.elapsed().as_secs_f64() * 1000.0;
                        s.bulk.bytes += n as u64;
                    }
                    self.checkpoints.insert(src, checkpoint);
                    self.credit += n;
                    return Ok(());
                }
                Some("Prepare" | "Dispatch" | "Resolve" | "Query" | "QueryCancel" | "BlobRead") => {
                    self.pending.push_back(reply);
                }
                Some("Ping") => {
                    send(&mut self.stream, &json!({"t":"Pong"}))?;
                }
                Some("Pong") => {}
                _ => return Err(invalid("capture reset")),
            }
        }
    }
    fn family(&self, state: &Shared, frame: &Value) -> Result<String> {
        let op = match frame["t"].as_str() {
            Some("Prepare") => string(&frame["action"], "op"),
            Some("Dispatch") => self
                .tickets
                .get(&string(frame, "ticket"))
                .map(|ticket| string(&ticket.prepare["action"], "op"))
                .unwrap_or_default(),
            Some("Resolve") => {
                let s = lock(state)?;
                let source = string(
                    &s.data["evidence"][string(frame, "command_id")]["evidence"],
                    "source",
                );
                return Ok(if source.is_empty() {
                    "reminders".into()
                } else {
                    source
                });
            }
            _ => String::new(),
        };
        Ok(op.split('.').next().unwrap_or_default().to_owned())
    }
    fn reject(&mut self, frame: &Value, code: &str) -> Result<()> {
        send(
            &mut self.stream,
            &json!({"t":"Rejected","command_id":frame["command_id"],"disposition":"rejected_before_dispatch","error":{"code":code,"message":code,"retryable":false}}),
        )?;
        Ok(())
    }
    #[allow(clippy::needless_pass_by_value)]
    fn command(&mut self, state: &Shared, frame: Value) -> Result<()> {
        let command = string(&frame, "command_id");
        if matches!(frame["t"].as_str(), Some("Prepare" | "Dispatch")) {
            let mut s = lock(state)?;
            let kind = string(&frame, "t");
            s.data["origins"][&command][&kind] = frame["origin"].clone();
            if !frame["action"].is_null() {
                s.data["origins"][&command]["action"] = frame["action"]["origin"].clone();
            }
        }
        if self.message_command(state, &frame)? {
            return Ok(());
        }
        let family = self.family(state, &frame)?;
        match (family.as_str(), frame["t"].as_str()) {
            ("notes", Some("Prepare")) => return self.note_prepare(state, &frame),
            ("notes", Some("Dispatch")) => return self.note_dispatch(state, &frame),
            ("calendar", Some("Prepare")) => return self.calendar_prepare(state, &frame),
            ("calendar", Some("Dispatch")) => return self.calendar_dispatch(state, &frame),
            ("contacts", Some("Prepare")) => return self.contacts_prepare(state, &frame),
            ("contacts", Some("Dispatch")) => return self.contacts_dispatch(state, &frame),
            ("contacts", Some("Resolve")) => return self.contacts_resolve(state, &frame),
            ("mac", Some("Prepare")) => return self.mac_prepare(state, &frame),
            ("mac", Some("Dispatch")) => return self.mac_dispatch(state, &frame),
            ("reminders", Some("Prepare")) => return self.reminder_prepare(state, &frame),
            ("reminders", Some("Dispatch")) => return self.reminder_dispatch(state, &frame),
            (_, Some("Prepare")) => return self.reject(&frame, "unsupported"),
            _ => {}
        }
        match frame["t"].as_str() {
            Some("Resolve") => {
                let s = lock(state)?;
                let mut outcome = s.data["evidence"][&command].clone();
                if outcome.is_null() {
                    outcome =
                        json!({"t":"Outcome","command_id":command,"disposition":"indeterminate"});
                    drop(s);
                } else {
                    let key = string(&outcome["evidence"], "native_key");
                    let record = s.data["records"][&key].clone();
                    let capture = if record["kind"] == "absent" {
                        s.scan(&family)?
                    } else {
                        json!({"t":"Capture","source":family,"observations":[record]})
                    };
                    drop(s);
                    self.emit(state, capture)?;
                    outcome["evidence"]["capture_checkpoint"] =
                        json!(self.checkpoints.get(&family));
                    outcome["disposition"] = json!("locally_observed");
                    let mut s = lock(state)?;
                    s.data["evidence"][&command] = outcome.clone();
                    s.save()?;
                    s.guard(&json!({"command_id":command,"outcome":outcome}))?;
                }
                send(&mut self.stream, &outcome)?;
            }
            Some("Ping") => {
                send(&mut self.stream, &json!({"t":"Pong"}))?;
            }
            Some("Pong") => {}
            Some("QueryCancel") => {
                if let Some(cancel) = frame["id"].as_str().and_then(|id| self.sleeping.get(id)) {
                    query::cancel(cancel);
                }
            }
            Some("Query") => {
                if let Some(ms) = query::sleep_request(&frame) {
                    let id = string(&frame, "id");
                    if !self.sleeping.contains_key(&id) {
                        let cancel = query::Cancel::default();
                        self.sleeping.insert(id.clone(), cancel.clone());
                        lock(state)?.data["effects"][&id] = json!(1);
                        query::sleep(
                            frame["id"].clone(),
                            ms,
                            frame["deadline_ms"].as_u64().unwrap_or(0),
                            cancel,
                            self.finished.clone(),
                        );
                    }
                    return Ok(());
                }
                let reply = match mac::query(state, &frame)? {
                    Some(reply) => reply,
                    None => query::query(&mut self.blobs, &frame),
                };
                send(&mut self.stream, &reply)?;
            }
            Some("BlobRead") => match blobs::read(state, &frame)? {
                blobs::Read::Drop => {}
                blobs::Read::Reply {
                    frame: reply,
                    delay_ms,
                    refresh,
                } => {
                    if let Some(digest) = refresh {
                        self.emit(state, blobs::refresh_frame(&digest))?;
                    }
                    if delay_ms > 0 {
                        let finished = self.finished.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(delay_ms));
                            let _ = finished.send((String::new(), reply));
                        });
                    } else {
                        send(&mut self.stream, &reply)?;
                    }
                }
                blobs::Read::Foreign => {
                    let stored = contacts::blob(state, &frame)?;
                    let reply = if stored["t"] == "BlobUnavailable" {
                        query::blob(&self.blobs, &frame).unwrap_or(stored)
                    } else {
                        stored
                    };
                    send(&mut self.stream, &reply)?;
                }
            },
            _ => return Err(invalid("session frame")),
        }
        Ok(())
    }
}
fn handshake(stream: &mut UnixStream, secret: &[u8]) -> Result<Value> {
    let (challenge, _) = receive(stream)?;
    let a = string(&challenge, "nonce");
    if challenge["t"] != "Challenge" || a.len() != 64 || secret.len() != 32 {
        return Err(invalid("challenge"));
    }
    let b = id("")?;
    send(
        stream,
        &json!({"t":"Authenticate","nonce":b,"proof":lab_wire::ipc_proof(secret,"helper",&a,&b)?}),
    )?;
    let (answer, _) = receive(stream)?;
    if answer["t"] != "Authenticated" {
        return Err(invalid("authentication"));
    }
    lab_wire::verify_ipc_proof(secret, "gateway", &a, &b, &string(&answer, "proof"))?;
    let (open, _) = receive(stream)?;
    if open["t"] != "Open"
        || open["protocol"] != "ipc/1"
        || open["owner_fence"].as_u64().is_none_or(|v| v == 0)
        || open["byte_credit"].as_u64().is_none_or(|v| v == 0)
    {
        return Err(invalid("open"));
    }
    Ok(open)
}
fn session(
    stream: UnixStream,
    secret: &[u8],
    state: &Shared,
    requests: &mpsc::Receiver<Request>,
) -> Result<()> {
    let mut stream = stream;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let open = handshake(&mut stream, secret)?;
    let fence = open["owner_fence"]
        .as_u64()
        .ok_or_else(|| invalid("fence"))?;
    let credit = open["byte_credit"]
        .as_u64()
        .ok_or_else(|| invalid("credit"))?;
    let sources = [
        "reminders",
        "messages",
        "contacts",
        "calendar",
        "notes",
        "usage",
        "mail",
        "phone",
        "files",
        "safari",
        "voicemail",
        "voice_memos",
        "mac",
    ];
    let mut capabilities = json!({});
    let mut coverage = json!({});
    for src in sources {
        capabilities[src] = json!([]);
        coverage[src] = json!({"fields":"complete","missing":[]});
    }
    capabilities["reminders"] = json!([
        "reminders.create",
        "reminders.update",
        "reminders.complete",
        "reminders.uncomplete",
        "reminders.delete",
        "reminders.create_list",
        "reminders.rename_list",
        "reminders.delete_list"
    ]);
    capabilities["mac"] = json!(mac::CAPABILITIES);
    capabilities["messages"] = json!(["messages.send", "messages.reply"]);
    capabilities["calendar"] = json!([
        "calendar.create_event",
        "calendar.update_event",
        "calendar.delete_event",
        "calendar.create_calendar"
    ]);
    capabilities["contacts"] = json!([
        "contacts.create",
        "contacts.update",
        "contacts.delete",
        "contacts.create_group",
        "contacts.rename_group",
        "contacts.delete_group",
        "contacts.add_member",
        "contacts.remove_member"
    ]);
    let proven: Vec<Value> = [
        "contacts.create_group",
        "contacts.rename_group",
        "contacts.delete_group",
        "contacts.add_member",
        "contacts.remove_member",
    ]
    .iter()
    .map(|op| json!({"operation":op,"proven":true,"reason":"simulated_public_contacts_save_request"}))
    .collect();
    coverage["contacts"] = json!({"fields":"partial","missing":["note","contact_poster: unavailable through public Contacts API"],"operations":proven});
    capabilities["notes"] = json!([
        "notes.create",
        "notes.update",
        "notes.delete",
        "notes.create_folder",
        "notes.move",
        "notes.rename_folder"
    ]);
    send(
        &mut stream,
        &json!({"t":"Ready","protocol":"ipc/1","capabilities":capabilities,"coverage":coverage,"helper_build":"lab-simapple/rust","os_build":"sim"}),
    )?;
    stream.set_read_timeout(None)?;
    let mut reader = stream.try_clone()?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        while let Ok((frame, _)) = receive(&mut reader) {
            if tx.send(frame).is_err() {
                break;
            }
        }
    });
    let (finished_tx, finished_rx) = mpsc::channel();
    let mut session = Session {
        requests,
        stream,
        incoming: rx,
        pending: VecDeque::new(),
        checkpoints: BTreeMap::new(),
        generation: string(&lock(state)?.data, "generation"),
        generations: BTreeMap::new(),
        fence,
        credit: usize::try_from(credit).map_err(|_| invalid("credit"))?,
        tickets: BTreeMap::new(),
        blobs: BTreeMap::new(),
        sleeping: BTreeMap::new(),
        finished: finished_tx,
        finished_rx,
    };
    for src in sources {
        let generation = generation_of(&lock(state)?.data, src, &session.generation);
        session.generations.insert(src.to_owned(), generation);
    }
    {
        let mut offered = serde_json::Map::new();
        if let Some(checkpoints) = open["checkpoints"].as_object() {
            for (src, checkpoint) in checkpoints {
                offered.insert(src.clone(), json!(string(checkpoint, "checkpoint")));
            }
        }
        let mut s = lock(state)?;
        let seen = s.data["protocol"]["commits"].as_array().map_or(0, Vec::len);
        let log = &mut s.data["protocol"]["opens"];
        if !log.is_array() {
            *log = json!([]);
        }
        if let Some(entries) = log.as_array_mut() {
            entries.push(json!({"fence": fence, "offered": offered, "commits_seen": seen}));
        }
    }
    if let Some(checkpoints) = open["checkpoints"].as_object() {
        for (src, checkpoint) in checkpoints {
            if session
                .generations
                .get(src)
                .is_some_and(|g| checkpoint["generation"] == *g)
            {
                session
                    .checkpoints
                    .insert(src.clone(), string(checkpoint, "checkpoint"));
            }
        }
    }
    let guard = lock(state)?.path.with_file_name("dispatch-guard.log");
    if let Ok(text) = fs::read_to_string(guard) {
        let mut started = std::collections::BTreeSet::new();
        for line in text.lines() {
            let entry: Value = serde_json::from_str(line)?;
            let command = string(&entry, "command_id");
            if entry["outcome"].is_null() {
                started.insert(command);
            } else {
                started.remove(&command);
            }
        }
        for command in started {
            send(
                &mut session.stream,
                &json!({"t":"Outcome","command_id":command,"disposition":"indeterminate"}),
            )?;
        }
    }
    for src in sources {
        if lock(state)?.denied.get(src) == Some(&true) {
            send(
                &mut session.stream,
                &json!({"t":"SourceState","source":src,"state":"permission_missing","coverage":{src:{"fields":"partial","missing":["permission_missing"]}}}),
            )?;
            continue;
        }
        let capture = {
            let mut s = lock(state)?;
            let scans = s.data["scans"].as_u64().unwrap_or_default() + 1;
            s.data["scans"] = json!(scans);
            let n = s.data["source_scans"][src].as_u64().unwrap_or_default() + 1;
            s.data["source_scans"][src] = json!(n);
            s.scan(src)?
        };
        session.emit(state, capture)?;
    }
    loop {
        if let Ok((id, reply)) = session.finished_rx.try_recv() {
            if !id.is_empty() {
                session.sleeping.remove(&id);
                lock(state)?.data["effects"][&id] = json!(0);
            }
            send(&mut session.stream, &reply)?;
            continue;
        }
        if let Some(frame) = session.pending.pop_front() {
            session.command(state, frame)?;
            continue;
        }
        match session.incoming.try_recv() {
            Ok(frame) => {
                session.command(state, frame)?;
                continue;
            }
            Err(mpsc::TryRecvError::Disconnected) => return Err(invalid("gateway disconnected")),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match requests.recv_timeout(Duration::from_millis(5)) {
            Ok(request) => session.serve(state, request)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}
fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut options = BTreeMap::new();
    while let Some(arg) = args.next() {
        if arg == "simapple" || arg == "--legacy-sources" {
            continue;
        }
        let value = args.next().ok_or_else(|| invalid("argument value"))?;
        options.insert(arg, value);
    }
    let option = |key: &str| options.get(key).cloned().ok_or_else(|| invalid(key));
    let socket = option("--socket")?;
    let secret = fs::read(option("--secret-file")?)?;
    let control = option("--control")?;
    let home = Path::new(&control)
        .parent()
        .ok_or_else(|| invalid("control parent"))?;
    fs::create_dir_all(home)?;
    let path = home.join("rust-sim-state.json");
    let data = if path.exists() {
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        json!({"generation":id("sim")?,"records":{},"effects":{},"evidence":{},"origins":{},"scans":0,"source_scans":{}})
    };
    let blob = blobs::restore(&data);
    let state = Arc::new(Mutex::new(State {
        bulk: Bulk::default(),
        data,
        path,
        fault: String::new(),
        pause: 0,
        denied: BTreeMap::new(),
        socket: socket.clone(),
        secret: secret.clone(),
        calendar: calendar::Live::default(),
        blob,
        clipboard: String::new(),
    }));
    {
        let mut s = lock(&state)?;
        let denied: Vec<String> = s.data["denied"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        for src in denied {
            s.denied.insert(src, true);
        }
    }
    lock(&state)?.save()?;
    match fs::remove_file(&control) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(&control)?;
    fs::set_permissions(&control, fs::Permissions::from_mode(0o600))?;
    let (tx, rx) = mpsc::channel();
    let shared = Arc::clone(&state);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            let state = Arc::clone(&shared);
            let tx = tx.clone();
            thread::spawn(move || {
                if let Err(e) = http(stream, &state, &tx) {
                    eprintln!("control: {e}");
                }
            });
        }
    });
    loop {
        if let Ok(stream) = UnixStream::connect(&socket)
            && let Err(e) = session(stream, &secret, &state, &rx)
        {
            eprintln!("session: {e}");
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
