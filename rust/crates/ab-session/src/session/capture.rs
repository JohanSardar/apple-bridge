use super::{Hub, Origin};
use crate::{
    Error,
    flight::{Flight, Refused, rows},
    frame::{MAX_FRAME, send, send_large, text},
    native_facts::Client,
    production::{Loaded, Options},
    queries::NativeQueries,
    readers::{BlobServer, CaptureReader, ScanContext},
    store::{Record, Store},
};
use ab_wire::{
    messages::IpcFrame,
    native::{Frame as Link, Scan, ScanCommitted, Source},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const REWALK: Duration = Duration::from_secs(30);
const LINK_LOST_LIMIT: u32 = 3;
const WALK_RETRY: Duration = Duration::from_secs(2);
const WALK_FLOOR: Duration = Duration::from_millis(500);
const WALK_DUTY: u32 = 20;
const SCAN_FLOOR: Duration = Duration::from_secs(1);
const SCAN_DUTY: u32 = 16;
const BULK_CHUNK_BYTES: usize = 16_000;
const LIVE_CHUNK_BYTES: usize = 64_000;
const BLOB_LOCK_WAIT: Duration = Duration::from_secs(2);

type Slot = Mutex<Box<dyn CaptureReader>>;
type Readers = BTreeMap<String, Slot>;
type Targets = Vec<(String, Vec<PathBuf>)>;

#[derive(Clone, Debug)]
pub(super) struct Cover {
    id: String,
    source: Source,
}

#[derive(Debug)]
pub(super) struct ReaderFlight {
    pub(super) last: bool,
    covers: Vec<Cover>,
    pass: u64,
    more: bool,
}

#[derive(Debug, Default)]
pub(super) struct State {
    saved: BTreeMap<String, (String, String)>,
    bound: BTreeMap<String, (String, String)>,
    touched: BTreeSet<String>,
    urgent: BTreeSet<String>,
    started: u64,
    finished: BTreeMap<String, u64>,
}

#[derive(Debug)]
pub struct Attached {
    pub readers: BTreeMap<String, Box<dyn CaptureReader>>,
    pub options: Options,
}

impl Loaded {
    pub fn attached(&self, facts: Option<UnixStream>) -> Result<Attached, Error> {
        let facts = facts
            .map(|stream| Client::inherited(stream).map(Arc::new))
            .transpose()?;
        Ok(Attached {
            readers: self.readers_with(facts.as_ref())?,
            options: self.options().clone(),
        })
    }
}

pub(super) struct Capture {
    readers: Readers,
    targets: Targets,
    servers: Vec<Arc<dyn BlobServer>>,
    prefixed: Vec<(&'static str, String)>,
    held: Mutex<BTreeMap<String, Vec<Cover>>>,
    store: Option<Store>,
    poll: Duration,
    watch: bool,
    queries: NativeQueries,
}

enum Point {
    Completed(String, String),
    Idle(u64),
}

enum Claim {
    Busy,
    Wait(Instant),
    Scan,
}

#[derive(Debug, Default)]
struct Pace {
    scanned: Option<Instant>,
    poked: Option<Instant>,
    cost: Duration,
}

impl Pace {
    fn due(
        &self,
        now: Instant,
        poll: Duration,
        touched: bool,
        urgent: bool,
    ) -> Result<(), Instant> {
        let Some(scanned) = self.scanned else {
            return Ok(());
        };
        if urgent {
            return Ok(());
        }
        let mut when = scanned + poll;
        if touched {
            let gap = poll.min(SCAN_FLOOR).max(self.cost * SCAN_DUTY);
            when = when.min(self.poked.map_or(now, |poked| poked + gap));
        }
        if now >= when { Ok(()) } else { Err(when) }
    }
}

fn now() -> Result<f64, Error> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Invalid)?
        .as_secs_f64())
}

