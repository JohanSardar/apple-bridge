use super::{Hub, rejected};
use crate::{
    Error,
    guard::{DispatchGuard, PreparedEffect, durable_action},
};
use ab_wire::{
    messages::IpcFrame,
    native::{
        BlobRead, Command, Dispatch, Frame as Link, Outcome, Prepared, Query, QueryCancel,
        Rejected, Resolve,
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io::Read,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TICKET_SECONDS: i64 = 30;
const BIND_WAIT: Duration = Duration::from_secs(15);
const DISPOSITIONS: &[&str] = &[
    "locally_observed",
    "indeterminate",
    "delivered",
    "delivery_failed",
    "failed_no_effect",
];

struct Ticket {
    command: Command,
    prepared: Prepared,
    expiry: Instant,
}

pub(super) struct Active {
    command: Command,
    prepared: Prepared,
    after: u64,
}

pub(super) struct Commands {
    guard: DispatchGuard,
    fence: u64,
    preparing: Option<Command>,
    tickets: BTreeMap<String, Ticket>,
    active: Option<Active>,
    resolving: BTreeSet<String>,
    queries: BTreeSet<String>,
}

impl Commands {
    pub(super) fn open(home: &Path, fence: u64) -> Result<Self, Error> {
        Ok(Self {
            guard: DispatchGuard::open(home)?,
            fence,
            preparing: None,
            tickets: BTreeMap::new(),
            active: None,
            resolving: BTreeSet::new(),
            queries: BTreeSet::new(),
        })
    }

    pub(super) fn unresolved(&self) -> Vec<Value> {
        self.guard.unresolved()
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn random_hex(bytes: usize) -> Result<String, Error> {
    let mut random = vec![0_u8; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
    Ok(random.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    }))
}

fn parsed(raw: &serde_json::value::RawValue) -> Value {
    serde_json::from_str(raw.get()).unwrap_or(Value::Null)
}

fn raw(value: &Value) -> Result<Box<serde_json::value::RawValue>, Error> {
    Ok(serde_json::value::RawValue::from_string(value.to_string())?)
}

fn facts_source(operation: &str) -> bool {
    matches!(
        operation.split('.').next().unwrap_or_default(),
        "calendar" | "contacts" | "reminders"
    )
}

fn needs_capture(operation: &str) -> bool {
    let source = operation.split('.').next().unwrap_or_default();
    source != "mac"
        && !matches!(
            operation,
            "focus.set"
                | "speech.say"
                | "capture.play_sound"
                | "capture.screenshot"
                | "capture.camera_photo"
                | "capture.record_audio"
                | "print.document"
                | "maps.open"
                | "shortcuts.run"
        )
}

fn saved_frame(saved: &Value) -> Result<IpcFrame, Error> {
    Ok(serde_json::from_value(saved.clone())?)
}

impl Hub {
    pub(super) fn gateway_command(&self, frame: &IpcFrame) -> Result<(), Error> {
        match frame.t.as_str() {
            "Prepare" => self.prepare(frame),
            "Dispatch" => self.dispatch(frame),
            "Resolve" => self.resolve(frame),
            "Query" => self.query(frame),
            "QueryCancel" => self.send_link(&Link::QueryCancel(QueryCancel {
                id: frame.id.clone(),
            })),
            "BlobRead" => self.send_link(&Link::BlobRead(BlobRead::from_ipc(frame))),
            _ => Err(Error::Invalid),
        }
    }

    fn refuse(&self, command_id: &str, code: &str) -> Result<(), Error> {
        self.send_gateway(&rejected(command_id, code)?)
    }

    fn prepare(&self, frame: &IpcFrame) -> Result<(), Error> {
        let id = frame.command_id.clone();
        let Ok(command) = Command::from_ipc(frame) else {
            return self.refuse(&id, "invalid");
        };
        let live = command.expires_unix().is_ok_and(|at| at > unix_now());
        if command.command_id.is_empty()
            || command.principal.is_empty()
            || command.request_hash.is_empty()
            || !live
        {
            return self.refuse(&id, "invalid");
        }
        let mut core = self.core()?;
        let commands = &mut core.commands;
        commands
            .tickets
            .retain(|_, ticket| Instant::now() < ticket.expiry);
        if commands.active.is_some()
            || !commands.tickets.is_empty()
            || commands.preparing.is_some()
            || commands.guard.outcome(&id).is_some()
        {
            drop(core);
            return self.refuse(&id, "conflict");
        }
        commands.preparing = Some(command.clone());
        drop(core);
        self.send_link(&Link::Prepare(command))
    }

    fn dispatch(&self, frame: &IpcFrame) -> Result<(), Error> {
        let mut core = self.core()?;
        let after = core.flights.sequence();
        let fence = core.commands.fence;
        let Some(ticket) = core.commands.tickets.remove(&frame.ticket) else {
            drop(core);
            return self.refuse(&frame.command_id, "conflict");
        };
        let id = ticket.command.command_id.clone();
        if core.commands.active.is_some() {
            drop(core);
            return self.refuse(&id, "conflict");
        }
        if frame.origin != ticket.command.origin
            || frame.command_id != id
            || frame.owner_fence != fence
            || Instant::now() >= ticket.expiry
        {
            drop(core);
            return self.refuse(&id, "expired");
        }
        let action = parsed(&ticket.command.action);
        let prepared = json!({
            "action": durable_action(&action),
            "native_state": parsed(&ticket.prepared.native_state),
            "fingerprint": ticket.prepared.fingerprint,
            "request_hash": ticket.command.request_hash,
        });
        let started = core.commands.guard.begin(PreparedEffect {
            command_id: id.clone(),
            owner_fence: fence,
            ticket: frame.ticket.clone(),
            request_hash: ticket.command.request_hash.clone(),
            operation: ticket.command.operation(),
            expires_at: u64::try_from(ticket.command.expires_unix().map_err(|_| Error::Invalid)?)
                .map_err(|_| Error::Invalid)?,
            prepared,
        });
        match started {
            Ok(_) => {}
            Err(Error::AlreadyStarted) => {
                drop(core);
                return self.refuse(&id, "conflict");
            }
            Err(_) => {
                drop(core);
                return self.refuse(&id, "unavailable");
            }
        }
        let message = Dispatch {
            command_id: id,
            request_hash: ticket.command.request_hash.clone(),
            action: ticket.command.action.clone(),
            native_state: ticket.prepared.native_state.clone(),
            fingerprint: ticket.prepared.fingerprint.clone(),
        };
        core.commands.active = Some(Active {
            command: ticket.command,
            prepared: ticket.prepared,
            after,
        });
        drop(core);
        self.send_link(&Link::Dispatch(message))
    }

    fn resolve(&self, frame: &IpcFrame) -> Result<(), Error> {
        let id = frame.command_id.clone();
        let mut core = self.core()?;
        let Some(saved) = core.commands.guard.outcome(&id).cloned() else {
            drop(core);
            return self.refuse(&id, "not_found");
        };
        let open = saved["t"] == "Outcome"
            && ["indeterminate", "locally_observed"]
                .contains(&saved["disposition"].as_str().unwrap_or_default());
        if open {
            core.commands.resolving.insert(id.clone());
            drop(core);
            return self.send_link(&Link::Resolve(Resolve {
                command_id: id,
                saved: raw(&saved)?,
            }));
        }
        drop(core);
        self.send_gateway(&saved_frame(&saved)?)
    }

    fn query(&self, frame: &IpcFrame) -> Result<(), Error> {
        let mut core = self.core()?;
        if frame.id.is_empty() || !core.commands.queries.insert(frame.id.clone()) {
            drop(core);
            return self.send_gateway(
                &ab_wire::native::QueryError {
                    id: frame.id.clone(),
                    code: "conflict".into(),
                }
                .into_ipc(),
            );
        }
        drop(core);
        self.send_link(&Link::Query(Query::from_ipc(frame)))
    }

    pub(super) fn finish_query(&self, id: &str) -> Result<(), Error> {
        if self.core()?.commands.queries.remove(id) {
            Ok(())
        } else {
            Err(Error::Invalid)
        }
    }

    pub(super) fn prepared(&self, prepared: Prepared) -> Result<(), Error> {
        let mut core = self.core()?;
        let command = match core.commands.preparing.take() {
            Some(command) if command.command_id == prepared.command_id => command,
            _ => return Err(Error::Invalid),
        };
        let remaining = command.expires_unix().map_err(|_| Error::Invalid)? - unix_now();
        if remaining <= 0 {
            drop(core);
            return self.refuse(&command.command_id, "expired");
        }
        let ticket = random_hex(16)?;
        let lifetime = u64::try_from(remaining.min(TICKET_SECONDS)).map_err(|_| Error::Invalid)?;
        let fingerprint = command.request_hash.clone();
        core.commands.tickets.insert(
            ticket.clone(),
            Ticket {
                command,
                prepared: prepared.clone(),
                expiry: Instant::now() + Duration::from_secs(lifetime),
            },
        );
        drop(core);
        self.send_gateway(&prepared.into_ipc(ticket, fingerprint))
    }

    pub(super) fn rejected_by_helper(&self, rejection: Rejected) -> Result<(), Error> {
        let id = rejection.command_id.clone();
        let mut core = self.core()?;
        if core
            .commands
            .preparing
            .as_ref()
            .is_some_and(|command| command.command_id == id)
        {
            core.commands.preparing = None;
            drop(core);
            return self.send_gateway(&rejection.into_ipc().map_err(|_| Error::Invalid)?);
        }
        let started = core
            .commands
            .active
            .as_ref()
            .is_some_and(|active| active.command.command_id == id);
        if started {
            core.commands.active = None;
            let evidence = rejection.evidence.as_deref().map_or(Value::Null, parsed);
            core.commands
                .guard
                .reject(&id, &rejection.code, &rejection.message, &evidence)?;
            let saved = core
                .commands
                .guard
                .outcome(&id)
                .cloned()
                .ok_or(Error::Invalid)?;
            drop(core);
            return self.send_gateway(&saved_frame(&saved)?);
        }
        if core.commands.resolving.remove(&id) {
            drop(core);
            return self.send_gateway(&rejection.into_ipc().map_err(|_| Error::Invalid)?);
        }
        Err(Error::Invalid)
    }

    pub(super) fn outcome_from_helper(
        &self,
        outcome: Outcome,
    ) -> Result<Option<(Active, Outcome)>, Error> {
        let id = outcome.command_id.clone();
        let mut core = self.core()?;
        if core.commands.resolving.remove(&id) {
            let saved = core.commands.guard.outcome(&id).cloned();
            let evidence = parsed(&outcome.evidence);
            let changed = saved.is_none_or(|saved| {
                saved["disposition"] != outcome.disposition || saved["evidence"] != evidence
            });
            if changed && DISPOSITIONS.contains(&outcome.disposition.as_str()) {
                core.commands
                    .guard
                    .finish(&id, &outcome.disposition, &evidence)?;
            }
            let saved = core
                .commands
                .guard
                .outcome(&id)
                .cloned()
                .ok_or(Error::Invalid)?;
            drop(core);
            self.send_gateway(&saved_frame(&saved)?)?;
            return Ok(None);
        }
        let matches = core
            .commands
            .active
            .as_ref()
            .is_some_and(|active| active.command.command_id == id);
        if !matches {
            return Err(Error::Invalid);
        }
        let active = core.commands.active.take().ok_or(Error::Invalid)?;
        Ok(Some((active, outcome)))
    }

    pub(super) fn settle(&self, active: &Active, outcome: Outcome) -> Result<(), Error> {
        let id = active.command.command_id.clone();
        let operation = active.command.operation();
        let mut disposition = outcome.disposition;
        let mut evidence = parsed(&outcome.evidence);
        if !DISPOSITIONS.contains(&disposition.as_str()) {
            disposition = "indeterminate".into();
        }
        if evidence.get("prepared").is_none() {
            evidence["prepared"] = json!({
                "action": durable_action(&parsed(&active.command.action)),
                "native_state": parsed(&active.prepared.native_state),
                "request_hash": active.command.request_hash,
            });
        }
        if disposition != "indeterminate"
            && evidence["observed"] == true
            && needs_capture(&operation)
        {
            let key = evidence["native_key"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let bound = if key.is_empty() {
                None
            } else {
                self.bind(&key, facts_source(&operation), active.after)?
            };
            if let Some(checkpoint) = bound {
                evidence["capture_checkpoint"] = Value::String(checkpoint);
            } else {
                disposition = "indeterminate".into();
                evidence["reason"] = json!("capture_not_observed");
            }
        }
        let saved = {
            let mut core = self.core()?;
            core.commands.guard.finish(&id, &disposition, &evidence)?;
            core.commands
                .guard
                .outcome(&id)
                .cloned()
                .ok_or(Error::Invalid)?
        };
        self.send_gateway(&saved_frame(&saved)?)
    }

    fn bind(&self, key: &str, facts: bool, after: u64) -> Result<Option<String>, Error> {
        let deadline = self.clock.elapsed() + BIND_WAIT;
        let mut core = self.core()?;
        loop {
            if let Some(seen) = core.flights.latest(key)
                && (facts || seen.sequence > after)
            {
                return Ok(Some(seen.checkpoint.clone()));
            }
            if core.closed || self.clock.elapsed() >= deadline {
                return Ok(None);
            }
            core = self
                .changed
                .wait_timeout(core, Duration::from_millis(20))
                .map_err(|_| Error::WorkerPanicked)?
                .0;
        }
    }
}
