use crate::{
    Error,
    flight::Ledger,
    frame::{read_frame, send, text},
    guard::{DispatchGuard, PreparedEffect},
};
use serde_json::{Number, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const EFFECT_QUERIES: &[&str] = &[
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
];

pub const COMMANDS: &[&str] = &[
    "calendar.create_event",
    "calendar.update_event",
    "calendar.delete_event",
    "calendar.create_calendar",
    "reminders.create",
    "reminders.update",
    "reminders.complete",
    "reminders.uncomplete",
    "reminders.delete",
    "reminders.create_list",
    "reminders.rename_list",
    "reminders.delete_list",
    "contacts.create",
    "contacts.update",
    "contacts.delete",
    "contacts.create_group",
    "contacts.rename_group",
    "contacts.delete_group",
    "contacts.add_member",
    "contacts.remove_member",
    "messages.send",
    "messages.reply",
    "messages.react",
    "messages.edit",
    "messages.unsend",
    "messages.mark_read",
    "messages.mark_unread",
    "messages.delete_message",
    "messages.delete_chat",
    "notes.create",
    "notes.update",
    "notes.delete",
    "notes.create_folder",
    "notes.move",
    "notes.rename_folder",
    "notes.delete_folder",
    "mail.create_draft",
    "mail.send",
    "mail.move",
    "mail.set_flag",
    "mail.mark_read",
    "mail.delete",
    "files.create",
    "files.write",
    "files.move",
    "files.rename",
    "files.trash",
    "files.set_tags",
    "files.download",
    "files.evict",
    "files.transform",
    "pdf.annotate",
    "speech.say",
    "capture.play_sound",
    "capture.screenshot",
    "capture.camera_photo",
    "capture.record_audio",
    "mac.screen_record",
    "print.document",
    "maps.open",
    "shortcuts.run",
    "focus.set",
    "mac.set_volume",
    "mac.set_mute",
    "mac.set_appearance",
    "mac.launch_app",
    "mac.activate_app",
    "mac.quit_app",
    "mac.keep_awake",
    "mac.clipboard_write",
    "mac.notify",
    "mac.open_settings",
    "mac.set_wallpaper",
    "mac.iwork_export",
];

const DISPOSITIONS: &[&str] = &[
    "locally_observed",
    "indeterminate",
    "delivered",
    "delivery_failed",
    "failed_no_effect",
];
const FACT_SOURCES: &[&str] = &["calendar", "contacts", "reminders"];

#[must_use]
pub fn source_of(operation: &str) -> &str {
    if matches!(operation, "mac.screen_record" | "print.document") {
        return "capture";
    }
    operation.split('.').next().unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct Routing {
    sources: Option<BTreeSet<String>>,
}

impl Routing {
    #[must_use]
    pub fn all() -> Self {
        Self { sources: None }
    }

    pub fn only<I: IntoIterator<Item = String>>(sources: I) -> Self {
        Self {
            sources: Some(sources.into_iter().collect()),
        }
    }

    #[must_use]
    pub fn parse(list: &str) -> Self {
        let names: BTreeSet<String> = list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if names.is_empty() || names.contains("*") {
            Self::all()
        } else {
            Self::only(names)
        }
    }

    #[must_use]
    pub fn routes(&self, operation: &str) -> bool {
        self.sources
            .as_ref()
            .is_none_or(|sources| sources.contains(source_of(operation)))
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub routing: Routing,
    pub rust_sources: BTreeSet<String>,
    pub notes_database: Option<PathBuf>,
    pub correlation_wait: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            routing: Routing::all(),
            rust_sources: BTreeSet::new(),
            notes_database: None,
            correlation_wait: Duration::from_secs(10),
        }
    }
}

fn ambiguous_float(value: f64) -> bool {
    value != 0.0 && (value.abs() < 1e-4 || value.abs() >= 1e15)
}

fn canonical(value: &Value) -> Result<Value, Error> {
    Ok(match value {
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Value::Number(u.into())
            } else if let Some(i) = n.as_i64() {
                if i.unsigned_abs() > 9_007_199_254_740_991 {
                    return Err(Error::Invalid);
                }
                Value::Number(i.into())
            } else {
                let f = n.as_f64().filter(|f| f.is_finite()).ok_or(Error::Invalid)?;
                if ambiguous_float(f) {
                    return Err(Error::Invalid);
                }
                if f.fract() == 0.0 {
                    #[allow(clippy::cast_possible_truncation)]
                    let i = f as i64;
                    Value::Number(i.into())
                } else {
                    Value::Number(Number::from_f64(f).ok_or(Error::Invalid)?)
                }
            }
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect::<Result<_, _>>()?),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| Ok((k.clone(), canonical(v)?)))
                .collect::<Result<_, Error>>()?,
        ),
        other => other.clone(),
    })
}

