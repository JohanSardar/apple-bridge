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
const REARM_COOLDOWN_CAP: std::time::Duration = std::time::Duration::from_secs(30 * 60);

struct Cooling {
    at: std::time::Instant,
    failures: u32,
}

struct RearmCooldown {
    base: std::time::Duration,
    session: Option<ab_operations::peer::PeerSession>,
    digests: std::collections::HashMap<String, Cooling>,
}
impl Default for RearmCooldown {
    fn default() -> Self {
        Self::new(REARM_COOLDOWN)
    }
}
impl RearmCooldown {
    fn new(base: std::time::Duration) -> Self {
        Self {
            base,
            session: None,
            digests: std::collections::HashMap::new(),
        }
    }
    fn window(&self, failures: u32) -> std::time::Duration {
        self.base
            .saturating_mul(1_u32 << failures.saturating_sub(1).min(10))
            .min(REARM_COOLDOWN_CAP)
    }
    fn session(&mut self, session: ab_operations::peer::PeerSession) {
        if self.session != Some(session) {
            self.session = Some(session);
            self.digests.clear();
        }
    }
    async fn rearm(
        &self,
        blobs: &Arc<Mutex<ab_blobs::Store>>,
        digest: &ab_blobs::Digest,
    ) -> Result<bool, crate::Error> {
        let cooling = self
            .digests
            .get(digest.as_str())
            .is_some_and(|cooling| cooling.at.elapsed() < self.window(cooling.failures));
        Ok(!cooling && rearm(blobs, digest).await?)
    }
    async fn note(
        &mut self,
        blobs: &Arc<Mutex<ab_blobs::Store>>,
        digest: &ab_blobs::Digest,
        attempt: &Attempt,
    ) {
        if attempt.succeeded {
            self.digests.remove(digest.as_str());
            return;
        }
        let exhausted = attempt.transferred
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
            let failures = self
                .digests
                .get(digest.as_str())
                .map_or(0, |cooling| cooling.failures);
            self.digests.insert(
                digest.as_str().to_owned(),
                Cooling {
                    at: std::time::Instant::now(),
                    failures: failures.saturating_add(1),
                },
            );
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
        cooldown.session(job.session);
        let source = crate::helper_blobs::HelperBlobs(commands.clone(), job.session);
        let total = job.capture.blob_handles.len();
        let handles = job
            .capture
            .blob_handles
            .into_iter()
            .filter_map(|(digest, handle)| {
                Some((
                    ab_blobs::Digest::try_from(digest).ok()?,
                    serde_json::from_str::<String>(handle.get()).ok()?,
                ))
            })
            .collect::<Vec<_>>();
        let live = || commands.session() == Some(job.session);
        let Some(tally) = offer(&blobs, &source, &mut cooldown, &mut stop, &live, handles).await?
        else {
            return Ok(());
        };
        tally.report(total);
    }
}

async fn offer<S: ab_blobs::BlobRead>(
    blobs: &Arc<Mutex<ab_blobs::Store>>,
    source: &S,
    cooldown: &mut RearmCooldown,
    stop: &mut watch::Receiver<bool>,
    live: &impl Fn() -> bool,
    handles: Vec<(ab_blobs::Digest, String)>,
) -> Result<Option<Tally>, crate::Error> {
    let mut tally = Tally::default();
    for (digest, handle) in handles {
        let rearmed = cooldown.rearm(blobs, &digest).await?;
        let Some(attempt) = attempt(blobs, source, stop, live, &handle, &digest).await? else {
            return Ok(None);
        };
        cooldown.note(blobs, &digest, &attempt).await;
        let rearmable = blobs
            .lock()
            .await
            .entries()
            .get(digest.as_str())
            .is_some_and(ab_blobs::Entry::rearmable);
        tally.record(Outcome::of(&attempt, rearmed, rearmable));
    }
    Ok(Some(tally))
}

struct Attempt {
    succeeded: bool,
    known: bool,
    transferred: bool,
    refused: bool,
}

