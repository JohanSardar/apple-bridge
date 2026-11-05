#![forbid(unsafe_code)]
mod clock;
mod command;
pub mod delegation;
mod dial;
pub mod gateway;
mod listener;
pub mod relay;
pub mod server;
mod session;
mod state;
pub mod upstream;
mod window;
use ab_blobs::{Reference, Store as BlobStore};
use ab_trust::wire::messages::{Batch, FeedFrame};
use ab_trust::{Authority, Enrollment, Principal};
pub use command::{CommandResponse, GatewayRoute, GatewayTransport};
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
    fn lab_home(&self) -> Option<&std::path::Path> {
        self.apply_delay
            .as_deref()
            .and_then(std::path::Path::parent)
    }
    #[cfg(feature = "lab-harness")]
    fn lab_legacy_hello(&self) -> bool {
        self.lab_home()
            .is_some_and(|home| home.join("lab-capture-legacy").exists())
    }
    #[cfg(feature = "lab-harness")]
    async fn lab_part_boundary(&self) -> Result<(), Error> {
        let Some(home) = self.lab_home() else {
            return Ok(());
        };
        if std::fs::remove_file(home.join("lab-capture-part-armed")).is_err() {
            return Ok(());
        }
        ab_trust::atomic_write(&home.join("lab-capture-part-reached"), b"partial_staged")
            .map_err(|_| Error::Invalid)?;
        while !home.join("lab-capture-part-release").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }
    #[cfg(feature = "lab-harness")]
    fn lab_capture_discarded(&self) {
        if let Some(home) = self.lab_home() {
            let _written =
                ab_trust::atomic_write(&home.join("lab-capture-discarded"), b"discarded");
        }
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