pub fn request_hash(args: &Value) -> Result<String, Error> {
    if !args.is_object() {
        return Err(Error::Invalid);
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical(args)?)?)
    ))
}

#[derive(Debug)]
struct Ticket {
    command: Value,
    prepared: Value,
    expires: Instant,
}

#[derive(Debug, Default)]
struct State {
    fence: u64,
    commands: BTreeMap<String, Value>,
    tickets: BTreeMap<String, Ticket>,
}

impl State {
    fn helper(&mut self, value: &Value) -> Result<(), Error> {
        match value["t"].as_str() {
            Some("Prepared") => {
                let id = text(value, "command_id")?;
                let Some(command) = self.commands.remove(id) else {
                    return Ok(());
                };
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
            _ => {}
        }
        Ok(())
    }
}

fn collections(operation: &str, evidence: &Value) -> Option<Vec<String>> {
    if let Some(collection) = evidence["collection"].as_str().filter(|c| !c.is_empty()) {
        return Some(vec![collection.to_owned()]);
    }
    let fixed: &[&str] = match source_of(operation) {
        "calendar" => &["calendar.event", "calendar.calendar"],
        "reminders" => &["reminders.reminder", "reminders.list"],
        "contacts" => &["contacts.contact", "contacts.group"],
        "notes" => &["notes.note", "notes.folder"],
        "mail" => &["mail.message"],
        "files" => &["files.item"],
        "pdf" => &["pdf.artifact"],
        "messages" => &["messages.message", "messages.chat", "messages.reaction"],
        _ => return None,
    };
    Some(fixed.iter().map(|c| (*c).to_owned()).collect())
}

fn durable_action(command: &Value) -> Value {
    let mut action = command["action"].clone();
    if action["op"] == "mac.clipboard_write" {
        action["args"] = json!({});
    }
    action
}

fn rejection(command_id: &Value, code: &str, message: &str, evidence: Option<Value>) -> Value {
    let mut frame = json!({"t":"Rejected","command_id":command_id,"disposition":"rejected_before_dispatch","error":{"code":code,"message":message}});
    if let Some(evidence) = evidence {
        frame["evidence"] = evidence;
    }
    frame
}

fn notes_pre_state(database: &Path, command: &Value) -> Result<Option<Value>, Error> {
    let action = &command["action"];
    let operation = action["op"].as_str().unwrap_or_default();
    let key = action["native_key"].as_str().unwrap_or_default();
    if !operation.starts_with("notes.") || key.is_empty() {
        return Ok(None);
    }
    let collection = if matches!(operation, "notes.rename_folder" | "notes.delete_folder") {
        "notes.folder"
    } else {
        "notes.note"
    };
    match ab_native::notes_reader::pre_state(database, collection, key) {
        Ok(Some(state)) => Ok(Some(state)),
        Ok(None) => Err(Error::Conflict),
        Err(error) => Err(Error::Reader(error.to_string())),
    }
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
    config: Config,
}

impl crate::routed::Overlay for Layer {
    fn gateway<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        value: &Value,
        output: &Arc<Mutex<UnixStream>>,
        helper: &Mutex<UnixStream>,
        ledger: &'scope dyn Ledger,
    ) -> Result<bool, Error> {
        Layer::gateway(self, scope, value, output, helper, ledger)
    }

