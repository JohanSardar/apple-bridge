use super::{BlobServer, CaptureReader, ScanContext, reader_error};
use crate::{
    Error,
    messages_media::{self, Job, Media},
    native_facts::Client,
};
use ab_native::messages;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SCOPE: [&str; 5] = [
    "messages.chat",
    "messages.handle",
    "messages.message",
    "messages.reaction",
    "messages.attachment",
];
const RECONCILE_EVERY: Duration = Duration::from_mins(15);
const BLOB_CHUNK: usize = 128 * 1024;
const MAX_IDLE_STEPS: u32 = 8;
const STEP_ROWS: usize = 128;
const STEP_BUDGET: Duration = Duration::from_millis(750);
const BULK_BYTES: usize = 16_000;
const LIVE_BYTES: usize = 64_000;
const FAILURES_BEFORE_REOPEN: u32 = 5;

#[derive(Debug, Clone)]
struct Known {
    digest: String,
    size: u64,
    declared: i64,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
struct Progress {
    generation: String,
    cursor: Option<i64>,
    settled_ms: Option<u64>,
}

#[derive(Debug)]
struct Item {
    collection: String,
    key: String,
    row: Value,
    bytes: usize,
    live: bool,
    absent: bool,
    handles: Vec<(String, String)>,
}

#[derive(Debug)]
struct Blobs {
    database: PathBuf,
    home: PathBuf,
    known: Mutex<BTreeMap<String, Known>>,
    serving: Mutex<Option<messages::Reader>>,
    media: Arc<Media>,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
}

impl Blobs {
    fn lookup(&self, guid: &str, declared: i64) -> Option<(String, u64)> {
        let known = self.known.lock().ok()?;
        known
            .get(guid)
            .filter(|known| known.declared == declared)
            .map(|known| (known.digest.clone(), known.size))
    }

    fn current(&self, guid: &str, digest: &str) -> bool {
        self.known
            .lock()
            .is_ok_and(|known| known.get(guid).is_some_and(|known| known.digest == digest))
    }

    fn remember(&self, guid: &str, entry: Known) {
        if let Ok(mut known) = self.known.lock() {
            known.insert(guid.to_owned(), entry);
        }
    }

    fn bytes(&self, handle: &str, offset: u64, length: usize) -> Result<Vec<u8>, Error> {
        let mut serving = self.serving.lock().map_err(|_| Error::WorkerPanicked)?;
        let mut last = None;
        for _ in 0..2 {
            if serving.is_none() {
                *serving =
                    Some(messages::Reader::open(&self.database, &self.home).map_err(reader_error)?);
            }
            let Some(reader) = serving.as_ref() else {
                return Err(Error::Conflict);
            };
            match reader.read_blob(handle, offset, length) {
                Ok(bytes) => return Ok(bytes),
                Err(error) => {
                    last = Some(error);
                    *serving = None;
                }
            }
        }
        Err(last.map_or(Error::Conflict, reader_error))
    }
}

impl BlobServer for Blobs {
    fn owns(&self, handle: &str) -> bool {
        self.media.owns(handle)
            || self
                .known
                .lock()
                .is_ok_and(|known| known.contains_key(handle))
    }

    fn read(&self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        if self.media.owns(handle) {
            return self.media.read(handle, offset, length);
        }
        let known = self
            .known
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .get(handle)
            .cloned()
            .ok_or(Error::Invalid)?;
        let bytes = self.bytes(handle, offset, length)?;
        let end = offset + u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
        Ok(json!({
            "t": "BlobChunk",
            "handle": handle,
            "offset": offset,
            "bytes": STANDARD.encode(&bytes),
            "digest": known.digest,
            "eof": end >= known.size,
        }))
    }
}

#[derive(Debug)]
pub struct MessagesReader {
    root: PathBuf,
    database: PathBuf,
    home: PathBuf,
    reader: Option<messages::Reader>,
    blobs: Arc<Blobs>,
    reconciled_at: Option<Instant>,
    settled_ms: Option<u64>,
    resuming: Option<Progress>,
    queue: VecDeque<Item>,
    queue_complete: bool,
    at: String,
    staged: Option<Progress>,
    committed: Option<Progress>,
    pending: bool,
    failures: u32,
    facts: Option<Arc<Client>>,
}

impl Drop for MessagesReader {
    fn drop(&mut self) {
        self.blobs.media.stop();
    }
}

impl MessagesReader {
    #[must_use]
    pub fn new(root: PathBuf, home: PathBuf) -> Self {
        let database = root.join("chat.db");
        Self {
            blobs: Arc::new(Blobs {
                database: database.clone(),
                home: home.clone(),
                known: Mutex::new(BTreeMap::new()),
                serving: Mutex::new(None),
                media: Media::new(),
            }),
            database,
            root,
            home,
            reader: None,
            reconciled_at: None,
            settled_ms: None,
            resuming: None,
            queue: VecDeque::new(),
            queue_complete: false,
            at: String::new(),
            staged: None,
            committed: None,
            pending: false,
            failures: 0,
            facts: None,
        }
    }

