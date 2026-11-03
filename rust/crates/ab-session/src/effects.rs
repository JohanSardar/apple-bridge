use crate::{
    Error,
    frame::{read_frame, send, text},
    guard::{DispatchGuard, PreparedEffect},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::Shutdown,
    os::unix::net::UnixStream,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const EFFECT_QUERIES: &[&str] = &[
    "media.derive",
    "vision.ocr",
    "pdf.text",
    "pdf.search",
    "pdf.render",
    "audio.transcribe",
    "speech.synthesize",
    "maps.snapshot",
    "maps.search",
    "maps.nearby",
    "maps.directions",
    "maps.eta",
    "location.current",
    "location.geocode",
    "location.reverse_geocode",
    "shortcuts.actions",
    "mac.clipboard_read",
    "mac.network_check",
    "mac.bluetooth_devices",
    "mac.time_machine_status",
    "mac.recognize_song",
    "mac.iwork_export",
];

pub fn request_hash(args: &Value) -> Result<String, Error> {
    fn valid(value: &Value) -> bool {
        match value {
            Value::Number(n) => {
                n.as_i64()
                    .is_some_and(|n| n.unsigned_abs() <= 9_007_199_254_740_991)
                    || n.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991)
            }
            Value::Array(a) => a.iter().all(valid),
            Value::Object(o) => o.values().all(valid),
            _ => true,
        }
    }
    if !args.is_object() || !valid(args) {
        return Err(Error::Invalid);
    }
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(args)?)))
}

#[derive(Debug)]
struct Ticket {
    command: Value,
    prepared: Value,
    expires: Instant,
}

#[derive(Debug)]
struct Flight {
    checkpoint: String,
    keys: Vec<(String, String)>,
    bytes: usize,
    sequence: u64,
}

#[derive(Debug, Default)]
struct State {
    fence: u64,
    commands: BTreeMap<String, Value>,
    tickets: BTreeMap<String, Ticket>,
    flights: BTreeMap<String, Flight>,
    sequence: u64,
    committed: BTreeMap<(String, String), (String, u64)>,
}

