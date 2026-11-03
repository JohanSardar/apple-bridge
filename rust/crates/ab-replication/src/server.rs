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
    principal: &Principal,
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
            let ack = receive(socket).await?;
            if ack.t == "BlobWant" {
                server.blob(socket, principal, ack).await?;
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
    pub async fn serve<T: AsyncRead + AsyncWrite + Unpin>(
        &self,
        mut socket: WebSocketStream<T>,
        certificate: &[u8],
    ) -> Result<(), Error> {
        let principal = self.authority.authenticate_certificate(certificate)?;
        let session = self.session(&mut socket, &principal);
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
        p: &Principal,
    ) -> Result<(), Error> {
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
        transmit(self, p, socket, welcome, None, limit).await?;
        let cursor = if resume {
            previous.ok_or(Error::Invalid)?
        } else {
            if previous.is_some_and(|c| c.sequence > 0) {
                let mut reset = frame("Reset");
                reset.reason = "history_or_scope_changed".into();
                reset.retention_floor = floor;
                transmit(self, p, socket, reset, None, limit).await?;
            }
            let mut snapshot = self
                .state
                .lock()
                .await
                .freeze_snapshot(&visibility, p.revision)?;
            let begin = snapshot.begin();
            let id = begin.id.clone();
            transmit(self, p, socket, begin, None, limit).await?;
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
                transmit(self, p, socket, page, None, limit).await?;
                offset = end;
                ordinal += 1;
            }
            let cursor = snapshot.cursor();
            let mut end = frame("SnapshotEnd");
            end.id = id;
            transmit(self, p, socket, end, Some(&cursor), limit).await?;
            cursor
        };
        let mut journal = cursor.clone();
        journal.subscription_revision = head.subscription_revision;
        let mut iterator = self
            .state
            .lock()
            .await
            .changes(&visibility, journal, atomic)?;
        let mut poll = tokio::time::interval(Duration::from_millis(20));
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                _ = poll.tick() => {
                    let next = self.state.lock().await.next_changes(&mut iterator);
                    match next {
                        Ok(Some(mut batch)) => {
                            batch.subscription_revision = p.revision;
                            let expected = Cursor { sequence: batch.through, ..cursor.clone() };
                            let partial = batch.partial;
                            let mut f = frame("Batch"); f.batch = Some(batch);
                            transmit(self, p, socket, f, if partial { None } else { Some(&expected) }, limit).await?;
                        },
                        Ok(None) => {},
                        Err(Error::Contract(ab_trust::domain::ErrorCode::Reset)) => {
                            let mut f = frame("Reset"); f.reason = "retention".into(); f.retention_floor = self.state.lock().await.retention_floor()?;
                            socket.send(Message::Text(String::from_utf8(f.encode()?).map_err(|_| Error::Invalid)?.into())).await?;
                            return Err(Error::Contract(ab_trust::domain::ErrorCode::Reset));
                        },
                        Err(e) => return Err(e),
                    }
                },
                _ = heartbeat.tick() => {
                    let mut f = frame("Heartbeat"); f.head = self.state.lock().await.head()?.sequence;
                    socket.send(Message::Text(String::from_utf8(f.encode()?).map_err(|_| Error::Invalid)?.into())).await?;
                },
                f = receive(socket) => { let f = f?; if f.t == "BlobWant" { self.blob(socket, p, f).await?; } else if f.t != "Heartbeat" { return Err(Error::Invalid); } },
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
                                .max_message_size(Some(256 << 10)),
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
    async fn blob<T: AsyncRead + AsyncWrite + Unpin>(
        &self,
        socket: &mut WebSocketStream<T>,
        p: &Principal,
        want: FeedFrame,
    ) -> Result<(), Error> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let digest = ab_blobs::Digest::try_from(want.digest.clone())?;
        if want.offset < 0 {
            return Err(Error::Invalid);
        }
        let file = async {
            let grant = self
                .state
                .lock()
                .await
                .blob_grant(&p.visibility(), &digest)?;
            let blobs = self.blobs.as_ref().ok_or(Error::Invalid)?;
            Ok::<_, Error>(blobs.lock().await.open_authorized(&grant, &digest)?)
        }
        .await;
        let Ok(file) = file else {
            let mut unavailable = frame("BlobUnavailable");
            unavailable.digest = want.digest;
            unavailable.reason = "unavailable".into();
            socket
                .send(Message::Text(
                    String::from_utf8(unavailable.encode()?)
                        .map_err(|_| Error::Invalid)?
                        .into(),
                ))
                .await?;
            return Ok(());
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
            socket.send(Message::Binary(chunk.encode()?.into())).await?;
            offset += i64::try_from(count).map_err(|_| Error::Invalid)?;
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
        Ok(())
    }
}
