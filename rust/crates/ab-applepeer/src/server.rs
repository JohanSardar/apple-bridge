use crate::{Commands, send};
use ab_operations::{Code, Cursor, Error, Task};
use ab_wire::messages::{Capture, IpcFrame};
use rand::RngCore;
use std::{
    collections::BTreeMap,
    fmt::Debug,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream, unix::OwnedReadHalf},
    sync::{Mutex, Semaphore, mpsc, watch},
};
const CREDIT: usize = 4 << 20;
type Active = Arc<Mutex<Option<(u64, tokio::task::AbortHandle)>>>;
pub trait Backend: Debug + Send + Sync + 'static {
    fn next_fence(&self) -> Task<'_, u64>;
    fn checkpoints(&self) -> Task<'_, BTreeMap<String, Box<serde_json::value::RawValue>>>;
    fn commit(&self, capture: Capture) -> Task<'_, (Cursor, bool)>;
    fn source_state(&self, frame: IpcFrame) -> Task<'_, ()>;
}
#[derive(Debug)]
pub struct Server {
    pub socket: PathBuf,
    pub secret: [u8; 32],
    pub backend: Arc<dyn Backend>,
    pub commands: Commands,
}
async fn receive(reader: &mut OwnedReadHalf) -> Result<(IpcFrame, usize), Error> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let n = reader.read_u32().await.map_err(|_| Code::Unavailable)? as usize;
        if n == 0 || n > ab_wire::framing::MAX_QUERY_FRAME {
            return Err(Code::Capacity.into());
        }
        let mut bytes = vec![0; n + 4];
        bytes[..4].copy_from_slice(&u32::try_from(n).map_err(|_| Code::Capacity)?.to_be_bytes());
        reader
            .read_exact(&mut bytes[4..])
            .await
            .map_err(|_| Code::Unavailable)?;
        ab_wire::framing::decode_ipc(&bytes).map_err(|_| Code::Corrupt)?;
        let mut frame: IpcFrame = serde_json::from_slice(&bytes[4..]).map_err(|_| Code::Corrupt)?;
        if frame.t == "Capture" {
            frame.capture = Some(serde_json::from_slice(&bytes[4..]).map_err(|_| Code::Corrupt)?);
        }
        Ok((frame, n + 4))
    })
    .await
    .map_err(|_| Code::Unavailable)?
}
impl Server {
    pub async fn serve(self: Arc<Self>, mut stop: watch::Receiver<bool>) -> Result<(), Error> {
        let parent = self.socket.parent().ok_or(Code::Invalid)?;
        std::fs::create_dir_all(parent).map_err(|_| Code::Unavailable)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Code::Unavailable)?;
        match std::fs::symlink_metadata(&self.socket) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() {
                    return Err(Code::Conflict.into());
                }
                std::fs::remove_file(&self.socket).map_err(|_| Code::Unavailable)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Code::Unavailable.into()),
        }
        let listener = UnixListener::bind(&self.socket).map_err(|_| Code::Unavailable)?;
        std::fs::set_permissions(&self.socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| Code::Unavailable)?;
        let mut tasks = tokio::task::JoinSet::new();
        let active = Arc::new(Mutex::new(None::<(u64, tokio::task::AbortHandle)>));
        loop {
            tokio::select! {
                _ = stop.changed() => break,
                result = listener.accept() => {
                    let (stream, _) = result.map_err(|_| Code::Unavailable)?;
                    let server = self.clone();
                    let active = active.clone();
                    tasks.spawn(async move { let _ = server.session(stream, active).await; });
                }
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }
        if let Some((_, active)) = active.lock().await.take() {
            active.abort();
        }
        self.commands
            .connected
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        self.commands.session.lock().await.writer = None;
        self.commands.session.lock().await.waiting.clear();
        self.commands.session.lock().await.blobs.clear();
        std::fs::remove_file(&self.socket).map_err(|_| Code::Unavailable)?;
        Ok(())
    }
    async fn session(self: Arc<Self>, stream: UnixStream, active: Active) -> Result<(), Error> {
        if stream.peer_cred().map_err(|_| Code::Unauthorized)?.uid()
            != nix::unistd::geteuid().as_raw()
        {
            return Err(Code::Unauthorized.into());
        }
        let (mut reader, writer) = stream.into_split();
        let writer = Arc::new(Mutex::new(writer));
        let mut nonce = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let nonce = hex::encode(nonce);
        send(
            &writer,
            &IpcFrame {
                t: "Challenge".into(),
                nonce: nonce.clone(),
                ..IpcFrame::default()
            },
        )
        .await?;
        let (auth, _) = receive(&mut reader).await?;
        let expected = ab_wire::crypto::ipc_proof(&self.secret, "helper", &nonce, &auth.nonce)
            .map_err(|_| Code::Unauthorized)?;
        if auth.t != "Authenticate"
            || auth.nonce.len() != 64
            || !bool::from(
                auth.proof
                    .as_bytes()
                    .ct_eq(hex::encode(expected).as_bytes()),
            )
        {
            return Err(Code::Unauthorized.into());
        }
        let proof = ab_wire::crypto::ipc_proof(&self.secret, "gateway", &nonce, &auth.nonce)
            .map_err(|_| Code::Unauthorized)?;
        send(
            &writer,
            &IpcFrame {
                t: "Authenticated".into(),
                proof: hex::encode(proof),
                ..IpcFrame::default()
            },
        )
        .await?;
        let mut lock = active.lock().await;
        if let Some((_, previous)) = lock.take() {
            previous.abort();
        }
        let fence = self.backend.next_fence().await?;
        let checkpoints = self.backend.checkpoints().await?;
        send(
            &writer,
            &IpcFrame {
                t: "Open".into(),
                protocol: "ipc/1".into(),
                owner_fence: fence,
                checkpoints,
                byte_credit: i64::try_from(CREDIT).map_err(|_| Code::Capacity)?,
                ..IpcFrame::default()
            },
        )
        .await?;
        let (ready, _) = receive(&mut reader).await?;
        if ready.t != "Ready" || ready.protocol != "ipc/1" {
            return Err(Code::Corrupt.into());
        }
        self.commands.attach(fence, writer.clone()).await;
        let server = self.clone();
        let disconnected = ready.clone();
        let session_active = active.clone();
        let task = tokio::spawn(async move {
            server
                .consume(reader, writer, fence, ready, session_active)
                .await
        });
        *lock = Some((fence, task.abort_handle()));
        drop(lock);
        let result = task.await.map_err(|_| Error::from(Code::Unavailable));
        let mut current = active.lock().await;
        if current
            .as_ref()
            .is_some_and(|(current, _)| *current == fence)
        {
            for source in disconnected.coverage.keys() {
                let mut frame = disconnected.clone();
                frame.t = "SourceState".into();
                frame.state = "degraded".into();
                frame.reason = "helper_disconnected".into();
                frame.source = Some(
                    serde_json::from_value(serde_json::Value::String(source.clone()))
                        .map_err(|_| Code::Corrupt)?,
                );
                self.backend.source_state(frame).await?;
            }
            *current = None;
        }
        drop(current);
        self.commands.detach(fence).await;
        result?
    }
    #[allow(clippy::too_many_lines)]
    async fn consume(
        &self,
        mut reader: OwnedReadHalf,
        writer: Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>,
        fence: u64,
        ready: IpcFrame,
        active: Active,
    ) -> Result<(), Error> {
        let initial = active.lock().await;
        if initial
            .as_ref()
            .is_none_or(|(current, _)| *current != fence)
        {
            return Err(Code::Unavailable.into());
        }
        for (source, coverage) in &ready.coverage {
            let mut frame = ready.clone();
            frame.t = "SourceState".into();
            frame.state = "ready".into();
            frame.source = Some(
                serde_json::from_value(serde_json::Value::String(source.clone()))
                    .map_err(|_| Code::Corrupt)?,
            );
            frame.coverage = BTreeMap::from([(source.clone(), coverage.clone())]);
            self.backend.source_state(frame).await?;
        }
        drop(initial);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let capture_budget = Arc::new(Semaphore::new(CREDIT));
        let state_budget = Arc::new(Semaphore::new(ab_wire::framing::MAX_FRAME + 4));
        let command_budget = Arc::new(Semaphore::new(ab_wire::framing::MAX_FRAME + 4));
        let mut ping = tokio::time::interval(Duration::from_secs(5));
        let read = async {
            loop {
                let incoming = receive(&mut reader);
                tokio::pin!(incoming);
                let received = loop {
                    tokio::select! {
                        _ = ping.tick() => send(&writer, &IpcFrame { t: "Ping".into(), ..IpcFrame::default() }).await?,
                        result = &mut incoming => break result,
                    }
                };
                {
                    let (f, size) = received?;
                    match f.t.as_str() {
                        "BlobChunk" | "BlobUnavailable" | "QueryResult" | "QueryError" => {
                            self.commands.receive(fence, f).await;
                        }
                        "Capture" | "SourceState" | "Prepared" | "Rejected" | "Outcome" => {
                            let budget = match f.t.as_str() {
                                "Capture" => &capture_budget,
                                "SourceState" => &state_budget,
                                _ => &command_budget,
                            };
                            let permit = budget
                                .clone()
                                .try_acquire_many_owned(
                                    u32::try_from(size).map_err(|_| Code::Capacity)?,
                                )
                                .map_err(|_| Code::Capacity)?;
                            tx.send((f, size, permit)).map_err(|_| Code::Capacity)?;
                        }
                        "Ping" => {
                            send(
                                &writer,
                                &IpcFrame {
                                    t: "Pong".into(),
                                    ..IpcFrame::default()
                                },
                            )
                            .await?;
                        }
                        "Pong" => {}
                        "Close" => return Ok(()),
                        _ => return Err(Error::from(Code::Corrupt)),
                    }
                }
            }
        };
        let mutations = async {
            while let Some((f, size, permit)) = rx.recv().await {
                let current = active.lock().await;
                if current
                    .as_ref()
                    .is_none_or(|(current, _)| *current != fence)
                {
                    return Err(Code::Unavailable.into());
                }
                match f.t.as_str() {
                    "Capture" => {
                        let capture = f.capture.ok_or(Code::Corrupt)?;
                        match self.backend.commit(capture.clone()).await {
                            Ok((cursor, degraded)) => {
                                if degraded {
                                    self.degraded(
                                        capture.source,
                                        "incomplete_scan",
                                        "incomplete_scan_absence_ignored",
                                    )
                                    .await?;
                                }
                                self.commands.record(fence, &capture, cursor).await;
                                drop(permit);
                                send(
                                    &writer,
                                    &IpcFrame {
                                        t: "Committed".into(),
                                        source: capture.source,
                                        checkpoint: capture.checkpoint,
                                        credit: i64::try_from(size).map_err(|_| Code::Capacity)?,
                                        ..IpcFrame::default()
                                    },
                                )
                                .await?;
                            }
                            Err(error) if error.code == Code::Reset => {
                                send(
                                    &writer,
                                    &IpcFrame {
                                        t: "Reset".into(),
                                        source: capture.source,
                                        ..IpcFrame::default()
                                    },
                                )
                                .await?;
                            }
                            Err(error) => {
                                let _ = self
                                    .degraded(capture.source, "capture_failed", "capture_failed")
                                    .await;
                                return Err(error);
                            }
                        }
                    }
                    "SourceState" => self.backend.source_state(f).await?,
                    _ => self.commands.receive(fence, f).await,
                }
            }
            Ok(())
        };
        tokio::select! { result = read => result, result = mutations => result }
    }
}

impl Server {
    async fn degraded(
        &self,
        source: Option<ab_operations::domain::Source>,
        reason: &str,
        missing: &str,
    ) -> Result<(), Error> {
        let key = serde_json::to_value(source)
            .map_err(|_| Code::Corrupt)?
            .as_str()
            .ok_or(Code::Corrupt)?
            .to_owned();
        let coverage = serde_json::value::to_raw_value(
            &serde_json::json!({"fields":"partial","missing":[missing]}),
        )
        .map_err(|_| Code::Corrupt)?;
        self.backend
            .source_state(IpcFrame {
                t: "SourceState".into(),
                source,
                state: "degraded".into(),
                reason: reason.into(),
                coverage: BTreeMap::from([(key, coverage)]),
                ..IpcFrame::default()
            })
            .await
    }
}
