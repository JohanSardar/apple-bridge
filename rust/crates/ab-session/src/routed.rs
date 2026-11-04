use crate::{
    Error,
    flight::{Flight, Flights, Ledger, Refused, rows},
    frame::{MAX_FRAME, read_frame, send, send_large, text},
    queries::NativeQueries,
    readers::{BlobServer, CaptureReader, ScanContext},
    store::{Record, Store},
};
use ab_wire::gateway::{BlobRead, CaptureHeader, Committed, Dispatch, GatewayFrame, HelperFrame};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Slot = Mutex<Box<dyn CaptureReader>>;
type Readers = BTreeMap<String, Slot>;

struct Blobs {
    servers: Vec<Arc<dyn BlobServer>>,
    prefixed: Vec<(&'static str, String)>,
}

fn slot<'a>(readers: &'a Readers, source: &str) -> Result<&'a Slot, Error> {
    readers.get(source).ok_or(Error::Conflict)
}

fn lock<'a>(
    readers: &'a Readers,
    source: &str,
) -> Result<MutexGuard<'a, Box<dyn CaptureReader>>, Error> {
    slot(readers, source)?
        .lock()
        .map_err(|_| Error::WorkerPanicked)
}

pub trait Overlay: Sync {
    fn gateway<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        value: &Value,
        output: &Arc<Mutex<UnixStream>>,
        helper: &Mutex<UnixStream>,
        ledger: &'scope dyn Ledger,
    ) -> Result<bool, Error>;
    fn helper(&self, value: &Value) -> Result<(), Error>;
    fn helper_ready(&self, output: &Mutex<UnixStream>) -> Result<(), Error>;
    fn shutdown(&self);
}

#[derive(Debug, Clone)]
pub struct Options {
    pub poll: Duration,
    pub watch: bool,
    pub state: Option<std::path::PathBuf>,
    pub queries: NativeQueries,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(100),
            watch: false,
            state: None,
            queries: NativeQueries::default(),
        }
    }
}

#[derive(Debug)]
struct Origin {
    rust: bool,
    last: bool,
    covers: Covers,
}

type Covers = Vec<(String, u64)>;
type Held = Mutex<BTreeMap<String, Covers>>;

#[derive(Debug, Default)]
struct Shared {
    fence: u64,
    opened: bool,
    ready: bool,
    closed: bool,
    flights: Flights<Origin>,
    saved: BTreeMap<String, (String, String)>,
    bound: BTreeMap<String, (String, String)>,
    dispatched: BTreeMap<String, u64>,
    touched: BTreeSet<String>,
    urgent: BTreeSet<String>,
}

#[derive(Debug)]
struct Gate {
    shared: Mutex<Shared>,
    changed: Condvar,
}

impl Ledger for Gate {
    fn sequence(&self) -> Result<u64, Error> {
        Ok(self.lock()?.flights.sequence())
    }

    fn written(
        &self,
        collections: &[&str],
        key: &str,
        after: Option<u64>,
        wait: Duration,
    ) -> Result<Option<String>, Error> {
        let deadline = std::time::Instant::now() + wait;
        let mut shared = self.lock()?;
        loop {
            if let Some(checkpoint) = shared.flights.written(collections, key, after) {
                return Ok(Some(checkpoint));
            }
            let now = std::time::Instant::now();
            if shared.closed || now >= deadline {
                return Ok(None);
            }
            shared = self
                .changed
                .wait_timeout(shared, (deadline - now).min(Duration::from_millis(100)))
                .map_err(|_| Error::WorkerPanicked)?
                .0;
        }
    }
}

impl Gate {
    fn lock(&self) -> Result<MutexGuard<'_, Shared>, Error> {
        self.shared.lock().map_err(|_| Error::WorkerPanicked)
    }

    fn poke(&self, sources: &[&str], urgent: bool) {
        let Ok(mut shared) = self.shared.lock() else {
            return;
        };
        let mut fresh = false;
        for source in sources {
            let pending = if urgent {
                &mut shared.urgent
            } else {
                &mut shared.touched
            };
            fresh |= pending.insert((*source).to_owned());
        }
        drop(shared);
        if fresh {
            self.changed.notify_all();
        }
    }

    fn close(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.closed = true;
        }
        self.changed.notify_all();
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

