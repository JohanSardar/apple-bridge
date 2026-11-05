use crate::{Error, Packet};
use ab_trust::{
    Authority, Principal,
    domain::{CommandState, Disposition},
    iphone::Request,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedCommand {
    pub command: Box<RawValue>,
    pub principal: String,
    pub source: String,
    pub request_hash: String,
    pub state: CommandState,
    pub created_at: String,
    pub expires_at: String,
}
impl QueuedCommand {
    fn fields(&self) -> Result<Value, Error> {
        Ok(serde_json::from_str(self.command.get())?)
    }
    fn id(&self) -> Result<String, Error> {
        string_field(&self.fields()?, "command_id")
    }
    fn target(&self) -> Result<String, Error> {
        string_field(&self.fields()?["args"], "device_id")
    }
    fn expires(&self) -> Result<i64, Error> {
        timestamp(&self.expires_at)
    }
    fn live(&self) -> Result<bool, Error> {
        Ok(is_live(self.fields()?["op"].as_str().unwrap_or_default()))
    }
}

pub trait Gateway: std::fmt::Debug + Send {
    fn commands(&mut self) -> Result<Vec<QueuedCommand>, Error>;
    fn command(&mut self, id: &str) -> Result<QueuedCommand, Error>;
    fn transition(
        &mut self,
        prior: &QueuedCommand,
        state: CommandState,
        disposition: Option<Disposition>,
        evidence: Option<&RawValue>,
    ) -> Result<QueuedCommand, Error>;
    fn authorize(&mut self, command: &QueuedCommand) -> Result<(), Error>;
    fn decide(&mut self, id: &str, hash: &str, approve: bool) -> Result<(), Error>;
    fn capture(
        &mut self,
        _row: &QueuedCommand,
        _upload: &CaptureUpload,
        _bytes: &[u8],
    ) -> Result<CaptureReceipt, Error> {
        Err(Error::Unavailable)
    }
    fn connector_pending(&mut self) -> Result<Vec<ConnectorApproval>, Error> {
        Ok(vec![])
    }
    fn connector_decide(&mut self, _id: &str, _hash: &str, _approve: bool) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
    fn ingest_health(
        &mut self,
        device: &str,
        batch: &HealthBatch,
        digest: &str,
    ) -> Result<(), Error>;
    fn ingest_locations(&mut self, device: &str, records: &[LocationRecord]) -> Result<(), Error>;
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureUpload {
    pub command_id: String,
    pub request_hash: String,
    pub content_type: String,
    pub data: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CaptureReceipt {
    pub blob_id: String,
    pub digest: String,
    pub size: u64,
    pub content_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorApproval {
    pub id: String,
    pub code: String,
    pub name: String,
    pub host: String,
    pub scopes: String,
    pub expires: i64,
}
fn connector_prompt(approval: &ConnectorApproval) -> String {
    format!(
        "{}\nCode {}\n{}",
        approval.name, approval.code, approval.scopes
    )
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Job {
    pub command: Box<RawValue>,
    pub request_hash: String,
    pub expires: i64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultReceipt {
    pub command_id: String,
    pub request_hash: String,
    pub state: CommandState,
    pub data: Box<RawValue>,
}

#[derive(Debug)]
pub struct Delivery<G: Gateway> {
    pub authority: Arc<Authority>,
    gateway: Mutex<G>,
    wake: tokio::sync::Notify,
    pub(crate) uploads: crate::http::Admission,
}
impl<G: Gateway> Delivery<G> {
    pub fn new(authority: Arc<Authority>, gateway: G) -> Self {
        Self {
            authority,
            gateway: Mutex::new(gateway),
            wake: tokio::sync::Notify::new(),
            uploads: crate::http::Admission::default(),
        }
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, G>, Error> {
        self.gateway.lock().map_err(|_| Error::Unavailable)
    }
    pub fn handle(&self, path: &str, request: &Request, now: i64) -> Result<Box<RawValue>, Error> {
        let (phone, payload) = self
            .authority
            .verify_iphone(path, request)
            .map_err(|_| Error::Authentication)?;
        let cap = if path == "/v1/iphone/capture" {
            24 << 20
        } else if path == "/v1/iphone/health" {
            16 << 20
        } else {
            32768
        };
        if payload.len() > cap {
            return Err(Error::Invalid);
        }
        match path {
            "/v1/iphone/capture" => raw(&self.capture(&phone, &payload)?),
            "/v1/iphone/pending" => {
                let mut packets = self.packets(&phone, now, true, false)?.0;
                if wants_live_state(&payload) {
                    packets.extend(self.live_state(&phone, now)?);
                }
                raw(&packets)
            }
            "/v1/iphone/result" => {
                self.complete(&phone, request, &payload)?;
                raw(&serde_json::json!({"accepted":true}))
            }
            "/v1/iphone/push-token" => {
                let token: PushToken = serde_json::from_slice(&payload)?;
                self.authority
                    .set_iphone_push_token(&phone.id, &token.token)?;
                self.wake();
                raw(&serde_json::json!({"registered":true}))
            }
            "/v1/iphone/live-token" => {
                let token: LiveToken = serde_json::from_slice(&payload)?;
                self.authority
                    .set_iphone_live_token(&phone.id, &token.task_id, &token.token)?;
                self.wake();
                raw(&serde_json::json!({"registered":true}))
            }
            "/v1/iphone/live-disable" => {
                self.authority.set_iphone_live_token(&phone.id, "", "")?;
                raw(&serde_json::json!({"disabled":true}))
            }
            "/v1/iphone/health" => {
                let batch: HealthBatch = serde_json::from_slice(&payload)?;
                batch.validate()?;
                self.lock()?
                    .ingest_health(&phone.id, &batch, &batch.digest()?)?;
                raw(&serde_json::json!({"accepted":true}))
            }
            "/v1/iphone/location-records" => {
                let records: Vec<LocationRecord> = serde_json::from_slice(&payload)?;
                validate_locations(&records)?;
                self.lock()?.ingest_locations(&phone.id, &records)?;
                raw(&serde_json::json!({"accepted":true}))
            }
            "/v1/iphone/approval" => {
                let decision: Decision = serde_json::from_slice(&payload)?;
                let mut gateway = self.lock()?;
                if decision.command_id.starts_with("connector:") {
                    gateway.connector_decide(
                        &decision.command_id,
                        &decision.request_hash,
                        decision.approve,
                    )?;
                } else {
                    gateway.decide(
                        &decision.command_id,
                        &decision.request_hash,
                        decision.approve,
                    )?;
                }
                raw(&serde_json::json!({"accepted":true}))
            }
            _ => Err(Error::NotFound),
        }
    }
    fn capture(&self, phone: &Principal, payload: &[u8]) -> Result<CaptureReceipt, Error> {
        let upload: CaptureUpload = serde_json::from_slice(payload)?;
        let bytes = STANDARD.decode(&upload.data).map_err(|_| Error::Invalid)?;
        if bytes.is_empty() || bytes.len() > 12 << 20 {
            return Err(Error::Invalid);
        }
        let mut gateway = self.lock()?;
        let row = gateway.command(&upload.command_id)?;
        let fields = row.fields()?;
        let op = fields["op"].as_str().unwrap_or_default();
        if !matches!(op, "iphone.capture.photo" | "iphone.capture.audio")
            || row.source != "iphone"
            || row.target()? != phone.id
            || row.request_hash != upload.request_hash
            || row.state != CommandState::Dispatched
            || ab_trust::decision_now()
                >= row
                    .expires_at
                    .parse::<jiff::Timestamp>()
                    .map_err(|_| Error::Invalid)?
        {
            return Err(Error::Unauthorized);
        }
        gateway.authorize(&row)?;
        let expected = if op == "iphone.capture.audio" {
            "audio/wav"
        } else {
            "image/jpeg"
        };
        if upload.content_type != expected {
            return Err(Error::Invalid);
        }
        gateway.capture(&row, &upload, &bytes)
    }
    fn complete(&self, phone: &Principal, request: &Request, payload: &[u8]) -> Result<(), Error> {
        let receipt: ResultReceipt = serde_json::from_slice(payload)?;
        let mut gateway = self.lock()?;
        let row = gateway.command(&receipt.command_id)?;
        if row.source != "iphone"
            || row.target()? != phone.id
            || row.request_hash != receipt.request_hash
        {
            return Err(Error::Unauthorized);
        }
        let fields = row.fields()?;
        let op = fields["op"].as_str().unwrap_or_default();
        if is_location(op)
            && (receipt.state == CommandState::Observed || receipt.data.get() != "null")
        {
            validate_location_summary(&receipt.data)?;
        }
        if !matches!(
            row.state,
            CommandState::Dispatched | CommandState::Indeterminate
        ) {
            return Ok(());
        }
        let disposition = match receipt.state {
            CommandState::Observed => Disposition::LocallyObserved,
            CommandState::Failed => Disposition::FailedNoEffect,
            CommandState::Indeterminate => Disposition::Indeterminate,
            CommandState::Rejected => Disposition::RejectedBeforeDispatch,
            _ => return Err(Error::Invalid),
        };
        if row.state == receipt.state {
            return Ok(());
        }
        let signing_key = phone
            .extra
            .get("iphone_signing_key")
            .and_then(Value::as_str)
            .ok_or(Error::Unauthorized)?;
        let evidence = raw(&Evidence {
            request,
            signing_key,
        })?;
        gateway.transition(&row, receipt.state, Some(disposition), Some(&evidence))?;
        Ok(())
    }
    pub fn sweep(&self, now: i64) -> Result<(), Error> {
        let mut gateway = self.lock()?;
        for row in gateway.commands()? {
            if row.source != "iphone" || !queued(row.state) {
                continue;
            }
            let expired = now >= row.expires()?;
            let active = self
                .authority
                .principal(&row.target()?)
                .is_ok_and(|p| p.kind == "iphone");
            if expired || !active {
                terminate(&mut *gateway, &row, expired)?;
            }
        }
        Ok(())
    }
    pub fn packets(
        &self,
        phone: &Principal,
        now: i64,
        dispatch: bool,
        allow_live: bool,
    ) -> Result<(Vec<Packet>, Vec<LivePush>), Error> {
        let phone = self.authority.principal(&phone.id)?;
        if phone.kind != "iphone" {
            return Err(Error::Unauthorized);
        }
        let mut gateway = self.lock()?;
        let mut rows = gateway.commands()?;
        order_live(&mut rows)?;
        let mut packets = vec![];
        let mut pushes = vec![];
        for approval in gateway.connector_pending()? {
            packets.push(self.approval(
                &phone,
                &approval.id,
                &approval.code,
                &connector_prompt(&approval),
                None,
                approval.expires,
            )?);
        }
        for mut row in rows {
            if row.state == CommandState::AwaitingApproval && now < row.expires()? {
                let fields = row.fields()?;
                let args = raw(&fields["args"])?;
                let body = format!(
                    "{}\nAgent: {}\n{}",
                    fields["op"].as_str().unwrap_or_default(),
                    row.principal,
                    args.get()
                );
                packets.push(self.approval(
                    &phone,
                    &row.id()?,
                    &row.request_hash,
                    &body,
                    Some("Apple Bridge approval"),
                    row.expires()?,
                )?);
                continue;
            }
            if row.source != "iphone" || row.target()? != phone.id || !queued(row.state) {
                continue;
            }
            if now >= row.expires()? {
                terminate(&mut *gateway, &row, true)?;
                continue;
            }
            match gateway.authorize(&row) {
                Ok(()) => (),
                Err(Error::Unavailable) => continue,
                Err(_) => {
                    terminate(&mut *gateway, &row, false)?;
                    continue;
                }
            }
            let live = row.live()?;
            if !dispatch && !live {
                continue;
            }
            if live && (!allow_live || row.state == CommandState::Dispatched) {
                continue;
            }
            let live_payload = if live {
                match live_payload(&phone, &row.fields()?, now) {
                    Ok(p) => Some(p),
                    Err(Error::Unavailable) => continue,
                    Err(e) => return Err(e),
                }
            } else {
                None
            };
            row = dispatch_row(&mut *gateway, row)?;
            let mut packet = self.authority.seal_iphone(
                &phone,
                &ab_wire::gojson::legacy_json(
                    &Job {
                        command: row.command.clone(),
                        request_hash: row.request_hash.clone(),
                        expires: row.expires()?,
                    },
                    ab_wire::gojson::EscapeMode::Html,
                )
                .map_err(|_| Error::Invalid)?,
            )?;
            if let Some((token, aps)) = live_payload {
                packets.push(packet.clone());
                packet.live_activity = Some(aps);
                pushes.push(LivePush { token, packet });
            } else {
                packets.push(packet);
            }
            if packets.len() >= 32 {
                break;
            }
        }
        Ok((packets, pushes))
    }
    fn live_state(&self, phone: &Principal, now: i64) -> Result<Vec<Packet>, Error> {
        let phone = self.authority.principal(&phone.id)?;
        let mut gateway = self.lock()?;
        let mut rows = gateway.commands()?;
        order_live(&mut rows)?;
        let mut packets = vec![];
        for row in rows {
            if !row.live()?
                || row.source != "iphone"
                || row.state != CommandState::Dispatched
                || row.target()? != phone.id
                || now >= row.expires()?
            {
                continue;
            }
            packets.push(self.authority.seal_iphone(
                &phone,
                &encode(&Job {
                    command: row.command.clone(),
                    request_hash: row.request_hash.clone(),
                    expires: row.expires()?,
                })?,
            )?);
            if packets.len() >= 32 {
                break;
            }
        }
        Ok(packets)
    }
    fn approval(
        &self,
        phone: &Principal,
        id: &str,
        hash: &str,
        body: &str,
        title: Option<&str>,
        expires: i64,
    ) -> Result<Packet, Error> {
        let mut args = serde_json::json!({"device_id":phone.id,"body":body});
        if let Some(title) = title {
            args["title"] = title.into();
        }
        let command = raw(&ApprovalCommand {
            id,
            op: "bridge.approval",
            args,
            expires_at: "0001-01-01T00:00:00Z",
        })?;
        Ok(self.authority.seal_iphone(
            phone,
            &encode(&Job {
                command,
                request_hash: hash.into(),
                expires,
            })?,
        )?)
    }
    pub async fn notified(&self) {
        self.wake.notified().await;
    }
    pub fn wake(&self) {
        self.wake.notify_one();
    }
    fn push_batch(&self, now: i64) -> Result<Vec<LivePush>, Error> {
        self.sweep(now)?;
        let mut batch = vec![];
        for device in self.authority.iphones()? {
            let Ok(phone) = self.authority.principal(&device.id) else {
                continue;
            };
            let token = phone
                .extra
                .get("iphone_push_token")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (packets, live) = self.packets(&phone, now, !token.is_empty(), true)?;
            batch.extend(live);
            if !token.is_empty() {
                batch.extend(packets.into_iter().map(|packet| LivePush {
                    token: token.into(),
                    packet,
                }));
            }
        }
        Ok(batch)
    }
    pub async fn push_once(
        self: &Arc<Self>,
        client: &crate::PushClient,
        now: i64,
    ) -> Result<(), Error>
    where
        G: 'static,
    {
        let delivery = self.clone();
        let batch = tokio::task::spawn_blocking(move || delivery.push_batch(now))
            .await
            .map_err(|_| Error::Unavailable)??;
        for push in batch {
            let _ = client.push(push.token, push.packet, now + 60).await;
        }
        Ok(())
    }
    pub async fn run(
        self: Arc<Self>,
        client: crate::PushClient,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) where
        G: 'static,
    {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            if *stop.borrow() {
                return;
            }
            tokio::select! {
                changed = stop.changed() => { if changed.is_err() || *stop.borrow() { return; } }
                _ = interval.tick() => { let _ = self.push_once(&client, ab_trust::decision_now().as_second()).await; }
                () = self.wake.notified() => { let _ = self.push_once(&client, ab_trust::decision_now().as_second()).await; }
            }
        }
    }
}
#[derive(Debug)]
pub struct LivePush {
    pub token: String,
    pub packet: Packet,
}
#[derive(Deserialize)]
struct PendingOptions {
    #[serde(default)]
    live_state: bool,
}
fn wants_live_state(payload: &[u8]) -> bool {
    serde_json::from_slice::<PendingOptions>(payload).is_ok_and(|options| options.live_state)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PushToken {
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveToken {
    task_id: String,
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    command_id: String,
    request_hash: String,
    approve: bool,
}
fn queued(state: CommandState) -> bool {
    matches!(
        state,
        CommandState::Admitted | CommandState::Prepared | CommandState::Dispatched
    )
}
fn terminate(gateway: &mut impl Gateway, row: &QueuedCommand, expired: bool) -> Result<(), Error> {
    let (state, disposition) = if row.state == CommandState::Dispatched {
        (CommandState::Indeterminate, Disposition::Indeterminate)
    } else if expired {
        (CommandState::Expired, Disposition::Expired)
    } else {
        (CommandState::Rejected, Disposition::RejectedBeforeDispatch)
    };
    gateway.transition(row, state, Some(disposition), None)?;
    Ok(())
}
fn is_live(op: &str) -> bool {
    matches!(
        op,
        "iphone.live.start" | "iphone.live.update" | "iphone.live.end"
    )
}
fn is_location(op: &str) -> bool {
    matches!(
        op,
        "iphone.location.current"
            | "iphone.location.significant"
            | "iphone.geofence.add"
            | "iphone.geofence.remove"
            | "iphone.geofence.list"
    )
}
fn validate_location_summary(data: &RawValue) -> Result<(), Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Summary {
        count: u32,
        significant: bool,
    }
    let summary: Summary = serde_json::from_str(data.get())?;
    let _ = summary.significant;
    if summary.count > 20 {
        return Err(Error::Invalid);
    }
    Ok(())
}
const LIVE_TITLE: &str = "Apple Bridge task";
const LIVE_STATUS: &str = "Open Apple Bridge for details";
const LIVE_FINISHED: &str = "Finished";

pub fn live_handle(encryption_key: &[u8], task: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::fmt::Write;
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(encryption_key) else {
        return String::new();
    };
    mac.update(format!("apple-bridge-iphone-live/1\0{task}").as_bytes());
    mac.finalize().into_bytes()[..16]
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}
pub fn live_payload(
    phone: &Principal,
    command: &Value,
    now: i64,
) -> Result<(String, Box<RawValue>), Error> {
    let op = command["op"].as_str().ok_or(Error::Invalid)?;
    if !is_live(op) {
        return Err(Error::Invalid);
    }
    let args = &command["args"];
    let allowed = ["device_id", "task_id", "title", "status", "progress"];
    let fields = args.as_object().ok_or(Error::Invalid)?;
    let title = args["title"].as_str().unwrap_or_default();
    let status = args["status"].as_str().unwrap_or_default();
    let progress = args["progress"].as_f64().unwrap_or(0.0);
    let task_value = args["task_id"].as_str().unwrap_or_default();
    if fields.keys().any(|k| !allowed.contains(&k.as_str()))
        || task_value.is_empty()
        || task_value.len() > 128
        || task_value == "_start"
        || title.trim().is_empty()
        || title.len() > 128
        || status.len() > 256
        || !(0.0..=1.0).contains(&progress)
    {
        return Err(Error::Invalid);
    }

    let task = string_field(args, "task_id")?;
    let event = op.trim_start_matches("iphone.live.");
    let encryption_key = phone
        .extra
        .get("iphone_encryption_key")
        .and_then(Value::as_str)
        .and_then(|key| STANDARD.decode(key).ok())
        .ok_or(Error::Invalid)?;
    let handle = live_handle(&encryption_key, &task);
    let tokens = phone.extra.get("iphone_live_tokens");
    let lookup = |key: &str| {
        tokens
            .and_then(|t| t.get(key))
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
    };
    let token = if event == "start" {
        lookup("_start")
    } else {
        lookup(&handle).or_else(|| lookup(&task))
    }
    .ok_or(Error::Unavailable)?;
    let status = if event == "end" {
        LIVE_FINISHED
    } else {
        LIVE_STATUS
    };
    let mut aps = serde_json::json!({"timestamp":now,"event":event,"content-state":{"status":status,"progress":0}});
    if event == "start" {
        aps["attributes-type"] = "AgentTaskAttributes".into();
        aps["attributes"] = serde_json::json!({"taskID":handle,"title":LIVE_TITLE});
        aps["alert"] = serde_json::json!({"title":"Apple Bridge","body":LIVE_TITLE});
    }
    if event == "end" {
        aps["dismissal-date"] = now.into();
    }
    Ok((token.into(), details_raw(&aps)?))
}
fn string_field(value: &Value, key: &str) -> Result<String, Error> {
    value[key].as_str().map(str::to_owned).ok_or(Error::Invalid)
}
fn timestamp(value: &str) -> Result<i64, Error> {
    value
        .parse::<jiff::Timestamp>()
        .map(jiff::Timestamp::as_second)
        .map_err(|_| Error::Invalid)
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    ab_wire::gojson::legacy_json(value, ab_wire::gojson::EscapeMode::Html)
        .map_err(|_| Error::Invalid)
}
fn raw(value: &impl Serialize) -> Result<Box<RawValue>, Error> {
    Ok(RawValue::from_string(
        String::from_utf8(encode(value)?).map_err(|_| Error::Invalid)?,
    )?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSample {
    pub id: String,
    pub r#type: String,
    #[serde(serialize_with = "serialize_time")]
    pub start: String,
    #[serde(serialize_with = "serialize_time")]
    pub end: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_quantity"
    )]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "empty_details",
        serialize_with = "serialize_details"
    )]
    pub details: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthBatch {
    pub stream: String,
    pub sequence: u64,
    pub samples: Option<Vec<HealthSample>>,
    pub deleted: Option<Vec<String>>,
}
impl HealthBatch {
    pub fn validate(&self) -> Result<(), Error> {
        if self.stream.is_empty()
            || self.stream.len() > 160
            || self.sequence == 0
            || self.sequence > i64::MAX.cast_unsigned()
            || self.samples.as_ref().map_or(0, Vec::len) + self.deleted.as_ref().map_or(0, Vec::len)
                > 100
        {
            return Err(Error::Invalid);
        }
        for sample in self.samples.iter().flatten() {
            let start = sample
                .start
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Invalid)?;
            let end = sample
                .end
                .parse::<jiff::Timestamp>()
                .map_err(|_| Error::Invalid)?;
            if sample.id.is_empty()
                || sample.r#type != self.stream
                || start.as_nanosecond() == -62_135_596_800_000_000_000
                || end < start
                || sample.value.is_some_and(|v| !v.is_finite())
            {
                return Err(Error::Invalid);
            }
        }
        if self.deleted.iter().flatten().any(String::is_empty) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn advance(&self, prior: Option<&HealthCursor>) -> Result<Option<HealthCursor>, Error> {
        self.validate()?;
        let digest = self.digest()?;
        let sequence = prior.map_or(0, |p| p.sequence);
        if prior.is_some_and(|p| p.sequence == self.sequence && p.digest == digest) {
            return Ok(None);
        }
        if sequence.checked_add(1) != Some(self.sequence) {
            return Err(Error::Conflict);
        }
        Ok(Some(HealthCursor {
            sequence: self.sequence,
            digest,
        }))
    }
    pub fn digest(&self) -> Result<String, Error> {
        use sha2::{Digest, Sha256};
        Ok(format!("{:x}", Sha256::digest(encode(self)?)))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationRecord {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fence: Option<Box<RawValue>>,
    pub timestamp: f64,
}
pub fn validate_locations(records: &[LocationRecord]) -> Result<(), Error> {
    if records.is_empty() || records.len() > 25 {
        return Err(Error::Invalid);
    }
    for record in records {
        if record.id.is_empty()
            || record.id.len() > 128
            || !record
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
            || !matches!(
                record.kind.as_str(),
                "current" | "arrive" | "leave" | "significant" | "geofence" | "geofence_removed"
            )
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct Evidence<'a> {
    #[serde(flatten)]
    request: &'a Request,
    signing_key: &'a str,
}
#[derive(Serialize)]
struct ApprovalCommand<'a> {
    #[serde(rename = "command_id")]
    id: &'a str,
    op: &'a str,
    args: Value,
    expires_at: &'a str,
}

fn order_live(rows: &mut [QueuedCommand]) -> Result<(), Error> {
    let mut live = Vec::new();
    for row in rows.iter() {
        if row.live()? {
            live.push(row.clone());
        }
    }
    live.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id().ok().cmp(&b.id().ok()))
    });
    let mut live = live.into_iter();
    for row in rows {
        if row.live()? {
            *row = live.next().ok_or(Error::Invalid)?;
        }
    }
    Ok(())
}

fn dispatch_row(
    gateway: &mut impl Gateway,
    mut row: QueuedCommand,
) -> Result<QueuedCommand, Error> {
    if row.state == CommandState::Admitted {
        row = gateway.transition(&row, CommandState::Prepared, None, None)?;
    }
    if row.state == CommandState::Prepared {
        row = gateway.transition(&row, CommandState::Dispatched, None, None)?;
    }
    Ok(row)
}

fn go_number(value: f64) -> String {
    let abs = value.abs();
    if abs != 0.0 && !(1e-6..1e21).contains(&abs) {
        let scientific = format!("{value:e}");
        let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
        format!(
            "{mantissa}e{}{exponent}",
            if exponent.starts_with('-') { "" } else { "+" }
        )
    } else {
        value.to_string()
    }
}
#[allow(clippy::ref_option)]
fn serialize_quantity<S: serde::Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let raw = value
        .map(|v| RawValue::from_string(go_number(v)))
        .transpose()
        .map_err(serde::ser::Error::custom)?;
    raw.serialize(serializer)
}
#[allow(clippy::ref_option)]
fn empty_details(value: &Option<Value>) -> bool {
    value
        .as_ref()
        .is_none_or(|v| v.is_null() || v.as_object().is_some_and(serde_json::Map::is_empty))
}
fn details_raw(value: &Value) -> Result<Box<RawValue>, Error> {
    match value {
        Value::Number(n) => Ok(RawValue::from_string(go_number(
            n.as_f64().ok_or(Error::Invalid)?,
        ))?),
        Value::Array(a) => raw(&a.iter().map(details_raw).collect::<Result<Vec<_>, _>>()?),
        Value::Object(m) => raw(&m
            .iter()
            .map(|(k, v)| Ok((k, details_raw(v)?)))
            .collect::<Result<std::collections::BTreeMap<_, _>, Error>>()?),
        _ => raw(value),
    }
}
#[allow(clippy::ref_option)]
fn serialize_details<S: serde::Serializer>(
    value: &Option<Value>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value
        .as_ref()
        .map(details_raw)
        .transpose()
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}
fn serialize_time<S: serde::Serializer>(value: &str, serializer: S) -> Result<S::Ok, S::Error> {
    let mut text = value.to_owned();
    if text.ends_with("+00:00") || text.ends_with("-00:00") {
        text.truncate(text.len() - 6);
        text.push('Z');
    }
    if let Some(dot) = text.find('.') {
        let end = text[dot + 1..]
            .find(|c: char| !c.is_ascii_digit())
            .map_or(text.len(), |i| i + dot + 1);
        let trimmed = text[dot + 1..end].trim_end_matches('0');
        text.replace_range(
            dot..end,
            &if trimmed.is_empty() {
                String::new()
            } else {
                format!(".{trimmed}")
            },
        );
    }
    text.serialize(serializer)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCursor {
    pub sequence: u64,
    pub digest: String,
}

#[cfg(test)]
mod connector_prompt_tests {
    use super::{ConnectorApproval, connector_prompt};

    fn approval(host: &str) -> ConnectorApproval {
        ConnectorApproval {
            id: "connector:synthetic".into(),
            code: "1234".into(),
            name: "Claude".into(),
            host: host.into(),
            scopes: "Read: Messages".into(),
            expires: 0,
        }
    }

    #[test]
    fn the_phone_shows_where_a_connector_returns() {
        let body = connector_prompt(&approval("evil.example.invalid"));
        assert!(body.starts_with("Returns to evil.example.invalid\n"), "{body}");
        assert!(body.contains("App calls itself: Claude"), "{body}");
        assert!(body.contains("Code 1234"), "{body}");
        assert!(body.contains("Read: Messages"), "{body}");
    }

    #[test]
    fn a_local_callback_is_called_out() {
        for host in ["localhost:3118", "127.0.0.1:49152", "[::1]:49152"] {
            let body = connector_prompt(&approval(host));
            assert!(body.starts_with("Returns to an app on this computer"), "{body}");
        }
    }
}
