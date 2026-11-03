use crate::{Error, Gateway, Result, certificate::Certificates};
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, watch},
    task::JoinSet,
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

fn frame(command: u8, id: u32, data: &[u8]) -> Result<Vec<u8>> {
    let len = u16::try_from(data.len()).map_err(|_| Error::Contract("smux frame too large"))?;
    let mut out = vec![2, command];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(data);
    Ok(out)
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
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Some(bytes) = rx.recv().await {
            sink.send(Message::Binary(bytes.into())).await?;
        }
        Ok::<(), Error>(())
    });
    let mut streams = HashMap::new();
    let mut buffered = Vec::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    let mut last_seen = tokio::time::Instant::now();
    loop {
        let message = tokio::select! {
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(3) { return Err(Error::Contract("smux keepalive timeout")); }
                tx.send(frame(3, 0, &[])?).await.map_err(|_| Error::Contract("smux closed"))?;
                continue;
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                match completed { Some(Ok(Ok(()))) => continue, _ => return Err(Error::Contract("smux worker ended")) }
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
                0 if n == 0 => {
                    if streams.len() >= 32 || streams.contains_key(&id) {
                        return Err(Error::Contract("smux stream limit"));
                    }
                    let (local, tunnel) = tokio::io::duplex(256 << 10);
                    let (input, output) = tokio::io::split(tunnel);
                    let (window, window_rx) = watch::channel((0_u32, 256_u32 << 10));
                    streams.insert(id, (output, window, 0_u32));
                    let tx = tx.clone();
                    let gateway = gateway.clone();
                    let certs = certificates.clone();
                    tasks.spawn(async move {
                        let forward = async {
                            let secure = tokio_rustls::TlsAcceptor::from(certs.config()?)
                                .accept(local)
                                .await?;
                            let binding = secure.get_ref().1.export_keying_material(
                                [0; 32],
                                b"EXPORTER-Channel-Binding",
                                Some(&[]),
                            )?;
                            let (mut secure, mut upstream) =
                                (secure, gateway.connect_bound(Some(binding)).await?);
                            tokio::io::copy_bidirectional(&mut secure, &mut upstream).await?;
                            Ok::<(), Error>(())
                        };
                        let send = outbound(id, input, tx.clone(), window_rx);
                        let _ = tokio::join!(forward, send);
                        let _ = tx.send(frame(1, id, &[])?).await;
                        Ok(())
                    });
                }
                1 if n == 0 => {
                    streams.remove(&id);
                }
                2 => {
                    if let Some((writer, _, consumed)) = streams.get_mut(&id) {
                        if writer.write_all(&payload).await.is_err() {
                            streams.remove(&id);
                            continue;
                        }
                        *consumed = consumed.wrapping_add(
                            u32::try_from(n).map_err(|_| Error::Contract("smux length"))?,
                        );
                        let mut update = consumed.to_le_bytes().to_vec();
                        update.extend_from_slice(&(256_u32 << 10).to_le_bytes());
                        tx.send(frame(4, id, &update)?)
                            .await
                            .map_err(|_| Error::Contract("smux closed"))?;
                    }
                }
                3 if n == 0 => {}
                4 if n == 8 => {
                    if let Some((_, window, _)) = streams.get(&id) {
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
                        let _ = window.send((consumed, size));
                    }
                }
                _ => return Err(Error::Contract("smux command")),
            }
        }
    }
}
async fn outbound(
    id: u32,
    mut input: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    tx: mpsc::Sender<Vec<u8>>,
    mut window: watch::Receiver<(u32, u32)>,
) -> Result<()> {
    let mut sent = 0_u32;
    let mut buf = [0; 16384];
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