fn frame_bytes(frame: &Value) -> Result<Vec<u8>, Error> {
    let body = ab_native::native_json::record_data(frame)
        .map_err(|error| Error::Reader(error.to_string()))?;
    if body.len() > MAX_FRAME {
        return Err(Error::Invalid);
    }
    let mut bytes = u32::try_from(body.len())
        .map_err(|_| Error::Invalid)?
        .to_be_bytes()
        .to_vec();
    bytes.extend(body);
    Ok(bytes)
}

fn split(capture: &Value) -> Result<Vec<Value>, Error> {
    let limit = if capture["live"].as_bool().unwrap_or(false) {
        LIVE_CHUNK_BYTES
    } else {
        BULK_CHUNK_BYTES
    };
    let observations = capture["observations"].as_array().ok_or(Error::Invalid)?;
    let absent = |row: &&Value| row["kind"] == "absent";
    let ordered: Vec<&Value> = observations
        .iter()
        .filter(|row| !absent(row))
        .chain(observations.iter().filter(absent))
        .collect();
    let mut groups: Vec<Vec<Value>> = vec![Vec::new()];
    let mut bytes = 0;
    for row in ordered {
        let size = serde_json::to_vec(row)?.len() + 1;
        if bytes + size > limit && groups.last().is_some_and(|group| !group.is_empty()) {
            groups.push(Vec::new());
            bytes = 0;
        }
        if let Some(group) = groups.last_mut() {
            group.push(row.clone());
        }
        bytes += size;
    }
    Ok(groups
        .into_iter()
        .map(|group| {
            let mut chunk = capture.clone();
            chunk["observations"] = Value::Array(group);
            chunk
        })
        .collect())
}

fn reported_generation(
    state: &mut State,
    committed: Option<&(String, String)>,
    source: &str,
    native: &str,
    adopts: bool,
) -> String {
    if !adopts {
        return native.to_owned();
    }
    if let Some((bound, reported)) = state.bound.get(source)
        && bound == native
    {
        return reported.clone();
    }
    let reported = if state.bound.contains_key(source) {
        native.to_owned()
    } else {
        committed
            .map(|(generation, _)| generation.clone())
            .filter(|generation| !generation.is_empty())
            .unwrap_or_else(|| native.to_owned())
    };
    state
        .bound
        .insert(source.to_owned(), (native.to_owned(), reported.clone()));
    reported
}

fn owners<'a>(targets: &'a Targets, path: &std::path::Path) -> impl Iterator<Item = &'a str> {
    let anywhere = path.as_os_str().is_empty();
    targets
        .iter()
        .filter(move |(_, roots)| anywhere || roots.iter().any(|root| path.starts_with(root)))
        .map(|(source, _)| source.as_str())
}

fn walk_trees(watcher: &mut ab_sys::watch::Watcher, targets: &Targets) -> (BTreeSet<String>, bool) {
    let mut grown = BTreeSet::new();
    let mut failed = false;
    for (source, roots) in targets {
        for root in roots {
            match watcher.watch_tree(root) {
                Ok(true) => {
                    grown.insert(source.clone());
                }
                Ok(false) => {}
                Err(_) => failed = true,
            }
        }
    }
    (grown, failed)
}

