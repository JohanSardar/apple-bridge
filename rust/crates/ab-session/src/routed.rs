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
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
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
    dispatched: BTreeMap<String, u64>,
}

#[derive(Debug)]
struct Gate {
    wake: AtomicBool,
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

    fn poke(&self) {
        self.wake.store(true, Ordering::SeqCst);
        drop(self.shared.lock());
        self.changed.notify_all();
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

fn persist(
    gate: &Gate,
    token: Option<String>,
    store: Option<&Store>,
    source: &str,
) -> Result<(), Error> {
    let Some(store) = store else {
        return Ok(());
    };
    let Some(token) = token else {
        return Ok(());
    };
    let shared = gate.lock()?;
    if shared.flights.contains(source) {
        return Ok(());
    }
    let Some((generation, checkpoint)) = shared.flights.committed(source).cloned() else {
        return Ok(());
    };
    if shared.saved.get(source) == Some(&(checkpoint.clone(), token.clone())) {
        return Ok(());
    }
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
        },
    )?;
    gate.lock()?
        .saved
        .insert(source.to_owned(), (checkpoint, token));
    Ok(())
}

fn watch(gate: &Gate, roots: &[std::path::PathBuf]) {
    let Ok(mut watcher) = ab_sys::watch::Watcher::new() else {
        return;
    };
    loop {
        let mut failed = false;
        for root in roots {
            failed |= watcher.watch_tree(root).is_err();
        }
        if failed {
            thread::sleep(Duration::from_millis(100));
        }
        match watcher.wait(Duration::from_millis(250)) {
            Ok(true) => {
                thread::sleep(Duration::from_millis(15));
                while matches!(watcher.wait(Duration::ZERO), Ok(true)) {}
                gate.poke();
            }
            Ok(false) => {}
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
    gate.poke();
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
) -> Result<bool, Error> {
    let generation = text(capture, "generation")?.to_owned();
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
    if !wait_open(gate)? {
        return Ok(());
    }
    loop {
        let mut again: Vec<&String> = Vec::new();
        for source in &sources {
            if gate.lock()?.flights.contains(source) {
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
            let (next, covers, token, more) = {
                let mut reader = lock(readers, source)?;
                let next = reader.next(&context);
                let covers = held
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .remove(source)
                    .unwrap_or_default();
                let more = next.is_ok() && reader.more();
                (next, covers, reader.token(), more)
            };
            if more {
                again.push(source);
            }
            let capture = match next {
                Ok(Some(capture)) => capture,
                Ok(None) => {
                    release(to_helper, source, &covers)?;
                    persist(gate, token, store, source)?;
                    continue;
                }
                Err(error) => {
                    eprintln!("ab-session: {source} reader: {error}");
                    release(to_helper, source, &covers)?;
                    continue;
                }
            };
            if !send_scan(gate, to_gateway, layer, source, &capture, covers)? {
                return Ok(());
            }
        }
        let shared = gate.lock()?;
        if shared.closed {
            return Ok(());
        }
        if gate.wake.swap(false, Ordering::SeqCst) {
            continue;
        }
        if again.iter().any(|source| !shared.flights.contains(source)) {
            continue;
        }
        let (shared, _) = gate
            .changed
            .wait_timeout(shared, poll)
            .map_err(|_| Error::WorkerPanicked)?;
        if shared.closed {
            return Ok(());
        }
    }
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
            persist(gate, token, store, source)?;
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
    let roots: Vec<std::path::PathBuf> = readers
        .values()
        .flat_map(|reader| reader.watch_paths())
        .collect();
    let store = options.state.as_deref().map(Store::open).transpose()?;
    let store = store.as_ref();
    let gate = Gate {
        wake: AtomicBool::new(false),
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
            let roots = &roots;
            let gate = &gate;
            scope.spawn(move || watch(gate, roots))
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
