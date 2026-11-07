use crate::{
    Code, Codec, Command, CommandQuery, Cursor, Error, Identity, NativeQuery, Principal,
    descriptor_for,
    peer::{ApplePeer, IpcFrame},
    valid_command_id,
};
use ab_trust::{
    domain::{CommandState as State, Disposition},
    wire::gojson::GoTime,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use std::{
    collections::BTreeMap,
    fmt::Debug,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;

mod contracts;
pub use contracts::*;
mod admission;
mod approvals;
mod execution;
mod location;
mod message_parts;
mod outcomes;
mod queries;
mod status;
use message_parts::resolvable;

#[derive(Debug)]
pub struct LocalGateway {
    pub state: Arc<dyn GatewayState>,
    pub identity: Arc<dyn Identity>,
    pub codec: Arc<dyn Codec>,
    pub peer: Arc<dyn ApplePeer>,
    pub actions: Arc<dyn ActionResolver>,
    pub clock: Arc<dyn crate::clock::Clock>,
    pub query_store: Option<Arc<crate::LocalQueryStore>>,
    pub query_audit: Option<Arc<dyn crate::QueryAuditSink>>,
    pub reauthorize: Option<Arc<dyn ab_trust::ApprovalReauthorizer>>,
    pub phone_notify: Option<Arc<tokio::sync::Notify>>,
    query_rates: Mutex<BTreeMap<String, (std::time::Instant, u32)>>,
    locks: Mutex<BTreeMap<String, Arc<AsyncMutex<()>>>>,
    stopping: std::sync::atomic::AtomicBool,
}
fn raw<T: Serialize>(value: &T) -> Result<Box<RawValue>, Error> {
    let direct = serde_json::value::to_raw_value(value).map_err(|_| Code::Corrupt)?;
    if !direct.get().contains("mac.clipboard_write") {
        return Ok(direct);
    }
    let mut value: Value = serde_json::from_str(direct.get()).map_err(|_| Code::Corrupt)?;
    redact_clipboard(&mut value);
    serde_json::value::to_raw_value(&value).map_err(|_| Code::Corrupt.into())
}
fn redact_clipboard(value: &mut serde_json::Value) {
    if value["command"]["op"] == "mac.clipboard_write" {
        value["command"]["args"] = serde_json::json!({});
    }
    if let Some(values) = value.as_array_mut() {
        for value in values {
            redact_clipboard(value);
        }
    }
}
fn timestamp(value: &str) -> Result<jiff::Timestamp, Error> {
    value.parse().map_err(|_| Code::Invalid.into())
}
impl LocalGateway {
    pub fn new(
        state: Arc<dyn GatewayState>,
        identity: Arc<dyn Identity>,
        codec: Arc<dyn Codec>,
        peer: Arc<dyn ApplePeer>,
        actions: Arc<dyn ActionResolver>,
    ) -> Self {
        Self {
            state,
            identity,
            codec,
            peer,
            actions,
            clock: Arc::new(crate::clock::SystemClock),
            query_store: None,
            query_audit: None,
            phone_notify: None,
            reauthorize: None,
            query_rates: Mutex::default(),
            locks: Mutex::default(),
            stopping: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn stop(&self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn stopping(&self) -> bool {
        self.stopping.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn lock(&self, key: String) -> Result<Arc<AsyncMutex<()>>, Error> {
        Ok(self
            .locks
            .lock()
            .map_err(|_| Code::Unavailable)?
            .entry(key)
            .or_default()
            .clone())
    }
    fn authorize(&self, principal: &Principal, op: &str) -> Result<Principal, Error> {
        let current = self.identity.current(principal)?;
        let scope = descriptor_for(op)?.scope;
        if current.is_agent() && current.level == "read" && scope.ends_with(":write") {
            return Err(Code::Unauthorized.into());
        }
        current.authorize(&scope)?;
        Ok(current)
    }
    fn transition(
        &self,
        mut record: CommandRecord,
        state: State,
        disposition: Option<Disposition>,
        evidence: Option<Box<RawValue>>,
        error: Option<Error>,
    ) -> Result<CommandRecord, Error> {
        let from = record.state;
        record.state = state;
        record.disposition = disposition;
        record.error = error;
        if evidence.is_some() {
            record.evidence = evidence;
        }
        self.state.transition(Some(from), record)
    }
    fn expire(&self, record: CommandRecord) -> Result<CommandRecord, Error> {
        if record.state == State::AwaitingApproval
            && timestamp(&record.expires_at)? <= self.clock.now()
        {
            self.transition(
                record,
                State::Expired,
                Some(Disposition::Expired),
                None,
                None,
            )
        } else {
            Ok(record)
        }
    }
}
fn outcome_timeout(command: &Command) -> Result<Duration, Error> {
    if command.op == "focus.set" {
        return Ok(Duration::from_secs(25));
    }
    if crate::outcome::capture(&command.op) {
        return Ok(Duration::from_secs(75));
    }
    if command.op.starts_with("messages.") {
        return Ok(Duration::from_secs(35));
    }
    if command.op.starts_with("mail.") {
        return Ok(Duration::from_secs(60));
    }
    if matches!(command.op.as_str(), "files.evict" | "files.download") {
        return Ok(Duration::from_secs(110));
    }
    if command.op == "shortcuts.run" {
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let millis = args["timeout_ms"].as_u64().unwrap_or(0);
        if millis > 300_000 {
            return Err(Code::Invalid.into());
        }
        return Ok(Duration::from_millis(
            if millis == 0 { 30_000 } else { millis } + 5_000,
        ));
    }
    Ok(Duration::from_secs(15))
}

fn target_collection(op: &str) -> String {
    match op {
        "reminders.create_list" | "reminders.rename_list" | "reminders.delete_list" => {
            "reminders.list"
        }
        "pdf.annotate" => "pdf.artifact",
        "calendar.create_calendar" => "calendar.calendar",
        "notes.create_folder" | "notes.rename_folder" => "notes.folder",
        op if op.starts_with("reminders.") => "reminders.reminder",
        op if op.starts_with("calendar.") => "calendar.event",
        op if op.starts_with("contacts.") && (op.contains("group") || op.ends_with("_member")) => {
            "contacts.group"
        }
        op if op.starts_with("contacts.") => "contacts.contact",
        op if op.starts_with("notes.") => "notes.note",
        "messages.mark_read" | "messages.mark_unread" | "messages.delete_chat" => "messages.chat",
        "messages.react" => "messages.reaction",
        op if op.starts_with("messages.") => "messages.message",
        op if op.starts_with("mail.") => "mail.message",
        op if op.starts_with("files.") => "files.item",
        _ => "",
    }
    .into()
}
#[cfg(test)]
#[path = "local_evidence.rs"]
mod evidence;
#[cfg(test)]
#[path = "local_races.rs"]
mod races;
#[cfg(test)]
#[path = "local_security.rs"]
mod security;