fn lock_within(
    slot: &Slot,
    wait: Duration,
) -> Result<Option<MutexGuard<'_, Box<dyn CaptureReader>>>, Error> {
    let deadline = Instant::now() + wait;
    loop {
        match slot.try_lock() {
            Ok(reader) => return Ok(Some(reader)),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::WorkerPanicked),
            Err(std::sync::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn unavailable(handle: &str) -> Value {
    json!({"t":"BlobUnavailable","handle":handle,"reason":"unavailable"})
}

impl Capture {
    pub(super) fn attach(attached: Option<Attached>) -> Result<Option<Self>, Error> {
        let Some(Attached { readers, options }) = attached else {
            return Ok(None);
        };
        let queries = options.queries.clone();
        Ok(Some(Self {
            targets: readers
                .iter()
                .map(|(source, reader)| (source.clone(), reader.watch_paths()))
                .collect(),
            servers: readers
                .values()
                .filter_map(|reader| reader.blobs())
                .chain(queries.blob_server())
                .collect(),
            prefixed: readers
                .iter()
                .filter_map(|(source, reader)| Some((reader.handle_prefix()?, source.clone())))
                .collect(),
            readers: readers
                .into_iter()
                .map(|(source, reader)| (source, Mutex::new(reader)))
                .collect(),
            held: Mutex::new(BTreeMap::new()),
            store: options.state.as_deref().map(Store::open).transpose()?,
            poll: options.poll,
            watch: options.watch,
            queries,
        }))
    }

    fn lock(&self, source: &str) -> Result<MutexGuard<'_, Box<dyn CaptureReader>>, Error> {
        self.readers
            .get(source)
            .ok_or(Error::Conflict)?
            .lock()
            .map_err(|_| Error::WorkerPanicked)
    }

    fn owns(&self, source: &str) -> bool {
        self.readers.contains_key(source)
    }

    pub(super) fn open(
        &self,
        state: &mut State,
        committed: &BTreeMap<String, (String, String)>,
    ) -> Result<(), Error> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        for source in self.readers.keys() {
            let Some(record) = store.load(source) else {
                continue;
            };
            let point = committed.get(source);
            if !record.native.is_empty() && point.is_some_and(|(g, _)| *g == record.generation) {
                state.bound.insert(
                    source.clone(),
                    (record.native.clone(), record.generation.clone()),
                );
            }
            if point.is_some_and(|(g, c)| *g == record.generation && *c == record.checkpoint)
                && self.lock(source)?.resume(&record.token)
            {
                state
                    .saved
                    .insert(source.clone(), (record.checkpoint, record.token));
            }
        }
        Ok(())
    }

    fn blob_reply(&self, frame: &IpcFrame) -> Result<Option<Value>, Error> {
        let handle = frame.handle.as_str();
        let offset = u64::try_from(frame.offset).ok();
        let length = Some(frame.length)
            .filter(|length| (1..=131_072).contains(length))
            .and_then(|length| usize::try_from(length).ok());
        for server in &self.servers {
            if server.owns(handle) {
                let reply = match (offset, length) {
                    (Some(offset), Some(length)) => server.read(handle, offset, length).ok(),
                    _ => None,
                };
                return Ok(Some(reply.unwrap_or_else(|| unavailable(handle))));
            }
        }
        for (prefix, source) in &self.prefixed {
            if !handle.starts_with(prefix) {
                continue;
            }
            let slot = self.readers.get(source).ok_or(Error::Conflict)?;
            let Some(mut reader) = lock_within(slot, BLOB_LOCK_WAIT)? else {
                return Ok(Some(unavailable(handle)));
            };
            if !reader.owns(handle) {
                continue;
            }
            let reply = match (offset, length) {
                (Some(offset), Some(length)) => reader.blob(handle, offset, length).ok(),
                _ => None,
            };
            return Ok(Some(reply.unwrap_or_else(|| unavailable(handle))));
        }
        Ok(None)
    }
}

impl Hub {
    pub(super) fn capture_owns(&self, source: &str) -> bool {
        self.capture.as_ref().is_some_and(|c| c.owns(source))
    }