async fn attempt<S: ab_blobs::BlobRead>(
    blobs: &Arc<Mutex<ab_blobs::Store>>,
    source: &S,
    stop: &mut watch::Receiver<bool>,
    live: &impl Fn() -> bool,
    handle: &str,
    digest: &ab_blobs::Digest,
) -> Result<Option<Attempt>, crate::Error> {
    let mut attempt = Attempt {
        succeeded: false,
        known: true,
        transferred: false,
        refused: false,
    };
    loop {
        let entry = blobs.lock().await.entries().get(digest.as_str()).cloned();
        let Some(entry) = entry else {
            attempt.known = false;
            break;
        };
        if matches!(
            entry.state,
            ab_blobs::BlobState::Present
                | ab_blobs::BlobState::Failed
                | ab_blobs::BlobState::OverCap
        ) {
            attempt.succeeded = entry.state == ab_blobs::BlobState::Present;
            break;
        }
        if let Ok(deadline) = entry.next_attempt_at.parse::<jiff::Timestamp>() {
            let millis = deadline
                .as_millisecond()
                .saturating_sub(jiff::Timestamp::now().as_millisecond());
            if millis > 0 {
                tokio::select! {
                    _ = stop.changed() => return Ok(None),
                    () = tokio::time::sleep(std::time::Duration::from_millis(u64::try_from(millis).map_err(|_| crate::Error::Invalid)?)) => {},
                }
            }
        }
        if !mutate(blobs, |store| store.retry(digest, false))
            .await
            .map_err(|_| crate::Error::Invalid)?
        {
            break;
        }
        attempt.transferred = true;
        let result = tokio::select! {
            _ = stop.changed() => return Ok(None),
            result = transfer(blobs, source, handle, digest) => result,
        };
        if result.is_ok() {
            mutate(blobs, |store| store.retry(digest, true))
                .await
                .map_err(|_| crate::Error::Invalid)?;
            attempt.succeeded = true;
            break;
        }
        attempt.refused = matches!(result, Err(ab_blobs::Error::Io(_)));
        if blobs
            .lock()
            .await
            .entries()
            .get(digest.as_str())
            .is_some_and(|entry| entry.retry_attempts >= 8)
        {
            mutate(blobs, |store| store.retry(digest, false))
                .await
                .map_err(|_| crate::Error::Invalid)?;
            break;
        }
        if !live()
            || matches!(
                result,
                Err(ab_blobs::Error::Corrupt | ab_blobs::Error::Capacity)
            )
        {
            break;
        }
    }
    Ok(Some(attempt))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Stored,
    Recovered,
    Unknown,
    Unserved,
    FailedAfterRetry,
    Cooling,
    NotRetryable,
}
impl Outcome {
    fn of(attempt: &Attempt, rearmed: bool, rearmable: bool) -> Self {
        match (attempt.succeeded, attempt.known, attempt.transferred) {
            (true, _, _) if rearmed => Self::Recovered,
            (true, _, _) => Self::Stored,
            (false, false, _) => Self::Unknown,
            (false, true, true) if attempt.refused => Self::Unserved,
            (false, true, true) => Self::FailedAfterRetry,
            (false, true, false) if rearmable => Self::Cooling,
            (false, true, false) => Self::NotRetryable,
        }
    }
}
#[derive(Default)]
struct Tally {
    recovered: usize,
    unserved: usize,
    failed_after_retry: usize,
    cooling: usize,
    not_retryable: usize,
    unknown: usize,
}
impl Tally {
    fn record(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Recovered => self.recovered += 1,
            Outcome::Unknown => self.unknown += 1,
            Outcome::Unserved => self.unserved += 1,
            Outcome::FailedAfterRetry => self.failed_after_retry += 1,
            Outcome::Cooling => self.cooling += 1,
            Outcome::NotRetryable => self.not_retryable += 1,
            Outcome::Stored => {}
        }
    }
    fn summary(&self, total: usize) -> Option<String> {
        (self.unserved + self.failed_after_retry + self.cooling + self.not_retryable + self.unknown
            > 0)
        .then(|| {
            format!(
                "Helper blob transfer incomplete: capture of {total} blobs, {} not served by the Helper on a fresh attempt, {} failed on a fresh attempt for another reason, {} waiting for their next fresh attempt after earlier retries were exhausted, {} not retryable (corrupt or over capacity), {} not in the manifest, {} recovered after an earlier failure. An exhausted blob gets one fresh attempt when the Helper offers it again, at most once per backoff window (one minute, doubling to thirty) and at once after a Helper restart.",
                self.unserved,
                self.failed_after_retry,
                self.cooling,
                self.not_retryable,
                self.unknown,
                self.recovered
            )
        })
    }
    fn report(&self, total: usize) {
        if let Some(summary) = self.summary(total) {
            eprintln!("{summary}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    type Check = Result<(), Box<dyn std::error::Error>>;

    struct Helper {
        bytes: Vec<u8>,
        serving: AtomicBool,
        reads: AtomicUsize,
    }
    impl Helper {
        fn new(bytes: &[u8], serving: bool) -> Self {
            Self {
                bytes: bytes.to_vec(),
                serving: AtomicBool::new(serving),
                reads: AtomicUsize::new(0),
            }
        }
        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
    }
    impl ab_blobs::BlobRead for Helper {
        async fn read(
            &self,
            _handle: &str,
            offset: u64,
            length: usize,
        ) -> Result<(Vec<u8>, bool), ab_blobs::Error> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if !self.serving.load(Ordering::SeqCst) {
                return Err(ab_blobs::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Helper blob read failed",
                )));
            }
            let start = usize::try_from(offset).map_err(|_| ab_blobs::Error::Invalid)?;
            let end = (start + length).min(self.bytes.len());
            Ok((self.bytes[start..end].to_vec(), end == self.bytes.len()))
        }
    }

    fn digest_of(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn seed(
        home: &std::path::Path,
        digest: &str,
        size: usize,
        corruptions: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let root = home.join("blobs");
        std::fs::create_dir_all(root.join("staging"))?;
        std::fs::write(root.join("staging").join(format!("{digest}.part")), b"")?;
        let manifest = serde_json::json!({
            "entries": {digest: {
                "state": "failed",
                "size": size,
                "received": 0,
                "resumed_bytes": 0,
                "corruptions": corruptions,
                "retry_attempts": 8,
                "next_attempt_at": "0001-01-01T00:00:00Z",
                "reason": "ipc_retry_exhausted",
                "unreferenced_at": "0001-01-01T00:00:00Z",
            }},
            "pins": {},
        });
        std::fs::write(root.join("manifest.json"), serde_json::to_vec(&manifest)?)?;
        Ok(())
    }

    fn open(home: &std::path::Path) -> Result<Arc<Mutex<ab_blobs::Store>>, ab_blobs::Error> {
        Ok(Arc::new(Mutex::new(ab_blobs::Store::open(home, 0)?)))
    }

    async fn run(
        blobs: &Arc<Mutex<ab_blobs::Store>>,
        helper: &Helper,
        cooldown: &mut RearmCooldown,
        digest: &str,
    ) -> Result<Tally, Box<dyn std::error::Error>> {
        let (_keep, mut stop) = watch::channel(false);
        let handles = vec![(
            ab_blobs::Digest::try_from(digest.to_owned())?,
            format!("media:{digest}"),
        )];
        offer(blobs, helper, cooldown, &mut stop, &|| true, handles)
            .await?
            .ok_or_else(|| "stopped".into())
    }

    async fn state(
        blobs: &Arc<Mutex<ab_blobs::Store>>,
        digest: &str,
    ) -> Option<ab_blobs::BlobState> {
        blobs
            .lock()
            .await
            .entries()
            .get(digest)
            .map(|entry| entry.state.clone())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exhausted_blob_recovers_when_the_helper_offers_it_after_a_restart() -> Check {
        let bytes = vec![7_u8; 40_000];
        let digest = digest_of(&bytes);
        let home = tempfile::tempdir()?;
        seed(home.path(), &digest, bytes.len(), 0)?;
        let blobs = open(home.path())?;
        let mut cooldown = RearmCooldown::default();
        cooldown.session(ab_operations::peer::PeerSession(1));

        let broken = Helper::new(&bytes, false);
        let first = run(&blobs, &broken, &mut cooldown, &digest).await?;
        assert_eq!((first.unserved, first.recovered), (1, 0));
        assert_eq!(broken.reads(), 1);
        assert_eq!(
            state(&blobs, &digest).await,
            Some(ab_blobs::BlobState::Failed)
        );

        let serving = Helper::new(&bytes, true);
        let hot = run(&blobs, &serving, &mut cooldown, &digest).await?;
        assert_eq!((hot.cooling, hot.recovered), (1, 0));
        assert_eq!(serving.reads(), 0);
        let text = hot.summary(1).ok_or("no summary")?;
        assert!(text.contains("1 waiting for their next fresh attempt"));
        assert!(text.contains("0 not retryable"));

        cooldown.session(ab_operations::peer::PeerSession(2));
        let recovered = run(&blobs, &serving, &mut cooldown, &digest).await?;
        assert_eq!(recovered.recovered, 1);
        assert!(recovered.summary(1).is_none());
        assert_eq!(
            state(&blobs, &digest).await,
            Some(ab_blobs::BlobState::Present)
        );
        let stored = std::fs::read(
            home.path()
                .join("blobs/sha256")
                .join(&digest[..2])
                .join(&digest[2..]),
        )?;
        assert_eq!(stored, bytes);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exhausted_blob_recovers_at_the_next_offer_once_the_backoff_has_passed() -> Check {
        let bytes = vec![9_u8; 1_000];
        let digest = digest_of(&bytes);
        let home = tempfile::tempdir()?;
        seed(home.path(), &digest, bytes.len(), 0)?;
        let blobs = open(home.path())?;
        let mut cooldown = RearmCooldown::new(std::time::Duration::ZERO);
        cooldown.session(ab_operations::peer::PeerSession(1));
        let helper = Helper::new(&bytes, false);
        assert_eq!(
            run(&blobs, &helper, &mut cooldown, &digest).await?.unserved,
            1
        );
        helper.serving.store(true, Ordering::SeqCst);
        assert_eq!(
            run(&blobs, &helper, &mut cooldown, &digest)
                .await?
                .recovered,
            1
        );
        assert_eq!(
            state(&blobs, &digest).await,
            Some(ab_blobs::BlobState::Present)
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn corrupt_blob_is_never_retried() -> Check {
        let bytes = vec![3_u8; 2_000];
        let digest = digest_of(&bytes);
        let home = tempfile::tempdir()?;
        seed(home.path(), &digest, bytes.len(), 1)?;
        let blobs = open(home.path())?;
        let mut cooldown = RearmCooldown::new(std::time::Duration::ZERO);
        cooldown.session(ab_operations::peer::PeerSession(1));
        let helper = Helper::new(&bytes, true);
        for session in 1..=3 {
            cooldown.session(ab_operations::peer::PeerSession(session));
            let tally = run(&blobs, &helper, &mut cooldown, &digest).await?;
            assert_eq!((tally.not_retryable, tally.recovered), (1, 0));
        }
        assert_eq!(helper.reads(), 0);
        assert_eq!(
            state(&blobs, &digest).await,
            Some(ab_blobs::BlobState::Failed)
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bytes_that_fail_the_digest_are_not_retried_afterwards() -> Check {
        let wanted = vec![5_u8; 2_000];
        let digest = digest_of(&wanted);
        let home = tempfile::tempdir()?;
        let blobs = open(home.path())?;
        blobs
            .lock()
            .await
            .protect_references(vec![ab_blobs::Reference {
                digest: digest.clone(),
                size: wanted.len() as u64,
                ..ab_blobs::Reference::default()
            }])?;
        let mut cooldown = RearmCooldown::new(std::time::Duration::ZERO);
        cooldown.session(ab_operations::peer::PeerSession(1));
        let helper = Helper::new(&vec![6_u8; 2_000], true);
        let first = run(&blobs, &helper, &mut cooldown, &digest).await?;
        assert_eq!(first.failed_after_retry, 1);
        let reads = helper.reads();
        let second = run(&blobs, &helper, &mut cooldown, &digest).await?;
        assert_eq!((second.not_retryable, second.recovered), (1, 0));
        assert_eq!(helper.reads(), reads);
        Ok(())
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        let cooldown = RearmCooldown::default();
        assert_eq!(cooldown.window(1), std::time::Duration::from_secs(60));
        assert_eq!(cooldown.window(2), std::time::Duration::from_secs(120));
        assert_eq!(cooldown.window(4), std::time::Duration::from_secs(480));
        assert_eq!(cooldown.window(30), REARM_COOLDOWN_CAP);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repeated_failures_back_off_instead_of_retrying_every_offer() -> Check {
        let bytes = vec![1_u8; 500];
        let digest = digest_of(&bytes);
        let home = tempfile::tempdir()?;
        seed(home.path(), &digest, bytes.len(), 0)?;
        let blobs = open(home.path())?;
        let mut cooldown = RearmCooldown::default();
        cooldown.session(ab_operations::peer::PeerSession(1));
        let helper = Helper::new(&bytes, false);
        for _ in 0..20 {
            run(&blobs, &helper, &mut cooldown, &digest).await?;
        }
        assert_eq!(helper.reads(), 1);
        Ok(())
    }
}