fn wait_open(gate: &Gate) -> Result<bool, Error> {
    let mut shared = gate.lock()?;
    while !(shared.opened && shared.ready) {
        if shared.closed {
            return Ok(false);
        }
        shared = gate
            .changed
            .wait_timeout(shared, Duration::from_millis(100))
            .map_err(|_| Error::WorkerPanicked)?
            .0;
    }
    Ok(true)
}

fn reported_generation(shared: &mut Shared, source: &str, native: &str, adopts: bool) -> String {
    if !adopts {
        return native.to_owned();
    }
    if let Some((bound, reported)) = shared.bound.get(source)
        && bound == native
    {
        return reported.clone();
    }
    let reported = if shared.bound.contains_key(source) {
        native.to_owned()
    } else {
        shared
            .flights
            .committed(source)
            .map(|(generation, _)| generation.clone())
            .filter(|generation| !generation.is_empty())
            .unwrap_or_else(|| native.to_owned())
    };
    shared
        .bound
        .insert(source.to_owned(), (native.to_owned(), reported.clone()));
    reported
}

enum Point {
    Completed(String, String),
    Idle(u64),
}

fn persist(
    gate: &Gate,
    token: Option<String>,
    store: Option<&Store>,
    source: &str,
    point: Point,
) -> Result<(), Error> {
    let Some(store) = store else {
        return Ok(());
    };
    let Some(token) = token else {
        return Ok(());
    };
    let shared = gate.lock()?;
    let (generation, checkpoint) = match point {
        Point::Completed(generation, checkpoint) => (generation, checkpoint),
        Point::Idle(sequence) => {
            if shared.flights.contains(source) || shared.flights.sequence() != sequence {
                return Ok(());
            }
            let Some(point) = shared.flights.committed(source).cloned() else {
                return Ok(());
            };
            point
        }
    };
    if shared.saved.get(source) == Some(&(checkpoint.clone(), token.clone())) {
        return Ok(());
    }
    let native = shared
        .bound
        .get(source)
        .filter(|(_, reported)| *reported == generation)
        .map(|(native, _)| native.clone())
        .unwrap_or_default();
    drop(shared);
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
    gate.lock()?
        .saved
        .insert(source.to_owned(), (checkpoint, token));
    Ok(())
}

const REWALK: Duration = Duration::from_secs(30);
const WALK_RETRY: Duration = Duration::from_secs(2);
const WALK_FLOOR: Duration = Duration::from_millis(500);
const WALK_DUTY: u32 = 20;
const SCAN_FLOOR: Duration = Duration::from_secs(1);
const SCAN_DUTY: u32 = 16;

type Targets = Vec<(String, Vec<PathBuf>)>;

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

fn watch(gate: &Gate, targets: &Targets) {
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
            if announce && !grown.is_empty() {
                let grown: Vec<&str> = grown.iter().map(String::as_str).collect();
                gate.poke(&grown, false);
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
        match watcher.poll(wait) {
            Ok(mut changes) => {
                if !changes.is_empty() {
                    thread::sleep(Duration::from_millis(15));
                    loop {
                        match watcher.poll(Duration::ZERO) {
                            Ok(more) if !more.is_empty() => changes.extend(more),
                            _ => break,
                        }
                    }
                    pending |= changes.iter().any(|change| change.structural);
                    let touched: BTreeSet<&str> = changes
                        .iter()
                        .flat_map(|change| owners(targets, &change.path))
                        .collect();
                    gate.poke(&touched.into_iter().collect::<Vec<_>>(), false);
                }
            }
            Err(_) => return,
        }
        if gate.shared.lock().map_or(true, |shared| shared.closed) {
            return;
        }
    }
}

