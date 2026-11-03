use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct Open {
    pub reply: oneshot::Sender<DuplexStream>,
}
#[derive(Debug)]
struct Incoming {
    data: Vec<u8>,
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StreamId(u32);

#[derive(Debug)]
struct Stream {
    incoming: Option<mpsc::Sender<Incoming>>,
    window: watch::Sender<(u32, u32)>,
}
#[derive(Debug)]
struct Frame {
    cmd: u8,
    id: StreamId,
    data: Vec<u8>,
}
impl Frame {
    fn encode(self) -> Option<Vec<u8>> {
        let len = u16::try_from(self.data.len()).ok()?;
        let mut b = vec![2, self.cmd];
        b.extend_from_slice(&len.to_le_bytes());
        b.extend_from_slice(&self.id.0.to_le_bytes());
        b.extend(self.data);
        Some(b)
    }
}
async fn stream_task(
    id: StreamId,
    io: DuplexStream,
    mut incoming: mpsc::Receiver<Incoming>,
    mut window: watch::Receiver<(u32, u32)>,
    frames: mpsc::Sender<Frame>,
    cancel: CancellationToken,
) {
    let (mut r, mut w) = tokio::io::split(io);
    let tx = frames.clone();
    let receive = async move {
        let mut consumed = 0u32;
        while let Some(data) = incoming.recv().await {
            w.write_all(&data.data).await?;
            consumed = consumed.wrapping_add(u32::try_from(data.data.len()).unwrap_or(0));
            let mut update = consumed.to_le_bytes().to_vec();
            update.extend_from_slice(&262_144_u32.to_le_bytes());
            if tx
                .send(Frame {
                    cmd: 4,
                    id,
                    data: update,
                })
                .await
                .is_err()
            {
                break;
            }
        }
        Ok::<_, std::io::Error>(())
    };
    let send = async move {
        let mut written = 0u32;
        let mut buf = [0; 4096];
        loop {
            let available = loop {
                let (consumed, size) = *window.borrow_and_update();
                let in_flight = written.wrapping_sub(consumed);
                if let Some(n) = size.checked_sub(in_flight).filter(|n| *n > 0) {
                    break usize::try_from(n).unwrap_or(4096).min(4096);
                }
                if window.changed().await.is_err() {
                    return Ok(());
                }
            };
            let n = r.read(&mut buf[..available]).await?;
            if n == 0 {
                break;
            }
            if frames
                .send(Frame {
                    cmd: 2,
                    id,
                    data: buf[..n].to_vec(),
                })
                .await
                .is_err()
            {
                break;
            }
            written = written.wrapping_add(u32::try_from(n).unwrap_or(0));
        }
        Ok::<_, std::io::Error>(())
    };
    tokio::select! { () = cancel.cancelled() => {}, _ = receive => {}, _ = send => {} }
}
pub async fn run(ws: WebSocket, mut opens: mpsc::Receiver<Open>, cancel: CancellationToken) {
    let (mut sink, mut source) = ws.split();
    let (frames, mut output) = mpsc::channel::<Frame>(64);
    let writer_cancel = cancel.clone();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = writer_cancel.cancelled() => break,
                frame = output.recv() => {
                    let Some(frame) = frame.and_then(Frame::encode) else { break };
                    if sink.send(Message::Binary(frame.into())).await.is_err() { break }
                }
            }
        }
        writer_cancel.cancel();
    });
    let mut streams = HashMap::<StreamId, Stream>::new();
    let mut workers = tokio::task::JoinSet::new();
    let permits = Arc::new(Semaphore::new(256));
    let buffer = Arc::new(Semaphore::new(4 << 20));
    let mut next = 0u32;
    let mut pending = Vec::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(1));
    let mut last = Instant::now();
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = heartbeat.tick() => {
                if last.elapsed() >= Duration::from_secs(3) { break }
                if frames.send(Frame { cmd:3,id:StreamId(0),data:Vec::new() }).await.is_err() { break }
            }
            Some(result) = workers.join_next(), if !workers.is_empty() => {
                if let Ok(id) = result {
                    streams.remove(&id);
                    if frames.send(Frame { cmd:1,id,data:Vec::new() }).await.is_err() { break }
                }
            }
            request = opens.recv() => {
                let Some(request) = request else { break };
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue };
                let Some(raw_id) = next.checked_add(2) else { break }; next = raw_id; let id=StreamId(raw_id);
                let (local,remote) = tokio::io::duplex(262_144);
                let (incoming,rx) = mpsc::channel(16);
                let (window,wrx) = watch::channel((0,262_144));
                let child = cancel.child_token();
                streams.insert(id,Stream { incoming:Some(incoming),window });
                if frames.send(Frame { cmd:0,id,data:Vec::new() }).await.is_err() { break }
                let tx = frames.clone();
                workers.spawn(async move { let _permit = permit; stream_task(id,remote,rx,wrx,tx,child).await; id });
                let _ = request.reply.send(local);
            }
            message = source.next() => {
                match message {
                    Some(Ok(Message::Binary(b))) => pending.extend_from_slice(&b),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => break,
                }
                last = Instant::now();
                let mut offset = 0;
                let mut bad = false;
                while pending.len()-offset >= 8 {
                    let h = &pending[offset..offset+8];
                    let n = usize::from(u16::from_le_bytes([h[2],h[3]]));
                    if pending.len()-offset < 8+n { break }
                    let id = StreamId(u32::from_le_bytes([h[4],h[5],h[6],h[7]])); let cmd = h[1];
                    if h[0] != 2 { bad = true; break }
                    let data = &pending[offset+8..offset+8+n];
                    match cmd {
                        1 if n==0 => { if let Some(s) = streams.get_mut(&id) { s.incoming.take(); } }
                        2 => {
                            if let Some(s) = streams.get(&id) && let Some(incoming_tx)=&s.incoming {
                                let Ok(size) = u32::try_from(n) else { bad = true; break };
                                let permit = tokio::select! { () = cancel.cancelled() => break, result = buffer.clone().acquire_many_owned(size) => result };
                                let Ok(permit) = permit else { bad = true; break };
                                let incoming = Incoming { data:data.to_vec(), _permit:permit };
                                let sent = tokio::select! { () = cancel.cancelled() => break, result = incoming_tx.send(incoming) => result };
                                if sent.is_err() { bad = true; break }
                                last=Instant::now();
                            }
                        }
                        3 if n==0 => {},
                        4 if n == 8 => { if let Some(s) = streams.get(&id) { let consumed = u32::from_le_bytes([data[0],data[1],data[2],data[3]]); let size = u32::from_le_bytes([data[4],data[5],data[6],data[7]]); s.window.send_replace((consumed,size)); } }
                        _ => { bad = true; break }
                    }
                    offset += 8+n;
                }
                pending.drain(..offset);
                if bad || pending.len() > 65543 { break }
            }
        }
    }
    cancel.cancel();
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    let _ = writer.await;
}
