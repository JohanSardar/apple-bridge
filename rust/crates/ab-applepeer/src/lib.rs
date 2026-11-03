#![forbid(unsafe_code)]
mod server;
mod supervisor;
use ab_operations::{Code, Cursor, Error, Task, peer::ApplePeer};
use ab_wire::messages::{Capture, IpcFrame};
pub use server::{Backend, Server};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};
pub use supervisor::{HelperConfig, supervise};
use tokio::{
    io::AsyncWriteExt,
    net::unix::OwnedWriteHalf,
    sync::{Mutex, oneshot, watch},
};

#[derive(Debug, Default)]
struct Session {
    fence: u64,
    writer: Option<Arc<Mutex<OwnedWriteHalf>>>,
    waiting: BTreeMap<String, (u64, oneshot::Sender<IpcFrame>)>,
    receipts: BTreeMap<String, (Cursor, Vec<String>)>,
    order: VecDeque<String>,
}
#[derive(Debug, Clone)]
pub struct Commands {
    session: Arc<Mutex<Session>>,
    revision: watch::Sender<u64>,
    requests: Arc<std::sync::atomic::AtomicU64>,
    connected: Arc<std::sync::atomic::AtomicBool>,
}
impl Default for Commands {
    fn default() -> Self {
        Self {
            session: Arc::default(),
            revision: watch::channel(0).0,
            requests: Arc::default(),
            connected: Arc::default(),
        }
    }
}
async fn send(writer: &Mutex<OwnedWriteHalf>, frame: &IpcFrame) -> Result<(), Error> {
    let bytes = frame.encode().map_err(|_| Code::Invalid)?;
    tokio::time::timeout(Duration::from_secs(10), async {
        writer
            .lock()
            .await
            .write_all(&bytes)
            .await
            .map_err(|_| Error::from(Code::Unavailable))
    })
    .await
    .map_err(|_| Code::Unavailable)?
}
impl Commands {
    async fn attach(&self, fence: u64, writer: Arc<Mutex<OwnedWriteHalf>>) {
        let mut s = self.session.lock().await;
        self.connected
            .store(true, std::sync::atomic::Ordering::SeqCst);
        *s = Session {
            fence,
            writer: Some(writer),
            ..Session::default()
        };
    }
    async fn detach(&self, fence: u64) {
        let mut s = self.session.lock().await;
        if s.fence == fence {
            self.connected
                .store(false, std::sync::atomic::Ordering::SeqCst);
            *s = Session::default();
        }
    }
    async fn receive(&self, fence: u64, frame: IpcFrame) {
        let mut s = self.session.lock().await;
        if s.fence != fence {
            return;
        }
        let id = if frame.id.is_empty() {
            &frame.command_id
        } else {
            &frame.id
        };
        if let Some((_, tx)) = s.waiting.remove(id) {
            let _ = tx.send(frame);
        }
    }
    async fn record(&self, fence: u64, capture: &Capture, cursor: Cursor) {
        let mut s = self.session.lock().await;
        if s.fence != fence {
            return;
        }
        let source = serde_json::to_value(capture.source)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let key = format!("{source}/{}", capture.checkpoint);
        if !s.receipts.contains_key(&key) {
            s.order.push_back(key.clone());
        }
        let receipt = s
            .receipts
            .entry(key)
            .or_insert_with(|| (cursor.clone(), Vec::new()));
        receipt.0 = cursor;
        for observation in capture.observations.iter().flatten() {
            if matches!(observation.kind.as_str(), "upsert" | "absent") {
                receipt.1.push(format!(
                    "{}/{}",
                    observation.collection, observation.native_key
                ));
            }
        }
        if s.order.len() > 4096
            && let Some(key) = s.order.pop_front()
        {
            s.receipts.remove(&key);
        }
        self.revision.send_modify(|v| *v = v.wrapping_add(1));
    }
}
#[derive(Debug)]
struct Pending {
    commands: Commands,
    fence: u64,
    id: String,
    query: bool,
    request: u64,
}
impl Drop for Pending {
    fn drop(&mut self) {
        let commands = self.commands.clone();
        let fence = self.fence;
        let id = self.id.clone();
        let query = self.query;
        let request = self.request;
        tokio::spawn(async move {
            let mut s = commands.session.lock().await;
            if s.fence != fence
                || s.waiting
                    .get(&id)
                    .is_none_or(|(token, _)| *token != request)
            {
                return;
            }
            s.waiting.remove(&id);
            let writer = s.writer.clone();
            drop(s);
            if query
                && let Some(writer) = writer
                && tokio::time::timeout(
                    Duration::from_millis(50),
                    send(
                        &writer,
                        &IpcFrame {
                            t: "QueryCancel".into(),
                            id,
                            ..IpcFrame::default()
                        },
                    ),
                )
                .await
                .is_err()
            {
                commands.detach(fence).await;
            }
        });
    }
}
impl ApplePeer for Commands {
    fn available(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn exchange(&self, frame: IpcFrame) -> Task<'_, IpcFrame> {
        Box::pin(self.exchange_inner(frame, false))
    }
    fn captured(
        &self,
        source: &str,
        checkpoint: &str,
        collection: &str,
        key: &str,
    ) -> Result<Cursor, Error> {
        let s = self.session.try_lock().map_err(|_| Code::Unavailable)?;
        s.receipts
            .get(&format!("{source}/{checkpoint}"))
            .filter(|(_, keys)| keys.contains(&format!("{collection}/{key}")))
            .map(|(cursor, _)| cursor.clone())
            .ok_or_else(|| Code::NotFound.into())
    }
    fn capture_revision(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }
}

