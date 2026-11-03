use crate::upstream::Session;
use crate::{Cursor, Error, frame, raw};
use ab_trust::wire::gojson;
use base64::{Engine, engine::general_purpose::STANDARD};
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Join {
    id: String,
    proof: String,
}
use ab_trust::{
    Authority, Principal, Visibility,
    wire::messages::{Batch, FeedFrame},
};
use futures_util::{SinkExt, StreamExt};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Mutex,
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

pub trait FrozenSnapshot: std::fmt::Debug + Send {
    fn begin(&self) -> FeedFrame;
    fn records(&self) -> &[Box<serde_json::value::RawValue>];
    fn cursor(&self) -> Cursor;
    fn renew(&mut self) -> Result<(), Error>;
}

pub trait GatewayState: std::fmt::Debug + Send {
    type Snapshot: FrozenSnapshot;
    type Changes: Send;
    fn blob_grant(
        &self,
        _visibility: &Visibility,
        _digest: &ab_blobs::Digest,
    ) -> Result<ab_blobs::RecordGrant, Error> {
        Err(Error::Invalid)
    }
    fn head(&self) -> Result<Cursor, Error>;
    fn retention_floor(&self) -> Result<u64, Error>;
    fn freeze_snapshot(
        &mut self,
        visibility: &Visibility,
        revision: u64,
    ) -> Result<Self::Snapshot, Error>;
    fn changes(
        &mut self,
        visibility: &Visibility,
        cursor: Cursor,
        capture_atomic: bool,
    ) -> Result<Self::Changes, Error>;
    fn next_changes(&mut self, iterator: &mut Self::Changes) -> Result<Option<Batch>, Error>;
}

#[derive(Debug)]
pub struct Server<S: GatewayState> {
    pub state: Arc<Mutex<S>>,
    pub blobs: Option<Arc<Mutex<ab_blobs::Store>>>,
    pub authority: Arc<Authority>,
    pub uploads: Option<Arc<dyn crate::upstream::UploadHandler>>,
}

async fn receive<T: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<T>,
) -> Result<FeedFrame, Error> {
    loop {
        match socket.next().await.ok_or(Error::Disconnected)?? {
            Message::Text(text) => return Ok(serde_json::from_str(&text)?),
            Message::Ping(_) | Message::Pong(_) => {}
            _ => return Err(Error::Invalid),
        }
    }
}

async fn transmit<S: GatewayState, T: AsyncRead + AsyncWrite + Unpin>(
    server: &Server<S>,
    session: &Session<'_>,
    socket: &mut WebSocketStream<T>,
    f: FeedFrame,
    cursor: Option<&Cursor>,
    limit: usize,
) -> Result<(), Error> {
    let bytes = f.encode()?;
    if bytes.len() > limit {
        return Err(Error::Contract(ab_trust::domain::ErrorCode::Capacity));
    }
    let size = bytes.len();
    socket
        .send(Message::Text(
            String::from_utf8(bytes).map_err(|_| Error::Invalid)?.into(),
        ))
        .await?;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let ack = server.receive(socket, session).await?;
            if ack.t == "BlobWant" {
                let _ = server
                    .blob(socket, session.principal, ack, usize::MAX)
                    .await?;
                continue;
            }
            if ack.t == "Heartbeat" {
                continue;
            }
            if ack.t != "Ack" || usize::try_from(ack.credits).ok() != Some(size) {
                return Err(Error::Invalid);
            }
            if let Some(expected) = cursor {
                let actual: Cursor =
                    serde_json::from_str(ack.cursor.as_ref().ok_or(Error::Invalid)?.get())?;
                if actual != *expected {
                    return Err(Error::Invalid);
                }
            }
            return Ok(());
        }
    })
    .await
    .map_err(|_| Error::Disconnected)?
}