    fn helper(&self, value: &Value) -> Result<(), Error> {
        Layer::helper(self, value)
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
        Self::open_with(native, home, Config::default())
    }

    pub fn open_with(native: UnixStream, home: &Path, config: Config) -> Result<Self, Error> {
        native.set_read_timeout(Some(Duration::from_secs(30)))?;
        native.set_write_timeout(Some(Duration::from_secs(30)))?;
        Ok(Self {
            guard: Mutex::new(DispatchGuard::open(home)?),
            state: Mutex::new(State::default()),
            native_shutdown: native.try_clone()?,
            native: Mutex::new(native),
            active: AtomicBool::new(false),
            config,
        })
    }

    pub fn shutdown(&self) {
        let _ = self.native_shutdown.shutdown(Shutdown::Both);
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, Error> {
        self.state.lock().map_err(|_| Error::WorkerPanicked)
    }

    fn guard(&self) -> Result<MutexGuard<'_, DispatchGuard>, Error> {
        self.guard.lock().map_err(|_| Error::WorkerPanicked)
    }

    pub fn helper(&self, value: &Value) -> Result<(), Error> {
        if value["t"] == "Outcome"
            && let Some(id) = value["command_id"].as_str()
        {
            let mut guard = self.guard()?;
            if guard.outcome(id).is_some_and(|saved| {
                saved["t"] == "Outcome"
                    && (saved["disposition"] != value["disposition"]
                        || saved["evidence"] != value["evidence"])
            }) && let Some(disposition) = value["disposition"].as_str()
            {
                let _ = guard.finish(id, disposition, &value["evidence"]);
            }
        }
        self.state()?.helper(value)
    }

    pub fn helper_ready(&self, output: &Mutex<UnixStream>) -> Result<(), Error> {
        let unresolved = self.guard()?.unresolved();
        for outcome in unresolved {
            send(
                &mut *output.lock().map_err(|_| Error::WorkerPanicked)?,
                &outcome,
            )?;
        }
        Ok(())
    }

    fn effect(
        &self,
        id: &str,
        operation: &str,
        args: &Value,
        permit: &impl serde::Serialize,
    ) -> Result<Value, Error> {
        let mut native = self.native.lock().map_err(|_| Error::WorkerPanicked)?;
        send(
            &mut native,
            &json!({"t":"Effect","id":id,"op":operation,"args":args,"permit":permit}),
        )?;
        let (_, response) = read_frame(&mut native)?;
        if response["t"] != "EffectResult" || response["id"] != id {
            return Err(Error::Conflict);
        }
        Ok(response["evidence"].clone())
    }

