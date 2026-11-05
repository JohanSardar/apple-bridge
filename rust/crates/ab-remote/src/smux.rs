use crate::{Error, Gateway, Result, certificate::Certificates};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::{HashMap, VecDeque},
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf},
    sync::{Notify, mpsc, watch},
    task::JoinSet,
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
use tokio_util::sync::CancellationToken;

const STREAM_WINDOW: u32 = 256 << 10;
const WRITE_CHUNK: usize = 16_384;
const MAX_STREAMS: usize = 32;
const TLS_HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);
const CLOSE_GRACE: Duration = Duration::from_secs(5);
const DRAIN_DEADLINE: Duration = Duration::from_secs(30);

fn frame(command: u8, id: u32, data: &[u8]) -> Result<Vec<u8>> {
    let len = u16::try_from(data.len()).map_err(|_| Error::Contract("smux frame too large"))?;
    let mut out = vec![2, command];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(data);
    Ok(out)
}

#[derive(Debug, Default)]
struct InboxQueue {
    bytes: VecDeque<u8>,
    closed: bool,
}
#[derive(Debug, Default)]
struct Inbox {
    queue: Mutex<InboxQueue>,
    ready: Notify,
    closed: CancellationToken,
}
impl Inbox {
    fn push(&self, data: &[u8]) -> bool {
        let Ok(mut queue) = self.queue.lock() else {
            return false;
        };
        if queue.bytes.len() + data.len() > STREAM_WINDOW as usize {
            return false;
        }
        queue.bytes.extend(data);
        drop(queue);
        self.ready.notify_one();
        true
    }
    fn close(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.closed = true;
        }
        self.ready.notify_one();
        self.closed.cancel();
    }
    async fn next(&self) -> Option<Vec<u8>> {
        loop {
            {
                let mut queue = self.queue.lock().ok()?;
                if !queue.bytes.is_empty() {
                    let take = queue.bytes.len().min(WRITE_CHUNK);
                    return Some(queue.bytes.drain(..take).collect());
                }
                if queue.closed {
                    return None;
                }
            }
            self.ready.notified().await;
        }
    }
}

struct Stream {
    inbox: Arc<Inbox>,
    window: watch::Sender<(u32, u32)>,
}

struct Worker {
    id: u32,
    tx: mpsc::Sender<Vec<u8>>,
    gateway: Gateway,
    certificates: Arc<Certificates>,
    source: Option<IpAddr>,
}