impl<S: GatewayState> Server<S> {
    async fn receive<T: AsyncRead + AsyncWrite + Unpin>(
        &self,
        socket: &mut WebSocketStream<T>,
        context: &Session<'_>,
    ) -> Result<FeedFrame, Error> {
        loop {
            let message = socket.next().await.ok_or(Error::Disconnected)??;
            if let Some(frame) = self.upstream(socket, context, message).await? {
                return Ok(frame);
            }
        }
    }
    async fn upstream<T: AsyncRead + AsyncWrite + Unpin>(
        &self,
        socket: &mut WebSocketStream<T>,
        context: &Session<'_>,
        message: Message,
    ) -> Result<Option<FeedFrame>, Error> {
        let response = match message {
            Message::Text(text) => {
                let frame: FeedFrame = serde_json::from_str(&text)?;
                if frame.t == "ClockPing" || frame.t == "ClockAck" {
                    if !context.clock {
                        return Err(Error::Invalid);
                    }
                    if let Some(response) = crate::clock::respond(&frame, jiff::Timestamp::now())? {
                        socket
                            .send(Message::Text(
                                String::from_utf8(response.encode()?)
                                    .map_err(|_| Error::Invalid)?
                                    .into(),
                            ))
                            .await?;
                    }
                    return Ok(None);
                }
                if frame.t != "UploadControl" && frame.t != "CommandRequest" {
                    return Ok(Some(frame));
                }
                if frame.id.is_empty() {
                    return Err(Error::Invalid);
                }
                let handler = context.uploads.ok_or(Error::Invalid)?;
                let response = tokio::time::timeout(
                    Duration::from_secs(20),
                    handler.control(
                        context.certificate,
                        &context.id,
                        &frame.delegation,
                        frame.request.as_deref().ok_or(Error::Invalid)?,
                    ),
                )
                .await
                .map_err(|_| Error::Disconnected)??;
                (frame.id, response)
            }
            Message::Binary(bytes) => {
                let handler = context.uploads.ok_or(Error::Invalid)?;
                tokio::time::timeout(
                    Duration::from_secs(20),
                    handler.chunk(context.certificate, &context.id, &bytes),
                )
                .await
                .map_err(|_| Error::Disconnected)??
            }
            Message::Ping(_) | Message::Pong(_) => return Ok(None),
            _ => return Err(Error::Invalid),
        };
        if response.0.is_empty() {
            return Err(Error::Invalid);
        }
        let mut frame = frame("CommandResponse");
        frame.id = response.0;
        frame.response = Some(raw(&response.1)?);
        socket
            .send(Message::Text(
                String::from_utf8(frame.encode()?)
                    .map_err(|_| Error::Invalid)?
                    .into(),
            ))
            .await?;
        Ok(None)
    }

