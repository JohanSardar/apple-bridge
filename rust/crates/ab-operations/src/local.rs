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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<CommandPart>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub upload_references: BTreeMap<String, ab_state::BlobReference>,
    pub command: Command,
    pub principal: String,
    pub source: String,
    pub request_hash: String,
    pub state: State,
    pub admission_era: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<Disposition>,
    pub commit_seq: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_record_ids: Vec<String>,
    pub cursor: Cursor,
    pub history: Vec<ab_state::CommandEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}
#[derive(Debug)]
pub struct PreparedAction {
    pub action: Box<RawValue>,
    pub expected_state: Option<Box<RawValue>>,
    pub collection: String,
    pub serialization_key: String,
}
pub trait GatewayState: Debug + Send + Sync {
    fn admission_era(&self) -> Result<String, Error>;
    fn serialization_key(
        &self,
        _p: &Principal,
        _c: &Command,
        key: String,
    ) -> Result<String, Error> {
        Ok(key)
    }
    fn command(&self, id: &str) -> Result<CommandRecord, Error>;
    fn commands(&self) -> Result<Vec<CommandRecord>, Error>;
    fn transition(
        &self,
        from: Option<State>,
        record: CommandRecord,
    ) -> Result<CommandRecord, Error>;
    fn update_parts(&self, record: CommandRecord) -> Result<CommandRecord, Error>;
    fn head(&self) -> Result<Cursor, Error>;
    fn observed_record_id(
        &self,
        collection: &str,
        key: &str,
        cursor: &Cursor,
    ) -> Result<String, Error>;
}
#[derive(Debug, Clone)]
pub struct HelperBlob {
    pub digest: String,
    pub size: u64,
    pub handle: String,
}
#[derive(Debug, Clone)]
pub enum OutcomeEvidence {
    CaptureBound,
    Standalone,
    Artifact(HelperBlob),
    Unproven,
}
pub type ActionLease = Arc<dyn Debug + Send + Sync>;
pub type PreparedWithLease = (PreparedAction, Option<ActionLease>);
#[derive(Debug, Default)]
pub struct BorrowedUploads {
    pub references: BTreeMap<String, ab_state::BlobReference>,
    pub lease: Option<ActionLease>,
}
pub trait ActionResolver: Debug + Send + Sync {
    fn serialization_key(&self, _p: &Principal, c: &Command) -> Result<String, Error> {
        crate::command_contract::serialization_key(c)
    }

    fn borrow_uploads(&self, _p: &Principal, c: &Command) -> Result<BorrowedUploads, Error> {
        if c.op.starts_with("messages.") {
            let args: Value = serde_json::from_str(c.args.get()).map_err(|_| Code::Invalid)?;
            if args.as_object().is_some_and(|o| {
                o.iter().any(|(key, value)| {
                    key.eq_ignore_ascii_case("files")
                        && value.as_array().is_some_and(|files| !files.is_empty())
                })
            }) {
                return Err(Code::Unavailable.into());
            }
        }
        Ok(BorrowedUploads::default())
    }

    fn prepare(&self, p: &Principal, c: &Command) -> Result<PreparedWithLease, Error> {
        self.resolve(p, c).map(|plan| (plan, None))
    }

    fn collection(&self, command: &Command) -> Result<String, Error> {
        Ok(target_collection(&command.op))
    }
    fn validate(&self, command: &Command) -> Result<(), Error> {
        crate::command_contract::validate(command).map(|_| ())
    }
    fn evidence(&self, c: &Command, hash: &str, f: &IpcFrame) -> Result<OutcomeEvidence, Error> {
        let args: Value = serde_json::from_str(c.args.get()).map_err(|_| Code::Invalid)?;
        let evidence: Value = f
            .evidence
            .as_ref()
            .and_then(|v| serde_json::from_str(v.get()).ok())
            .unwrap_or(Value::Null);
        let observed = f.disposition == "locally_observed" || f.disposition == "absent_confirmed";
        if crate::outcome::playback(&c.op)
            && matches!(
                f.disposition.as_str(),
                "locally_observed" | "delivered" | "delivery_failed"
            )
            && (!observed || !crate::outcome::valid(&c.op, &args, &evidence, hash))
        {
            return Ok(OutcomeEvidence::Unproven);
        }
        if observed && !crate::outcome::valid(&c.op, &args, &evidence, hash) {
            return Ok(OutcomeEvidence::Unproven);
        }
        if observed && crate::outcome::capture(&c.op) {
            return Ok(OutcomeEvidence::Artifact(HelperBlob {
                digest: evidence["digest"].as_str().ok_or(Code::Corrupt)?.into(),
                size: evidence["size"].as_u64().ok_or(Code::Corrupt)?,
                handle: evidence["handle"].as_str().ok_or(Code::Corrupt)?.into(),
            }));
        }
        Ok(if crate::outcome::requires_capture(&c.op) {
            OutcomeEvidence::CaptureBound
        } else {
            OutcomeEvidence::Standalone
        })
    }