    fn correlate(
        &self,
        evidence: &Value,
        command: &Value,
        prepared: &Value,
        after: u64,
        ledger: &dyn Ledger,
    ) -> Result<(String, Value), Error> {
        let operation = command["action"]["op"].as_str().unwrap_or_default();
        let unavailable = || {
            (
                "indeterminate".to_owned(),
                json!({"reason":"native_effect_evidence_unavailable","prepared":prepared}),
            )
        };
        let disposition = evidence["disposition"]
            .as_str()
            .filter(|d| !d.is_empty())
            .unwrap_or("locally_observed");
        if !DISPOSITIONS.contains(&disposition) {
            return Ok(unavailable());
        }
        let Some(fields) = evidence.as_object() else {
            return Ok(unavailable());
        };
        let mut result = fields.clone();
        result.insert("prepared".into(), prepared.clone());
        if disposition == "indeterminate" {
            return Ok((disposition.to_owned(), Value::Object(result)));
        }
        if evidence["observed"] != true {
            return Ok((disposition.to_owned(), Value::Object(result)));
        }
        if evidence["fingerprint"] != command["request_hash"] {
            return Ok(unavailable());
        }
        let source = source_of(operation);
        let verified = match operation {
            "focus.set" => evidence["readback"]["authorization"] == "authorized",
            "speech.say" | "capture.play_sound" => {
                evidence["finished_callback"] == true
                    && prepared["request_hash"] == command["request_hash"]
            }
            "capture.screenshot"
            | "capture.camera_photo"
            | "capture.record_audio"
            | "mac.screen_record"
            | "print.document"
            | "maps.open"
            | "shortcuts.run" => true,
            _ if source == "mac" => !evidence["readback"].is_null(),
            _ => {
                let Some(key) = evidence["native_key"].as_str() else {
                    return Ok(unavailable());
                };
                let Some(candidates) = collections(operation, evidence) else {
                    return Ok(unavailable());
                };
                let names: Vec<&str> = candidates.iter().map(String::as_str).collect();
                let rust_owned =
                    self.config.rust_sources.contains(source) && !FACT_SOURCES.contains(&source);
                let required = rust_owned.then_some(after);
                let Some(checkpoint) =
                    ledger.written(&names, key, required, self.config.correlation_wait)?
                else {
                    result.insert("reason".into(), json!("capture_not_observed"));
                    return Ok(("indeterminate".to_owned(), Value::Object(result)));
                };
                result.insert("capture_checkpoint".into(), Value::String(checkpoint));
                true
            }
        };
        if !verified {
            return Ok(unavailable());
        }
        Ok((disposition.to_owned(), Value::Object(result)))
    }

    fn settle_rejection(
        &self,
        id: &str,
        command: &Value,
        rejected: &Value,
        ledger: &dyn Ledger,
    ) -> Result<Value, Error> {
        let code = rejected["code"]
            .as_str()
            .filter(|c| !c.is_empty())
            .unwrap_or("unavailable");
        let message = rejected["message"]
            .as_str()
            .filter(|c| !c.is_empty())
            .unwrap_or(code);
        let mut code = code;
        let mut evidence = None;
        if let Some(key) = rejected["evidence"]["native_key"].as_str() {
            let operation = command["action"]["op"].as_str().unwrap_or_default();
            let found = collections(operation, &rejected["evidence"]).and_then(|candidates| {
                let names: Vec<&str> = candidates.iter().map(String::as_str).collect();
                ledger.written(&names, key, None, Duration::ZERO).ok()?
            });
            if let Some(checkpoint) = found {
                evidence = Some(json!({"native_key":key,"capture_checkpoint":checkpoint}));
            } else {
                code = "unavailable";
            }
        }
        self.guard()?
            .reject(id, code, message, evidence.as_ref().unwrap_or(&Value::Null))?;
        self.guard()?.outcome(id).cloned().ok_or(Error::Invalid)
    }