#[allow(clippy::too_many_lines)]
pub async fn serve<S>(
    ws: WebSocketStream<S>,
    gateway: Gateway,
    certificates: Arc<Certificates>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
    let mut writer = JoinSet::new();
    writer.spawn(async move {
        while let Some(bytes) = rx.recv().await {
            sink.send(Message::Binary(bytes.into())).await?;
        }
        Ok::<(), Error>(())
    });
    let mut tasks = JoinSet::<u32>::new();
    let mut streams = HashMap::<u32, Stream>::new();
    let mut buffered = Vec::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    let mut last_seen = tokio::time::Instant::now();
    loop {
        let message = tokio::select! {
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(3) { return Err(Error::Contract("smux keepalive timeout")); }
                if matches!(tx.try_send(frame(3, 0, &[])?), Err(mpsc::error::TrySendError::Closed(_))) {
                    return Err(Error::Contract("smux closed"));
                }
                continue;
            }
            _ = writer.join_next() => return Err(Error::Contract("smux worker ended")),
            finished = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok(id)) = finished { streams.remove(&id); }
                continue;
            }
            message = source.next() => message,
        };
        let Some(message) = message else {
            return Ok(());
        };
        match message? {
            Message::Binary(bytes) => buffered.extend_from_slice(&bytes),
            Message::Close(_) => return Ok(()),
            Message::Ping(_) | Message::Pong(_) => continue,
            _ => return Err(Error::Contract("smux requires binary WebSocket")),
        }
        last_seen = tokio::time::Instant::now();
        while buffered.len() >= 8 {
            if buffered[0] != 2 {
                return Err(Error::Contract("smux version"));
            }
            let n = usize::from(u16::from_le_bytes([buffered[2], buffered[3]]));
            if buffered.len() < n + 8 {
                break;
            }
            let id = u32::from_le_bytes(
                buffered[4..8]
                    .try_into()
                    .map_err(|_| Error::Contract("smux header"))?,
            );
            let cmd = buffered[1];
            let payload = buffered[8..8 + n].to_vec();
            buffered.drain(..8 + n);
            match cmd {
                0 if n == 0 || n == 4 || n == 16 => {
                    if streams.contains_key(&id) {
                        return Err(Error::Contract("smux stream reused"));
                    }
                    if streams.len() >= MAX_STREAMS {
                        if tasks.len() >= MAX_STREAMS * 2 {
                            return Err(Error::Contract("smux stream limit"));
                        }
                        let tx = tx.clone();
                        let close = frame(1, id, &[])?;
                        tasks.spawn(async move {
                            let _ = tx.send(close).await;
                            id
                        });
                        continue;
                    }
                    let inbox = Arc::new(Inbox::default());
                    let (window, window_rx) = watch::channel((0_u32, STREAM_WINDOW));
                    streams.insert(
                        id,
                        Stream {
                            inbox: inbox.clone(),
                            window,
                        },
                    );
                    let worker = Worker {
                        id,
                        tx: tx.clone(),
                        gateway: gateway.clone(),
                        certificates: certificates.clone(),
                        source: ab_wire::framing::decode_tunnel_source(&payload),
                    };
                    tasks.spawn(worker.run(inbox, window_rx));
                }
                1 if n == 0 => {
                    if let Some(stream) = streams.get(&id) {
                        stream.inbox.close();
                    }
                }
                2 => {
                    if let Some(stream) = streams.get(&id)
                        && !stream.inbox.push(&payload)
                    {
                        stream.inbox.close();
                    }
                }
                3 if n == 0 => {}
                4 if n == 8 => {
                    if let Some(stream) = streams.get(&id) {
                        let consumed = u32::from_le_bytes(
                            payload[..4]
                                .try_into()
                                .map_err(|_| Error::Contract("smux update"))?,
                        );
                        let size = u32::from_le_bytes(
                            payload[4..]
                                .try_into()
                                .map_err(|_| Error::Contract("smux update"))?,
                        );
                        let _ = stream.window.send((consumed, size));
                    }
                }
                _ => return Err(Error::Contract("smux command")),
            }
        }
    }
}

impl Worker {
    async fn run(self, inbox: Arc<Inbox>, window: watch::Receiver<(u32, u32)>) -> u32 {
        let (local, tunnel) = tokio::io::duplex(STREAM_WINDOW as usize);
        let (input, output) = tokio::io::split(tunnel);
        let peer_closed = inbox.closed.clone();
        let finishing = CancellationToken::new();
        let forward = {
            let finishing = finishing.clone();
            let gateway = self.gateway.clone();
            let certificates = self.certificates.clone();
            let source = self.source;
            async move {
                let _ = Box::pin(terminate(local, gateway, certificates, source)).await;
                finishing.cancel();
            }
        };
        let send = {
            let finishing = finishing.clone();
            let tx = self.tx.clone();
            let id = self.id;
            async move {
                let _ = outbound(id, input, tx, window).await;
                finishing.cancel();
            }
        };
        let expired = async {
            tokio::select! {
                () = peer_closed.cancelled() => tokio::time::sleep(CLOSE_GRACE).await,
                () = finishing.cancelled() => tokio::time::sleep(DRAIN_DEADLINE).await,
            }
        };
        let deliver = async {
            let _ = inbound(self.id, &inbox, output, &self.tx).await;
            std::future::pending::<()>().await;
        };
        tokio::select! {
            _ = async { tokio::join!(forward, send) } => {}
            () = deliver => {}
            () = expired => {}
        }
        if let Ok(close) = frame(1, self.id, &[]) {
            let _ = self.tx.send(close).await;
        }
        self.id
    }
}

async fn terminate(
    local: DuplexStream,
    gateway: Gateway,
    certificates: Arc<Certificates>,
    source: Option<IpAddr>,
) -> Result<()> {
    let acceptor = tokio_rustls::TlsAcceptor::from(certificates.config()?);
    let mut secure = tokio::time::timeout(TLS_HANDSHAKE_DEADLINE, acceptor.accept(local))
        .await
        .map_err(|_| Error::Contract("smux TLS handshake timeout"))??;
    let binding = secure.get_ref().1.export_keying_material(
        [0; 32],
        b"EXPORTER-Channel-Binding",
        Some(&[]),
    )?;
    let mut upstream = gateway.connect_stream(Some(binding), source).await?;
    tokio::io::copy_bidirectional(&mut secure, &mut upstream).await?;
    Ok(())
}