    fn resolve(&self, principal: &Principal, command: &Command) -> Result<PreparedAction, Error>;
    fn conflict(
        &self,
        _principal: &Principal,
        _command: &Command,
        frame: &IpcFrame,
    ) -> Result<Option<Error>, Error> {
        frame
            .error
            .as_ref()
            .map(|v| serde_json::from_str(v.get()).map_err(|_| Code::Corrupt.into()))
            .transpose()
    }
}
#[derive(Debug)]
pub struct LocalGateway {
    pub state: Arc<dyn GatewayState>,
    pub identity: Arc<dyn Identity>,
    pub codec: Arc<dyn Codec>,
    pub peer: Arc<dyn ApplePeer>,
    pub actions: Arc<dyn ActionResolver>,
    pub query_store: Option<Arc<crate::LocalQueryStore>>,
    pub query_audit: Option<Arc<dyn crate::QueryAuditSink>>,
    pub phone_notify: Option<Arc<tokio::sync::Notify>>,
    query_rates: Mutex<BTreeMap<String, (std::time::Instant, u32)>>,
    locks: Mutex<BTreeMap<String, Arc<AsyncMutex<()>>>>,
    stopping: std::sync::atomic::AtomicBool,
}
fn raw<T: Serialize>(value: &T) -> Result<Box<RawValue>, Error> {
    serde_json::value::to_raw_value(value).map_err(|_| Code::Corrupt.into())
}
fn now() -> jiff::Timestamp {
    jiff::Timestamp::now()
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
            query_store: None,
            query_audit: None,
            phone_notify: None,
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
    fn authorize(&self, p: &Principal, op: &str) -> Result<Principal, Error> {
        let current = self.identity.current(p)?;
        current.authorize(&descriptor_for(op)?.scope)?;
        Ok(current)
    }
    fn transition(
        &self,
        mut r: CommandRecord,
        state: State,
        disposition: Option<Disposition>,
        evidence: Option<Box<RawValue>>,
        error: Option<Error>,
    ) -> Result<CommandRecord, Error> {
        let from = r.state;
        r.state = state;
        r.disposition = disposition;
        r.error = error;
        if evidence.is_some() {
            r.evidence = evidence;
        }
        self.state.transition(Some(from), r)
    }
    fn expire(&self, r: CommandRecord) -> Result<CommandRecord, Error> {
        if r.state == State::AwaitingApproval && timestamp(&r.expires_at)? <= now() {
            self.transition(r, State::Expired, Some(Disposition::Expired), None, None)
        } else {
            Ok(r)
        }
    }
    #[allow(clippy::too_many_lines)]
    pub async fn submit(
        self: &Arc<Self>,
        principal: &Principal,
        mut command: Command,
    ) -> Result<Box<RawValue>, Error> {
        let p = self.authorize(principal, &command.op)?;
        if p.is_agent() && p.level == "read" {
            return Err(Code::Unauthorized.into());
        }
        if !valid_command_id(&command.id) || command.wait_ms > 15000 {
            return Err(Code::Invalid.into());
        }
        if matches!(
            command.op.as_str(),
            "mac.lock_screen" | "mac.sleep_display" | "mac.restart" | "mac.shutdown"
        ) && (!p.is_agent() || p.level != "approve")
        {
            return Err(Code::Forbidden.into());
        }
        if command.op == "mac.clipboard_write" && p.is_agent() && p.level == "approve" {
            return Err(Code::Unsupported.into());
        }
        let descriptor = descriptor_for(&command.op)?;
        if descriptor.kind != ab_catalog::OperationKind::Command {
            return Err(Code::Unsupported.into());
        }
        self.actions.validate(&command)?;
        command.origin = if p.kind == "device"
            || p.kind == "delegated" && p.id.contains("/device:")
            || p.is_agent() && matches!(p.level.as_str(), "full" | "approve")
        {
            "interactive".into()
        } else if command.op == "messages.send"
            && p.kind == "token"
            && !p.remote
            && p.id.starts_with("token:")
        {
            "owner_self".into()
        } else {
            String::new()
        };
        let hash = self.codec.command_hash(&command)?;
        let lock = self.lock(format!("command:{}", command.id))?;
        let guard = lock.lock().await;
        match self.state.command(&command.id) {
            Ok(old) => {
                if old.principal != p.id || old.request_hash != hash {
                    return Err(Code::IdReused.into());
                }
                let old = self.expire(old)?;
                if let Some(error) = old.error {
                    return Err(error);
                }
                return raw(&old);
            }
            Err(error) if error.code == Code::NotFound => {}
            Err(error) => return Err(error),
        }
        let era = self.state.admission_era()?;
        if !command.admission_era.is_empty() && command.admission_era != era {
            return Err(Code::Expired.into());
        }
        let created = now();
        let requested = if command.expires_at == "0001-01-01T00:00:00Z" {
            None
        } else {
            Some(timestamp(&command.expires_at)?)
        };
        let expiry = crate::command_contract::expiry(&command.op, created, requested)?;
        let approval = p.is_agent() && (p.level == "approve" || command.op == "focus.set");
        if !(approval
            || self.peer.available()
            || command.op.starts_with("iphone.") && self.phone_notify.is_some())
        {
            return Err(Code::Unavailable.into());
        }
        let wait = command.wait_ms;
        let id = command.id.clone();
        let borrowed = if let Some(store) = &self.query_store {
            BorrowedUploads {
                references: store.message_uploads(&p.id, &command).await?,
                lease: None,
            }
        } else {
            self.actions.borrow_uploads(&p, &command)?
        };
        let _upload_lease = borrowed.lease;
        let r = CommandRecord {
            parts: Vec::new(),
            upload_references: borrowed.references,
            command,
            principal: p.id.clone(),
            source: descriptor
                .scope
                .split(':')
                .next()
                .ok_or(Code::Corrupt)?
                .to_owned(),
            request_hash: hash,
            state: if approval {
                State::AwaitingApproval
            } else {
                State::Admitted
            },
            admission_era: era,
            created_at: created.to_string(),
            expires_at: if approval {
                created
                    .checked_add(jiff::SignedDuration::from_hours(24))
                    .map_err(|_| Code::Invalid)?
                    .to_string()
            } else {
                expiry.to_string()
            },
            evidence: None,
            disposition: approval.then_some(Disposition::AwaitingApproval),
            commit_seq: 0,
            observed_record_ids: Vec::new(),
            cursor: Cursor::default(),
            history: Vec::new(),
            error: None,
        };
        let r = self.state.transition(None, r)?;
        drop(guard);
        if approval {
            return raw(&r);
        }
        let gateway = self.clone();
        let mut task = tokio::spawn(async move { gateway.execute(r, p).await });
        match tokio::time::timeout(Duration::from_millis(wait), &mut task).await {
            Ok(result) => {
                let record = result.map_err(|_| Code::Unavailable)??;
                if let Some(error) = record.error {
                    return Err(error);
                }
                raw(&record)
            }
            Err(_) => raw(&self.state.command(&id)?),
        }
    }
    #[allow(clippy::too_many_lines)]
    async fn execute(&self, mut r: CommandRecord, p: Principal) -> Result<CommandRecord, Error> {
        if r.source == "iphone"
            && let Some(notify) = &self.phone_notify
        {
            notify.notify_one();
            return Ok(r);
        }
        let remaining = timestamp(&r.expires_at)?.duration_since(now());
        if remaining.is_negative() {
            return self.transition(r, State::Expired, Some(Disposition::Expired), None, None);
        }
        let deadline = tokio::time::Instant::now()
            + Duration::try_from(remaining).map_err(|_| Code::Expired)?;
        let key = match self.actions.serialization_key(&p, &r.command) {
            Ok(plan) => plan,
            Err(error) => {
                return self.transition(
                    r,
                    State::Rejected,
                    Some(Disposition::RejectedBeforeDispatch),
                    None,
                    Some(error),
                );
            }
        };
        let key = self.state.serialization_key(&p, &r.command, key)?;
        let lock = self.lock(format!("object:{key}"))?;
        let Ok(_guard) = tokio::time::timeout_at(deadline, lock.lock()).await else {
            return self.transition(r, State::Expired, Some(Disposition::Expired), None, None);
        };
        let (mut plan, _lease) = match self.actions.prepare(&p, &r.command) {
            Ok(plan) => plan,
            Err(error) => {
                return self.transition(
                    r,
                    State::Rejected,
                    Some(Disposition::RejectedBeforeDispatch),
                    None,
                    Some(error),
                );
            }
        };
        let _files = if let Some(store) = &self.query_store {
            match store.message_files(&r, &mut plan).await {
                Ok(files) => files,
                Err(error) => {
                    return self.transition(
                        r,
                        State::Rejected,
                        Some(Disposition::RejectedBeforeDispatch),
                        None,
                        Some(error),
                    );
                }
            }
        } else {
            None
        };
        plan.action = match crate::command_contract::prepared(&r.command, &plan.action) {
            Ok(action) => action,
            Err(error) => {
                return self.transition(
                    r,
                    State::Rejected,
                    Some(Disposition::RejectedBeforeDispatch),
                    None,
                    Some(error),
                );
            }
        };
        let prepared = tokio::time::timeout_at(
            deadline,
            self.peer.exchange(IpcFrame {
                t: "Prepare".into(),
                origin: r.command.origin.clone(),
                command_id: r.command.id.clone(),
                principal: p.id.clone(),
                request_hash: r.request_hash.clone(),
                action: Some(plan.action),
                expected_state: plan.expected_state,
                expires_at: GoTime::parse(&r.expires_at).map_err(|_| Code::Invalid)?,
                ..IpcFrame::default()
            }),
        )
        .await;
        let Ok(Ok(prepared)) = prepared else {
            if self.stopping() {
                return Ok(r);
            }
            if tokio::time::Instant::now() >= deadline {
                return self.transition(r, State::Expired, Some(Disposition::Expired), None, None);
            }
            return self.transition(
                r,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                None,
                Some(Code::Unavailable.into()),
            );
        };
        if self.stopping() {
            return Ok(r);
        }
        if prepared.t == "Rejected" {
            let error = self.actions.conflict(&p, &r.command, &prepared)?;
            return self.transition(
                r,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                prepared.evidence,
                error,
            );
        }
        if prepared.t != "Prepared" || prepared.ticket.is_empty() {
            return self.transition(
                r,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                None,
                Some(Code::Corrupt.into()),
            );
        }
        r = self.transition(r, State::Prepared, None, Some(raw(&prepared)?), None)?;
        if tokio::time::Instant::now() >= deadline {
            return self.transition(r, State::Expired, Some(Disposition::Expired), None, None);
        }
        if let Err(error) = self.authorize(&p, &r.command.op) {
            return self.transition(
                r,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                None,
                Some(error),
            );
        }
        r = self.transition(r, State::Dispatched, None, None, None)?;
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            self.peer.exchange(IpcFrame {
                t: "Dispatch".into(),
                origin: r.command.origin.clone(),
                command_id: r.command.id.clone(),
                ticket: prepared.ticket,
                ..IpcFrame::default()
            }),
        )
        .await;
        let Ok(Ok(frame)) = result else {
            return self.transition(
                r,
                State::Indeterminate,
                Some(Disposition::Indeterminate),
                None,
                None,
            );
        };
        let frame = self.await_capture(&r, frame).await;
        self.apply_outcome(&p, r, frame, &plan.collection).await
    }
    async fn apply_outcome(
        &self,
        p: &Principal,
        r: CommandRecord,
        mut frame: IpcFrame,
        collection: &str,
    ) -> Result<CommandRecord, Error> {
        if frame.t == "Rejected" {
            frame.error = self
                .actions
                .conflict(p, &r.command, &frame)?
                .map(|error| raw(&error))
                .transpose()?;
        }
        let mut policy = self
            .actions
            .evidence(&r.command, &r.request_hash, &frame)
            .unwrap_or(OutcomeEvidence::Unproven);
        if let OutcomeEvidence::Artifact(blob) = &policy {
            let mut evidence: Value = frame
                .evidence
                .as_ref()
                .and_then(|v| serde_json::from_str(v.get()).ok())
                .unwrap_or(Value::Null);
            let id = format!("qry_{}", ulid::Ulid::new());
            let mut blobs =
                vec![json!({"digest":blob.digest,"size":blob.size,"handle":blob.handle})];
            let descriptor = descriptor_for(&r.command.op)?;
            let result = async {
                self.query_store
                    .as_ref()
                    .ok_or(Code::Unavailable)?
                    .hydrate(
                        self.peer.as_ref(),
                        &crate::QueryBlobContext {
                            principal: p,
                            id: &id,
                            scope: &descriptor.scope,
                            max_size: 128 << 20,
                            session: crate::peer::PeerSession(frame.owner_fence),
                        },
                        &mut blobs,
                    )
                    .await
            };
            if matches!(
                tokio::time::timeout(Duration::from_secs(75), result).await,
                Ok(Ok(()))
            ) {
                let object = evidence.as_object_mut().ok_or(Code::Corrupt)?;
                object.remove("handle");
                object.insert("blob_id".into(), Value::String(id));
                frame.evidence = Some(raw(&evidence)?);
                policy = OutcomeEvidence::Standalone;
            } else {
                policy = OutcomeEvidence::Unproven;
            }
        }
        self.outcome(r, frame, collection, &policy)
    }
    fn bound(
        &self,
        r: &CommandRecord,
        frame: &IpcFrame,
        collection: &str,
    ) -> Option<(Cursor, String)> {
        let evidence: Value = serde_json::from_str(frame.evidence.as_ref()?.get()).ok()?;
        if evidence["fingerprint"] != r.request_hash
            || evidence["observed"] != true
            || evidence["ambiguous"] == true
            || evidence["candidate_count"].as_u64().unwrap_or(0) > 1
        {
            return None;
        }
        let key = evidence["native_key"].as_str()?.to_string();
        if key.is_empty() {
            return None;
        }
        let cursor = self
            .peer
            .captured(
                &r.source,
                evidence["capture_checkpoint"].as_str()?,
                collection,
                &key,
            )
            .ok()?;
        if cursor.sequence == 0 || cursor.epoch != self.state.head().ok()?.epoch {
            return None;
        }
        Some((cursor, key))
    }
    async fn await_capture(&self, r: &CommandRecord, mut frame: IpcFrame) -> IpcFrame {
        if r.command.op != "messages.send" {
            return frame;
        }
        let mut changes = self.peer.capture_revision();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while matches!(frame.disposition.as_str(), "locally_observed" | "delivered")
            && self.bound(r, &frame, "messages.message").is_none()
        {
            let evidence: Value = frame
                .evidence
                .as_ref()
                .and_then(|v| serde_json::from_str(v.get()).ok())
                .unwrap_or(Value::Null);
            if evidence["fingerprint"] != r.request_hash
                || evidence["observed"] != true
                || evidence["ambiguous"] == true
                || evidence["candidate_count"].as_u64().unwrap_or(0) > 1
            {
                break;
            }
            if !matches!(
                tokio::time::timeout_at(deadline, changes.changed()).await,
                Ok(Ok(()))
            ) {
                break;
            }
            match tokio::time::timeout_at(
                deadline,
                self.peer.exchange(IpcFrame {
                    t: "Resolve".into(),
                    command_id: r.command.id.clone(),
                    ..IpcFrame::default()
                }),
            )
            .await
            {
                Ok(Ok(next)) => frame = next,
                _ => break,
            }
        }
        frame
    }
    #[allow(clippy::too_many_lines)]
    fn outcome(
        &self,
        mut r: CommandRecord,
        mut frame: IpcFrame,
        collection: &str,
        policy: &OutcomeEvidence,
    ) -> Result<CommandRecord, Error> {
        let prior = r.clone();
        let (mut state, mut disposition) = match frame.disposition.as_str() {
            "locally_observed" => (State::Observed, Disposition::LocallyObserved),
            "absent_confirmed" => (State::Observed, Disposition::AbsentConfirmed),
            "delivered" => (State::Delivered, Disposition::Delivered),
            "delivery_failed" => (State::DeliveryFailed, Disposition::DeliveryFailed),
            "failed_no_effect" => (State::Failed, Disposition::FailedNoEffect),
            "rejected_before_dispatch" => (State::Rejected, Disposition::RejectedBeforeDispatch),
            _ => (State::Indeterminate, Disposition::Indeterminate),
        };
        if frame.t == "Rejected" {
            state = State::Rejected;
            disposition = Disposition::RejectedBeforeDispatch;
        }
        if matches!(policy, OutcomeEvidence::Unproven)
            && matches!(
                state,
                State::Observed | State::Delivered | State::DeliveryFailed
            )
        {
            state = State::Indeterminate;
            disposition = Disposition::Indeterminate;
        }
        if matches!(policy, OutcomeEvidence::CaptureBound)
            && matches!(
                state,
                State::Observed | State::Delivered | State::DeliveryFailed
            )
        {
            if let Some((cursor, key)) = self.bound(&r, &frame, collection) {
                r.commit_seq = cursor.sequence;
                match self.state.observed_record_id(collection, &key, &cursor) {
                    Ok(id) => r.observed_record_ids = vec![id],
                    Err(error) if error.code == Code::NotFound => {}
                    Err(error) => {
                        let mut evidence: Value = serde_json::from_str(
                            frame.evidence.as_ref().ok_or(Code::Corrupt)?.get(),
                        )
                        .map_err(|_| Code::Corrupt)?;
                        evidence["record_id_resolution_error"] =
                            serde_json::to_value(error.code).map_err(|_| Code::Corrupt)?;
                        frame.evidence = Some(raw(&evidence)?);
                    }
                }
            } else if state != State::DeliveryFailed {
                state = State::Indeterminate;
                disposition = Disposition::Indeterminate;
            }
        }
        if self.message_parts(&mut r, &mut frame)? {
            let failed = r.parts.iter().any(|p| {
                matches!(
                    p.disposition.as_str(),
                    "delivery_failed" | "failed_no_effect"
                )
            });
            let delivered = r.parts.iter().all(|p| p.disposition == "delivered");
            let observed = r
                .parts
                .iter()
                .all(|p| matches!(p.disposition.as_str(), "observed" | "delivered"));
            (state, disposition) = if failed {
                (State::DeliveryFailed, Disposition::DeliveryFailed)
            } else if delivered {
                (State::Delivered, Disposition::Delivered)
            } else if observed {
                (State::Observed, Disposition::LocallyObserved)
            } else {
                (State::Indeterminate, Disposition::Indeterminate)
            };
            r.commit_seq = r.commit_seq.max(prior.commit_seq);
            if r.state == state {
                if prior.parts == r.parts {
                    return Ok(prior);
                }
                r.disposition = Some(disposition);
                r.evidence = frame.evidence;
                return self.state.update_parts(r);
            }
        }
        if r.state == State::Observed && matches!(state, State::Observed | State::Indeterminate) {
            return Ok(r);
        }
        if r.state == state && matches!(state, State::Observed | State::Indeterminate) {
            return Ok(r);
        }
        if r.state == State::Dispatched && state == State::Delivered {
            r = self.transition(
                r,
                State::Observed,
                Some(Disposition::LocallyObserved),
                frame.evidence.clone(),
                None,
            )?;
        }
        let error = frame
            .error
            .map(|v| serde_json::from_str(v.get()).map_err(|_| Code::Corrupt))
            .transpose()?;
        self.transition(r, state, Some(disposition), frame.evidence, error)
    }
    pub async fn status(
        &self,
        p: &Principal,
        query: &CommandQuery,
    ) -> Result<Box<RawValue>, Error> {
        let r = self.state.command(&query.id)?;
        if r.principal != p.id
            || !p.visibility().get(&r.source).copied().unwrap_or(false)
                && !p.visibility().contains_key("admin")
        {
            return Err(Code::NotFound.into());
        }
        let mut r = self.expire(r)?;
        if r.source == "iphone" {
            return raw(&r);
        }
        if query.resolve && r.admission_era == self.state.admission_era()? && resolvable(&r) {
            let lock = self.lock(format!("command:{}", query.id))?;
            let _guard = lock.lock().await;
            r = self.state.command(&query.id)?;
            if resolvable(&r) {
                let collection = self.actions.collection(&r.command)?;
                let frame = self
                    .peer
                    .exchange(IpcFrame {
                        t: "Resolve".into(),
                        command_id: query.id.clone(),
                        ..IpcFrame::default()
                    })
                    .await?;
                r = self.apply_outcome(p, r, frame, &collection).await?;
            }
        }
        raw(&r)
    }
    pub fn list(&self, p: &Principal, limit: u64) -> Result<Box<RawValue>, Error> {
        let mut rows = Vec::new();
        for r in self.state.commands()? {
            if r.principal == p.id
                && (p.visibility().contains_key("admin")
                    || p.visibility().get(&r.source) == Some(&true))
            {
                rows.push(self.expire(r)?);
            }
        }
        if limit > 0 && rows.len() as u64 > limit {
            rows.drain(..rows.len() - usize::try_from(limit).map_err(|_| Code::Invalid)?);
        }
        raw(&rows)
    }
    pub fn approvals(&self) -> Result<Vec<CommandRecord>, Error> {
        self.state
            .commands()?
            .into_iter()
            .filter(|record| record.state == State::AwaitingApproval)
            .map(|record| self.expire(record))
            .filter(|record| {
                record
                    .as_ref()
                    .map_or(true, |record| record.state == State::AwaitingApproval)
            })
            .collect()
    }
    pub fn recover(&self) -> Result<(), Error> {
        for r in self.state.commands()? {
            if r.source == "iphone" {
                continue;
            }
            match r.state {
                State::AwaitingApproval => {
                    self.expire(r)?;
                }
                State::Dispatched => {
                    self.transition(
                        r,
                        State::Indeterminate,
                        Some(Disposition::Indeterminate),
                        None,
                        None,
                    )?;
                }
                State::Admitted | State::Prepared => {
                    if timestamp(&r.expires_at)? <= now() {
                        self.transition(r, State::Expired, Some(Disposition::Expired), None, None)?;
                    } else {
                        self.transition(
                            r,
                            State::Rejected,
                            Some(Disposition::RejectedBeforeDispatch),
                            None,
                            Some(Code::Unavailable.into()),
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub async fn decide(
        self: &Arc<Self>,
        owner: &Principal,
        id: &str,
        approve: bool,
    ) -> Result<Box<RawValue>, Error> {
        if owner.kind != "device" {
            return Err(Code::Unauthorized.into());
        }
        self.identity.current(owner)?.authorize("admin")?;
        self.decide_inner(id, approve, None).await
    }
    pub async fn decide_phone(
        self: &Arc<Self>,
        id: &str,
        hash: &str,
        approve: bool,
    ) -> Result<Box<RawValue>, Error> {
        if hash.is_empty() {
            return Err(Code::Invalid.into());
        }
        self.decide_inner(id, approve, Some(hash)).await
    }
    async fn decide_inner(
        self: &Arc<Self>,
        id: &str,
        approve: bool,
        hash: Option<&str>,
    ) -> Result<Box<RawValue>, Error> {
        let lock = self.lock(format!("command:{id}"))?;
        let _guard = lock.lock().await;
        let row = self.state.command(id)?;
        if hash.is_some_and(|hash| hash != row.request_hash) {
            return Err(Code::Unauthorized.into());
        }
        let mut r = self.expire(row)?;
        if r.state != State::AwaitingApproval {
            return raw(&r);
        }
        if !approve {
            return raw(&self.transition(
                r,
                State::RejectedByOwner,
                Some(Disposition::RejectedByOwner),
                None,
                None,
            )?);
        }
        if r.admission_era != self.state.admission_era()? {
            return raw(&self.transition(
                r,
                State::Expired,
                Some(Disposition::Expired),
                None,
                None,
            )?);
        }
        let p = match self
            .identity
            .approval_principal(&r.principal, &r.command.id, &r.request_hash)
            .and_then(|p| self.authorize(&p, &r.command.op))
        {
            Ok(p) => p,
            Err(error) if error.code == Code::Unavailable => return Err(error),
            Err(error) => {
                return raw(&self.transition(
                    r,
                    State::Rejected,
                    Some(Disposition::RejectedBeforeDispatch),
                    None,
                    Some(error),
                )?);
            }
        };
        if !(self.peer.available() || r.source == "iphone" && self.phone_notify.is_some()) {
            return Err(Code::Unavailable.into());
        }
        let ttl = if r.command.expires_at == "0001-01-01T00:00:00Z" {
            if r.source == "messages" { 60 } else { 300 }
        } else {
            timestamp(&r.command.expires_at)?
                .duration_since(timestamp(&r.created_at)?)
                .as_secs()
        };
        r.expires_at = now()
            .checked_add(jiff::SignedDuration::from_secs(ttl))
            .map_err(|_| Code::Invalid)?
            .to_string();
        r = self.transition(r, State::Admitted, None, None, None)?;
        let result = raw(&r)?;
        let gateway = self.clone();
        tokio::spawn(async move {
            let _ = gateway.execute(r, p).await;
        });
        Ok(result)
    }
    pub async fn query(&self, p: &Principal, mut q: NativeQuery) -> Result<Box<RawValue>, Error> {
        let descriptor = descriptor_for(&q.op)?;
        if descriptor.kind != ab_catalog::OperationKind::Query {
            return Err(Code::Unsupported.into());
        }
        self.authorize(p, &q.op).map_err(|error| {
            if error.code == Code::Unauthorized {
                Code::Forbidden.into()
            } else {
                error
            }
        })?;
        {
            let mut rates = self.query_rates.lock().map_err(|_| Code::Unavailable)?;
            rates.retain(|_, (at, _)| at.elapsed() <= Duration::from_secs(1));
            let (_, count) = rates
                .entry(p.id.clone())
                .or_insert((std::time::Instant::now(), 0));
            *count += 1;
            if *count > 100 {
                return Err(Code::Capacity.into());
            }
        }
        let args: Value = serde_json::from_str(q.args.get()).map_err(|_| Code::Invalid)?;
        if matches!(
            q.op.as_str(),
            "helper.version"
                | "mac.clipboard_read"
                | "mac.network_check"
                | "mac.bluetooth_devices"
                | "mac.time_machine_status"
        ) && !args.as_object().is_some_and(serde_json::Map::is_empty)
        {
            return Err(Code::Invalid.into());
        }
        if q.op == "mac.recognize_song" {
            use base64::Engine;
            if !args
                .as_object()
                .is_some_and(|a| a.keys().all(|k| k == "audio_blob"))
            {
                return Err(Code::Invalid.into());
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(
                    args["audio_blob"]
                        .as_str()
                        .unwrap_or_default()
                        .replace(['\r', '\n'], ""),
                )
                .map_err(|_| Code::Invalid)?;
            if data.is_empty() || data.len() > 8 << 20 {
                return Err(Code::Invalid.into());
            }
        }
        if q.id.is_empty() {
            q.id = format!("qry_{}", ulid::Ulid::new());
        }
        if !valid_command_id(&q.id)
            || !q.id.starts_with("qry_")
            || q.id.len() <= 4
            || q.deadline_ms > 60000
        {
            return Err(Code::Invalid.into());
        }
        if q.deadline_ms == 0 {
            q.deadline_ms = 1000;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(q.deadline_ms);
        let mut audit = crate::query_audit::AuditGuard::new(
            self.query_audit.clone(),
            p,
            &q.id,
            &q.op,
            q.args.get().len(),
            deadline,
        );
        let result = self
            .route_query(p, q, &descriptor.scope, deadline, &mut audit.stats)
            .await;
        let recorded = audit.finish();
        match (result, recorded) {
            (Ok(_), Err(error)) => Err(error),
            (result, _) => result,
        }
    }
    async fn route_query(
        &self,
        p: &Principal,
        mut q: NativeQuery,
        scope: &str,
        deadline: tokio::time::Instant,
        audit: &mut crate::QueryAudit,
    ) -> Result<Box<RawValue>, Error> {
        let args: Value = serde_json::from_str(q.args.get()).map_err(|_| Code::Invalid)?;
        let document = if matches!(q.op.as_str(), "vision.ocr" | "pdf.text" | "pdf.search") {
            audit.argument_bytes = 0;
            let store = self.query_store.as_ref().ok_or(Code::Unavailable)?;
            Some(
                tokio::time::timeout_at(deadline, store.document_input(p, &q.args))
                    .await
                    .map_err(|_| Code::Deadline)??,
            )
        } else {
            None
        };
        if let Some(document) = &document {
            q.args = document.args.clone();
            audit.argument_bytes = q.args.get().len();
        }
        if q.op == "mail.body" {
            q.args = self
                .query_store
                .as_ref()
                .ok_or(Code::Unavailable)?
                .mail_args(p, &args)?;
            audit.argument_bytes = q.args.get().len();
        }
        let frame = tokio::time::timeout_at(
            deadline,
            self.peer.exchange(IpcFrame {
                t: "Query".into(),
                id: q.id.clone(),
                op: q.op,
                args: Some(q.args),
                deadline_ms: i64::try_from(q.deadline_ms).map_err(|_| Code::Invalid)?,
                ..IpcFrame::default()
            }),
        )
        .await
        .map_err(|_| Code::Deadline)??;
        if frame.t == "QueryError" {
            return Err(serde_json::from_value(
                json!({"code": frame.code, "message": "query failed", "retryable": false}),
            )
            .map_err(|_| Code::Corrupt)?);
        }
        if frame.t != "QueryResult"
            || frame.result_json.is_none()
            || frame
                .result_json
                .as_ref()
                .is_some_and(|v| v.get().len() > 1 << 20)
            || frame.blobs.as_ref().is_some_and(|v| v.len() > 16)
        {
            return Err(Code::Corrupt.into());
        }
        audit.result_bytes = frame.result_json.as_ref().map_or(0, |v| v.get().len());
        audit.blob_bytes = frame
            .blobs
            .as_ref()
            .into_iter()
            .flatten()
            .filter_map(|b| serde_json::from_str::<Value>(b.get()).ok())
            .filter_map(|b| b["size"].as_i64())
            .sum();
        let mut blobs: Vec<Value> = frame
            .blobs
            .unwrap_or_default()
            .into_iter()
            .map(|b| serde_json::from_str(b.get()).map_err(|_| Code::Corrupt))
            .collect::<Result<_, _>>()?;
        if !blobs.is_empty() {
            let store = self.query_store.as_ref().ok_or(Code::Corrupt)?;
            tokio::time::timeout_at(
                deadline,
                store.hydrate(
                    self.peer.as_ref(),
                    &crate::QueryBlobContext {
                        principal: p,
                        id: &q.id,
                        scope,
                        max_size: 64 << 20,
                        session: crate::peer::PeerSession(frame.owner_fence),
                    },
                    &mut blobs,
                ),
            )
            .await
            .map_err(|_| Code::Deadline)??;
        }
        raw(&json!({"id": q.id, "result_json": frame.result_json, "blobs": blobs}))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPart {
    pub index: usize,
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub upload_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub record_id: String,
    pub disposition: String,
}
fn target_collection(op: &str) -> String {
    match op {
        "reminders.create_list" | "reminders.rename_list" | "reminders.delete_list" => {
            "reminders.list"
        }
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
impl LocalGateway {
    #[allow(clippy::too_many_lines)]
    fn message_parts(&self, r: &mut CommandRecord, frame: &mut IpcFrame) -> Result<bool, Error> {
        if r.command.op != "messages.send" {
            return Ok(false);
        }
        let Some(raw_proof) = &frame.evidence else {
            return Ok(false);
        };
        let mut proof: Value = serde_json::from_str(raw_proof.get()).map_err(|_| Code::Corrupt)?;
        if proof["fingerprint"] != r.request_hash
            || proof["parts"].as_array().is_none_or(Vec::is_empty) && r.parts.is_empty()
        {
            return Ok(false);
        }
        let args: Value = serde_json::from_str(r.command.args.get()).map_err(|_| Code::Corrupt)?;
        let mut parts = Vec::new();
        if args["text"].as_str().is_some_and(|s| !s.is_empty()) {
            parts.push(CommandPart {
                index: 0,
                kind: "text".into(),
                upload_id: String::new(),
                record_id: String::new(),
                disposition: "indeterminate".into(),
            });
        }
        for file in args["files"].as_array().into_iter().flatten() {
            parts.push(CommandPart {
                index: parts.len(),
                kind: "attachment".into(),
                upload_id: file["upload_id"].as_str().unwrap_or_default().into(),
                record_id: String::new(),
                disposition: "indeterminate".into(),
            });
        }
        if !r.parts.is_empty() {
            if r.parts.len() != parts.len() {
                return Ok(false);
            }
            parts.clone_from(&r.parts);
        }
        let previous: Value = r
            .evidence
            .as_ref()
            .and_then(|v| serde_json::from_str(v.get()).ok())
            .unwrap_or(Value::Null);
        let mut proofs = BTreeMap::new();
        for p in previous["parts"].as_array().into_iter().flatten() {
            if let Some(index) = p["index"].as_u64() {
                proofs.insert(index, p.clone());
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut problems = serde_json::Map::new();
        let mut commit = r.commit_seq;
        for p in proof["parts"].as_array().into_iter().flatten() {
            let Some(index) = p["index"].as_u64().and_then(|i| usize::try_from(i).ok()) else {
                return Ok(false);
            };
            if index >= parts.len() || !seen.insert(index) || p["kind"] != parts[index].kind {
                return Ok(false);
            }
            let prior = parts[index].clone();
            let mut next = prior.clone();
            next.record_id.clear();
            next.disposition = match p["disposition"].as_str().unwrap_or_default() {
                "delivery_failed" => "delivery_failed",
                "failed_no_effect" | "not_dispatched" | "rejected_before_dispatch" => {
                    "failed_no_effect"
                }
                _ => "indeterminate",
            }
            .into();
            let mut bound = false;
            let mut candidate = p.clone();
            candidate["fingerprint"] = proof["fingerprint"].clone();
            candidate["capture_checkpoint"] = proof["capture_checkpoint"].clone();
            if proof["ambiguous"] != true
                && proof["candidate_count"].as_u64().unwrap_or(0) <= 1
                && matches!(
                    p["disposition"].as_str(),
                    Some("locally_observed" | "delivered" | "delivery_failed")
                )
            {
                let part_frame = IpcFrame {
                    evidence: Some(raw(&candidate)?),
                    ..IpcFrame::default()
                };
                if let Some((cursor, key)) = self.bound(r, &part_frame, "messages.message") {
                    bound = true;
                    next.disposition = if p["disposition"] == "locally_observed" {
                        "observed".into()
                    } else {
                        p["disposition"].as_str().ok_or(Code::Corrupt)?.into()
                    };
                    match self
                        .state
                        .observed_record_id("messages.message", &key, &cursor)
                    {
                        Ok(id) => next.record_id = id,
                        Err(error) if error.code == Code::NotFound => {}
                        Err(error) => {
                            problems.insert(
                                index.to_string(),
                                serde_json::to_value(error.code).map_err(|_| Code::Corrupt)?,
                            );
                        }
                    }
                    commit = commit.max(cursor.sequence);
                }
            }
            let prior_proof = proofs.get(&(index as u64));
            let same_key = prior_proof.is_some_and(|old| {
                old["native_key"].as_str().is_some_and(|v| !v.is_empty())
                    && old["native_key"] == p["native_key"]
            });
            let bound_failure = prior.disposition == "observed"
                && next.disposition == "delivery_failed"
                && bound
                && same_key
                && (prior.record_id.is_empty() || prior.record_id == next.record_id);
            if prior.disposition != "indeterminate"
                && prior.disposition != next.disposition
                && !(prior.disposition == "observed" && next.disposition == "delivered")
                && !bound_failure
            {
                continue;
            }
            if prior.disposition != "indeterminate"
                && prior_proof.is_some_and(|old| {
                    old["native_key"].as_str().is_some_and(|v| !v.is_empty())
                        && p["native_key"].as_str().is_some_and(|v| !v.is_empty())
                        && old["native_key"] != p["native_key"]
                })
            {
                continue;
            }
            if !prior.record_id.is_empty() {
                if !next.record_id.is_empty() && next.record_id != prior.record_id {
                    continue;
                }
                next.record_id = prior.record_id;
            }
            if next == parts[index] && prior_proof.is_some() {
                continue;
            }
            parts[index] = next;
            proofs.insert(index as u64, p.clone());
        }
        proof["parts"] = Value::Array(proofs.into_values().collect());
        if !problems.is_empty() {
            proof["part_record_id_resolution_error"] = Value::Object(problems);
        }
        frame.evidence = Some(raw(&proof)?);
        r.parts = parts;
        r.commit_seq = commit;
        Ok(true)
    }
}
fn resolvable(r: &CommandRecord) -> bool {
    r.state == State::Indeterminate
        || r.source == "messages"
            && (r.state == State::Observed
                || r.state == State::DeliveryFailed
                    && r.parts.iter().any(|p| {
                        p.disposition == "indeterminate"
                            || p.disposition == "observed"
                            || p.disposition == "delivery_failed" && p.record_id.is_empty()
                    }))
}
#[cfg(test)]
#[path = "local_races.rs"]
mod races;