    fn dispatch(&self, frame: &Value, ledger: &dyn Ledger) -> Result<Value, Error> {
        let command_id = &frame["command_id"];
        let sequence = ledger.sequence()?;
        let (ticket, fence) = {
            let mut state = self.state()?;
            let Some(ticket) = state.tickets.remove(text(frame, "ticket")?) else {
                return Ok(rejection(command_id, "conflict", "conflict", None));
            };
            if ticket.command["command_id"] != frame["command_id"]
                || ticket.command["origin"] != frame["origin"]
                || frame["owner_fence"].as_u64() != Some(state.fence)
                || Instant::now() >= ticket.expires
            {
                return Ok(rejection(command_id, "conflict", "conflict", None));
            }
            (ticket, state.fence)
        };
        let command = ticket.command;
        let id = text(&command, "command_id")?;
        let operation = text(&command["action"], "op")?;
        let expires = text(&command, "expires_at")?
            .parse::<jiff::Timestamp>()
            .map_err(|_| Error::Invalid)?
            .as_second();
        let mut args = command.clone();
        if let Some(database) = &self.config.notes_database {
            match notes_pre_state(database, &command) {
                Ok(Some(pre_state)) => args["rust_pre_state"] = pre_state,
                Ok(None) => {}
                Err(Error::Conflict) => {
                    return Ok(rejection(command_id, "conflict", "conflict", None));
                }
                Err(_) => {
                    return Ok(rejection(
                        command_id,
                        "unavailable",
                        "notes_pre_state_unavailable",
                        None,
                    ));
                }
            }
        }
        let args = canonical(&args)?;
        let prepared = json!({"action":durable_action(&command),"native_state":ticket.prepared["native_state"],"request_hash":command["request_hash"]});
        let permit = self.guard()?.begin(PreparedEffect {
            command_id: id.into(),
            owner_fence: fence,
            ticket: text(frame, "ticket")?.into(),
            request_hash: request_hash(&args)?,
            operation: operation.into(),
            expires_at: u64::try_from(expires).map_err(|_| Error::Invalid)?,
            prepared: prepared.clone(),
        })?;
        let response = self.effect(id, operation, &args, &permit);
        if let Ok(evidence) = &response
            && evidence["rejected"].is_object()
        {
            return self.settle_rejection(id, &command, &evidence["rejected"], ledger);
        }
        let (disposition, evidence) = match response {
            Ok(evidence) => self.correlate(&evidence, &command, &prepared, sequence, ledger)?,
            Err(_) => (
                "indeterminate".to_owned(),
                json!({"reason":"native_effect_evidence_unavailable","prepared":prepared}),
            ),
        };
        self.guard()?.finish(id, &disposition, &evidence)?;
        self.guard()?.outcome(id).cloned().ok_or(Error::Invalid)
    }