    #[must_use]
    pub fn with_media(mut self, client: Option<Arc<Client>>, retry: Duration) -> Self {
        if let Some(client) = &client {
            self.blobs.media.retry_after(retry);
            let blobs = Arc::clone(&self.blobs);
            messages_media::spawn(
                &self.blobs.media,
                Arc::clone(client),
                move |guid, digest| blobs.current(guid, digest),
            );
        }
        self.facts = client;
        self
    }

    fn digest(reader: &messages::Reader, guid: &str) -> Result<(String, u64), Error> {
        let mut hash = Sha256::new();
        let mut offset = 0_u64;
        loop {
            let bytes = reader
                .read_blob(guid, offset, BLOB_CHUNK)
                .map_err(reader_error)?;
            hash.update(&bytes);
            offset += u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
            if bytes.len() < BLOB_CHUNK {
                return Ok((format!("{:x}", hash.finalize()), offset));
            }
        }
    }

    fn attach(&self, data: &mut Value, guid: &str, live: bool) -> Vec<(String, String)> {
        let Some(reader) = self.reader.as_ref() else {
            return Vec::new();
        };
        if data.pointer("/blob/state").and_then(Value::as_str) != Some("present") {
            return Vec::new();
        }
        let declared = data["size"].as_i64().unwrap_or(-1);
        let found = match self.blobs.lookup(guid, declared) {
            Some(known) => Ok(known),
            None => Self::digest(reader, guid),
        };
        let Ok((digest, size)) = found else {
            data["blob"] = json!({"state":"missing"});
            return Vec::new();
        };
        self.blobs.remember(
            guid,
            Known {
                digest: digest.clone(),
                size,
                declared,
            },
        );
        data["blob"] = json!({"state":"present","digest":digest});
        let mut handles = vec![(digest.clone(), guid.to_owned())];
        if self.facts.is_some() {
            handles.extend(self.derive(reader, data, guid, &digest, live));
        }
        handles
    }

    fn derive(
        &self,
        reader: &messages::Reader,
        data: &mut Value,
        guid: &str,
        digest: &str,
        live: bool,
    ) -> Vec<(String, String)> {
        let media = &self.blobs.media;
        let Some(plan) = media.plan(guid, digest) else {
            return Vec::new();
        };
        let handles = match (&plan.derived, plan.failed) {
            (Some(derived), _) => derived.apply(data),
            (None, true) => {
                messages_media::placeholder(data);
                Vec::new()
            }
            (None, false) => Vec::new(),
        };
        if plan.fetch
            && let Ok(Some(source)) = reader.media_source(guid)
        {
            let companions = if plan.companions {
                reader.live_companions(guid).unwrap_or_default()
            } else {
                Vec::new()
            };
            media.queue(Job {
                guid: guid.to_owned(),
                digest: digest.to_owned(),
                file: source.path,
                uti: data["uti"].as_str().unwrap_or_default().to_owned(),
                mime: data["mime_type"].as_str().unwrap_or_default().to_owned(),
                sticker: source.sticker,
                companions,
                live,
            });
        }
        handles
    }

    fn items(&self, batches: &[messages::Batch], out: &mut Vec<Option<Item>>) -> Result<(), Error> {
        let mut index: BTreeMap<(String, String), usize> = BTreeMap::new();
        for (position, item) in out.iter().enumerate() {
            if let Some(item) = item {
                index.insert((item.collection.clone(), item.key.clone()), position);
            }
        }
        for batch in batches {
            for observation in &batch.observations {
                let mut row: Value =
                    serde_json::from_slice(&observation.bytes().map_err(reader_error)?)?;
                let absent = row["kind"] == "absent";
                row["observed_at"] = Value::String(self.at.clone());
                row["revalidated"] = Value::Bool(true);
                row["scoped_alias"] = Value::String(format!(
                    "{}/{}",
                    observation.collection, observation.native_key
                ));
                let mut handles = Vec::new();
                if !absent && observation.collection == "messages.attachment" {
                    let mut data = row["data"].take();
                    handles = self.attach(&mut data, &observation.native_key, batch.live);
                    row["data"] = data;
                }
                let bytes = serde_json::to_vec(&row)?.len() + 1;
                let key = (
                    observation.collection.clone(),
                    observation.native_key.clone(),
                );
                if let Some(previous) = index.get(&key).copied() {
                    out[previous] = None;
                }
                index.insert(key.clone(), out.len());
                out.push(Some(Item {
                    collection: key.0,
                    key: key.1,
                    row,
                    bytes,
                    live: batch.live,
                    absent,
                    handles,
                }));
            }
        }
        Ok(())
    }