fn release(
    to_helper: &Mutex<UnixStream>,
    source: &str,
    covers: &[(String, u64)],
) -> Result<(), Error> {
    for (checkpoint, credit) in covers {
        send(
            &mut *to_helper.lock().map_err(|_| Error::WorkerPanicked)?,
            &json!({"t":"Committed","source":source,"checkpoint":checkpoint,"credit":credit}),
        )?;
    }
    Ok(())
}

fn ingest(
    gate: &Gate,
    readers: &Readers,
    held: &Held,
    to_helper: &Mutex<UnixStream>,
    source: &str,
    value: &Value,
    cover: (String, u64),
) -> Result<(), Error> {
    let mut reader = lock(readers, source)?;
    match reader.ingest(value) {
        Ok(true) => held
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .entry(source.to_owned())
            .or_default()
            .push(cover),
        Ok(false) => release(to_helper, source, &[cover])?,
        Err(error) => {
            eprintln!("ab-session: {source} facts: {error}");
            release(to_helper, source, &[cover])?;
        }
    }
    drop(reader);
    gate.poke(&[source], true);
    Ok(())
}

fn register(
    gate: &Gate,
    source: &str,
    value: &Value,
    generation: String,
    checkpoint: String,
    size: u64,
) -> Result<(), Error> {
    let observations: &[Value] = value["observations"].as_array().map_or(&[], Vec::as_slice);
    gate.lock()?.flights.track(
        source,
        Flight::new(
            generation,
            checkpoint,
            size,
            rows(observations),
            Origin {
                rust: false,
                last: true,
                covers: Vec::new(),
            },
        ),
    )
}

fn note_dispatch(shared: &mut Shared, dispatch: Dispatch) {
    let sequence = shared.flights.sequence();
    if let Some(id) = dispatch.command_id {
        shared.dispatched.insert(id, sequence);
    }
}

fn holds_facts(source: &str) -> bool {
    matches!(source, "calendar" | "contacts" | "reminders")
}

fn bind_outcome(gate: &Gate, value: &mut Value, rejected: bool) -> Result<bool, Error> {
    let key = match value["evidence"]["native_key"].as_str() {
        Some(key) if value["evidence"]["capture_checkpoint"].is_string() => key.to_owned(),
        _ => return Ok(false),
    };
    let mut shared = gate.lock()?;
    let after = value["command_id"]
        .as_str()
        .and_then(|id| shared.dispatched.remove(id))
        .unwrap_or_else(|| shared.flights.sequence());
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(seen) = shared.flights.latest(&key)
            && seen.rust
            && (rejected || seen.facts || seen.sequence > after)
        {
            value["evidence"]["capture_checkpoint"] = Value::String(seen.checkpoint.clone());
            return Ok(true);
        }
        if shared.closed || std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        shared = gate
            .changed
            .wait_timeout(shared, Duration::from_millis(50))
            .map_err(|_| Error::WorkerPanicked)?
            .0;
    }
}

fn forward_outcome(
    gate: &Gate,
    to_gateway: &Mutex<UnixStream>,
    value: &mut Value,
) -> Result<bool, Error> {
    let rejected = value["t"] == "Rejected";
    if !bind_outcome(gate, value, rejected)? {
        return Ok(false);
    }
    send(
        &mut *to_gateway.lock().map_err(|_| Error::WorkerPanicked)?,
        value,
    )?;
    Ok(true)
}

