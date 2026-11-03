#![forbid(unsafe_code)]
mod clock;
mod command;
pub mod delegation;
mod dial;
pub mod gateway;
mod listener;
pub mod relay;
pub mod server;
mod state;
pub mod upstream;
mod window;
use ab_blobs::{Digest, Reference, Store as BlobStore};
use ab_trust::wire::{
    framing::BlobChunk,
    messages::{Batch, FeedFrame},
};
use ab_trust::{Authority, Enrollment, Grant, Principal};
pub use command::{CommandResponse, GatewayRoute, GatewayTransport};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Mutex, mpsc, watch},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Trust(#[from] ab_trust::Error),
    #[error(transparent)]
    Blob(#[from] ab_blobs::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error(transparent)]
    Contract(#[from] ab_trust::domain::ErrorCode),
    #[error("invalid feed state")]
    Invalid,
    #[error("feed disconnected")]
    Disconnected,
    #[error("state: {0}")]
    State(String),
    #[error(transparent)]
    Storage(#[from] ab_state::Error),
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: String,
    pub subscription_revision: u64,
    pub sequence: u64,
}
pub trait ReplicaState: std::fmt::Debug + Send {
    type Install: Send;
    fn head(&self) -> Result<Cursor, Error>;
    fn discard_capture(&mut self) -> Result<(), Error>;
    fn restrict(&mut self, visibility: &ab_trust::Visibility) -> Result<(), Error>;
    fn begin_install(
        &mut self,
        frame: &FeedFrame,
        epoch: &str,
        revision: u64,
    ) -> Result<Self::Install, Error>;
    fn snapshot_page(
        &mut self,
        install: &mut Self::Install,
        frame: &FeedFrame,
    ) -> Result<(), Error>;
    fn install_snapshot(&mut self, install: Self::Install) -> Result<Cursor, Error>;
    fn apply_batch(&mut self, batch: &Batch) -> Result<Cursor, Error>;
    fn blob_references(&self) -> Result<Vec<Reference>, Error>;
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Snapshot,
    Connected,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct FollowStatus {
    pub partial_batches: u64,
    pub clock_sample: Option<serde_json::Value>,
    pub state: ConnectionState,
    pub mode: String,
    pub gateway_head: u64,
    pub schema: i64,
    pub snapshot_sequence: u64,
    pub snapshots: u64,
    pub resumes: u64,
    pub snapshot_pages: u64,
    pub last_heartbeat: String,
}
#[derive(Debug)]
pub(crate) struct Live {
    message: Message,
    flushed: Option<tokio::sync::oneshot::Sender<()>>,
}
#[derive(Clone, Debug)]
pub struct Writer {
    live: mpsc::Sender<Live>,
    bulk: mpsc::Sender<Message>,
}
impl Writer {
    pub async fn send(&self, frame: FeedFrame) -> Result<(), Error> {
        self.live
            .send(Live {
                message: Message::Text(
                    String::from_utf8(frame.encode()?)
                        .map_err(|_| Error::Invalid)?
                        .into(),
                ),
                flushed: None,
            })
            .await
            .map_err(|_| Error::Disconnected)
    }
    async fn send_flushed(&self, frame: FeedFrame) -> Result<(), Error> {
        let (flushed, receiver) = tokio::sync::oneshot::channel();
        let message = Message::Text(
            String::from_utf8(frame.encode()?)
                .map_err(|_| Error::Invalid)?
                .into(),
        );
        self.live
            .send(Live {
                message,
                flushed: Some(flushed),
            })
            .await
            .map_err(|_| Error::Disconnected)?;
        receiver.await.map_err(|_| Error::Disconnected)
    }
    pub async fn bulk(&self, bytes: Vec<u8>) -> Result<(), Error> {
        self.bulk
            .send(Message::Binary(bytes.into()))
            .await
            .map_err(|_| Error::Disconnected)
    }
}
#[derive(Debug)]
pub struct Client<S: ReplicaState> {
    pub state: Arc<Mutex<S>>,
    pub authority: Arc<Authority>,
    pub blobs: Option<Arc<Mutex<BlobStore>>>,
    status: watch::Sender<FollowStatus>,
    commands: Arc<command::Hub>,
    #[cfg(feature = "lab-harness")]
    apply_delay: Option<std::path::PathBuf>,
}
fn raw<T: Serialize>(v: &T) -> Result<Box<serde_json::value::RawValue>, Error> {
    Ok(serde_json::value::to_raw_value(v)?)
}
fn frame(t: &str) -> FeedFrame {
    FeedFrame {
        t: t.into(),
        ..FeedFrame::default()
    }
}
impl<S: ReplicaState> Client<S> {
    pub fn new(
        state: Arc<Mutex<S>>,
        authority: Arc<Authority>,
        blobs: Option<Arc<Mutex<BlobStore>>>,
    ) -> Self {
        let (status, _) = watch::channel(FollowStatus::default());
        Self {
            state,
            authority,
            blobs,
            status,
            commands: Arc::default(),
            #[cfg(feature = "lab-harness")]
            apply_delay: None,
        }
    }
    #[cfg(feature = "lab-harness")]
    #[must_use]
    pub fn with_apply_delay(mut self, path: std::path::PathBuf) -> Self {
        self.apply_delay = Some(path);
        self
    }
    #[cfg(feature = "lab-harness")]
    async fn lab_apply_delay(&self) {
        let Some(path) = &self.apply_delay else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        if std::fs::remove_file(path).is_err() {
            return;
        }
        if let Ok(ms) = text.trim().parse::<u64>()
            && ms <= 5000
        {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }
    pub fn status(&self) -> watch::Receiver<FollowStatus> {
        self.status.subscribe()
    }
    pub async fn follow(&self, mut stop: watch::Receiver<bool>) -> Result<(), Error> {
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            self.status
                .send_modify(|s| s.state = ConnectionState::Connecting);
            tokio::select! { _=stop.changed()=>return Ok(()), result=self.connect()=>{ if let Err(error) = result { eprintln!("replica feed: {error}"); } } }
            self.status
                .send_modify(|s| s.state = ConnectionState::Disconnected);
            tokio::select! { _=stop.changed()=>return Ok(()), ()=tokio::time::sleep(Duration::from_secs(1))=>{} }
        }
    }
    pub async fn connect(&self) -> Result<(), Error> {
        let e = self.authority.enrollment()?.ok_or(Error::Invalid)?;
        if !e.dial.is_empty() {
            return Err(Error::Invalid);
        }
        if e.remote {
            let mut ws = relay::dial(&e.address).await?;
            let binding = relay::client_binding(&ws)?;
            let principal = tokio::time::timeout(
                Duration::from_secs(5),
                relay::client(&self.authority, &mut ws, &binding, &e.fingerprint, None),
            )
            .await
            .map_err(|_| Error::Disconnected)??;
            if principal.id != e.replica_id {
                return Err(Error::Invalid);
            }
            return self.consume(ws, e).await;
        }
        let connector = tokio_tungstenite::Connector::Rustls(Arc::new(
            self.authority.client_tls(&e.fingerprint)?,
        ));
        let (ws, _) = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async_tls_with_config(
                format!("wss://{}/v1/feed", e.address),
                None,
                false,
                Some(connector),
            ),
        )
        .await
        .map_err(|_| Error::Disconnected)??;
        self.consume(ws, e).await
    }
    #[allow(clippy::result_large_err)]
    pub async fn listen(&self, address: &str, stop: watch::Receiver<bool>) -> Result<(), Error> {
        self.listen_bound(TcpListener::bind(address).await?, stop)
            .await
    }
    #[allow(clippy::result_large_err)]
    pub async fn listen_bound(
        &self,
        listener: TcpListener,
        mut stop: watch::Receiver<bool>,
    ) -> Result<(), Error> {
        let e = self.authority.enrollment()?.ok_or(Error::Invalid)?;
        if e.dial.is_empty() {
            return Err(Error::Invalid);
        }
        let acceptor =
            tokio_rustls::TlsAcceptor::from(Arc::new(self.authority.server_tls(&e.fingerprint)?));
        loop {
            let (socket, _) =
                tokio::select! { _=stop.changed()=>return Ok(()), r=listener.accept()=>r? };
            let connection = async {
                let tls = tokio::time::timeout(Duration::from_secs(15), acceptor.accept(socket))
                    .await
                    .map_err(|_| Error::Disconnected)??;
                let Some(tls) =
                    tokio::time::timeout(Duration::from_secs(15), listener::request(tls))
                        .await
                        .map_err(|_| Error::Disconnected)??
                else {
                    return Ok(());
                };
                let ws = tokio_tungstenite::accept_hdr_async(
                    tls,
                    |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                     response| {
                        if request.uri().path() != "/v1/feed"
                            || request.headers().contains_key("origin")
                        {
                            return Err(tokio_tungstenite::tungstenite::http::Response::builder()
                                .status(401)
                                .body(Some("unauthorized".into()))
                                .unwrap_or_default());
                        }
                        Ok(response)
                    },
                )
                .await?;
                self.consume(ws, e.clone()).await
            };
            tokio::select! { _=stop.changed()=>return Ok(()), _=connection=>{} }
        }
    }
    pub async fn consume<T>(
        &self,
        ws: WebSocketStream<T>,
        mut enrollment: Enrollment,
    ) -> Result<(), Error>
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        self.state.lock().await.discard_capture()?;
        let result = self.session(ws, &mut enrollment).await;
        self.status
            .send_modify(|s| s.state = ConnectionState::Disconnected);
        self.state.lock().await.discard_capture()?;
        result
    }
    #[allow(clippy::too_many_lines)]
    async fn session<T>(
        &self,
        ws: WebSocketStream<T>,
        enrollment: &mut Enrollment,
    ) -> Result<(), Error>
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    {
        let (mut sink, mut source) = ws.split();
        let (live, mut live_rx) = mpsc::channel(32);
        let (bulk, mut bulk_rx) = mpsc::channel(4);
        let writer = Writer { live, bulk };
        let write_loop = async {
            loop {
                let message = tokio::select! {biased;msg=live_rx.recv()=>msg.map(|item: Live|(item.message,item.flushed)),msg=bulk_rx.recv()=>msg.map(|message|(message,None))};
                let Some((message, flushed)) = message else {
                    return Err::<(), Error>(Error::Disconnected);
                };
                tokio::time::timeout(Duration::from_secs(10), sink.send(message))
                    .await
                    .map_err(|_| Error::Disconnected)??;
                if let Some(flushed) = flushed {
                    let _sent = flushed.send(());
                }
            }
        };
        let registration = command::Registration::new(self.commands.clone(), writer.clone())?;
        let read_loop = async {
            let head = self.state.lock().await.head()?;
            let mut hello = frame("Hello");
            hello.protocol = "feed/1".into();
            hello.clock_protocol = "ntp/1".into();
            hello.delegation_protocol = "delegation/1".into();
            hello.features = Some(vec![raw(&"capture_atomic")?]);
            hello.replica_id = enrollment.replica_id.clone();
            hello.cursor = Some(raw(&head)?);
            hello.schema_offer = Some(vec![raw(&1)?]);
            hello.credits = 4 << 20;
            writer.send(hello).await?;
            let mut welcome: Option<FeedFrame> = None;
            let mut clock = clock::Probe::default();
            let mut ready = false;
            let mut install = None;
            let mut active_blob: Option<Digest> = None;
            let mut ignored_blob = false;
            let mut snapshot_id = String::new();

            let mut blob_queue = std::collections::VecDeque::new();
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            let mut last = tokio::time::Instant::now();
            loop {
                let message = tokio::select! {
                    _=tick.tick()=>{ if last.elapsed()>Duration::from_secs(15) {return Err(Error::Disconnected);}
                        if ready && welcome.as_ref().is_some_and(|w| w.clock_protocol == "ntp/1") { clock.ping(&writer).await?; }let mut heartbeat=frame("Heartbeat");heartbeat.at=serde_json::from_value(serde_json::json!(jiff::Timestamp::now().to_string()))?;writer.send(heartbeat).await?;continue;},
                    m=source.next()=>m.ok_or(Error::Disconnected)??,
                };
                if let Message::Binary(bytes) = message {
                    let chunk = BlobChunk::decode(&bytes)?;
                    let d = Digest::try_from(hex::encode(chunk.digest))?;
                    if active_blob.as_ref() != Some(&d) {
                        return Err(Error::Invalid);
                    }
                    if !ignored_blob && let Some(blobs) = &self.blobs {
                        match blobs.lock().await.chunk(
                            &d,
                            u64::try_from(chunk.offset).map_err(|_| Error::Invalid)?,
                            &chunk.bytes,
                        ) {
                            Ok(()) => {}
                            Err(ab_blobs::Error::Capacity) => ignored_blob = true,
                            Err(e) => return Err(e.into()),
                        }
                    }
                    last = tokio::time::Instant::now();
                    self.status
                        .send_modify(|s| s.last_heartbeat = jiff::Timestamp::now().to_string());
                    continue;
                }
                let Message::Text(text) = message else {
                    if message.is_close() {
                        return Err(Error::Disconnected);
                    }
                    continue;
                };
                ab_trust::wire::framing::Frame::from_json(
                    ab_trust::wire::framing::Protocol::Feed,
                    text.as_bytes(),
                )?;
                let mut f: FeedFrame = serde_json::from_str(&text)?;
                if f.t == "Batch" {
                    f.batch = Some(serde_json::from_str(&text)?);
                }
                last = tokio::time::Instant::now();
                self.status
                    .send_modify(|s| s.last_heartbeat = jiff::Timestamp::now().to_string());
                if f.t == "ClockPong" {
                    let sample = clock.complete(&f, jiff::Timestamp::now())?;
                    self.status.send_modify(|s| s.clock_sample = Some(sample));
                    if !ready {
                        ready = true;
                        registration.session.ready();
                        self.status
                            .send_modify(|s| s.state = ConnectionState::Connected);
                    }
                    continue;
                }
                if f.t == "CommandResponse" {
                    registration.session.deliver(&f)?;
                    continue;
                }
                if f.t == "DelegationRequest" {
                    if welcome
                        .as_ref()
                        .is_none_or(|w| w.delegation_protocol != "delegation/1")
                    {
                        return Err(Error::Invalid);
                    }
                    self.answer_delegation(&writer, f).await?;
                    continue;
                }
                let mut ack = frame("Ack");
                ack.credits = i64::try_from(text.len()).map_err(|_| Error::Invalid)?;
                match f.t.as_str() {
                    "Welcome" => {
                        let scope: Vec<Grant> = f
                            .scope
                            .as_ref()
                            .map(|v| v.iter().map(|r| serde_json::from_str(r.get())).collect())
                            .transpose()?
                            .unwrap_or_default();
                        let ceiling = if enrollment.ceiling.is_empty() {
                            &enrollment.grants
                        } else {
                            &enrollment.ceiling
                        };
                        if welcome.is_some()
                            || f.protocol != "feed/1"
                            || f.schema != 1
                            || f.epoch.is_empty()
                            || f.subscription_revision == 0
                            || !ab_trust::subset(&scope, ceiling)
                        {
                            return Err(Error::Invalid);
                        }
                        self.state.lock().await.restrict(
                            &Principal {
                                grants: scope.clone(),
                                kind: "replica".into(),
                                ..Principal::default()
                            }
                            .visibility(),
                        )?;
                        enrollment.grants = scope;
                        self.authority.save_enrollment(enrollment.clone())?;
                        self.status.send_modify(|s| {
                            s.gateway_head = f.head;
                            s.schema = f.schema;
                            s.mode.clone_from(&f.reason);
                            if f.reason == "resume" {
                                s.resumes += 1;
                            }
                        });
                        welcome = Some(f.clone());
                    }
                    "SnapshotBegin" => {
                        let w = welcome.as_ref().ok_or(Error::Invalid)?;
                        if install.is_some() {
                            return Err(Error::Invalid);
                        }
                        snapshot_id.clone_from(&f.id);
                        install = Some(self.state.lock().await.begin_install(
                            &f,
                            &w.epoch,
                            w.subscription_revision,
                        )?);
                        self.status.send_modify(|s| {
                            s.state = ConnectionState::Snapshot;
                            s.snapshot_pages = 0;
                            s.snapshot_sequence = f.s;
                        });
                    }
                    "SnapshotPage" => {
                        Self::validate_records(&f, enrollment)?;
                        self.state
                            .lock()
                            .await
                            .snapshot_page(install.as_mut().ok_or(Error::Invalid)?, &f)?;
                        self.status.send_modify(|s| s.snapshot_pages += 1);
                    }
                    "SnapshotEnd" => {
                        if f.id != snapshot_id {
                            return Err(Error::Invalid);
                        }
                        let cursor = self
                            .state
                            .lock()
                            .await
                            .install_snapshot(install.take().ok_or(Error::Invalid)?)?;
                        ack.cursor = Some(raw(&cursor)?);
                        self.status.send_modify(|s| {
                            s.snapshots += 1;
                        });
                    }
                    "Batch" => {
                        if welcome.is_none() || install.is_some() {
                            return Err(Error::Invalid);
                        }
                        let batch = f.batch.as_ref().ok_or(Error::Invalid)?;
                        if batch.partial
                            && !welcome.as_ref().is_some_and(|w| {
                                w.features
                                    .iter()
                                    .flatten()
                                    .any(|v| v.get() == "\"capture_atomic\"")
                            })
                        {
                            return Err(Error::Invalid);
                        }
                        Self::validate_records(&f, enrollment)?;
                        #[cfg(feature = "lab-harness")]
                        self.lab_apply_delay().await;
                        let cursor = self.state.lock().await.apply_batch(batch)?;
                        if batch.partial {
                            self.status.send_modify(|s| s.partial_batches += 1);
                        }
                        if !batch.partial {
                            ack.cursor = Some(raw(&cursor)?);
                        }
                    }
                    "Reset" => {
                        if welcome.is_none() || install.is_some() {
                            return Err(Error::Invalid);
                        }
                        self.state.lock().await.discard_capture()?;
                    }
                    "Heartbeat" => {
                        self.status.send_modify(|s| s.gateway_head = f.head);
                    }
                    "BlobAvailable" => {}
                    "BlobDone" | "BlobUnavailable" => {
                        let d = Digest::try_from(f.digest.clone())?;
                        if active_blob.as_ref() != Some(&d) {
                            return Err(Error::Invalid);
                        }
                        if f.t == "BlobDone"
                            && !ignored_blob
                            && let Some(blobs) = &self.blobs
                        {
                            blobs.lock().await.finish(&d)?;
                        }
                        active_blob = None;
                        ignored_blob = false;
                    }
                    _ => return Err(Error::Invalid),
                }
                if !["Heartbeat", "BlobAvailable", "BlobDone", "BlobUnavailable"]
                    .contains(&f.t.as_str())
                {
                    if f.t == "SnapshotEnd" || f.t == "Welcome" && f.reason == "resume" {
                        writer.send_flushed(ack).await?;
                        if welcome
                            .as_ref()
                            .is_some_and(|w| w.clock_protocol == "ntp/1")
                        {
                            clock.ping(&writer).await?;
                        } else {
                            ready = true;
                            registration.session.ready();
                            self.status
                                .send_modify(|s| s.state = ConnectionState::Connected);
                        }
                    } else {
                        writer.send(ack).await?;
                    }
                }
                if install.is_none() && self.blobs.is_some() {
                    if blob_queue.is_empty()
                        && active_blob.is_none()
                        && [
                            "Welcome",
                            "Heartbeat",
                            "SnapshotEnd",
                            "Batch",
                            "BlobAvailable",
                        ]
                        .contains(&f.t.as_str())
                    {
                        let mut refs = self.state.lock().await.blob_references()?;
                        if let Some(blobs) = &self.blobs {
                            blobs
                                .lock()
                                .await
                                .protect(refs.iter().map(|r| r.digest.clone()).collect())?;
                        }
                        refs.sort_by(|a, b| {
                            b.variant.cmp(&a.variant).then(a.digest.cmp(&b.digest))
                        });
                        blob_queue.extend(refs.into_iter().filter(|r| !r.awaiting_variants));
                    }
                    if active_blob.is_none()
                        && let Some(blobs) = &self.blobs
                    {
                        while let Some(r) = blob_queue.pop_front() {
                            let d = Digest::try_from(r.digest)?;
                            match blobs.lock().await.begin(&d, r.size) {
                                Ok(Some(offset)) => {
                                    let mut want = frame("BlobWant");
                                    want.digest = d.as_str().into();
                                    want.offset =
                                        i64::try_from(offset).map_err(|_| Error::Invalid)?;
                                    writer.send(want).await?;
                                    active_blob = Some(d);
                                    break;
                                }
                                Ok(None) | Err(ab_blobs::Error::Capacity) => {}
                                Err(e) => return Err(e.into()),
                            }
                        }
                    }
                }
            }
        };
        tokio::select! {r=write_loop=>r,r=read_loop=>r}
    }
    fn validate_records(f: &FeedFrame, e: &Enrollment) -> Result<(), Error> {
        let visibility = Principal {
            kind: "replica".into(),
            grants: e.grants.clone(),
            ..Principal::default()
        }
        .visibility();
        if let Some(records) = &f.records {
            for r in records {
                let record: ab_trust::domain::Record = serde_json::from_str(r.get())?;
                if !visibility.allows(&record) {
                    return Err(Error::Invalid);
                }
            }
        }
        if let Some(batch) = &f.batch
            && let Some(changes) = &batch.changes
        {
            for c in changes {
                #[derive(Deserialize)]
                struct Change {
                    record: ab_trust::domain::Record,
                }
                let change: Change = serde_json::from_str(c.get())?;
                if !visibility.allows(&change.record) {
                    return Err(Error::Invalid);
                }
            }
        }
        Ok(())
    }
}