    fn poke(&self, source: &str, urgent: bool) -> Result<(), Error> {
        let mut core = self.core()?;
        if urgent {
            core.capture.urgent.insert(source.to_owned());
        } else {
            core.capture.touched.insert(source.to_owned());
        }
        drop(core);
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn capture_reconcile(&self, source: &str, wait: Duration) -> Option<Result<(), ()>> {
        let capture = self.capture.as_ref()?;
        if !capture.owns(source) {
            return None;
        }
        let scanning = {
            let mut reader = capture.lock(source).ok()?;
            if !reader.refresh() {
                return None;
            }
            reader.more()
        };
        if scanning {
            return Some(self.poke(source, true).map_err(|_| ()));
        }
        Some(self.await_pass(source, wait).map_err(|_| ()))
    }

    fn await_pass(&self, source: &str, wait: Duration) -> Result<(), Error> {
        let deadline = self.clock.elapsed() + wait;
        let mut core = self.core()?;
        let mark = core.capture.started;
        core.capture.urgent.insert(source.to_owned());
        self.changed.notify_all();
        loop {
            if core.capture.finished.get(source).is_some_and(|f| *f > mark) {
                return Ok(());
            }
            if core.closed || self.clock.elapsed() >= deadline {
                return Err(Error::LinkLost);
            }
            core = self
                .changed
                .wait_timeout(core, Duration::from_millis(20))
                .map_err(|_| Error::WorkerPanicked)?
                .0;
        }
    }

    pub(super) fn capture_nudge(&self, source: &str) -> Result<(), Error> {
        if let Some(capture) = &self.capture
            && capture.owns(source)
        {
            capture.lock(source)?.refresh();
            self.poke(source, true)?;
        }
        Ok(())
    }

    pub(super) fn capture_scan(&self, scan: &Scan, source: &str) -> Result<(), Error> {
        let capture = self.capture.as_ref().ok_or(Error::Conflict)?;
        let value = serde_json::to_value(scan)?;
        let cover = Cover {
            id: scan.id.clone(),
            source: scan.source,
        };
        let mut reader = capture.lock(source)?;
        let release = match reader.ingest(&value) {
            Ok(true) => {
                capture
                    .held
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .entry(source.to_owned())
                    .or_default()
                    .push(cover);
                None
            }
            Ok(false) => Some(cover),
            Err(error) => {
                eprintln!("ab-session: {source} facts: {error}");
                Some(cover)
            }
        };
        drop(reader);
        if let Some(cover) = release {
            self.release(std::slice::from_ref(&cover), None)?;
        }
        self.poke(source, true)
    }

    fn release(&self, covers: &[Cover], checkpoint: Option<&str>) -> Result<(), Error> {
        for cover in covers {
            let checkpoint = checkpoint.map_or_else(|| self.next_checkpoint(), str::to_owned);
            self.send_link(&Link::ScanCommitted(ScanCommitted {
                id: cover.id.clone(),
                source: cover.source,
                checkpoint,
            }))?;
        }
        Ok(())
    }

    pub(super) fn capture_blob(&self, frame: &IpcFrame) -> Result<bool, Error> {
        let Some(capture) = &self.capture else {
            return Ok(false);
        };
        let Some(reply) = capture.blob_reply(frame)? else {
            return Ok(false);
        };
        send(
            &mut *self.gateway.lock().map_err(|_| Error::WorkerPanicked)?,
            &reply,
        )?;
        Ok(true)
    }

    pub(super) fn capture_handles_query(&self, operation: &str) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|capture| capture.queries.handles_operation(operation))
    }

    pub(super) fn capture_query(&self, frame: &IpcFrame) -> Result<(), Error> {
        let capture = self.capture.as_ref().ok_or(Error::Conflict)?;
        let args = frame
            .args
            .as_deref()
            .map_or(Ok(Value::Null), |args| serde_json::from_str(args.get()))?;
        let reply = capture
            .queries
            .answer(&json!({"id": frame.id, "op": frame.op, "args": args}));
        send_large(
            &mut *self.gateway.lock().map_err(|_| Error::WorkerPanicked)?,
            &reply,
        )
    }

    pub(super) fn capture_commit(&self, source: &str) -> Result<Option<String>, Error> {
        let capture = self.capture.as_ref().ok_or(Error::Conflict)?;
        let mut reader = capture.lock(source)?;
        reader.commit()?;
        Ok(reader.token())
    }

    pub(super) fn capture_committed(
        &self,
        source: &str,
        flight: &Flight<Origin>,
        token: Option<String>,
    ) -> Result<(), Error> {
        let Origin::Reader(origin) = &flight.origin else {
            return Err(Error::Invalid);
        };
        if !origin.last {
            return Ok(());
        }
        self.persist(
            token,
            source,
            Point::Completed(flight.generation.clone(), flight.checkpoint.clone()),
        )?;
        self.release(&origin.covers, Some(&flight.checkpoint))?;
        if !origin.more {
            self.finish_pass(source, origin.pass)?;
        }
        Ok(())
    }

    fn finish_pass(&self, source: &str, pass: u64) -> Result<(), Error> {
        self.core()?
            .capture
            .finished
            .insert(source.to_owned(), pass);
        self.changed.notify_all();
        Ok(())
    }

    fn persist(&self, token: Option<String>, source: &str, point: Point) -> Result<(), Error> {
        let Some(capture) = &self.capture else {
            return Ok(());
        };
        let (Some(store), Some(token)) = (&capture.store, token) else {
            return Ok(());
        };
        let core = self.core()?;
        let (generation, checkpoint) = match point {
            Point::Completed(generation, checkpoint) => (generation, checkpoint),
            Point::Idle(sequence) => {
                if core.flights.contains(source) || core.flights.sequence() != sequence {
                    return Ok(());
                }
                let Some(point) = core.flights.committed(source).cloned() else {
                    return Ok(());
                };
                point
            }
        };
        if core.capture.saved.get(source) == Some(&(checkpoint.clone(), token.clone())) {
            return Ok(());
        }
        let native = core
            .capture
            .bound
            .get(source)
            .filter(|(_, reported)| *reported == generation)
            .map(|(native, _)| native.clone())
            .unwrap_or_default();
        drop(core);
        if generation.is_empty() || checkpoint.is_empty() {
            return Ok(());
        }
        store.save(
            source,
            &Record {
                generation,
                checkpoint: checkpoint.clone(),
                token: token.clone(),
                native,
            },
        )?;
        self.core()?
            .capture
            .saved
            .insert(source.to_owned(), (checkpoint, token));
        Ok(())
    }

    fn claim(
        &self,
        source: &str,
        pace: &mut Pace,
        began: Instant,
        poll: Duration,
    ) -> Result<Claim, Error> {
        let (touched, urgent) = {
            let mut core = self.core()?;
            if core.flights.contains(source) {
                return Ok(Claim::Busy);
            }
            (
                core.capture.touched.remove(source),
                core.capture.urgent.remove(source),
            )
        };
        if let Err(when) = pace.due(began, poll, touched, urgent) {
            if touched {
                self.core()?.capture.touched.insert(source.to_owned());
            }
            return Ok(Claim::Wait(when));
        }
        pace.scanned = Some(began);
        if touched {
            pace.poked = Some(began);
        }
        Ok(Claim::Scan)
    }

    fn send_capture(
        &self,
        source: &str,
        capture: &Value,
        mut flight: ReaderFlight,
        adopts: bool,
    ) -> Result<bool, Error> {
        let generation = {
            let mut core = self.core()?;
            let committed = core.flights.committed(source).cloned();
            reported_generation(
                &mut core.capture,
                committed.as_ref(),
                source,
                text(capture, "generation")?,
                adopts,
            )
        };
        let checkpoint = text(capture, "checkpoint")?.to_owned();
        let chunks = split(capture)?;
        let final_index = chunks.len() - 1;
        for (index, mut chunk) in chunks.into_iter().enumerate() {
            let last = index == final_index;
            let chunk_checkpoint = if last {
                checkpoint.clone()
            } else {
                format!("{checkpoint}-{index}")
            };
            chunk["checkpoint"] = Value::String(chunk_checkpoint.clone());
            chunk["generation"] = Value::String(generation.clone());
            let predecessor = self.core()?.flights.predecessor(source, &generation);
            chunk["predecessor"] = Value::String(predecessor);
            chunk["t"] = json!("Capture");
            let bytes = frame_bytes(&chunk)?;
            let size = u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
            let observations: &[Value] =
                chunk["observations"].as_array().map_or(&[], Vec::as_slice);
            let keys = rows(observations);
            let mut core = self.core()?;
            loop {
                let next = Flight::new(
                    generation.clone(),
                    chunk_checkpoint.clone(),
                    size,
                    keys.clone(),
                    Origin::Reader(ReaderFlight {
                        last,
                        covers: if last {
                            flight.covers.clone()
                        } else {
                            Vec::new()
                        },
                        pass: flight.pass,
                        more: flight.more,
                    }),
                );
                match core.flights.reserve(source, next) {
                    Ok(()) => break,
                    Err(Refused::TooLarge) => return Err(Error::Invalid),
                    Err(Refused::Busy | Refused::Credit) => {
                        if core.closed {
                            return Ok(false);
                        }
                        core = self
                            .changed
                            .wait_timeout(core, Duration::from_millis(100))
                            .map_err(|_| Error::WorkerPanicked)?
                            .0;
                    }
                }
            }
            if last {
                flight.covers.clear();
            }
            drop(core);
            self.write_gateway(&bytes)?;
            if !last && !self.chunk_settled(source)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn chunk_settled(&self, source: &str) -> Result<bool, Error> {
        let mut core = self.core()?;
        while core.flights.contains(source) {
            if core.closed {
                return Ok(false);
            }
            core = self
                .changed
                .wait_timeout(core, Duration::from_millis(100))
                .map_err(|_| Error::WorkerPanicked)?
                .0;
        }
        Ok(true)
    }

    fn idle(&self, deadline: Option<Instant>, poll: Duration) -> Result<bool, Error> {
        let core = self.core()?;
        if core.closed {
            return Ok(false);
        }
        if core
            .capture
            .urgent
            .iter()
            .any(|source| !core.flights.contains(source))
        {
            return Ok(true);
        }
        let wait = deadline
            .map_or(poll, |when| when.saturating_duration_since(Instant::now()))
            .max(Duration::from_millis(1));
        let (core, _) = self
            .changed
            .wait_timeout(core, wait)
            .map_err(|_| Error::WorkerPanicked)?;
        Ok(!core.closed)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn produce(&self) -> Result<(), Error> {
        let Some(capture) = &self.capture else {
            return Ok(());
        };
        let mut ordinal = 0_u64;
        let sources: Vec<String> = capture.readers.keys().cloned().collect();
        let mut pace: BTreeMap<String, Pace> = BTreeMap::new();
        let mut lost: BTreeMap<String, u32> = BTreeMap::new();
        loop {
            let mut deadline: Option<Instant> = None;
            for source in &sources {
                let state = pace.entry(source.clone()).or_default();
                let began = Instant::now();
                let postponed = match self.claim(source, state, began, capture.poll)? {
                    Claim::Busy => continue,
                    Claim::Wait(when) => Some(when),
                    Claim::Scan => None,
                };
                let soonest = postponed.unwrap_or(began + capture.poll);
                deadline = Some(deadline.map_or(soonest, |earlier| earlier.min(soonest)));
                if postponed.is_some() {
                    continue;
                }
                ordinal += 1;
                let context = ScanContext {
                    checkpoint: format!("rs-{:x}-{ordinal}", self.epoch),
                    scan_id: format!("rs-scan-{:x}-{ordinal}", self.epoch),
                    now: now()?,
                    fence: self.fence,
                };
                let (sequence, pass) = {
                    let mut core = self.core()?;
                    core.capture.started += 1;
                    (core.flights.sequence(), core.capture.started)
                };
                let (next, covers, token, more, adopts) = {
                    let mut reader = capture.lock(source)?;
                    let started = Instant::now();
                    let next = reader.next(&context);
                    if started.elapsed() >= Duration::from_secs(5) {
                        eprintln!(
                            "ab-session: {source} reader step {ordinal} took {} ms",
                            started.elapsed().as_millis()
                        );
                    }
                    let covers = capture
                        .held
                        .lock()
                        .map_err(|_| Error::WorkerPanicked)?
                        .remove(source)
                        .unwrap_or_default();
                    let more = next.is_ok() && reader.more();
                    (
                        next,
                        covers,
                        reader.token(),
                        more,
                        reader.adopts_generation(),
                    )
                };
                state.cost = began.elapsed();
                if more {
                    self.core()?.capture.urgent.insert(source.clone());
                }
                if next.is_ok() {
                    lost.remove(source);
                }
                let found = match next {
                    Ok(Some(found)) => found,
                    Ok(None) => {
                        self.release(&covers, None)?;
                        self.persist(token, source, Point::Idle(sequence))?;
                        if !more {
                            self.finish_pass(source, pass)?;
                        }
                        continue;
                    }
                    Err(error) => {
                        eprintln!("ab-session: {source} reader: {error}");
                        self.release(&covers, None)?;
                        self.finish_pass(source, pass)?;
                        if matches!(error, Error::LinkLost) {
                            let strikes = lost.entry(source.clone()).or_default();
                            *strikes += 1;
                            if *strikes >= LINK_LOST_LIMIT {
                                eprintln!(
                                    "ab-session: native/1 facts link is gone for good; exiting so the Helper session restarts"
                                );
                                return Err(error);
                            }
                        }
                        continue;
                    }
                };
                let flight = ReaderFlight {
                    last: true,
                    covers,
                    pass,
                    more,
                };
                if !self.send_capture(source, &found, flight, adopts)? {
                    return Ok(());
                }
            }
            if !self.idle(deadline, capture.poll)? {
                return Ok(());
            }
        }
    }

    pub(super) fn watch(&self) {
        let Some(capture) = self.capture.as_ref().filter(|capture| capture.watch) else {
            return;
        };
        let targets = &capture.targets;
        let Ok(mut watcher) = ab_sys::watch::Watcher::new() else {
            return;
        };
        let mut pending = true;
        let mut announce = false;
        let mut spacing = WALK_FLOOR;
        let mut walked = Instant::now();
        let mut fallback = walked;
        loop {
            let now = Instant::now();
            if (pending && now.saturating_duration_since(walked) >= spacing) || now >= fallback {
                let (grown, failed) = walk_trees(&mut watcher, targets);
                walked = Instant::now();
                spacing = WALK_FLOOR.max(walked.saturating_duration_since(now) * WALK_DUTY);
                pending = !grown.is_empty();
                fallback = walked + if failed { WALK_RETRY } else { REWALK };
                if announce {
                    for source in &grown {
                        let _ = self.poke(source, false);
                    }
                }
                announce = true;
            }
            let wait = if pending {
                (walked + spacing)
                    .saturating_duration_since(Instant::now())
                    .clamp(Duration::from_millis(1), Duration::from_millis(250))
            } else {
                Duration::from_millis(250)
            };
            let Ok(mut changes) = watcher.poll(wait) else {
                return;
            };
            if !changes.is_empty() {
                thread::sleep(Duration::from_millis(15));
                while let Ok(more) = watcher.poll(Duration::ZERO) {
                    if more.is_empty() {
                        break;
                    }
                    changes.extend(more);
                }
                pending |= changes.iter().any(|change| change.structural);
                let touched: BTreeSet<&str> = changes
                    .iter()
                    .flat_map(|change| owners(targets, &change.path))
                    .collect();
                for source in touched {
                    let _ = self.poke(source, false);
                }
            }
            if self.core().map_or(true, |core| core.closed) {
                return;
            }
        }
    }
}