fn send_scan(
    gate: &Gate,
    to_gateway: &Mutex<UnixStream>,
    layer: Option<&dyn Overlay>,
    source: &str,
    capture: &Value,
    mut covers: Covers,
    adopts: bool,
) -> Result<bool, Error> {
    let generation = reported_generation(
        &mut *gate.lock()?,
        source,
        text(capture, "generation")?,
        adopts,
    );
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
        let predecessor = gate.lock()?.flights.predecessor(source, &generation);
        chunk["predecessor"] = Value::String(predecessor);
        chunk["t"] = json!("Capture");
        let bytes = frame_bytes(&chunk)?;
        let size = u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
        let observations: &[Value] = chunk["observations"].as_array().map_or(&[], Vec::as_slice);
        let mut shared = gate.lock()?;
        loop {
            let flight = Flight::new(
                generation.clone(),
                chunk_checkpoint.clone(),
                size,
                rows(observations),
                Origin {
                    rust: true,
                    last,
                    covers: if last { covers.clone() } else { Vec::new() },
                },
            );
            match shared.flights.reserve(source, flight) {
                Ok(()) => break,
                Err(Refused::TooLarge) => return Err(Error::Invalid),
                Err(Refused::Busy | Refused::Credit) => {
                    if shared.closed {
                        return Ok(false);
                    }
                    shared = gate
                        .changed
                        .wait_timeout(shared, Duration::from_millis(100))
                        .map_err(|_| Error::WorkerPanicked)?
                        .0;
                }
            }
        }
        if last {
            covers.clear();
        }
        drop(shared);
        if let Some(layer) = layer {
            layer.helper(&chunk)?;
        }
        to_gateway
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .write_all(&bytes)?;
        if !last && !wait_committed(gate, source)? {
            return Ok(false);
        }
    }
    Ok(true)
}

const BULK_CHUNK_BYTES: usize = 16_000;
const LIVE_CHUNK_BYTES: usize = 64_000;

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

fn wait_committed(gate: &Gate, source: &str) -> Result<bool, Error> {
    let mut shared = gate.lock()?;
    while shared.flights.contains(source) {
        if shared.closed {
            return Ok(false);
        }
        shared = gate
            .changed
            .wait_timeout(shared, Duration::from_millis(100))
            .map_err(|_| Error::WorkerPanicked)?
            .0;
    }
    Ok(true)
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

enum Claim {
    Busy,
    Wait(Instant),
    Scan,
}

fn claim(
    gate: &Gate,
    source: &str,
    state: &mut Pace,
    began: Instant,
    poll: Duration,
) -> Result<Claim, Error> {
    let (touched, urgent) = {
        let mut shared = gate.lock()?;
        if shared.flights.contains(source) {
            return Ok(Claim::Busy);
        }
        (shared.touched.remove(source), shared.urgent.remove(source))
    };
    if let Err(when) = state.due(began, poll, touched, urgent) {
        if touched {
            gate.lock()?.touched.insert(source.to_owned());
        }
        return Ok(Claim::Wait(when));
    }
    state.scanned = Some(began);
    if touched {
        state.poked = Some(began);
    }
    Ok(Claim::Scan)
}

#[allow(clippy::too_many_arguments)]
fn produce(
    gate: &Gate,
    readers: &Readers,
    held: &Held,
    to_helper: &Mutex<UnixStream>,
    to_gateway: &Mutex<UnixStream>,
    layer: Option<&dyn Overlay>,
    store: Option<&Store>,
    poll: Duration,
) -> Result<(), Error> {
    let mut ordinal = 0_u64;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Invalid)?
        .as_nanos();
    let sources: Vec<String> = readers.keys().cloned().collect();
    let mut pace: BTreeMap<String, Pace> = BTreeMap::new();
    if !wait_open(gate)? {
        return Ok(());
    }
    loop {
        let mut deadline: Option<Instant> = None;
        for source in &sources {
            let state = pace.entry(source.clone()).or_default();
            let began = Instant::now();
            let postponed = match claim(gate, source, state, began, poll)? {
                Claim::Busy => continue,
                Claim::Wait(when) => Some(when),
                Claim::Scan => None,
            };
            let soonest = postponed.unwrap_or(began + poll);
            deadline = Some(deadline.map_or(soonest, |earlier| earlier.min(soonest)));
            if postponed.is_some() {
                continue;
            }
            ordinal += 1;
            let fence = gate.lock()?.fence;
            let context = ScanContext {
                checkpoint: format!("rs-{epoch:x}-{ordinal}"),
                scan_id: format!("rs-scan-{epoch:x}-{ordinal}"),
                now: now()?,
                fence,
            };
            let sequence = gate.lock()?.flights.sequence();
            let (next, covers, token, more, adopts) = {
                let mut reader = lock(readers, source)?;
                let started = std::time::Instant::now();
                let next = reader.next(&context);
                if started.elapsed() >= Duration::from_secs(5) {
                    eprintln!(
                        "ab-session: {source} reader step {ordinal} took {} ms",
                        started.elapsed().as_millis()
                    );
                }
                let covers = held
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
                gate.lock()?.urgent.insert(source.clone());
            }
            let capture = match next {
                Ok(Some(capture)) => capture,
                Ok(None) => {
                    release(to_helper, source, &covers)?;
                    persist(gate, token, store, source, Point::Idle(sequence))?;
                    continue;
                }
                Err(error) => {
                    eprintln!("ab-session: {source} reader: {error}");
                    release(to_helper, source, &covers)?;
                    continue;
                }
            };
            if !send_scan(gate, to_gateway, layer, source, &capture, covers, adopts)? {
                return Ok(());
            }
        }
        if !idle(gate, deadline, poll)? {
            return Ok(());
        }
    }
}

