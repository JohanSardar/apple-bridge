use crate::{Error, native_facts::Client};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub const OPERATION: &str = "messages.media";
pub const HANDLE_PREFIX: &str = "media:";
pub const RETRY: Duration = Duration::from_secs(60);
const PAUSED_RETRY: Duration = Duration::from_secs(5);
const PATIENCE: Duration = Duration::from_secs(600);
const MAX_VARIANT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Job {
    pub guid: String,
    pub digest: String,
    pub file: PathBuf,
    pub uti: String,
    pub mime: String,
    pub sticker: bool,
    pub companions: Vec<PathBuf>,
    pub live: bool,
}

#[derive(Debug, Clone)]
struct Variant {
    path: PathBuf,
    size: u64,
}

#[derive(Debug, Clone)]
pub struct Derived {
    media: Value,
    derivatives: Value,
    handles: Vec<(String, String)>,
    awaiting_live: bool,
}

#[derive(Debug, Default)]
struct Entry {
    unsupported: bool,
    failed: bool,
    derived: Option<Derived>,
    retry_at: Option<Instant>,
    queued: bool,
    guids: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct State {
    entries: BTreeMap<String, Entry>,
    variants: BTreeMap<String, Variant>,
    jobs: VecDeque<Job>,
    ready: BTreeSet<String>,
    closed: bool,
}

#[derive(Debug)]
pub struct Media {
    state: Mutex<State>,
    wake: Condvar,
    fence: AtomicU64,
    stopped: AtomicBool,
    retry_ms: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub derived: Option<Derived>,
    pub failed: bool,
    pub fetch: bool,
    pub companions: bool,
}

fn safe_relative(file: &str) -> Option<PathBuf> {
    let path = Path::new(file);
    (!file.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_))))
    .then(|| path.to_path_buf())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn shape(result: &Value, root: &Path) -> Result<(Derived, Vec<(String, Variant)>), Error> {
    let media = match result.get("media") {
        Some(Value::Object(fields)) => Value::Object(fields.clone()),
        None | Some(Value::Null) => json!({}),
        _ => return Err(Error::Invalid),
    };
    let mut derivatives = Map::new();
    let mut handles = Vec::new();
    let mut paths: Vec<(String, Variant)> = Vec::new();
    if let Some(variants) = result.get("derivatives") {
        for (name, variant) in variants.as_object().ok_or(Error::Invalid)? {
            let digest = variant["digest"].as_str().ok_or(Error::Invalid)?;
            if !valid_digest(digest) {
                return Err(Error::Invalid);
            }
            let relative = safe_relative(variant["file"].as_str().ok_or(Error::Invalid)?)
                .ok_or(Error::Invalid)?;
            let size = variant["size"].as_u64().ok_or(Error::Invalid)?;
            let mut fields = variant.as_object().ok_or(Error::Invalid)?.clone();
            fields.remove("file");
            fields.insert("state".into(), json!("present"));
            derivatives.insert(name.clone(), Value::Object(fields));
            let handle = format!("{HANDLE_PREFIX}{digest}");
            handles.push((digest.to_owned(), handle.clone()));
            paths.push((
                handle,
                Variant {
                    path: root.join(relative),
                    size,
                },
            ));
        }
    }
    if let Some(unavailable) = result.get("unavailable") {
        for (name, reason) in unavailable.as_object().ok_or(Error::Invalid)? {
            derivatives.insert(name.clone(), json!({"state":"unavailable","reason":reason}));
        }
    }
    let awaiting_live = result["awaitingLive"].as_bool() == Some(true);
    let derived = Derived {
        media,
        derivatives: Value::Object(derivatives),
        handles,
        awaiting_live,
    };
    Ok((derived, paths))
}

impl Derived {
    pub fn apply(&self, data: &mut Value) -> Vec<(String, String)> {
        data["media"] = self.media.clone();
        data["derivatives"] = self.derivatives.clone();
        self.handles.clone()
    }
}

pub fn placeholder(data: &mut Value) {
    data["media"] = json!({});
    data["derivatives"] = json!({});
}

