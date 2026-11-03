use ab_operations::{Code, Cursor, GatewayStore, Task, peer::ApplePeer};
use ab_wire::messages::{Capture, IpcFrame};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex, mpsc, watch};

#[derive(Debug)]
pub struct CaptureJob {
    pub capture: Capture,
    pub session: ab_operations::peer::PeerSession,
}

#[derive(Debug)]
pub struct Backend {
    pub state: GatewayStore,
    pub blobs: Arc<Mutex<ab_blobs::Store>>,
    pub jobs: mpsc::Sender<CaptureJob>,
    #[cfg(feature = "lab-harness")]
    pub home: std::path::PathBuf,
}
impl ab_applepeer::Backend for Backend {
    fn next_fence(&self) -> Task<'_, u64> {
        Box::pin(async { self.state.next_fence() })
    }
    fn checkpoints(&self) -> Task<'_, BTreeMap<String, Box<serde_json::value::RawValue>>> {
        Box::pin(async {
            let mut checkpoints = self.state.checkpoints()?;
            if self
                .blobs
                .lock()
                .await
                .entries()
                .values()
                .any(|entry| entry.state == ab_blobs::BlobState::Pending)
            {
                for value in checkpoints.values_mut() {
                    let mut checkpoint: serde_json::Value =
                        serde_json::from_str(value.get()).map_err(|_| Code::Corrupt)?;
                    checkpoint["refresh_blobs"] = serde_json::json!(true);
                    *value =
                        serde_json::value::to_raw_value(&checkpoint).map_err(|_| Code::Corrupt)?;
                }
            }
            Ok(checkpoints)
        })
    }
    fn commit(&self, capture: Capture) -> Task<'_, (Cursor, bool)> {
        Box::pin(self.commit_bound(None, capture))
    }
    fn commit_from(
        &self,
        session: ab_operations::peer::PeerSession,
        capture: Capture,
    ) -> Task<'_, (Cursor, bool)> {
        Box::pin(self.commit_bound(Some(session), capture))
    }
    fn source_state(&self, frame: IpcFrame) -> Task<'_, ()> {
        Box::pin(async move { self.state.source_state(&frame) })
    }
}

impl Backend {
    async fn commit_bound(
        &self,
        session: Option<ab_operations::peer::PeerSession>,
        capture: Capture,
    ) -> Result<(Cursor, bool), ab_operations::Error> {
        if !capture.blob_handles.is_empty() && session.is_none() {
            return Err(Code::Unavailable.into());
        }
        let result = self.state.commit(&capture)?;
        if !capture.blob_handles.is_empty() {
            self.jobs
                .send(CaptureJob {
                    capture,
                    session: session.ok_or(Code::Unavailable)?,
                })
                .await
                .map_err(|_| Code::Unavailable)?;
        }
        #[cfg(feature = "lab-harness")]
        crate::lab_gateway::capture_boundary(&self.home)
            .await
            .map_err(|_| Code::Unavailable)?;
        Ok(result)
    }
}

pub async fn hydrate(
    blobs: Arc<Mutex<ab_blobs::Store>>,
    commands: ab_applepeer::Commands,
    mut jobs: mpsc::Receiver<CaptureJob>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), crate::Error> {
    loop {
        let job = tokio::select! {
            _ = stop.changed() => return Ok(()),
            capture = jobs.recv() => match capture { Some(capture) => capture, None => return Ok(()) },
        };
        if commands.session() != Some(job.session) {
            continue;
        }
        let source = crate::helper_blobs::HelperBlobs(commands.clone(), job.session);
        for (digest, handle) in job.capture.blob_handles {
            let Ok(digest) = ab_blobs::Digest::try_from(digest) else {
                continue;
            };
            let Ok(handle) = serde_json::from_str::<String>(handle.get()) else {
                continue;
            };
            let mut succeeded = false;
            for attempt in 0..8 {
                let transfer = transfer(&blobs, &source, &handle, &digest);
                let result = tokio::select! {
                    _ = stop.changed() => return Ok(()),
                    result = transfer => result,
                };
                if result.is_ok() {
                    succeeded = true;
                    break;
                }
                if commands.session() != Some(job.session) {
                    break;
                }
                if matches!(
                    result,
                    Err(ab_blobs::Error::Corrupt | ab_blobs::Error::Capacity)
                ) {
                    break;
                }
                tokio::select! {
                    _ = stop.changed() => return Ok(()),
                    () = tokio::time::sleep(std::time::Duration::from_millis((250_u64 << attempt).min(4000))) => {},
                }
            }
            if !succeeded {
                eprintln!("Helper blob transfer incomplete");
            }
        }
    }
}
async fn transfer(
    blobs: &Arc<Mutex<ab_blobs::Store>>,
    source: &impl ab_blobs::BlobRead,
    handle: &str,
    digest: &ab_blobs::Digest,
) -> Result<(), ab_blobs::Error> {
    let size = {
        let store = blobs.lock().await;
        if let Some(entry) = store.entries().get(digest.as_str()) {
            if matches!(
                entry.state,
                ab_blobs::BlobState::Offloaded | ab_blobs::BlobState::Missing
            ) {
                return Ok(());
            }
            entry.size
        } else {
            0
        }
    };
    let Some(mut offset) = mutate(blobs, |store| store.begin(digest, size)).await? else {
        return Ok(());
    };
    loop {
        let (bytes, done) = source.read(handle, offset, 128 << 10).await?;
        if bytes.is_empty() && !done || bytes.len() > 128 << 10 {
            return Err(ab_blobs::Error::Corrupt);
        }
        let next = offset
            .checked_add(u64::try_from(bytes.len()).map_err(|_| ab_blobs::Error::Corrupt)?)
            .ok_or(ab_blobs::Error::Corrupt)?;
        if size > 0 && (next > size || done && next != size) {
            return Err(ab_blobs::Error::Corrupt);
        }
        mutate(blobs, |store| store.chunk(digest, offset, &bytes)).await?;
        offset = next;
        if done {
            return mutate(blobs, |store| store.finish(digest)).await;
        }
    }
}
async fn mutate<T>(
    blobs: &Arc<Mutex<ab_blobs::Store>>,
    operation: impl FnOnce(&mut ab_blobs::Store) -> Result<T, ab_blobs::Error>,
) -> Result<T, ab_blobs::Error> {
    let mut blobs = blobs.lock().await;
    tokio::task::block_in_place(|| operation(&mut blobs))
}