    fn query(&self, frame: &Value) -> Result<Value, Error> {
        let id = text(frame, "id")?;
        let operation = text(frame, "op")?;
        let fence = self.state()?.fence;
        let command_id = format!("native-query:{fence}:{id}");
        let args = canonical(&json!({"session_query":true,"args":frame["args"]}))?;
        let expires_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Invalid)?
            .as_secs()
            .checked_add(30)
            .ok_or(Error::Invalid)?;
        let permit = self.guard()?.begin(PreparedEffect {
            command_id: command_id.clone(),
            owner_fence: fence,
            ticket: id.into(),
            request_hash: request_hash(&args)?,
            operation: operation.into(),
            expires_at,
            prepared: json!({"op":operation,"query_id":id}),
        })?;
        let response = self
            .effect(id, operation, &args, &permit)
            .and_then(|evidence| {
                if evidence["native_query_error"].is_string() {
                    return Ok(evidence);
                }
                if evidence["observed"] != true || evidence.get("native_query_result").is_none() {
                    return Err(Error::Conflict);
                }
                Ok(evidence)
            });
        let (disposition, evidence, result) = match response {
            Ok(evidence) if evidence["native_query_error"].is_string() => (
                "failed_no_effect",
                json!({"query_id":id,"observed":false,"error":evidence["native_query_error"]}),
                json!({"t":"QueryError","id":id,"code":evidence["native_query_error"]}),
            ),
            Ok(evidence) => (
                "locally_observed",
                json!({"query_id":id,"observed":true}),
                json!({"t":"QueryResult","id":id,"result_json":evidence["native_query_result"],"blobs":if evidence["native_query_blobs"].is_array() { evidence["native_query_blobs"].clone() } else { json!([]) }}),
            ),
            Err(_) => (
                "indeterminate",
                json!({"query_id":id,"reason":"native_query_evidence_unavailable"}),
                json!({"t":"QueryError","id":id,"code":"unavailable"}),
            ),
        };
        self.guard()?.finish(&command_id, disposition, &evidence)?;
        Ok(result)
    }

    pub fn gateway<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        value: &Value,
        output: &Arc<Mutex<UnixStream>>,
        helper: &Mutex<UnixStream>,
        ledger: &'scope dyn Ledger,
    ) -> Result<bool, Error> {
        match value["t"].as_str() {
            Some("Open") => {
                let fence = value["owner_fence"]
                    .as_u64()
                    .filter(|f| *f > 0)
                    .ok_or(Error::Invalid)?;
                let mut state = self.state()?;
                if state.fence != 0 {
                    return Err(Error::Conflict);
                }
                state.fence = fence;
                handshake(&self.native, fence)?;
            }
            Some("Prepare") => {
                let operation = value["action"]["op"].as_str().unwrap_or_default();
                if !self.config.routing.routes(operation) {
                    return Ok(true);
                }
                let id = text(value, "command_id")?.to_owned();
                if !COMMANDS.contains(&operation) {
                    send(
                        &mut *output.lock().map_err(|_| Error::WorkerPanicked)?,
                        &rejection(&value["command_id"], "unsupported", "unsupported", None),
                    )?;
                    return Ok(false);
                }
                if self.guard()?.outcome(&id).is_some() {
                    return Err(Error::AlreadyStarted);
                }
                let mut state = self.state()?;
                if state.commands.len() >= 8 || state.tickets.len() >= 8 {
                    return Err(Error::Conflict);
                }
                state.tickets.retain(|_, t| Instant::now() < t.expires);
                state.commands.insert(id, value.clone());
            }
            Some("Dispatch")
                if self
                    .state()?
                    .tickets
                    .contains_key(value["ticket"].as_str().unwrap_or_default()) =>
            {
                self.spawn(scope, value, output, ledger, false)?;
                return Ok(false);
            }
            Some("Query")
                if value["op"].as_str().is_some_and(|op| {
                    EFFECT_QUERIES.contains(&op) && self.config.routing.routes(op)
                }) =>
            {
                self.spawn(scope, value, output, ledger, true)?;
                return Ok(false);
            }
            Some("Resolve") => {
                let saved = self.guard()?.outcome(text(value, "command_id")?).cloned();
                let Some(saved) = saved else {
                    return Ok(true);
                };
                if saved["t"] == "Outcome" {
                    let mut forwarded = value.clone();
                    forwarded["saved"] = saved;
                    send(
                        &mut *helper.lock().map_err(|_| Error::WorkerPanicked)?,
                        &forwarded,
                    )?;
                    return Ok(false);
                }
                send(
                    &mut *output.lock().map_err(|_| Error::WorkerPanicked)?,
                    &saved,
                )?;
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    }

    fn spawn<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        value: &Value,
        output: &Arc<Mutex<UnixStream>>,
        ledger: &'scope dyn Ledger,
        is_query: bool,
    ) -> Result<(), Error> {
        if self.active.swap(true, Ordering::SeqCst) {
            return Err(Error::Conflict);
        }
        let output = Arc::clone(output);
        let value = value.clone();
        scope.spawn(move || {
            let result = if is_query {
                self.query(&value)
            } else {
                self.dispatch(&value, ledger)
            };
            self.active.store(false, Ordering::SeqCst);
            if let Ok(mut output) = output.lock() {
                match result {
                    Ok(result) => {
                        let _ = send(&mut output, &result);
                    }
                    Err(error) => {
                        eprintln!("ab-session: effect failed: {error}");
                        let _ = output.shutdown(Shutdown::Both);
                    }
                }
            }
        });
        Ok(())
    }
}

pub fn relay(
    gateway: UnixStream,
    helper: UnixStream,
    native: UnixStream,
    home: &Path,
) -> Result<(), Error> {
    relay_with(gateway, helper, native, home, Config::default())
}

pub fn relay_with(
    gateway: UnixStream,
    helper: UnixStream,
    native: UnixStream,
    home: &Path,
    config: Config,
) -> Result<(), Error> {
    let layer = Layer::open_with(native, home, config)?;
    crate::routed::relay_layered(
        gateway,
        helper,
        std::collections::BTreeMap::new(),
        &crate::routed::Options::default(),
        Some(&layer),
    )
}