impl Media {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            retry_ms: AtomicU64::new(u64::try_from(RETRY.as_millis()).unwrap_or(60_000)),
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            fence: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        })
    }

    pub fn retry_after(&self, retry: Duration) {
        self.retry_ms.store(
            u64::try_from(retry.as_millis()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
    }

    fn retry(&self) -> Duration {
        Duration::from_millis(self.retry_ms.load(Ordering::SeqCst))
    }

    pub fn fence(&self, fence: u64) {
        self.fence.store(fence, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.wake.notify_all();
    }

    pub fn plan(&self, guid: &str, digest: &str) -> Option<Plan> {
        let mut state = self.state.lock().ok()?;
        let entry = state.entries.entry(digest.to_owned()).or_default();
        entry.guids.insert(guid.to_owned());
        if entry.unsupported {
            return None;
        }
        let waiting = entry.derived.as_ref().is_some_and(|d| d.awaiting_live);
        let fetch =
            !entry.queued && entry.retry_at.is_none() && (entry.derived.is_none() || waiting);
        Some(Plan {
            derived: entry.derived.clone(),
            failed: entry.failed,
            fetch,
            companions: waiting,
        })
    }

    pub fn queue(&self, job: Job) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let entry = state.entries.entry(job.digest.clone()).or_default();
        if entry.queued {
            return;
        }
        entry.queued = true;
        entry.retry_at = None;
        entry.guids.insert(job.guid.clone());
        if job.live {
            state.jobs.push_front(job);
        } else {
            state.jobs.push_back(job);
        }
        drop(state);
        self.wake.notify_one();
    }

    pub fn due(&self) -> Vec<String> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        let now = Instant::now();
        let mut out: BTreeSet<String> = std::mem::take(&mut state.ready);
        for entry in state.entries.values_mut() {
            if !entry.queued && entry.retry_at.is_some_and(|at| at <= now) {
                entry.retry_at = None;
                out.extend(entry.guids.iter().cloned());
            }
        }
        out.into_iter().collect()
    }

    pub fn owns(&self, handle: &str) -> bool {
        handle.starts_with(HANDLE_PREFIX)
            && self
                .state
                .lock()
                .is_ok_and(|state| state.variants.contains_key(handle))
    }

    pub fn read(&self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        let variant = self
            .state
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .variants
            .get(handle)
            .cloned()
            .ok_or(Error::Invalid)?;
        if length == 0 || length > 128 * 1024 || offset > variant.size {
            return Err(Error::Invalid);
        }
        let mut file = std::fs::File::open(&variant.path)?;
        if file.metadata()?.len() != variant.size {
            return Err(Error::Conflict);
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(u64::try_from(length).map_err(|_| Error::Invalid)?)
            .read_to_end(&mut bytes)?;
        let end = offset + u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
        Ok(json!({
            "t": "BlobChunk",
            "handle": handle,
            "offset": offset,
            "bytes": STANDARD.encode(&bytes),
            "digest": handle.trim_start_matches(HANDLE_PREFIX),
            "eof": end >= variant.size,
        }))
    }

    fn finish(&self, job: &Job, outcome: Outcome) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let mut publish = false;
        let mut register: Vec<(String, Variant)> = Vec::new();
        {
            let entry = state.entries.entry(job.digest.clone()).or_default();
            entry.queued = false;
            match outcome {
                Outcome::Unsupported => {
                    entry.unsupported = true;
                    entry.retry_at = None;
                }
                Outcome::Paused => {
                    entry.retry_at = Some(Instant::now() + PAUSED_RETRY);
                }
                Outcome::Failed => {
                    entry.failed = true;
                    entry.retry_at = Some(Instant::now() + self.retry());
                    publish = true;
                }
                Outcome::Ready(derived, variants) => {
                    entry.failed = false;
                    entry.retry_at = derived.awaiting_live.then(|| Instant::now() + self.retry());
                    entry.derived = Some(derived);
                    register = variants;
                    publish = true;
                }
            }
        }
        for (handle, variant) in register {
            state.variants.insert(handle, variant);
        }
        if publish {
            let guids: Vec<String> = state
                .entries
                .get(&job.digest)
                .map(|entry| entry.guids.iter().cloned().collect())
                .unwrap_or_default();
            state.ready.extend(guids);
        }
    }

    fn next_job(&self) -> Option<Job> {
        let mut state = self.state.lock().ok()?;
        loop {
            if state.closed {
                return None;
            }
            if self.fence.load(Ordering::SeqCst) != 0
                && let Some(job) = state.jobs.pop_front()
            {
                return Some(job);
            }
            state = self
                .wake
                .wait_timeout(state, Duration::from_millis(500))
                .ok()?
                .0;
        }
    }
}

enum Outcome {
    Unsupported,
    Paused,
    Failed,
    Ready(Derived, Vec<(String, Variant)>),
}

fn arguments(job: &Job) -> Result<Value, Error> {
    let text = |path: &Path| path.to_str().map(str::to_owned).ok_or(Error::Invalid);
    let companions = job
        .companions
        .iter()
        .map(|path| text(path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "digest": job.digest,
        "file": text(&job.file)?,
        "uti": job.uti,
        "mime": job.mime,
        "sticker": job.sticker,
        "companions": companions,
    }))
}

fn resolve(client: &Client, fence: u64, job: &Job) -> Result<Outcome, Error> {
    let set = client.fetch_patient(fence, OPERATION, &arguments(job)?, PATIENCE)?;
    if !set.complete || set.items.len() != 1 {
        return Err(Error::Invalid);
    }
    let item = &set.items[0];
    if item["digest"] != json!(job.digest) {
        return Err(Error::Invalid);
    }
    match item["status"].as_str() {
        Some("unsupported") => Ok(Outcome::Unsupported),
        Some("paused") => Ok(Outcome::Paused),
        Some("failed") => Ok(Outcome::Failed),
        Some("ready") => {
            let root = PathBuf::from(item["root"].as_str().ok_or(Error::Invalid)?);
            if !root.is_absolute() {
                return Err(Error::Invalid);
            }
            let (derived, variants) = shape(&item["result"], &root)?;
            for (_, variant) in &variants {
                let metadata = std::fs::metadata(&variant.path)?;
                if !metadata.is_file()
                    || metadata.len() != variant.size
                    || variant.size > MAX_VARIANT_BYTES
                {
                    return Err(Error::Invalid);
                }
            }
            Ok(Outcome::Ready(derived, variants))
        }
        _ => Err(Error::Invalid),
    }
}

pub fn spawn(
    media: &Arc<Media>,
    client: Arc<Client>,
    current: impl Fn(&str, &str) -> bool + Send + 'static,
) {
    let media = Arc::clone(media);
    std::thread::spawn(move || {
        while let Some(job) = media.next_job() {
            if !current(&job.guid, &job.digest) {
                media.finish(&job, Outcome::Paused);
                continue;
            }
            let fence = media.fence.load(Ordering::SeqCst);
            let outcome = match resolve(&client, fence, &job) {
                Ok(outcome) => outcome,
                Err(error) => {
                    eprintln!("ab-session: messages media {}: {error}", job.guid);
                    Outcome::Failed
                }
            };
            if let Outcome::Ready(..) = &outcome
                && !current(&job.guid, &job.digest)
            {
                media.finish(&job, Outcome::Paused);
                continue;
            }
            media.finish(&job, outcome);
        }
    });
}
