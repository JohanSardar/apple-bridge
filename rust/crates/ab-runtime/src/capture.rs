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
    pub jobs: mpsc::UnboundedSender<CaptureJob>,
    #[cfg(feature = "lab-harness")]
    pub home: std::path::PathBuf,
}
impl ab_applepeer::Backend for Backend {
    fn next_fence(&self) -> Task<'_, u64> {
        Box::pin(async { self.state.next_fence() })
    }
    fn checkpoints(&self) -> Task<'_, BTreeMap<String, Box<serde_json::value::RawValue>>> {
        Box::pin(async {
            #[cfg(feature = "seeded-faults")]
            if seeded_fault("skip-checkpoint-commit") {
                return Ok(BTreeMap::new());
            }
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

#[cfg(feature = "seeded-faults")]
fn seeded_fault(name: &str) -> bool {
    std::env::var("AB_LAB_SEEDED_FAULT").is_ok_and(|value| value == name)
}

impl Backend {
    #[cfg_attr(
        not(feature = "lab-harness"),
        allow(clippy::unused_async, clippy::unused_async_trait_impl)
    )]
    async fn commit_bound(
        &self,
        session: Option<ab_operations::peer::PeerSession>,
        capture: Capture,
    ) -> Result<(Cursor, bool), ab_operations::Error> {
        if !capture.blob_handles.is_empty() && session.is_none() {
            return Err(Code::Unavailable.into());
        }
        #[cfg(feature = "seeded-faults")]
        if seeded_fault("ack-uncommitted-data") {
            return Ok((
                Cursor {
                    epoch: String::new(),
                    subscription_revision: 0,
                    sequence: 0,
                },
                false,
            ));
        }
        let result = self.state.commit(&capture)?;
        if !capture.blob_handles.is_empty() {
            self.jobs
                .send(CaptureJob {
                    capture,
                    session: session.ok_or(Code::Unavailable)?,
                })
                .map_err(|_| Code::Unavailable)?;
        }
        #[cfg(feature = "lab-harness")]
        crate::lab_gateway::capture_boundary(&self.home)
            .await
            .map_err(|_| Code::Unavailable)?;
        Ok(result)
    }
}

const REARM_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Default)]
struct RearmCooldown(std::collections::HashMap<String, std::time::Instant>);
impl RearmCooldown {
    async fn rearm(
        &self,
        blobs: &Arc<Mutex<ab_blobs::Store>>,
        digest: &ab_blobs::Digest,
    ) -> Result<bool, crate::Error> {
        let cooling = self
            .0
            .get(digest.as_str())
            .is_some_and(|at| at.elapsed() < REARM_COOLDOWN);
        Ok(!cooling && rearm(blobs, digest).await?)
    }
    async fn note(
        &mut self,
        blobs: &Arc<Mutex<ab_blobs::Store>>,
        digest: &ab_blobs::Digest,
        transferred: bool,
    ) {
        let exhausted = transferred
            && blobs
                .lock()
                .await
                .entries()
                .get(digest.as_str())
                .is_some_and(|entry| {
                    entry.state == ab_blobs::BlobState::Failed
                        && entry.reason == "ipc_retry_exhausted"
                });
        if exhausted {
            self.0
                .insert(digest.as_str().to_owned(), std::time::Instant::now());
        }
    }
}