impl State {
    fn helper(&mut self, value: &Value, size: usize) -> Result<(), Error> {
        match value["t"].as_str() {
            Some("Prepared") => {
                let id = text(value, "command_id")?;
                let command = self.commands.remove(id).ok_or(Error::Conflict)?;
                if value["fingerprint"] != command["request_hash"] {
                    return Err(Error::Conflict);
                }
                let ticket = text(value, "ticket")?.to_owned();
                self.tickets.insert(
                    ticket,
                    Ticket {
                        command,
                        prepared: value.clone(),
                        expires: Instant::now() + Duration::from_secs(30),
                    },
                );
            }
            Some("Rejected") => {
                if let Some(id) = value["command_id"].as_str() {
                    self.commands.remove(id);
                }
            }
            Some("Capture") => {
                let source = text(value, "source")?.to_owned();
                let checkpoint = text(value, "checkpoint")?.to_owned();
                let observations = value["observations"].as_array().ok_or(Error::Invalid)?;
                let keys = observations
                    .iter()
                    .filter(|o| o["kind"] == "upsert" || o["kind"] == "absent")
                    .map(|o| {
                        Ok((
                            text(o, "collection")?.to_owned(),
                            text(o, "native_key")?.to_owned(),
                        ))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                self.sequence = self.sequence.checked_add(1).ok_or(Error::Invalid)?;
                if self
                    .flights
                    .insert(
                        source,
                        Flight {
                            checkpoint,
                            keys,
                            bytes: size,
                            sequence: self.sequence,
                        },
                    )
                    .is_some()
                {
                    return Err(Error::Conflict);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn committed(&mut self, value: &Value) -> Result<(), Error> {
        let source = text(value, "source")?;
        let flight = self.flights.get(source).ok_or(Error::Conflict)?;
        if value["checkpoint"] != flight.checkpoint
            || value["credit"].as_u64()
                != Some(u64::try_from(flight.bytes).map_err(|_| Error::Invalid)?)
        {
            return Err(Error::Conflict);
        }
        let flight = self.flights.remove(source).ok_or(Error::Conflict)?;
        for key in flight.keys {
            self.committed
                .insert(key, (flight.checkpoint.clone(), flight.sequence));
        }
        Ok(())
    }
}

fn correlate(state: &State, evidence: &Value, command: &Value, after: u64) -> Option<Value> {
    if evidence["observed"] != true || evidence["fingerprint"] != command["request_hash"] {
        return None;
    }
    let operation = command["action"]["op"].as_str()?;
    let mut result = evidence.as_object()?.clone();
    if operation == "focus.set" {
        if evidence["readback"]["authorization"] != "authorized" {
            return None;
        }
    } else if operation.starts_with("mac.") {
        if evidence["readback"].is_null() {
            return None;
        }
    } else {
        let key = evidence["native_key"].as_str()?;
        let collections: &[&str] = match operation.split('.').next()? {
            "calendar" => &["calendar.event", "calendar.calendar"],
            "reminders" => &["reminders.reminder", "reminders.list"],
            "contacts" => &["contacts.contact", "contacts.group"],
            "notes" => &["notes.note", "notes.folder"],
            "mail" => &["mail.message"],
            "files" => &["files.item"],
            "pdf" => &["pdf.artifact"],
            "messages" => &["messages.message", "messages.chat"],
            _ => return None,
        };
        let (checkpoint, sequence) = collections
            .iter()
            .find_map(|c| state.committed.get(&(String::from(*c), key.to_owned())))?;
        if *sequence <= after {
            return None;
        }
        result.insert(
            "capture_checkpoint".into(),
            Value::String(checkpoint.clone()),
        );
    }
    Some(Value::Object(result))
}

fn durable_action(command: &Value) -> Value {
    let mut action = command["action"].clone();
    if action["op"] == "mac.clipboard_write" {
        action["args"] = json!({});
    }
    action
}

fn dispatch(
    state: &Mutex<State>,
    guard: &Mutex<DispatchGuard>,
    native: &Mutex<UnixStream>,
    frame: &Value,
) -> Result<Value, Error> {
    let (ticket, fence, sequence) = {
        let mut state = state.lock().map_err(|_| Error::WorkerPanicked)?;
        let ticket = state
            .tickets
            .remove(text(frame, "ticket")?)
            .ok_or(Error::Conflict)?;
        if ticket.command["command_id"] != frame["command_id"]
            || ticket.command["origin"] != frame["origin"]
            || frame["owner_fence"].as_u64() != Some(state.fence)
            || Instant::now() >= ticket.expires
        {
            return Err(Error::Conflict);
        }
        (ticket, state.fence, state.sequence)
    };
    let command = ticket.command;
    let id = text(&command, "command_id")?;
    let operation = text(&command["action"], "op")?;
    let expires = text(&command, "expires_at")?
        .parse::<jiff::Timestamp>()
        .map_err(|_| Error::Invalid)?
        .as_second();
    let permit = guard.lock().map_err(|_| Error::WorkerPanicked)?.begin(PreparedEffect {
        command_id: id.into(), owner_fence: fence, ticket: text(frame, "ticket")?.into(), request_hash: request_hash(&command)?, operation: operation.into(), expires_at: u64::try_from(expires).map_err(|_| Error::Invalid)?, prepared: json!({"action":durable_action(&command),"native_state":ticket.prepared["native_state"],"request_hash":command["request_hash"]}),
    })?;
    let response = (|| {
        let mut native = native.lock().map_err(|_| Error::WorkerPanicked)?;
        send(
            &mut native,
            &json!({"t":"Effect","id":id,"op":operation,"args":command,"permit":permit}),
        )?;
        let (_, response) = read_frame(&mut native)?;
        if response["t"] != "EffectResult" || response["id"] != id {
            return Err(Error::Conflict);
        }
        Ok(response["evidence"].clone())
    })();
    let correlated = match response {
        Ok(evidence) => correlate(
            &*state.lock().map_err(|_| Error::WorkerPanicked)?,
            &evidence,
            &command,
            sequence,
        ),
        Err(_) => None,
    };
    let disposition = if correlated.is_some() {
        "locally_observed"
    } else {
        "indeterminate"
    };
    let evidence = correlated.unwrap_or_else(|| json!({"reason":"native_effect_evidence_unavailable","prepared":{"action":durable_action(&command),"native_state":ticket.prepared["native_state"],"request_hash":command["request_hash"]}}));
    let mut guard = guard.lock().map_err(|_| Error::WorkerPanicked)?;
    guard.finish(id, disposition, &evidence)?;
    guard.outcome(id).cloned().ok_or(Error::Invalid)
}

fn query(
    state: &Mutex<State>,
    guard: &Mutex<DispatchGuard>,
    native: &Mutex<UnixStream>,
    frame: &Value,
) -> Result<Value, Error> {
    let id = text(frame, "id")?;
    let operation = text(frame, "op")?;
    let fence = state.lock().map_err(|_| Error::WorkerPanicked)?.fence;
    let command_id = format!("native-query:{fence}:{id}");
    let args = json!({"session_query":true,"args":frame["args"]});
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Invalid)?
        .as_secs()
        .checked_add(30)
        .ok_or(Error::Invalid)?;
    let permit = guard
        .lock()
        .map_err(|_| Error::WorkerPanicked)?
        .begin(PreparedEffect {
            command_id: command_id.clone(),
            owner_fence: fence,
            ticket: id.into(),
            request_hash: request_hash(&args)?,
            operation: operation.into(),
            expires_at,
            prepared: json!({"op":operation,"query_id":id}),
        })?;
    let response = (|| {
        let mut native = native.lock().map_err(|_| Error::WorkerPanicked)?;
        send(
            &mut native,
            &json!({"t":"Effect","id":id,"op":operation,"args":args,"permit":permit}),
        )?;
        let (_, response) = read_frame(&mut native)?;
        if response["t"] != "EffectResult"
            || response["id"] != id
            || response["evidence"]["observed"] != true
            || response["evidence"].get("native_query_result").is_none()
        {
            return Err(Error::Conflict);
        }
        Ok(response["evidence"]["native_query_result"].clone())
    })();
    let (disposition, evidence, result) = match response {
        Ok(result) => (
            "locally_observed",
            json!({"query_id":id,"observed":true}),
            json!({"t":"QueryResult","id":id,"result_json":result,"blobs":[]}),
        ),
        Err(_) => (
            "indeterminate",
            json!({"query_id":id,"reason":"native_query_evidence_unavailable"}),
            json!({"t":"QueryError","id":id,"code":"unavailable"}),
        ),
    };
    guard
        .lock()
        .map_err(|_| Error::WorkerPanicked)?
        .finish(&command_id, disposition, &evidence)?;
    Ok(result)
}

fn handshake(native: &Mutex<UnixStream>, fence: u64) -> Result<(), Error> {
    let mut native = native.lock().map_err(|_| Error::WorkerPanicked)?;
    send(
        &mut native,
        &json!({"t":"Hello","protocol":"native/1","version":1,"owner_fence":fence}),
    )?;
    let (_, welcome) = read_frame(&mut native)?;
    if welcome != json!({"t":"Welcome","protocol":"native/1","version":1}) {
        return Err(Error::Conflict);
    }
    Ok(())
}

#[derive(Debug)]
pub struct Layer {
    guard: Mutex<DispatchGuard>,
    state: Mutex<State>,
    native: Mutex<UnixStream>,
    native_shutdown: UnixStream,
    active: AtomicBool,
}

impl crate::routed::Overlay for Layer {
    fn gateway<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        value: &Value,
        output: &Arc<Mutex<UnixStream>>,
    ) -> Result<bool, Error> {
        Layer::gateway(self, scope, value, output)
    }

    fn helper(&self, value: &Value, size: usize) -> Result<(), Error> {
        Layer::helper(self, value, size)
    }

    fn helper_ready(&self, output: &Mutex<UnixStream>) -> Result<(), Error> {
        Layer::helper_ready(self, output)
    }

    fn shutdown(&self) {
        Layer::shutdown(self);
    }
}

impl Layer {
    pub fn open(native: UnixStream, home: &Path) -> Result<Self, Error> {
        native.set_read_timeout(Some(Duration::from_secs(30)))?;
        native.set_write_timeout(Some(Duration::from_secs(30)))?;
        Ok(Self {
            guard: Mutex::new(DispatchGuard::open(home)?),
            state: Mutex::new(State::default()),
            native_shutdown: native.try_clone()?,
            native: Mutex::new(native),
            active: AtomicBool::new(false),
        })
    }

    pub fn shutdown(&self) {
        let _ = self.native_shutdown.shutdown(Shutdown::Both);
    }

    pub fn helper(&self, value: &Value, size: usize) -> Result<(), Error> {
        self.state
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .helper(value, size)
    }

    pub fn helper_ready(&self, output: &Mutex<UnixStream>) -> Result<(), Error> {
        let unresolved = self
            .guard
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .unresolved();
        for outcome in unresolved {
            send(
                &mut *output.lock().map_err(|_| Error::WorkerPanicked)?,
                &outcome,
            )?;
        }
        Ok(())
    }

    pub fn gateway<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        value: &Value,
        output: &Arc<Mutex<UnixStream>>,
    ) -> Result<bool, Error> {
        match value["t"].as_str() {
            Some("Open") => {
                let fence = value["owner_fence"]
                    .as_u64()
                    .filter(|f| *f > 0)
                    .ok_or(Error::Invalid)?;
                let mut state = self.state.lock().map_err(|_| Error::WorkerPanicked)?;
                if state.fence != 0 {
                    return Err(Error::Conflict);
                }
                state.fence = fence;
                handshake(&self.native, fence)?;
            }
            Some("Prepare") => {
                let id = text(value, "command_id")?.to_owned();
                if self
                    .guard
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .outcome(&id)
                    .is_some()
                {
                    return Err(Error::AlreadyStarted);
                }
                let mut state = self.state.lock().map_err(|_| Error::WorkerPanicked)?;
                if state.commands.len() >= 8 || state.tickets.len() >= 8 {
                    return Err(Error::Conflict);
                }
                state.tickets.retain(|_, t| Instant::now() < t.expires);
                state.commands.insert(id, value.clone());
            }
            Some("Committed") => self
                .state
                .lock()
                .map_err(|_| Error::WorkerPanicked)?
                .committed(value)?,
            Some(kind @ ("Dispatch" | "Query"))
                if kind == "Dispatch"
                    || value["op"]
                        .as_str()
                        .is_some_and(|op| EFFECT_QUERIES.contains(&op)) =>
            {
                if self.active.swap(true, Ordering::SeqCst) {
                    return Err(Error::Conflict);
                }
                let is_query = kind == "Query";
                let output = Arc::clone(output);
                let value = value.clone();
                scope.spawn(move || {
                    let result = if is_query {
                        query(&self.state, &self.guard, &self.native, &value)
                    } else {
                        dispatch(&self.state, &self.guard, &self.native, &value)
                    };
                    self.active.store(false, Ordering::SeqCst);
                    if let Ok(mut output) = output.lock() {
                        if let Ok(result) = result {
                            let _ = send(&mut output, &result);
                        } else {
                            let _ = output.shutdown(Shutdown::Both);
                        }
                    }
                });
                return Ok(false);
            }
            Some("Resolve") => {
                let saved = self
                    .guard
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .outcome(text(value, "command_id")?)
                    .cloned();
                let mut output = output.lock().map_err(|_| Error::WorkerPanicked)?;
                send(
                    &mut output,
                    &saved.unwrap_or_else(|| json!({"t":"Rejected","command_id":value["command_id"],"disposition":"rejected_before_dispatch","error":{"code":"not_found","message":"not_found"}})),
                )?;
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    }
}

pub fn relay(
    gateway: UnixStream,
    helper: UnixStream,
    native: UnixStream,
    home: &Path,
) -> Result<(), Error> {
    let layer = Layer::open(native, home)?;
    crate::routed::relay_layered(
        gateway,
        helper,
        std::collections::BTreeMap::new(),
        &crate::routed::Options::default(),
        Some(&layer),
    )
}
