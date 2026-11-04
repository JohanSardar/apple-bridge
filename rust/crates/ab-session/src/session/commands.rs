use super::{
    Hub,
    owned::{self, Probe, Refusal},
    rejected,
};
use crate::{
    Error,
    guard::{DispatchGuard, PreparedEffect, durable_action},
};
use ab_wire::{
    messages::IpcFrame,
    native::{
        BlobRead, Command, Dispatch, Frame as Link, Outcome, Prepared, Primitive, Query,
        QueryCancel, Rejected, Resolve,
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io::Read,
    path::Path,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TICKET_SECONDS: i64 = 30;
const BIND_WAIT: Duration = Duration::from_millis(500);
const READ_WAIT: Duration = Duration::from_secs(10);
const WRITE_WAIT: Duration = Duration::from_secs(30);
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
    resolving: BTreeMap<String, u32>,
    queries: BTreeSet<String>,
    owned: BTreeSet<String>,
    primitives: BTreeMap<String, Option<Result<Value, String>>>,
    abandoned: BTreeSet<String>,
    next_primitive: u64,
}

impl Commands {
    pub(super) fn open(home: &Path, fence: u64, owned: &[String]) -> Result<Self, Error> {
        Ok(Self {
            guard: DispatchGuard::open(home)?,
            fence,
            preparing: None,
            tickets: BTreeMap::new(),
            active: None,
            resolving: BTreeMap::new(),
            queries: BTreeSet::new(),
            owned: owned.iter().cloned().collect(),
            primitives: BTreeMap::new(),
            abandoned: BTreeSet::new(),
            next_primitive: 0,
        })
    }

    fn owns(&self, operation: &str) -> bool {
        self.owned.contains(source_of(operation)) || self.owned.contains(operation)
    }

    fn take_resolving(&mut self, id: &str) -> bool {
        match self.resolving.get_mut(id) {
            Some(count) if *count > 1 => {
                *count -= 1;
                true
            }
            Some(_) => self.resolving.remove(id).is_some(),
            None => false,
        }
    }

    pub(super) fn unresolved(&self) -> Vec<Value> {
        self.guard.unresolved()
    }
}

fn ticket_lifetime(expires_at_millis: i64) -> Duration {
    let ticket = TICKET_SECONDS * 1000;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        });
    let remaining = expires_at_millis.saturating_sub(now);
    let millis = if remaining > -ticket && remaining < ticket {
        remaining.max(0)
    } else {
        ticket
    };
    Duration::from_millis(u64::try_from(millis).unwrap_or(0))
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