    fn progress(&self) -> Option<Progress> {
        self.reader.as_ref().map(|reader| Progress {
            generation: reader.generation.clone(),
            cursor: reader.scan_cursor(),
            settled_ms: self.settled_ms,
        })
    }

    fn begin(&mut self) -> bool {
        let Some(reader) = self.reader.as_mut() else {
            return true;
        };
        let resumed = self
            .resuming
            .take()
            .filter(|progress| progress.generation == reader.generation);
        match resumed {
            Some(Progress {
                cursor: Some(cursor),
                settled_ms,
                ..
            }) => {
                self.settled_ms = settled_ms;
                reader.resume_scan(cursor).is_err()
            }
            Some(Progress {
                settled_ms: Some(settled),
                ..
            }) => {
                let elapsed = Duration::from_millis(unix_ms().saturating_sub(settled));
                if elapsed < RECONCILE_EVERY {
                    self.settled_ms = Some(settled);
                    self.reconciled_at = Instant::now().checked_sub(elapsed);
                    false
                } else {
                    true
                }
            }
            _ => true,
        }
    }

    fn fill(&mut self, context: &ScanContext) -> Result<(), Error> {
        self.at = context.observed_at()?;
        self.blobs.media.fence(context.fence);
        let due = self.blobs.media.due();
        if let Some(reader) = self.reader.as_mut() {
            reader.invalidate(
                due.iter()
                    .map(|guid| ("messages.attachment", guid.as_str())),
            );
        }
        let mut request = false;
        if self.reader.is_none() {
            let Ok(reader) = messages::Reader::open(&self.database, &self.home) else {
                return Ok(());
            };
            self.reader = Some(reader);
            request = self.begin();
        }
        let due = self
            .reconciled_at
            .is_none_or(|at| at.elapsed() >= RECONCILE_EVERY);
        if due
            && self
                .reader
                .as_ref()
                .is_some_and(|reader| !reader.reconciliation_pending())
        {
            request = true;
        }
        let started = Instant::now();
        let mut collected: Vec<Option<Item>> = Vec::new();
        let mut idle = 0;
        let mut complete = false;
        let mut failure = None;
        loop {
            let reader = self.reader.as_mut().ok_or(Error::Conflict)?;
            let batches = match reader.step(request) {
                Ok(batches) => batches,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            };
            request = false;
            idle = if batches.is_empty() { idle + 1 } else { 0 };
            let finished = batches.iter().any(|batch| batch.complete);
            self.items(&batches, &mut collected)?;
            complete |= finished;
            let reader = self.reader.as_ref().ok_or(Error::Conflict)?;
            let rows = collected.iter().flatten().count();
            if finished
                || rows >= STEP_ROWS
                || !reader.reconciliation_pending()
                || idle >= MAX_IDLE_STEPS
                || started.elapsed() >= STEP_BUDGET
            {
                break;
            }
        }
        let mut items: Vec<Item> = collected.into_iter().flatten().collect();
        if let Some(error) = failure {
            self.failures += 1;
            if self.failures >= FAILURES_BEFORE_REOPEN {
                self.reader = None;
                self.failures = 0;
            }
            if items.is_empty() && !complete {
                return Err(reader_error(error));
            }
        } else {
            self.failures = 0;
        }
        if complete {
            self.reconciled_at = Some(Instant::now());
            self.settled_ms = Some(unix_ms());
            self.queue_complete = true;
        }
        items.sort_by_key(|item| item.absent);
        self.queue.extend(items);
        if self.queue.is_empty() && !self.queue_complete {
            if let Some(progress) = self.progress() {
                self.committed = Some(progress);
            }
        } else {
            self.staged = self.progress();
        }
        Ok(())
    }