    pub async fn serve<T: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut socket: WebSocketStream<T>,
        certificate: &[u8],
    ) -> Result<(), Error> {
        static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let principal = self.authority.authenticate_certificate(certificate)?;
        let mut context = Session {
            clock: false,
            principal: &principal,
            certificate,
            id: format!(
                "feed-{}-{}",
                std::process::id(),
                NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            uploads: self.uploads.as_deref(),
        };
        let session = self.session(&mut socket, &mut context);
        let invalidated = async {
            let mut timer = tokio::time::interval(Duration::from_millis(100));
            loop {
                timer.tick().await;
                if !self.authority.session_valid(&principal)? {
                    return Ok::<(), Error>(());
                }
            }
        };
        tokio::select! { result = session => result, result = invalidated => { result?; Err(Error::Disconnected) } }
    }

    #[allow(clippy::too_many_lines)]
    async fn session<T: AsyncRead + AsyncWrite + Unpin>(
        &self,
        socket: &mut WebSocketStream<T>,
        context: &mut Session<'_>,
    ) -> Result<(), Error> {
        let p = context.principal;
        let hello = tokio::time::timeout(Duration::from_secs(15), receive(socket))
            .await
            .map_err(|_| Error::Disconnected)??;
        if hello.t != "Hello"
            || hello.protocol != "feed/1"
            || hello.replica_id != p.id
            || hello.credits <= 0
            || !hello
                .schema_offer
                .as_ref()
                .is_some_and(|schemas| schemas.iter().any(|s| s.get() == "1"))
        {
            return Err(Error::Invalid);
        }
        context.clock = hello.clock_protocol == "ntp/1";
        let limit = usize::try_from(hello.credits.min(4 << 20)).map_err(|_| Error::Invalid)?;
        let visibility = p.visibility();
        let atomic = hello
            .features
            .as_ref()
            .is_some_and(|features| features.iter().any(|f| f.get() == "\"capture_atomic\""));
        let head = self.state.lock().await.head()?;
        let floor = self.state.lock().await.retention_floor()?;
        let previous: Option<Cursor> = hello
            .cursor
            .as_ref()
            .map(|v| serde_json::from_str(v.get()))
            .transpose()?;
        let resume = previous.as_ref().is_some_and(|c| {
            c.epoch == head.epoch
                && c.subscription_revision == p.revision
                && c.sequence <= head.sequence
                && c.sequence >= floor
        });
        let mut welcome = frame("Welcome");
        welcome.protocol = "feed/1".into();
        if context.clock {
            welcome.clock_protocol = "ntp/1".into();
        }
        welcome.gateway_id.clone_from(&self.authority.fingerprint);
        welcome.epoch.clone_from(&head.epoch);
        welcome.subscription_revision = p.revision;
        welcome.scope = Some(p.grants.iter().map(raw).collect::<Result<_, _>>()?);
        welcome.head = head.sequence;
        welcome.retention_floor = floor;
        welcome.schema = 1;
        welcome.reason = if resume { "resume" } else { "snapshot" }.into();
        if atomic {
            welcome.features = Some(vec![raw(&"capture_atomic")?]);
        }
        transmit(self, context, socket, welcome, None, limit).await?;
        let cursor = if resume {
            previous.ok_or(Error::Invalid)?
        } else {
            if previous.is_some_and(|c| c.sequence > 0) {
                let mut reset = frame("Reset");
                reset.reason = "history_or_scope_changed".into();
                reset.retention_floor = floor;
                transmit(self, context, socket, reset, None, limit).await?;
            }
            let mut snapshot = self
                .state
                .lock()
                .await
                .freeze_snapshot(&visibility, p.revision)?;
            let begin = snapshot.begin();
            let id = begin.id.clone();
            transmit(self, context, socket, begin, None, limit).await?;
            let mut offset = 0;
            let mut ordinal = 0;
            while offset < snapshot.records().len() {
                let mut end = (offset + 64).min(snapshot.records().len());
                let mut page = frame("SnapshotPage");
                page.id.clone_from(&id);
                page.ordinal = ordinal;
                loop {
                    page.records = Some(snapshot.records()[offset..end].to_vec());
                    if page.encode().is_ok_and(|b| b.len() <= limit) {
                        break;
                    }
                    end -= 1;
                    if end == offset {
                        return Err(Error::Contract(ab_trust::domain::ErrorCode::Capacity));
                    }
                }
                snapshot.renew()?;
                transmit(self, context, socket, page, None, limit).await?;
                offset = end;
                ordinal += 1;
            }
            let cursor = snapshot.cursor();
            let mut end = frame("SnapshotEnd");
            end.id = id;
            transmit(self, context, socket, end, Some(&cursor), limit).await?;
            cursor
        };
        let mut journal = cursor.clone();
        journal.subscription_revision = head.subscription_revision;
        let mut iterator = self
            .state
            .lock()
            .await
            .changes(&visibility, journal, atomic)?;
        let mut window = crate::window::Window::new(limit);
        let mut pending: Option<(String, Option<Cursor>)> = None;
        let mut blob_pending = None;
        let mut blob_states = std::collections::BTreeMap::new();
        let mut poll = tokio::time::interval(Duration::from_millis(20));
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                _ = poll.tick() => {
                    if let Some(blobs) = &self.blobs {
                        let entries = blobs.lock().await.entries().clone();
                        for (digest, entry) in entries {
                            let changed = blob_states.get(&digest) != Some(&entry.state);
                            if changed && matches!(entry.state, ab_blobs::BlobState::Present | ab_blobs::BlobState::Failed) {
                                let parsed = ab_blobs::Digest::try_from(digest.clone())?;
                                if self.state.lock().await.blob_grant(&visibility, &parsed).is_ok() {
                                    let mut available = frame("BlobAvailable");
                                    available.digest = digest.clone();
                                    socket.send(Message::Text(String::from_utf8(available.encode()?).map_err(|_| Error::Invalid)?.into())).await?;
                                }
                            }
                            blob_states.insert(digest, entry.state);
                        }
                    }
                    if pending.is_none() {
                    let next = self.state.lock().await.next_changes(&mut iterator);
                    match next {
                        Ok(Some(mut batch)) => {
                            batch.subscription_revision = p.revision;
                            let expected = Cursor { sequence: batch.through, ..cursor.clone() };
                            let partial = batch.partial;
                            let mut f = frame("Batch"); f.batch = Some(batch);
                            let bytes = f.encode()?;
                            if bytes.len() > limit { return Err(Error::Contract(ab_trust::domain::ErrorCode::Capacity)); }
                            pending = Some((String::from_utf8(bytes).map_err(|_| Error::Invalid)?, if partial { None } else { Some(expected) }));
                        },
                        Ok(None) => {},
                        Err(Error::Contract(ab_trust::domain::ErrorCode::Reset)) => {
                            let mut f = frame("Reset"); f.reason = "retention".into(); f.retention_floor = self.state.lock().await.retention_floor()?;
                            socket.send(Message::Text(String::from_utf8(f.encode()?).map_err(|_| Error::Invalid)?.into())).await?;
                            return Err(Error::Contract(ab_trust::domain::ErrorCode::Reset));
                        },
                        Err(e) => return Err(e),
                    }
                    }
                    if let Some((bytes, expected)) = &pending
                        && window.reserve(bytes.len(), expected.clone()) {
                        let (bytes, _) = pending.take().ok_or(Error::Invalid)?;
                        socket.send(Message::Text(bytes.into())).await?;
                    }
                    if let Some(want) = blob_pending.take() {
                        blob_pending = self.blob(socket, p, want, 1).await?;
                    }
                },
                () = tokio::time::sleep_until(window.deadline()), if window.pending() => return Err(Error::Disconnected),
                _ = heartbeat.tick() => {
                    let mut f = frame("Heartbeat"); f.head = self.state.lock().await.head()?.sequence;
                    socket.send(Message::Text(String::from_utf8(f.encode()?).map_err(|_| Error::Invalid)?.into())).await?;
                },
                message = socket.next() => { let message = message.ok_or(Error::Disconnected)??; let Some(f) = self.upstream(socket, context, message).await? else { continue; }; if f.t == "BlobWant" { blob_pending = Some(f); } else if f.t == "Ack" { window.acknowledge(&f)?; } else if f.t != "Heartbeat" { return Err(Error::Invalid); } },
            }
        }
    }
}