impl Commands {
    #[cfg(feature = "lab-harness")]
    pub async fn exchange_unfenced(&self, frame: IpcFrame) -> Result<IpcFrame, Error> {
        self.exchange_inner(frame, true).await
    }
    async fn exchange_inner(
        &self,
        mut frame: IpcFrame,
        preserve_fence: bool,
    ) -> Result<IpcFrame, Error> {
        let id = if frame.id.is_empty() {
            frame.command_id.clone()
        } else {
            frame.id.clone()
        };
        let mut s = self.session.lock().await;
        let writer = s.writer.clone().ok_or(Code::Unavailable)?;
        if s.waiting.len() >= 64 {
            return Err(Code::Capacity.into());
        }
        if s.waiting.contains_key(&id) {
            return Err(Code::Conflict.into());
        }
        if frame.t == "Dispatch" && !preserve_fence {
            frame.owner_fence = s.fence;
        }
        let request = self
            .requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let guard = Pending {
            request,
            commands: self.clone(),
            fence: s.fence,
            id: id.clone(),
            query: frame.t == "Query",
        };
        let (tx, rx) = oneshot::channel();
        s.waiting.insert(id, (request, tx));
        drop(s);
        send(&writer, &frame).await?;
        let answer = rx.await.map_err(|_| Error::from(Code::Unavailable));
        drop(guard);
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_exchange_cannot_remove_next_phase_waiter() {
        let commands = Commands::default();
        let (tx, mut rx) = oneshot::channel();
        commands.session.lock().await.fence = 1;
        commands
            .session
            .lock()
            .await
            .waiting
            .insert("cmd_race".into(), (2, tx));
        drop(Pending {
            commands: commands.clone(),
            fence: 1,
            id: "cmd_race".into(),
            query: false,
            request: 1,
        });
        tokio::task::yield_now().await;
        commands
            .receive(
                1,
                IpcFrame {
                    t: "Outcome".into(),
                    command_id: "cmd_race".into(),
                    ..IpcFrame::default()
                },
            )
            .await;
        assert!(rx.try_recv().is_ok_and(|frame| frame.t == "Outcome"));
    }
}