    fn piece(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.queue.is_empty() && !self.queue_complete {
            return Ok(None);
        }
        let generation = self
            .reader
            .as_ref()
            .map(|reader| reader.generation.clone())
            .ok_or(Error::Conflict)?;
        let live = self.queue.front().is_none_or(|item| item.live);
        let limit = if live { LIVE_BYTES } else { BULK_BYTES };
        let mut rows: Vec<Value> = Vec::new();
        let mut aliases: BTreeMap<String, Value> = BTreeMap::new();
        let mut handles: BTreeMap<String, String> = BTreeMap::new();
        let mut bytes = 0;
        while let Some(front) = self.queue.front() {
            if front.live != live {
                break;
            }
            let mut added: BTreeMap<String, Value> = BTreeMap::new();
            collect(&front.row["data"], &self.at, &mut added);
            added.retain(|key, _| !aliases.contains_key(key));
            let extra: usize = added
                .values()
                .map(|alias| serde_json::to_vec(alias).map_or(0, |bytes| bytes.len() + 1))
                .sum::<usize>()
                + front.bytes;
            if !front.absent && !rows.is_empty() && bytes + extra > limit {
                break;
            }
            let Some(item) = self.queue.pop_front() else {
                break;
            };
            bytes += extra;
            aliases.extend(added);
            handles.extend(item.handles);
            rows.push(item.row);
        }
        let complete = self.queue.is_empty() && self.queue_complete;
        let mut observations: Vec<Value> = aliases.into_values().collect();
        observations.extend(rows);
        Ok(Some(json!({
            "source": "messages",
            "generation": generation,
            "checkpoint": context.checkpoint,
            "scan_id": context.scan_id,
            "scan_started_at": self.at,
            "scope": SCOPE,
            "completion_evidence": if complete { "complete" } else { "" },
            "observations": observations,
            "blob_handles": handles,
            "live": live && !complete,
        })))
    }
}

fn collect(value: &Value, at: &str, aliases: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(fields) => {
            if let Some(reference) = fields.get("$ref") {
                if let (Some(collection), Some(key)) = (
                    reference["collection"].as_str(),
                    reference["native_key"].as_str(),
                ) {
                    aliases.insert(
                        format!("{collection}/{key}"),
                        json!({"kind":"alias","collection":collection,"native_key":key,
                               "scoped_alias":format!("{collection}/{key}"),"observed_at":at}),
                    );
                }
            } else {
                for field in fields.values() {
                    collect(field, at, aliases);
                }
            }
        }
        Value::Array(values) => {
            for field in values {
                collect(field, at, aliases);
            }
        }
        _ => {}
    }
}

impl CaptureReader for MessagesReader {
    fn source(&self) -> &'static str {
        "messages"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let name = self.database.display().to_string();
        vec![
            self.database.clone(),
            PathBuf::from(format!("{name}-wal")),
            PathBuf::from(format!("{name}-shm")),
        ]
    }

    fn token(&self) -> Option<String> {
        self.committed
            .as_ref()
            .and_then(|progress| serde_json::to_string(progress).ok())
    }

    fn resume(&mut self, token: &str) -> bool {
        self.resuming = serde_json::from_str(token).ok();
        self.resuming.is_some()
    }

    fn ingest(&mut self, capture: &Value) -> Result<bool, Error> {
        let Some(reader) = self.reader.as_mut() else {
            return Ok(false);
        };
        let identities: Vec<(&str, &str)> = capture["observations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some((
                    row["collection"].as_str().filter(|c| SCOPE.contains(c))?,
                    row["native_key"].as_str().filter(|k| !k.is_empty())?,
                ))
            })
            .collect();
        reader.invalidate(identities);
        Ok(false)
    }

    fn owns(&self, handle: &str) -> bool {
        self.blobs.owns(handle)
    }

    fn blobs(&self) -> Option<Arc<dyn BlobServer>> {
        Some(self.blobs.clone())
    }

    fn blob(&mut self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        self.blobs.read(handle, offset, length)
    }

    fn more(&self) -> bool {
        !self.queue.is_empty()
            || self.queue_complete
            || self
                .reader
                .as_ref()
                .is_some_and(messages::Reader::reconciliation_pending)
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending {
            return Err(Error::Conflict);
        }
        if self.queue.is_empty() && !self.queue_complete {
            self.fill(context)?;
        }
        let piece = self.piece(context)?;
        self.pending = piece.is_some();
        Ok(piece)
    }

    fn commit(&mut self) -> Result<(), Error> {
        if !self.pending {
            return Err(Error::Conflict);
        }
        self.pending = false;
        if self.queue.is_empty() {
            self.queue_complete = false;
            if let Some(progress) = self.staged.take() {
                self.committed = Some(progress);
            }
        }
        Ok(())
    }
}