pub async fn hydrate(
    blobs: Arc<Mutex<ab_blobs::Store>>,
    commands: ab_applepeer::Commands,
    mut jobs: mpsc::UnboundedReceiver<CaptureJob>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), crate::Error> {
    let mut cooldown = RearmCooldown::default();
    loop {
        let job = tokio::select! {
            _ = stop.changed() => return Ok(()),
            capture = jobs.recv() => match capture { Some(capture) => capture, None => return Ok(()) },
        };
        if commands.session() != Some(job.session) {
            continue;
        }
        let source = crate::helper_blobs::HelperBlobs(commands.clone(), job.session);
        let total = job.capture.blob_handles.len();
        let mut tally = Tally::default();
        for (digest, handle) in job.capture.blob_handles {
            let Ok(digest) = ab_blobs::Digest::try_from(digest) else {
                continue;
            };
            let Ok(handle) = serde_json::from_str::<String>(handle.get()) else {
                continue;
            };
            let mut succeeded = false;
            let mut transferred = false;
            let mut known = true;
            let rearmed = cooldown.rearm(&blobs, &digest).await?;
            loop {
                let entry = blobs.lock().await.entries().get(digest.as_str()).cloned();
                let Some(entry) = entry else {
                    known = false;
                    break;
                };
                if matches!(
                    entry.state,
                    ab_blobs::BlobState::Present
                        | ab_blobs::BlobState::Failed
                        | ab_blobs::BlobState::OverCap
                ) {
                    succeeded = entry.state == ab_blobs::BlobState::Present;
                    break;
                }
                if let Ok(deadline) = entry.next_attempt_at.parse::<jiff::Timestamp>() {
                    let millis = deadline
                        .as_millisecond()
                        .saturating_sub(jiff::Timestamp::now().as_millisecond());
                    if millis > 0 {
                        tokio::select! {
                            _ = stop.changed() => return Ok(()),
                            () = tokio::time::sleep(std::time::Duration::from_millis(u64::try_from(millis).map_err(|_| crate::Error::Invalid)?)) => {},
                        }
                    }
                }
                if !mutate(&blobs, |store| store.retry(&digest, false))
                    .await
                    .map_err(|_| crate::Error::Invalid)?
                {
                    break;
                }
                transferred = true;
                let result = tokio::select! {
                    _ = stop.changed() => return Ok(()),
                    result = transfer(&blobs, &source, &handle, &digest) => result,
                };
                if result.is_ok() {
                    mutate(&blobs, |store| store.retry(&digest, true))
                        .await
                        .map_err(|_| crate::Error::Invalid)?;
                    succeeded = true;
                    break;
                }
                if blobs
                    .lock()
                    .await
                    .entries()
                    .get(digest.as_str())
                    .is_some_and(|entry| entry.retry_attempts >= 8)
                {
                    mutate(&blobs, |store| store.retry(&digest, false))
                        .await
                        .map_err(|_| crate::Error::Invalid)?;
                    break;
                }
                if commands.session() != Some(job.session)
                    || matches!(
                        result,
                        Err(ab_blobs::Error::Corrupt | ab_blobs::Error::Capacity)
                    )
                {
                    break;
                }
            }
            cooldown.note(&blobs, &digest, transferred).await;
            tally.record(Outcome::of((succeeded, known, transferred), rearmed));
        }
        tally.report(total);
    }
}
#[derive(Clone, Copy)]
enum Outcome {
    Stored,
    Recovered,
    Unknown,
    FailedAfterRetry,
    PreviouslyFailed,
}
impl Outcome {
    fn of(state: (bool, bool, bool), rearmed: bool) -> Self {
        match state {
            (true, _, _) if rearmed => Self::Recovered,
            (true, _, _) => Self::Stored,
            (false, false, _) => Self::Unknown,
            (false, true, true) => Self::FailedAfterRetry,
            (false, true, false) => Self::PreviouslyFailed,
        }
    }
}
#[derive(Default)]
struct Tally {
    recovered: usize,
    failed_after_retry: usize,
    previously_failed: usize,
    unknown: usize,
}
impl Tally {
    fn record(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Recovered => self.recovered += 1,
            Outcome::Unknown => self.unknown += 1,
            Outcome::FailedAfterRetry => self.failed_after_retry += 1,
            Outcome::PreviouslyFailed => self.previously_failed += 1,
            Outcome::Stored => {}
        }
    }
    fn report(&self, total: usize) {
        if self.failed_after_retry + self.previously_failed + self.unknown > 0 {
            eprintln!(
                "Helper blob transfer incomplete: capture of {total} blobs, {} not retryable (corrupt or over capacity), {} failed after retry, {} not in the manifest, {} recovered after an earlier failure. Failed blobs get one fresh attempt whenever the Helper offers them again.",
                self.previously_failed, self.failed_after_retry, self.unknown, self.recovered
            );
        }
    }
}
async fn rearm(
    blobs: &Arc<Mutex<ab_blobs::Store>>,
    digest: &ab_blobs::Digest,
) -> Result<bool, crate::Error> {
    mutate(blobs, |store| store.rearm(digest))
        .await
        .map_err(|_| crate::Error::Invalid)
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
