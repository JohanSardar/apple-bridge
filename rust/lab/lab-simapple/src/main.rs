#![allow(clippy::too_many_lines)]
mod messages;
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
fn source(collection: &str) -> &str {
    collection.split('.').next().unwrap_or(collection)
}
#[allow(clippy::needless_pass_by_value)]
fn observation(collection: &str, key: &str, data: Value) -> Value {
    json!({"kind":"upsert", "collection":collection,"native_key":key,"data":data,"observed_at":now()})
}
#[derive(Debug)]
struct State {
    data: Value,
    fault: String,
    pause: u64,
    denied: BTreeMap<String, bool>,
    path: PathBuf,
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
        let scope: std::collections::BTreeSet<_> = if src == "messages" {
            ["chat", "handle", "message", "reaction", "attachment"]
                .into_iter()
                .map(|name| format!("messages.{name}"))
                .collect()
        } else {
            records.iter().map(|v| string(v, "collection")).collect()
        };
        Ok(
            json!({"t":"Capture","source":src,"observations":records,"scan_id":id("scan")?,"scope":scope,"scan_started_at":now(),"completion_evidence":"complete"}),
        )
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
        rx.recv_timeout(Duration::from_secs(20))
            .map_err(|_| invalid("capture not committed"))?;
    }
    Ok(())
}
fn control(state: &Shared, tx: &mpsc::Sender<Request>, route: &str, input: Value) -> Result<Value> {
    if route == "/messages" {
        return messages::control_messages(state, tx, &input);
    }
    let mut s = lock(state)?;
    match route {
        "/messages-status" => Ok(json!({
            "generation":s.data["message_generation"].as_str().unwrap_or_else(|| s.data["generation"].as_str().unwrap_or_default()),
            "watcher_armed":s.data["message_watcher_armed"] != false,
            "poll_pending":s.data["message_poll_pending"] == true,
            "permitted":s.denied.get("messages") != Some(&true)
        })),
        "/inspect" => Ok(json!(s.records())),
        "/effects" => Ok(s.data["effects"].clone()),
        "/command-origins" => Ok(s.data["origins"].clone()),
        "/stats" => Ok(
            json!({"blob_digests":{},"scans":s.data["scans"],"source_scans":s.data["source_scans"],"revision":{},"committed":{}}),
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
            drop(s);
            enqueue(
                tx,
                json!({"t":"SourceState","source":src,"state":"permission_missing","coverage":{src:{"fields":"partial","missing":["permission_missing"]}}}),
                false,
            )?;
            Ok(json!({"armed":true}))
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
            for record in records {
                s.put(record.clone());
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
        Err(err) => (400, json!({"error":err.to_string()})),
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
struct Session {
    stream: UnixStream,
    incoming: mpsc::Receiver<Value>,
    pending: VecDeque<Value>,
    checkpoints: BTreeMap<String, String>,
    generation: String,
    message_generation: String,
    fence: u64,
    credit: usize,
    tickets: BTreeMap<String, Ticket>,
}
impl Session {
    fn emit(&mut self, state: &Shared, mut frame: Value) -> Result<()> {
        if let Some(observations) = frame["observations"].as_array_mut() {
            observations.sort_by_key(|record| record["kind"] == "absent");
        }
        let Some(observations) = frame["observations"].as_array().filter(|v| v.len() > 64) else {
            return self.emit_part(state, frame);
        };
        for (index, chunk) in observations.chunks(64).enumerate() {
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
    fn emit_part(&mut self, state: &Shared, mut frame: Value) -> Result<()> {
        if frame["t"] != "Capture" {
            send(&mut self.stream, &frame)?;
            return Ok(());
        }
        let src = string(&frame, "source");
        if src == "messages" {
            messages::aliases(&mut frame);
        }
        if lock(state)?.denied.get(&src) == Some(&true) {
            return Ok(());
        }
        frame["generation"] = if src == "messages" {
            lock(state)?.data["message_generation"]
                .as_str()
                .map_or_else(|| json!(self.generation), |v| json!(v))
        } else {
            json!(self.generation)
        };
        if src == "messages" && frame["generation"] != self.message_generation {
            self.checkpoints.remove(&src);
            self.message_generation = string(&frame, "generation");
        }
        frame["predecessor"] = json!(self.checkpoints.get(&src).cloned().unwrap_or_default());
        let checkpoint = id("cp")?;
        frame["checkpoint"] = json!(checkpoint);
        let size = lab_wire::frame_bytes(&lab_wire::encode_json(&frame)?)?.len();
        if size > self.credit {
            return Err(invalid("capture credit"));
        }
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
                    self.checkpoints.insert(src, checkpoint);
                    self.credit += n;
                    return Ok(());
                }
                Some("Prepare" | "Dispatch" | "Resolve") => self.pending.push_back(reply),
                Some("Ping") => {
                    send(&mut self.stream, &json!({"t":"Pong"}))?;
                }
                Some("Pong") => {}
                _ => return Err(invalid("capture reset")),
            }
        }
    }
    fn reject(&mut self, frame: &Value, code: &str) -> Result<()> {
        send(
            &mut self.stream,
            &json!({"t":"Rejected","command_id":frame["command_id"],"disposition":"rejected_before_dispatch","error":{"code":code,"message":code}}),
        )?;
        Ok(())
    }
    #[allow(clippy::needless_pass_by_value)]
    fn command(&mut self, state: &Shared, frame: Value) -> Result<()> {
        if self.message_command(state, &frame)? {
            return Ok(());
        }
        let command = string(&frame, "command_id");
        match frame["t"].as_str() {
            Some("Prepare") => {
                let mut s = lock(state)?;
                let action = &frame["action"];
                let op = string(action, "op");
                if !op.starts_with("reminders.") {
                    drop(s);
                    return self.reject(&frame, "unsupported");
                }
                if string(&frame, "principal").is_empty()
                    || string(&frame, "request_hash").is_empty()
                {
                    drop(s);
                    return self.reject(&frame, "invalid");
                }
                let key = string(action, "native_key");
                let before = s.data["records"][&key]["data"].clone();
                if !key.is_empty() && before.is_null() {
                    drop(s);
                    return self.reject(&frame, "not_found");
                }
                if !op.ends_with("_list")
                    && s.data["records"][string(action, "list_key")]["data"]["writable"] != true
                {
                    drop(s);
                    return self.reject(&frame, "unsupported");
                }
                self.tickets.retain(|_, t| Instant::now() < t.deadline);
                if !self.tickets.is_empty() {
                    drop(s);
                    return self.reject(&frame, "unavailable");
                }
                s.data["origins"][&command] =
                    json!({"Prepare":frame["origin"],"action":action["origin"]});
                let ticket = id("ticket")?;
                let requested = string(&frame, "expires_at")
                    .parse::<jiff::Timestamp>()
                    .map_err(|_| invalid("expiry"))?;
                let seconds =
                    (requested.as_second() - jiff::Timestamp::now().as_second()).clamp(0, 30);
                self.tickets.insert(
                    ticket.clone(),
                    Ticket {
                        prepare: frame.clone(),
                        before: before.clone(),
                        deadline: Instant::now() + Duration::from_secs(seconds.cast_unsigned()),
                    },
                );
                let pause = if s.fault == "pause" {
                    s.fault.clear();
                    s.pause
                } else {
                    0
                };
                drop(s);
                thread::sleep(Duration::from_millis(pause));
                send(
                    &mut self.stream,
                    &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":before,"route":"sim-reminders","fingerprint":frame["request_hash"],"candidate_ids":[key]}),
                )?;
            }
            Some("Dispatch") => {
                let Some(ticket) = self.tickets.remove(&string(&frame, "ticket")) else {
                    return self.reject(&frame, "expired");
                };
                if ticket.prepare["command_id"] != command
                    || frame["owner_fence"].as_u64() != Some(self.fence)
                    || Instant::now() >= ticket.deadline
                {
                    return self.reject(&frame, "expired");
                }
                let action = &ticket.prepare["action"];
                let op = string(action, "op");
                let mut key = string(action, "native_key");
                let mut s = lock(state)?;
                if !key.is_empty() && s.data["records"][&key]["data"] != ticket.before {
                    drop(s);
                    return self.reject(&frame, "conflict");
                }
                s.guard(
                    &json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}),
                )?;
                if key.is_empty() {
                    key = id("native")?;
                }
                let collection = if op.ends_with("_list") {
                    "reminders.list"
                } else {
                    "reminders.reminder"
                };
                let mut data = if ticket.before.is_object() {
                    ticket.before
                } else {
                    json!({})
                };
                if let Some(args) = action["args"].as_object() {
                    for (k, v) in args {
                        if k != "id" && k != "list_id" {
                            data[k] = v.clone();
                        }
                    }
                }
                if collection == "reminders.list" {
                    data["writable"] = json!(true);
                } else {
                    if data["completed"].is_null() {
                        data["completed"] = json!(false);
                    }
                    data["list_id"] = json!({"$ref":{"collection":"reminders.list","native_key":action["list_key"]}});
                }
                if op == "reminders.complete" {
                    data["completed"] = json!(true);
                }
                if op == "reminders.uncomplete" {
                    data["completed"] = json!(false);
                }
                let mut record = observation(collection, &key, data);
                if op == "reminders.delete" || op == "reminders.delete_list" {
                    record["kind"] = json!("absent");
                    record["revalidated"] = json!(true);
                }
                s.put(record.clone());
                let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
                s.data["effects"][&command] = json!(count);
                s.data["origins"][&command]["Dispatch"] = frame["origin"].clone();
                let mut outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"native_key":key,"fingerprint":ticket.prepare["request_hash"],"observed":true,"absent":record["kind"]=="absent"}});
                s.data["evidence"][&command] = outcome.clone();
                s.save()?;
                let fault = std::mem::take(&mut s.fault);
                if fault == "crash-after-effect" {
                    std::process::exit(78);
                }
                let capture = if record["kind"] == "absent" {
                    s.scan("reminders")?
                } else {
                    json!({"t":"Capture","source":"reminders","observations":[record]})
                };
                drop(s);
                self.emit(state, capture)?;
                outcome["evidence"]["capture_checkpoint"] =
                    json!(self.checkpoints.get("reminders"));
                let mut s = lock(state)?;
                s.data["evidence"][&command] = outcome.clone();
                s.save()?;
                if fault == "crash-before-outcome" {
                    std::process::exit(78);
                }
                if fault == "ambiguous-outcome" {
                    outcome["disposition"] = json!("indeterminate");
                }
                s.guard(&json!({"command_id":command,"outcome":outcome}))?;
                drop(s);
                send(&mut self.stream, &outcome)?;
            }
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
                        s.scan("reminders")?
                    } else {
                        json!({"t":"Capture","source":"reminders","observations":[record]})
                    };
                    drop(s);
                    self.emit(state, capture)?;
                    outcome["evidence"]["capture_checkpoint"] =
                        json!(self.checkpoints.get("reminders"));
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
            Some("BlobRead") => {
                send(
                    &mut self.stream,
                    &json!({"t":"BlobUnavailable","handle":frame["handle"],"reason":"unavailable"}),
                )?;
            }
            _ => return Err(invalid("session frame")),
        }
        Ok(())
    }
}
fn session(
    stream: UnixStream,
    secret: &[u8],
    state: &Shared,
    requests: &mpsc::Receiver<Request>,
) -> Result<()> {
    let mut stream = stream;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let (challenge, _) = receive(&mut stream)?;
    let a = string(&challenge, "nonce");
    if challenge["t"] != "Challenge" || a.len() != 64 || secret.len() != 32 {
        return Err(invalid("challenge"));
    }
    let b = id("")?;
    send(
        &mut stream,
        &json!({"t":"Authenticate","nonce":b,"proof":lab_wire::ipc_proof(secret,"helper",&a,&b)?}),
    )?;
    let (answer, _) = receive(&mut stream)?;
    if answer["t"] != "Authenticated" {
        return Err(invalid("authentication"));
    }
    lab_wire::verify_ipc_proof(secret, "gateway", &a, &b, &string(&answer, "proof"))?;
    let (open, _) = receive(&mut stream)?;
    let fence = open["owner_fence"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("fence"))?;
    let credit = open["byte_credit"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("credit"))?;
    if open["t"] != "Open" || open["protocol"] != "ipc/1" {
        return Err(invalid("open"));
    }
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
    capabilities["messages"] = json!(["messages.send", "messages.reply"]);
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
    let message_generation = {
        let s = lock(state)?;
        s.data["message_generation"]
            .as_str()
            .map_or_else(|| string(&s.data, "generation"), str::to_owned)
    };
    let mut session = Session {
        stream,
        incoming: rx,
        pending: VecDeque::new(),
        checkpoints: BTreeMap::new(),
        generation: string(&lock(state)?.data, "generation"),
        message_generation,
        fence,
        credit: usize::try_from(credit).map_err(|_| invalid("credit"))?,
        tickets: BTreeMap::new(),
    };
    if let Some(checkpoints) = open["checkpoints"].as_object() {
        for (src, checkpoint) in checkpoints {
            let generation = if src == "messages" {
                &session.message_generation
            } else {
                &session.generation
            };
            if checkpoint["generation"] == *generation {
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
            Ok(request) => {
                if request.frame["t"] == "Close" {
                    send(&mut session.stream, &request.frame)?;
                    loop {
                        thread::park();
                    }
                }
                if request.whole {
                    session.emit_part(state, request.frame)?;
                } else {
                    session.emit(state, request.frame)?;
                }
                if let Some(done) = request.done {
                    let _ = done.send(());
                }
            }
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
    let state = Arc::new(Mutex::new(State {
        data,
        path,
        fault: String::new(),
        pause: 0,
        denied: BTreeMap::new(),
    }));
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

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