impl<S: GatewayState + 'static> Server<S> {
    pub async fn listen(
        self: Arc<Self>,
        listener: tokio::net::TcpListener,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), Error> {
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(self.authority.gateway_tls()?));
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            tokio::select! {
                _ = stop.changed() => return Ok(()),
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
                accepted = listener.accept(), if tasks.len() < 64 => {
                    let (socket, _) = accepted?;
                    let tls = tls.clone(); let server = self.clone(); let stop = stop.clone();
                    tasks.spawn(async move {
                        let stream = tokio::time::timeout(Duration::from_secs(5), tls.accept(socket)).await.map_err(|_| Error::Disconnected)??;
                        let certificates = stream.get_ref().1.peer_certificates().ok_or(Error::Invalid)?;
                        if certificates.len() != 1 { return Err(Error::Invalid); }
                        let cert = certificates[0].to_vec();
                        let service = hyper::service::service_fn(move |request| {
                            let server = server.clone(); let cert = cert.clone(); let stop = stop.clone();
                            async move { server.http(request, cert, stop).await }
                        });
                        hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), service).with_upgrades().await.map_err(|e| Error::State(e.to_string()))
                    });
                },
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn http(
        self: Arc<Self>,
        mut request: hyper::Request<hyper::body::Incoming>,
        certificate: Vec<u8>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<hyper::Response<http_body_util::Full<hyper::body::Bytes>>, std::convert::Infallible>
    {
        use http_body_util::BodyExt;
        let result: Result<_, Error> = async {
            if request.method() == hyper::Method::POST && request.uri().path() == "/v1/pair" {
                let bytes = http_body_util::Limited::new(request.into_body(), 4096)
                    .collect()
                    .await
                    .map_err(|_| Error::Invalid)?
                    .to_bytes();
                let join: Join = serde_json::from_slice(&bytes)?;
                let principal = self.authority.accept_pair(
                    &join.id,
                    &STANDARD.decode(join.proof).map_err(|_| Error::Invalid)?,
                    &certificate,
                    false,
                )?;
                let bytes = gojson::legacy_json(&principal, gojson::EscapeMode::Html)
                    .map_err(|_| Error::Invalid)?;
                return Ok(hyper::Response::new(http_body_util::Full::new(
                    bytes.into(),
                )));
            }
            if request.method() == hyper::Method::POST && request.uri().path() == "/v1/commands" {
                let delegation = request.headers().get("X-Apple-Bridge-Delegation").and_then(|value| value.to_str().ok()).ok_or(Error::Invalid)?.to_owned();
                let bytes = http_body_util::Limited::new(request.into_body(), ab_trust::wire::framing::MAX_QUERY_FRAME)
                    .collect().await.map_err(|_| Error::Invalid)?.to_bytes();
                let raw: Box<serde_json::value::RawValue> = serde_json::from_slice(&bytes)?;
                let value: serde_json::Value = serde_json::from_str(raw.get())?;
                if !value["method"].is_string() { return Err(Error::Invalid); }
                let handler = self.uploads.as_ref().ok_or(Error::Invalid)?;
                let reply = handler.control(&certificate, "direct-command", &delegation, &raw).await?;
                let mut response = if let Some(error) = reply.error {
                    let code = serde_json::to_value(error.code)?;
                    let status = match code.as_str() {
                        Some("unauthorized") => hyper::StatusCode::UNAUTHORIZED,
                        Some("forbidden") => hyper::StatusCode::FORBIDDEN,
                        Some("not_found") => hyper::StatusCode::NOT_FOUND,
                        Some("conflict" | "reset") => hyper::StatusCode::CONFLICT,
                        Some("unavailable") => hyper::StatusCode::SERVICE_UNAVAILABLE,
                        _ => hyper::StatusCode::BAD_REQUEST,
                    };
                    let bytes = gojson::legacy_json(&serde_json::json!({"error":error}), gojson::EscapeMode::Html).map_err(|_| Error::Invalid)?;
                    let mut response = hyper::Response::new(http_body_util::Full::new(bytes.into()));
                    *response.status_mut() = status;
                    response
                } else {
                    hyper::Response::new(http_body_util::Full::new(reply.result.ok_or(Error::Invalid)?.get().as_bytes().to_vec().into()))
                };
                response.headers_mut().insert(hyper::header::CONTENT_TYPE, hyper::header::HeaderValue::from_static("application/json"));
                if reply.lookup_only {
                    response.headers_mut().insert("X-Apple-Bridge-Command-Lookup-Only", hyper::header::HeaderValue::from_static("true"));
                }
                return Ok(response);
            }
            if request.method() != hyper::Method::GET || request.uri().path() != "/v1/feed" {
                return Err(Error::Invalid);
            }
            self.authority.authenticate_certificate(&certificate)?;
            let upgrade = hyper::upgrade::on(&mut request);
            let response = tokio_tungstenite::tungstenite::handshake::server::create_response(
                &request.map(|_| ()),
            )?;
            let server = self.clone();
            tokio::spawn(async move {
                if let Ok(upgraded) = upgrade.await {
                    let socket = WebSocketStream::from_raw_socket(
                        hyper_util::rt::TokioIo::new(upgraded),
                        tokio_tungstenite::tungstenite::protocol::Role::Server,
                        Some(
                            tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                                .max_message_size(Some((1 << 20) + 64)),
                        ),
                    )
                    .await;
                    if !*stop.borrow() { tokio::select! { _ = stop.changed() => {}, _ = server.serve(socket, &certificate) => {} } }
                }
            });
            Ok(response.map(|()| http_body_util::Full::new(hyper::body::Bytes::new())))
        }
        .await;
        Ok(result.unwrap_or_else(|_| {
            let mut response = hyper::Response::new(http_body_util::Full::new(
                hyper::body::Bytes::from_static(b"unauthorized"),
            ));
            *response.status_mut() = hyper::StatusCode::UNAUTHORIZED;
            response
        }))
    }
}