async fn inbound(
    id: u32,
    inbox: &Inbox,
    mut output: WriteHalf<DuplexStream>,
    tx: &mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    let mut consumed = 0_u32;
    while let Some(data) = inbox.next().await {
        output.write_all(&data).await?;
        consumed = consumed
            .wrapping_add(u32::try_from(data.len()).map_err(|_| Error::Contract("smux length"))?);
        let mut update = consumed.to_le_bytes().to_vec();
        update.extend_from_slice(&STREAM_WINDOW.to_le_bytes());
        tx.send(frame(4, id, &update)?)
            .await
            .map_err(|_| Error::Contract("smux closed"))?;
    }
    output.shutdown().await?;
    Ok(())
}

async fn outbound(
    id: u32,
    mut input: ReadHalf<DuplexStream>,
    tx: mpsc::Sender<Vec<u8>>,
    mut window: watch::Receiver<(u32, u32)>,
) -> Result<()> {
    let mut sent = 0_u32;
    let mut buf = vec![0; 16384];
    loop {
        let (consumed, size) = *window.borrow_and_update();
        let available = size.saturating_sub(sent.wrapping_sub(consumed));
        if available == 0 {
            window
                .changed()
                .await
                .map_err(|_| Error::Contract("smux closed"))?;
            continue;
        }
        let length = buf
            .len()
            .min(usize::try_from(available).map_err(|_| Error::Contract("smux window"))?);
        let n = input.read(&mut buf[..length]).await?;
        if n == 0 {
            return Ok(());
        }
        tx.send(frame(2, id, &buf[..n])?)
            .await
            .map_err(|_| Error::Contract("smux closed"))?;
        sent = sent.wrapping_add(u32::try_from(n).map_err(|_| Error::Contract("smux length"))?);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::relay::{Credential, Identity};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use tokio_tungstenite::tungstenite::protocol::Role;

    async fn loaded_certificates(home: &std::path::Path) -> Arc<Certificates> {
        let host = "mac.relay.test";
        let certified = rcgen::generate_simple_self_signed(vec![host.to_owned()]).unwrap();
        let remote = home.join("remote");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("relay-cert.pem"), certified.cert.pem()).unwrap();
        std::fs::write(
            remote.join("relay-key.pem"),
            certified.signing_key.serialize_pem(),
        )
        .unwrap();
        let certificates = Arc::new(Certificates::new(
            home.to_owned(),
            "https://acme.invalid/directory".into(),
        ));
        let key = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
        let identity = Identity {
            key: STANDARD.encode(key.to_keypair_bytes()),
            credential: Credential {
                hostname: host.into(),
                credential: "c".into(),
            },
            endpoint: "https://relay.invalid".into(),
        };
        let reported = certificates
            .ensure(&identity, host, &reqwest::Client::new())
            .await;
        assert!(matches!(reported, Err(Error::Http(_))), "{reported:?}");
        certificates.config().unwrap();
        certificates
    }

    #[tokio::test]
    async fn a_flood_of_stream_opens_past_the_limit_ends_the_tunnel_instead_of_growing_tasks() {
        let home = tempfile::tempdir().unwrap();
        let certificates = loaded_certificates(home.path()).await;
        let (mac_side, relay_side) = tokio::io::duplex(4096);
        let mac = WebSocketStream::from_raw_socket(mac_side, Role::Server, None).await;
        let mut relay = WebSocketStream::from_raw_socket(relay_side, Role::Client, None).await;
        let gateway = Gateway::new("127.0.0.1:1".parse().unwrap());
        let serving = tokio::spawn(serve(mac, gateway, certificates));
        let flood = async {
            for id in 0..u32::try_from(MAX_STREAMS * 200).unwrap() {
                let opened = frame(0, id * 2 + 2, &[]).unwrap();
                if relay.send(Message::Binary(opened.into())).await.is_err() {
                    break;
                }
            }
            std::future::pending::<()>().await;
        };
        let ended = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::select! {
                result = serving => result.unwrap(),
                () = flood => unreachable!(),
            }
        })
        .await;
        assert!(
            matches!(ended, Ok(Err(Error::Contract("smux stream limit")))),
            "{ended:?}"
        );
    }
}