fn idle(gate: &Gate, deadline: Option<Instant>, poll: Duration) -> Result<bool, Error> {
    let shared = gate.lock()?;
    if shared.closed {
        return Ok(false);
    }
    if shared
        .urgent
        .iter()
        .any(|source| !shared.flights.contains(source))
    {
        return Ok(true);
    }
    let wait = deadline
        .map_or(poll, |when| when.saturating_duration_since(Instant::now()))
        .max(Duration::from_millis(1));
    let (shared, _) = gate
        .changed
        .wait_timeout(shared, wait)
        .map_err(|_| Error::WorkerPanicked)?;
    Ok(!shared.closed)
}

const BLOB_LOCK_WAIT: Duration = Duration::from_secs(2);

fn lock_within(
    slot: &Slot,
    wait: Duration,
) -> Result<Option<MutexGuard<'_, Box<dyn CaptureReader>>>, Error> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match slot.try_lock() {
            Ok(reader) => return Ok(Some(reader)),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(Error::WorkerPanicked),
            Err(std::sync::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
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

fn blob_reply(
    blobs: &Blobs,
    readers: &Readers,
    request: &BlobRead,
) -> Result<Option<Value>, Error> {
    let handle = request.handle.as_str();
    let offset = request.offset;
    let length = request
        .length
        .filter(|length| (1..=131_072).contains(length))
        .and_then(|length| usize::try_from(length).ok());
    for server in &blobs.servers {
        if server.owns(handle) {
            let reply = match (offset, length) {
                (Some(offset), Some(length)) => server.read(handle, offset, length).ok(),
                _ => None,
            };
            return Ok(Some(reply.unwrap_or_else(|| unavailable(handle))));
        }
    }
    for (prefix, source) in &blobs.prefixed {
        if !handle.starts_with(prefix) {
            continue;
        }
        let Some(mut reader) = lock_within(slot(readers, source)?, BLOB_LOCK_WAIT)? else {
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

fn committed(
    gate: &Gate,
    readers: &Readers,
    store: Option<&Store>,
    to_helper: &Mutex<UnixStream>,
    frame: &Committed,
) -> Result<bool, Error> {
    let source = frame.source.as_str();
    let shared = gate.lock()?;
    let Some(flight) = shared.flights.get(source) else {
        return Ok(true);
    };
    if flight.origin.rust && !flight.matches(&frame.checkpoint, frame.credit) {
        return Err(Error::Conflict);
    }
    let (rust, last) = (flight.origin.rust, flight.origin.last);
    drop(shared);
    let mut token = None;
    if rust && last {
        let mut reader = lock(readers, source)?;
        reader.commit()?;
        token = reader.token();
    }
    let flight = gate
        .lock()?
        .flights
        .complete(source, rust, holds_facts(source))
        .ok_or(Error::Conflict)?;
    gate.changed.notify_all();
    if rust {
        if last {
            let point = Point::Completed(flight.generation.clone(), flight.checkpoint.clone());
            persist(gate, token, store, source, point)?;
            release(to_helper, source, &flight.origin.covers)?;
        }
        return Ok(false);
    }
    Ok(true)
}

fn gateway_frame(
    gate: &Gate,
    blobs: &Blobs,
    readers: &Readers,
    store: Option<&Store>,
    to_gateway: &Mutex<UnixStream>,
    to_helper: &Mutex<UnixStream>,
    frame: GatewayFrame,
) -> Result<bool, Error> {
    match frame {
        GatewayFrame::BlobRead(request) => {
            let Some(reply) = blob_reply(blobs, readers, &request)? else {
                return Ok(true);
            };
            send(
                &mut *to_gateway.lock().map_err(|_| Error::WorkerPanicked)?,
                &reply,
            )?;
            Ok(false)
        }
        GatewayFrame::Open(open) => {
            let mut shared = gate.lock()?;
            shared.fence = open.owner_fence;
            let committed = open
                .checkpoints
                .into_iter()
                .map(|(source, point)| (source, (point.generation, point.checkpoint)))
                .collect();
            shared.flights.open(open.byte_credit, committed);
            if let Some(store) = store {
                for source in readers.keys() {
                    let Some(record) = store.load(source) else {
                        continue;
                    };
                    if !record.native.is_empty()
                        && shared
                            .flights
                            .committed(source)
                            .is_some_and(|(g, _)| *g == record.generation)
                    {
                        shared.bound.insert(
                            source.clone(),
                            (record.native.clone(), record.generation.clone()),
                        );
                    }
                    let matches = shared
                        .flights
                        .committed(source)
                        .is_some_and(|(g, c)| *g == record.generation && *c == record.checkpoint);
                    if matches && lock(readers, source)?.resume(&record.token) {
                        shared
                            .saved
                            .insert(source.clone(), (record.checkpoint, record.token));
                    }
                }
            }
            shared.opened = true;
            drop(shared);
            gate.changed.notify_all();
            Ok(true)
        }
        GatewayFrame::Committed(frame) => committed(gate, readers, store, to_helper, &frame),
        GatewayFrame::Dispatch(dispatch) => {
            note_dispatch(&mut *gate.lock()?, dispatch);
            Ok(true)
        }
        GatewayFrame::Query(_) | GatewayFrame::Other => Ok(true),
    }
}

pub fn relay(
    gateway: UnixStream,
    helper: UnixStream,
    readers: BTreeMap<String, Box<dyn CaptureReader>>,
    poll: Duration,
) -> Result<(), Error> {
    relay_layered(
        gateway,
        helper,
        readers,
        &Options {
            poll,
            ..Options::default()
        },
        None,
    )
}

#[allow(clippy::too_many_lines)]
pub fn relay_layered(
    gateway: UnixStream,
    helper: UnixStream,
    readers: BTreeMap<String, Box<dyn CaptureReader>>,
    options: &Options,
    layer: Option<&dyn Overlay>,
) -> Result<(), Error> {
    let rust: BTreeSet<String> = readers.keys().cloned().collect();
    let targets: Targets = readers
        .iter()
        .map(|(source, reader)| (source.clone(), reader.watch_paths()))
        .collect();
    let store = options.state.as_deref().map(Store::open).transpose()?;
    let store = store.as_ref();
    let gate = Gate {
        shared: Mutex::new(Shared::default()),
        changed: Condvar::new(),
    };
    let blobs = Blobs {
        servers: readers
            .values()
            .filter_map(|reader| reader.blobs())
            .chain(options.queries.blob_server())
            .collect(),
        prefixed: readers
            .iter()
            .filter_map(|(source, reader)| Some((reader.handle_prefix()?, source.clone())))
            .collect(),
    };
    let readers: Readers = readers
        .into_iter()
        .map(|(source, reader)| (source, Mutex::new(reader)))
        .collect();
    let held: Held = Mutex::new(BTreeMap::new());
    let to_gateway = Arc::new(Mutex::new(gateway.try_clone()?));
    let to_helper = Mutex::new(helper.try_clone()?);
    let mut gateway_read = gateway;
    let mut helper_read = helper;
    let gateway_shutdown = gateway_read.try_clone()?;
    let helper_shutdown = helper_read.try_clone()?;
    thread::scope(|scope| {
        let upstream = scope.spawn(|| -> Result<(), Error> {
            let result = (|| loop {
                let (bytes, value) = read_frame(&mut gateway_read)?;
                let typed = GatewayFrame::from_value(&value).map_err(|_| Error::Invalid)?;
                if options.queries.handles(&typed) {
                    let (to_gateway, queries) = (&to_gateway, &options.queries);
                    scope.spawn(move || {
                        let reply = queries.answer(&value);
                        if let Ok(mut output) = to_gateway.lock() {
                            let _ = send_large(&mut output, &reply);
                        }
                    });
                    continue;
                }
                let layered = match layer {
                    Some(layer) => layer.gateway(scope, &value, &to_gateway, &to_helper, &gate)?,
                    None => true,
                };
                let pass = gateway_frame(
                    &gate,
                    &blobs,
                    &readers,
                    store,
                    &to_gateway,
                    &to_helper,
                    typed,
                )?;
                if layered && pass {
                    to_helper
                        .lock()
                        .map_err(|_| Error::WorkerPanicked)?
                        .write_all(&bytes)?;
                }
            })();
            gate.close();
            let _ = helper_shutdown.shutdown(Shutdown::Both);
            let _ = gateway_shutdown.shutdown(Shutdown::Both);
            if let Some(layer) = layer {
                layer.shutdown();
            }
            result
        });
        let producer = (!rust.is_empty()).then(|| {
            scope.spawn(|| -> Result<(), Error> {
                let result = produce(
                    &gate,
                    &readers,
                    &held,
                    &to_helper,
                    &to_gateway,
                    layer,
                    store,
                    options.poll,
                );
                gate.close();
                if result.is_err() {
                    let _ = helper_shutdown.shutdown(Shutdown::Both);
                    let _ = gateway_shutdown.shutdown(Shutdown::Both);
                }
                result
            })
        });
        let watcher = (options.watch && !rust.is_empty()).then(|| {
            let targets = &targets;
            let gate = &gate;
            scope.spawn(move || watch(gate, targets))
        });
        let result = (|| -> Result<(), Error> {
            loop {
                let (bytes, mut value) = read_frame(&mut helper_read)?;
                match HelperFrame::from_value(&value).map_err(|_| Error::Invalid)? {
                    HelperFrame::Capture(CaptureHeader {
                        source,
                        checkpoint,
                        generation,
                    }) => {
                        let size = u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
                        if rust.contains(&source) {
                            ingest(
                                &gate,
                                &readers,
                                &held,
                                &to_helper,
                                &source,
                                &value,
                                (checkpoint, size),
                            )?;
                            continue;
                        }
                        register(&gate, &source, &value, generation, checkpoint, size)?;
                    }
                    HelperFrame::Outcome | HelperFrame::Rejected if !rust.is_empty() => {
                        if forward_outcome(&gate, &to_gateway, &mut value)? {
                            if let Some(layer) = layer {
                                layer.helper(&value)?;
                            }
                            continue;
                        }
                    }
                    HelperFrame::Ready => {
                        if let Some(layer) = layer {
                            layer.helper(&value)?;
                        }
                        to_gateway
                            .lock()
                            .map_err(|_| Error::WorkerPanicked)?
                            .write_all(&bytes)?;
                        gate.lock()?.ready = true;
                        gate.changed.notify_all();
                        if let Some(layer) = layer {
                            layer.helper_ready(&to_gateway)?;
                        }
                        continue;
                    }
                    _ => {}
                }
                if let Some(layer) = layer {
                    layer.helper(&value)?;
                }
                to_gateway
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .write_all(&bytes)?;
            }
        })();
        gate.close();
        let _ = helper_read.shutdown(Shutdown::Both);
        let _ = gateway_shutdown.shutdown(Shutdown::Both);
        if let Some(layer) = layer {
            layer.shutdown();
        }
        let upstream = upstream.join().map_err(|_| Error::WorkerPanicked)?;
        let producer = producer.map_or(Ok(()), |p| p.join().map_err(|_| Error::WorkerPanicked)?);
        if let Some(watcher) = watcher {
            let _ = watcher.join();
        }
        result.and(upstream).and(producer)
    })
}