impl<S: GatewayState> Server<S> {
    #[allow(clippy::too_many_lines)]
    async fn blob<T: AsyncRead + AsyncWrite + Unpin>(
        &self,
        socket: &mut WebSocketStream<T>,
        p: &Principal,
        mut want: FeedFrame,
        max_chunks: usize,
    ) -> Result<Option<FeedFrame>, Error> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let digest = ab_blobs::Digest::try_from(want.digest.clone())?;
        if want.offset < 0 {
            return Err(Error::Invalid);
        }
        let mut unavailable_reason = "unauthorized";
        let file = async {
            let grant = self
                .state
                .lock()
                .await
                .blob_grant(&p.visibility(), &digest)?;
            unavailable_reason = "unavailable";
            let blobs = self.blobs.as_ref().ok_or(Error::Invalid)?.lock().await;
            unavailable_reason = blobs
                .entries()
                .get(digest.as_str())
                .map_or("missing", |entry| match entry.state {
                    ab_blobs::BlobState::Offloaded => "offloaded",
                    ab_blobs::BlobState::Failed => "failed",
                    ab_blobs::BlobState::Missing => "missing",
                    _ => "pending",
                });
            Ok::<_, Error>(blobs.open_authorized(&grant, &digest)?)
        }
        .await;
        let Ok(file) = file else {
            let mut unavailable = frame("BlobUnavailable");
            unavailable.digest = want.digest;
            unavailable.reason = unavailable_reason.into();
            socket
                .send(Message::Text(
                    String::from_utf8(unavailable.encode()?)
                        .map_err(|_| Error::Invalid)?
                        .into(),
                ))
                .await?;
            return Ok(None);
        };
        let mut file = tokio::fs::File::from_std(file);
        file.seek(std::io::SeekFrom::Start(
            u64::try_from(want.offset).map_err(|_| Error::Invalid)?,
        ))
        .await?;
        let mut offset = want.offset;
        let mut buffer = vec![0; ab_trust::wire::framing::BLOB_CHUNK_SIZE];
        let digest_bytes: [u8; 32] = hex::decode(digest.as_str())
            .map_err(|_| Error::Invalid)?
            .try_into()
            .map_err(|_| Error::Invalid)?;
        let mut chunks = 0;
        loop {
            let count = file.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            let chunk = ab_trust::wire::framing::BlobChunk {
                digest: digest_bytes,
                offset,
                bytes: buffer[..count].to_vec(),
            };
            let mut encoded = chunk.encode()?;
            #[cfg(feature = "lab-harness")]
            {
                use std::os::unix::fs::OpenOptionsExt;
                if let Ok(path) = std::env::var("APPLE_BRIDGE_LAB_BLOB_CORRUPT")
                    && encoded.len() > 64
                    && std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(path)
                        .is_ok()
                {
                    encoded[64] ^= 0xff;
                }
                if std::env::var_os("APPLE_BRIDGE_LAB_BLOB_SLOW").is_some() {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                if let Ok(path) = std::env::var("APPLE_BRIDGE_LAB_BLOB_CUT_WINDOW")
                    && offset == ab_trust::wire::framing::BLOB_CHUNK_SIZE as i64
                    && std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(format!("{path}.once"))
                        .is_ok()
                {
                    use tokio_tungstenite::tungstenite::protocol::frame::{
                        Frame,
                        coding::{Data, OpCode},
                    };
                    let halfway = encoded.len() / 2;
                    socket
                        .send(Message::Frame(Frame::message(
                            encoded[..halfway].to_vec(),
                            OpCode::Data(Data::Binary),
                            false,
                        )))
                        .await?;
                    std::fs::write(path, b"in_flight")?;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    socket
                        .send(Message::Frame(Frame::message(
                            encoded[halfway..].to_vec(),
                            OpCode::Data(Data::Continue),
                            true,
                        )))
                        .await?;
                } else {
                    socket.send(Message::Binary(encoded.into())).await?;
                }
            }
            #[cfg(not(feature = "lab-harness"))]
            socket
                .send(Message::Binary(std::mem::take(&mut encoded).into()))
                .await?;
            offset += i64::try_from(count).map_err(|_| Error::Invalid)?;
            chunks += 1;
            if chunks >= max_chunks {
                want.offset = offset;
                return Ok(Some(want));
            }
        }
        let mut done = frame("BlobDone");
        done.digest = want.digest;
        socket
            .send(Message::Text(
                String::from_utf8(done.encode()?)
                    .map_err(|_| Error::Invalid)?
                    .into(),
            ))
            .await?;
        Ok(None)
    }
}