fn part_keys(evidence: &Value) -> Vec<String> {
    evidence["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["native_key"].as_str().map(str::to_owned))
        .filter(|key| !key.is_empty())
        .collect()
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

fn source_of(operation: &str) -> &str {
    operation.split('.').next().unwrap_or_default()
}

impl Probe for Hub {
    fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
        let wait = if matches!(
            operation,
            "reminders.write"
                | "calendar.write"
                | "contacts.write"
                | "contacts.write_group"
                | "notes.write"
        ) {
            WRITE_WAIT
        } else {
            READ_WAIT
        };
        self.primitive(operation, args, wait)
    }
}

impl Hub {
    pub(super) fn gateway_command<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        frame: &IpcFrame,
    ) -> Result<(), Error> {
        match frame.t.as_str() {
            "Prepare" => self.prepare(scope, frame),
            "Dispatch" => self.dispatch(scope, frame),
            "Resolve" => self.resolve(scope, frame),
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

    fn prepare<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        frame: &IpcFrame,
    ) -> Result<(), Error> {
        let id = frame.command_id.clone();
        let Ok(command) = Command::from_ipc(frame) else {
            return self.refuse(&id, "invalid");
        };
        if command.command_id.is_empty()
            || command.principal.is_empty()
            || command.request_hash.is_empty()
            || command.expires_unix().is_err()
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
        let owned = commands.owns(&command.operation());
        drop(core);
        if owned {
            scope.spawn(move || {
                if let Err(error) = self.prepare_owned(&command) {
                    eprintln!("ab-session: prepare failed: {error}");
                    self.close();
                }
            });
            return Ok(());
        }
        self.send_link(&Link::Prepare(command))
    }

    fn prepare_owned(&self, command: &Command) -> Result<(), Error> {
        let action = parsed(&command.action);
        let expected = command
            .expected_state
            .as_deref()
            .map_or(Value::Null, parsed);
        let Some(driver) = owned::driver(source_of(&command.operation())) else {
            return Err(Error::Invalid);
        };
        let command_value = json!({
            "command_id": command.command_id,
            "principal": command.principal,
            "origin": command.origin,
            "expires_at": command.expires_at,
            "action": action,
            "expected_state": expected,
        });
        let decided = match driver.prepare_command {
            Some(prepare) => prepare(self, &command_value),
            None => (driver.prepare)(self, &action, &expected),
        };
        match decided {
            Ok(state) => self.prepared(Prepared {
                command_id: command.command_id.clone(),
                native_state: raw(&state)?,
                route: state["route"].as_str().unwrap_or_default().to_owned(),
                candidate_ids: state["candidate_ids"]
                    .as_array()
                    .map(|items| items.iter().map(raw).collect::<Result<Vec<_>, _>>())
                    .transpose()?
                    .unwrap_or_default(),
                fingerprint: command.request_hash.clone(),
            }),
            Err(refusal) => self.rejected_by_helper(self.rejection(
                source_of(&command.operation()),
                &command.command_id,
                &refusal,
            )?),
        }
    }

    fn rejection(
        &self,
        source: &str,
        command_id: &str,
        refusal: &Refusal,
    ) -> Result<Rejected, Error> {
        let evidence = refusal
            .key
            .as_deref()
            .map(|key| self.conflict_evidence(source, key));
        let (code, evidence) = match evidence {
            Some(None) => ("unavailable", None),
            Some(Some(found)) => (refusal.code.as_str(), Some(found)),
            None => (refusal.code.as_str(), None),
        };
        eprintln!("ab-session: refused {command_id} as {code} ({})", refusal.code);
        Ok(Rejected {
            command_id: command_id.into(),
            code: code.into(),
            message: refusal.message.clone().unwrap_or_else(|| code.into()),
            evidence: evidence.map(|value| raw(&value)).transpose()?,
        })
    }

    fn conflict_evidence(&self, source: &str, key: &str) -> Option<Value> {
        let reconciled = self.primitive(
            &format!("{source}.reconcile"),
            &json!({"conflict_key": key}),
            WRITE_WAIT,
        );
        let checkpoint = reconciled
            .ok()
            .and_then(|_| self.bind(key, &[], true, 0).ok().flatten())?;
        Some(json!({"native_key": key, "capture_checkpoint": checkpoint}))
    }

    pub(super) fn primitive(
        &self,
        operation: &str,
        args: &Value,
        wait: Duration,
    ) -> Result<Value, String> {
        let unavailable = || "unavailable".to_owned();
        let id = {
            let mut core = self.core().map_err(|_| unavailable())?;
            core.commands.next_primitive += 1;
            let id = format!("primitive-{}", core.commands.next_primitive);
            core.commands.primitives.insert(id.clone(), None);
            id
        };
        let frame = Link::Primitive(Primitive {
            id: id.clone(),
            op: operation.into(),
            args: Some(raw(args).map_err(|_| unavailable())?),
        });
        let deadline = self.clock.elapsed() + wait;
        let sent = self.send_link(&frame);
        let mut core = self.core().map_err(|_| unavailable())?;
        loop {
            if sent.is_ok() && matches!(core.commands.primitives.get(&id), Some(Some(_))) {
                return core
                    .commands
                    .primitives
                    .remove(&id)
                    .flatten()
                    .unwrap_or_else(|| Err(unavailable()));
            }
            if sent.is_err() || core.closed || self.clock.elapsed() >= deadline {
                if core
                    .commands
                    .primitives
                    .remove(&id)
                    .is_some_and(|slot| slot.is_none())
                {
                    core.commands.abandoned.insert(id);
                }
                return Err(unavailable());
            }
            core = self
                .changed
                .wait_timeout(core, Duration::from_millis(20))
                .map_err(|_| unavailable())?
                .0;
        }
    }

    pub(super) fn finish_primitive(
        &self,
        id: &str,
        answer: Result<Value, String>,
    ) -> Result<(), Error> {
        let mut core = self.core()?;
        if core.commands.abandoned.remove(id) {
            return Ok(());
        }
        match core.commands.primitives.get_mut(id) {
            Some(slot @ None) => *slot = Some(answer),
            _ => return Err(Error::Invalid),
        }
        drop(core);
        self.changed.notify_all();
        Ok(())
    }

    fn resolve_owned(&self, id: &str, saved: &Value) -> Result<(), Error> {
        let evidence = &saved["evidence"];
        let operation = evidence["prepared"]["action"]["op"]
            .as_str()
            .unwrap_or_default();
        let driver = owned::driver(source_of(operation)).ok_or(Error::Invalid)?;
        let observed = (driver.resolved)(self, evidence).filter(|found| {
            let args = (driver.reconcile_args)(found, operation);
            self.primitive(&format!("{}.reconcile", driver.name), &args, WRITE_WAIT)
                .is_ok()
        });
        let (disposition, evidence) = match observed {
            Some(mut found) => {
                let key = found["native_key"].as_str().unwrap_or_default().to_owned();
                match self.bind(&key, &[], true, 0)? {
                    Some(checkpoint) if !key.is_empty() => {
                        found["capture_checkpoint"] = Value::String(checkpoint);
                        let disposition = match found["disposition"].as_str() {
                            Some("delivered") if driver.name == "messages" => "delivered",
                            Some("delivery_failed") if driver.name == "messages" => {
                                "delivery_failed"
                            }
                            Some("indeterminate") if driver.name == "messages" => "indeterminate",
                            _ => "locally_observed",
                        };
                        (disposition, found)
                    }
                    _ => {
                        let mut kept = evidence.clone();
                        kept["reason"] = json!("capture_not_observed");
                        ("indeterminate", kept)
                    }
                }
            }
            None => ("indeterminate", evidence.clone()),
        };
        let outcome = Outcome {
            command_id: id.into(),
            disposition: disposition.into(),
            evidence: raw(&evidence)?,
        };
        self.outcome_from_helper(outcome)?;
        Ok(())
    }

    fn dispatch_owned(&self, command: &Command, state: &Value) -> Result<(), Error> {
        let action = parsed(&command.action);
        let operation = command.operation();
        let source = source_of(&operation);
        let driver = owned::driver(source).ok_or(Error::Invalid)?;
        if let Err(refusal) = (driver.validate)(self, &action, state) {
            return self.rejected_by_helper(self.rejection(
                source,
                &command.command_id,
                &refusal,
            )?);
        }
        let prepared = json!({
            "action": durable_action(&action),
            "native_state": state,
            "request_hash": command.request_hash,
        });
        let failed = |code: &str| {
            (
                "indeterminate",
                json!({"observed": false, "reason": "dispatch_evidence_unavailable", "native_error_code": code, "prepared": prepared}),
            )
        };
        let (disposition, evidence) =
            match (driver.write)(self, &command.command_id, &action, state) {
                Ok(written) => match (driver.settle)(&operation, &written) {
                    Some(settled) => {
                        let mut fields = settled.evidence;
                        fields["fingerprint"] = json!(command.request_hash);
                        fields["before"] = state.clone();
                        fields["candidate_ids"] = state["candidate_ids"].clone();
                        fields["prepared"] = prepared.clone();
                        (settled.disposition, fields)
                    }
                    None => failed("unavailable"),
                },
                Err(code) => failed(&code),
            };
        let outcome = Outcome {
            command_id: command.command_id.clone(),
            disposition: disposition.into(),
            evidence: raw(&evidence)?,
        };
        if let Some((active, outcome)) = self.outcome_from_helper(outcome)? {
            self.settle(&active, outcome)?;
        }
        Ok(())
    }

    fn dispatch<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        frame: &IpcFrame,
    ) -> Result<(), Error> {
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
            expires_at: u64::try_from(unix_now().saturating_add(TICKET_SECONDS))
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
        let owned = core.commands.owns(&ticket.command.operation()).then(|| {
            (
                ticket.command.clone(),
                parsed(&ticket.prepared.native_state),
            )
        });
        core.commands.active = Some(Active {
            command: ticket.command,
            prepared: ticket.prepared,
            after,
        });
        drop(core);
        if let Some((command, state)) = owned {
            scope.spawn(move || {
                if let Err(error) = self.dispatch_owned(&command, &state) {
                    eprintln!("ab-session: dispatch failed: {error}");
                    self.close();
                }
            });
            return Ok(());
        }
        self.send_link(&Link::Dispatch(message))
    }

    fn resolve<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        frame: &IpcFrame,
    ) -> Result<(), Error> {
        let id = frame.command_id.clone();
        let mut core = self.core()?;
        let Some(saved) = core.commands.guard.outcome(&id).cloned() else {
            drop(core);
            return self.refuse(&id, "not_found");
        };
        let open = saved["t"] == "Outcome" && saved["disposition"] != "failed_no_effect";
        let owned = open
            && ["indeterminate", "locally_observed"]
                .contains(&saved["disposition"].as_str().unwrap_or_default())
            && core.commands.owns(
                saved["evidence"]["prepared"]["action"]["op"]
                    .as_str()
                    .unwrap_or_default(),
            );
        if open {
            *core.commands.resolving.entry(id.clone()).or_default() += 1;
            drop(core);
            if owned {
                scope.spawn(move || {
                    if let Err(error) = self.resolve_owned(&id, &saved) {
                        eprintln!("ab-session: resolve failed: {error}");
                        self.close();
                    }
                });
                return Ok(());
            }
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
        let ticket = random_hex(16)?;
        let lifetime = ticket_lifetime(command.expires_unix_millis().map_err(|_| Error::Invalid)?);
        let fingerprint = command.request_hash.clone();
        core.commands.tickets.insert(
            ticket.clone(),
            Ticket {
                command,
                prepared: prepared.clone(),
                expiry: Instant::now() + lifetime,
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
        if core.commands.take_resolving(&id) {
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
        if core.commands.take_resolving(&id) {
            drop(core);
            let mut evidence = parsed(&outcome.evidence);
            if outcome.disposition != "indeterminate" && evidence["observed"] == true {
                let key = evidence["native_key"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                if !key.is_empty()
                    && let Some(checkpoint) = self.bind(&key, &part_keys(&evidence), true, 0)?
                {
                    evidence["capture_checkpoint"] = Value::String(checkpoint);
                }
            }
            let mut core = self.core()?;
            let saved = core.commands.guard.outcome(&id).cloned();
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
                self.bind(
                    &key,
                    &part_keys(&evidence),
                    facts_source(&operation),
                    active.after,
                )?
            };
            if let Some(checkpoint) = bound {
                evidence["capture_checkpoint"] = Value::String(checkpoint);
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

    fn bind(
        &self,
        key: &str,
        others: &[String],
        facts: bool,
        after: u64,
    ) -> Result<Option<String>, Error> {
        let deadline = self.clock.elapsed() + BIND_WAIT;
        let mut core = self.core()?;
        loop {
            if let Some(seen) = core.flights.latest(key)
                && (facts || seen.sequence > after)
            {
                let mut best = (seen.sequence, seen.checkpoint.clone());
                for other in others {
                    if let Some(other) = core.flights.latest(other)
                        && other.sequence > best.0
                    {
                        best = (other.sequence, other.checkpoint.clone());
                    }
                }
                return Ok(Some(best.1));
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
