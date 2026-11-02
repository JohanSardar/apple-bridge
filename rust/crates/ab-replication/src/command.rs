use crate::{Client, Error, ReplicaState, Writer, frame, raw};
use ab_trust::{
    Authority,
    domain::ContractError,
    wire::{framing::MAX_QUERY_FRAME, messages::FeedFrame},
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::oneshot;
#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResponse {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "result_value"
    )]
    pub result: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ContractError>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lookup_only: bool,
}
fn result_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(deserializer).map(Some)
}
#[derive(Debug)]
pub(crate) struct Session {
    writer: Writer,
    ready: AtomicBool,
    pending: Mutex<BTreeMap<String, oneshot::Sender<CommandResponse>>>,
}
impl Session {
    pub(crate) fn ready(&self) {
        self.ready.store(true, Ordering::Release);
    }
    pub(crate) fn deliver(&self, frame: &FeedFrame) -> Result<(), Error> {
        if frame.id.is_empty() {
            return Err(Error::Invalid);
        }
        let response: CommandResponse =
            serde_json::from_str(frame.response.as_ref().ok_or(Error::Invalid)?.get())?;
        if response.result.is_none() && response.error.is_none() {
            return Err(Error::Invalid);
        }
        if let Some(pending) = self
            .pending
            .lock()
            .map_err(|_| Error::Disconnected)?
            .remove(&frame.id)
        {
            let _ = pending.send(response);
        }
        Ok(())
    }
}
#[derive(Debug, Default)]
pub(crate) struct Hub {
    current: Mutex<Option<Arc<Session>>>,
    sequence: AtomicU64,
}
#[derive(Debug)]
pub(crate) struct Registration {
    hub: Arc<Hub>,
    pub(crate) session: Arc<Session>,
}
impl Registration {
    pub(crate) fn new(hub: Arc<Hub>, writer: Writer) -> Result<Self, Error> {
        let session = Arc::new(Session {
            writer,
            ready: AtomicBool::new(false),
            pending: Mutex::new(BTreeMap::new()),
        });
        {
            let mut current = hub.current.lock().map_err(|_| Error::Disconnected)?;
            if current.is_some() {
                return Err(Error::Disconnected);
            }
            *current = Some(session.clone());
        }
        Ok(Self { hub, session })
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.session.ready.store(false, Ordering::Release);
        if let Ok(mut current) = self.hub.current.lock()
            && current
                .as_ref()
                .is_some_and(|s| Arc::ptr_eq(s, &self.session))
        {
            *current = None;
        }
        if let Ok(mut pending) = self.session.pending.lock() {
            pending.clear();
        }
    }
}
#[derive(Debug)]
struct Pending {
    session: Arc<Session>,
    id: String,
}
impl Drop for Pending {
    fn drop(&mut self) {
        let removed = self
            .session
            .pending
            .lock()
            .is_ok_and(|mut pending| pending.remove(&self.id).is_some());
        if removed {
            let mut cancel = frame("CommandCancel");
            cancel.id.clone_from(&self.id);
            if let Ok(bytes) = cancel.encode()
                && let Ok(text) = String::from_utf8(bytes)
            {
                let _ = self
                    .session
                    .writer
                    .live
                    .try_send(tokio_tungstenite::tungstenite::Message::Text(text.into()));
            }
        }
    }
}
#[derive(Clone, Debug)]
pub enum GatewayRoute {
    Direct {
        address: String,
        fingerprint: String,
    },
    Feed,
}
#[derive(Clone, Debug)]
pub struct GatewayTransport {
    authority: Arc<Authority>,
    hub: Arc<Hub>,
}
impl GatewayTransport {
    pub fn connected(&self) -> bool {
        self.hub
            .current
            .lock()
            .is_ok_and(|s| s.as_ref().is_some_and(|s| s.ready.load(Ordering::Acquire)))
    }
    pub async fn exchange(
        &self,
        route: &GatewayRoute,
        delegation: String,
        request: Vec<u8>,
    ) -> Result<CommandResponse, Error> {
        if !self.connected() {
            return Err(Error::Disconnected);
        }
        match route {
            GatewayRoute::Feed => self.feed_exchange(delegation, request).await,
            GatewayRoute::Direct {
                address,
                fingerprint,
            } => {
                self.direct_exchange(address, fingerprint, delegation, request)
                    .await
            }
        }
    }
    pub async fn feed_exchange(
        &self,
        delegation: String,
        request: Vec<u8>,
    ) -> Result<CommandResponse, Error> {
        if request.len() > 256 << 10 {
            return Err(Error::Invalid);
        }
        let session = self
            .hub
            .current
            .lock()
            .map_err(|_| Error::Disconnected)?
            .clone()
            .ok_or(Error::Disconnected)?;
        if !session.ready.load(Ordering::Acquire) {
            return Err(Error::Disconnected);
        }
        let id = format!(
            "request_{}",
            self.hub.sequence.fetch_add(1, Ordering::Relaxed)
        );
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = session.pending.lock().map_err(|_| Error::Disconnected)?;
            if !session.ready.load(Ordering::Acquire) || pending.len() >= 16 {
                return Err(Error::Disconnected);
            }
            pending.insert(id.clone(), sender);
        }
        let _pending = Pending {
            session: session.clone(),
            id: id.clone(),
        };
        let mut f = frame("CommandRequest");
        f.id = id;
        f.delegation = delegation;
        f.request = Some(RawValue::from_string(
            String::from_utf8(request).map_err(|_| Error::Invalid)?,
        )?);
        let timeout = forward_timeout(f.request.as_ref().ok_or(Error::Invalid)?.get());
        tokio::time::timeout(timeout, async {
            session.writer.send(f).await?;
            receiver.await.map_err(|_| Error::Disconnected)
        })
        .await
        .map_err(|_| Error::Disconnected)?
    }
    async fn direct_exchange(
        &self,
        address: &str,
        pin: &str,
        delegation: String,
        request: Vec<u8>,
    ) -> Result<CommandResponse, Error> {
        let client = reqwest::Client::builder()
            .use_preconfigured_tls(self.authority.client_tls(pin)?)
            .connect_timeout(Duration::from_millis(1500))
            .timeout(forward_timeout(
                std::str::from_utf8(&request).map_err(|_| Error::Invalid)?,
            ))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(ab_trust::Error::from)?;
        let mut response = client
            .post(format!("https://{address}/v1/commands"))
            .header("X-Apple-Bridge-Delegation", delegation)
            .header("Content-Type", "application/json")
            .body(request)
            .send()
            .await
            .map_err(ab_trust::Error::from)?;
        let lookup_only = response
            .headers()
            .get("X-Apple-Bridge-Command-Lookup-Only")
            .is_some_and(|v| v == "true");
        let success = response.status().is_success();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ab_trust::Error::from)? {
            if bytes.len() + chunk.len() > MAX_QUERY_FRAME {
                return Err(Error::Invalid);
            }
            bytes.extend_from_slice(&chunk);
        }
        if success {
            Ok(CommandResponse {
                result: Some(serde_json::from_slice(&bytes)?),
                error: None,
                lookup_only,
            })
        } else {
            #[derive(Deserialize)]
            struct Envelope {
                error: ContractError,
            }
            let error: Envelope = serde_json::from_slice(&bytes)?;
            Ok(CommandResponse {
                result: None,
                error: Some(error.error),
                lookup_only,
            })
        }
    }
}
fn forward_timeout(raw: &str) -> Duration {
    let request: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
    if request["method"] == "query" {
        let ms = request["native_query"]["deadline_ms"]
            .as_u64()
            .unwrap_or(1000);
        if (1..=60000).contains(&ms) {
            return Duration::from_millis(ms);
        }
    }
    Duration::from_secs(20)
}
impl<S: ReplicaState> Client<S> {
    pub fn gateway_transport(&self) -> GatewayTransport {
        GatewayTransport {
            authority: self.authority.clone(),
            hub: self.commands.clone(),
        }
    }
    pub async fn exchange(
        &self,
        delegation: String,
        request: Vec<u8>,
    ) -> Result<CommandResponse, Error> {
        self.gateway_transport()
            .feed_exchange(delegation, request)
            .await
    }
    pub(crate) async fn answer_delegation(
        &self,
        writer: &Writer,
        request: FeedFrame,
    ) -> Result<(), Error> {
        #[derive(Deserialize)]
        struct Recheck {
            principal: String,
            command_id: String,
            request_hash: String,
            challenge: String,
        }
        let recheck: Recheck =
            serde_json::from_str(request.request.as_ref().ok_or(Error::Invalid)?.get())?;
        if recheck.challenge.len() != 64
            || !recheck.challenge.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::Invalid);
        }
        let response = match self.authority.principal(&recheck.principal).and_then(|p| {
            self.authority.sign_delegation(
                &p,
                &recheck.command_id,
                &recheck.request_hash,
                &recheck.challenge,
            )
        }) {
            Ok(proof) => CommandResponse {
                result: Some(raw(&proof)?),
                error: None,
                lookup_only: false,
            },
            Err(_) => CommandResponse {
                result: None,
                error: Some(ab_trust::domain::ErrorCode::Unauthorized.into()),
                lookup_only: false,
            },
        };
        let mut reply = frame("DelegationResponse");
        reply.id = request.id;
        reply.response = Some(raw(&response)?);
        writer.send(reply).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    type Fixture = (
        tempfile::TempDir,
        GatewayTransport,
        Registration,
        mpsc::Receiver<tokio_tungstenite::tungstenite::Message>,
    );
    fn setup() -> Result<Fixture, Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let authority = Arc::new(Authority::open(home.path())?);
        let hub = Arc::new(Hub::default());
        let (live, rx) = mpsc::channel(32);
        let (bulk, _) = mpsc::channel(4);
        let registration = Registration::new(hub.clone(), Writer { live, bulk })?;
        registration.session.ready();
        Ok((home, GatewayTransport { authority, hub }, registration, rx))
    }
    #[test]
    fn nullable_command_result_preserves_wire_value() -> Result<(), Box<dyn std::error::Error>> {
        let response: CommandResponse =
            serde_json::from_str(r#"{"result":null,"lookup_only":true}"#)?;
        assert_eq!(response.result.ok_or("result")?.get(), "null");
        assert!(response.lookup_only);
        let missing: CommandResponse = serde_json::from_str("{}")?;
        assert!(missing.result.is_none());
        Ok(())
    }
    #[tokio::test]
    async fn cancellation_drops_pending_and_sends_cancel() -> Result<(), Box<dyn std::error::Error>>
    {
        let (_home, transport, registration, mut wire) = setup()?;
        let task = tokio::spawn(async move {
            transport
                .feed_exchange("signed".into(), b"{}".to_vec())
                .await
        });
        let request = wire.recv().await.ok_or("request")?;
        let request: FeedFrame = serde_json::from_str(request.to_text()?)?;
        task.abort();
        let _ = task.await;
        let cancel = wire.recv().await.ok_or("cancel")?;
        let cancel: FeedFrame = serde_json::from_str(cancel.to_text()?)?;
        assert_eq!(cancel.t, "CommandCancel");
        assert_eq!(cancel.id, request.id);
        assert!(
            registration
                .session
                .pending
                .lock()
                .map_err(|_| "lock")?
                .is_empty()
        );
        Ok(())
    }
    #[tokio::test]
    async fn disconnect_fails_all_pending_without_replay() -> Result<(), Box<dyn std::error::Error>>
    {
        let (_home, transport, registration, mut wire) = setup()?;
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let transport = transport.clone();
            tasks.push(tokio::spawn(async move {
                transport
                    .feed_exchange("signed".into(), b"{}".to_vec())
                    .await
            }));
        }
        wire.recv().await.ok_or("first request")?;
        wire.recv().await.ok_or("second request")?;
        drop(registration);
        for task in tasks {
            assert!(matches!(task.await?, Err(Error::Disconnected)));
        }
        assert!(!transport.connected());
        assert!(matches!(
            transport
                .feed_exchange("signed".into(), b"{}".to_vec())
                .await,
            Err(Error::Disconnected)
        ));
        Ok(())
    }
}