impl<S: GatewayState + 'static> Server<S> {
    pub fn relay_http<B>(
        self: Arc<Self>,
        mut request: hyper::Request<B>,
        binding: Option<[u8; 32]>,
    ) -> hyper::Response<http_body_util::Full<hyper::body::Bytes>> {
        let upgrade = hyper::upgrade::on(&mut request);
        let request = request.map(|_| ());
        let result = (|| {
            let binding = binding.ok_or(Error::Invalid)?;
            if request.method() != hyper::Method::GET || request.headers().contains_key("origin") {
                return Err(Error::Invalid);
            }
            let response =
                tokio_tungstenite::tungstenite::handshake::server::create_response(&request)?;
            Ok::<_, Error>((response, binding))
        })();
        let Ok((response, binding)) = result else {
            let mut response =
                hyper::Response::new(http_body_util::Full::new(hyper::body::Bytes::new()));
            *response.status_mut() = hyper::StatusCode::UNAUTHORIZED;
            return response;
        };
        tokio::spawn(async move {
            if let Ok(upgraded) = upgrade.await {
                let mut ws = WebSocketStream::from_raw_socket(
                    hyper_util::rt::TokioIo::new(upgraded),
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    Some(
                        tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                            .max_message_size(Some((1 << 20) + 64)),
                    ),
                )
                .await;
                let auth = tokio::time::timeout(
                    Duration::from_secs(5),
                    crate::relay::server(&self.authority, &mut ws, &binding),
                )
                .await;
                if let Ok(Ok(cert)) = auth {
                    let _ = self.serve(ws, &cert).await;
                }
            }
        });
        response.map(|()| http_body_util::Full::new(hyper::body::Bytes::new()))
    }
}
