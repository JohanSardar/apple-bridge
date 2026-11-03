use crate::{
    Error,
    frame::{MAX_FRAME, read_frame, send, text},
    readers::{CaptureReader, ScanContext},
    store::{Record, Store},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Readers = Mutex<BTreeMap<String, Box<dyn CaptureReader>>>;

pub trait Overlay: Sync {
    fn gateway<'scope>(
        &'scope self,
        scope: &'scope thread::Scope<'scope, '_>,
        value: &Value,
        output: &Arc<Mutex<UnixStream>>,
    ) -> Result<bool, Error>;
    fn helper(&self, value: &Value, size: usize) -> Result<(), Error>;
    fn helper_ready(&self, output: &Mutex<UnixStream>) -> Result<(), Error>;
    fn shutdown(&self);
}

#[derive(Debug, Clone)]
pub struct Options {
    pub poll: Duration,
    pub watch: bool,
    pub state: Option<std::path::PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(100),
            watch: false,
            state: None,
        }
    }
}

#[derive(Debug)]
struct Flight {
    generation: String,
    checkpoint: String,
    bytes: u64,
    rust: bool,
    keys: Vec<String>,
}

#[derive(Debug, Default)]
struct Shared {
    opened: bool,
    ready: bool,
    closed: bool,
    credit: u64,
    total: u64,
    committed: BTreeMap<String, (String, String)>,
    flights: BTreeMap<String, Flight>,
    saved: BTreeMap<String, String>,
    sequence: u64,
    observed: BTreeMap<String, (String, u64)>,
    dispatched: BTreeMap<String, u64>,
}

#[derive(Debug)]
struct Gate {
    shared: Mutex<Shared>,
    changed: Condvar,
}

impl Gate {
    fn lock(&self) -> Result<MutexGuard<'_, Shared>, Error> {
        self.shared.lock().map_err(|_| Error::WorkerPanicked)
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
    readers: &Readers,
    store: Option<&Store>,
    source: &str,
) -> Result<(), Error> {
    let Some(store) = store else {
        return Ok(());
    };
    let token = readers
        .lock()
        .map_err(|_| Error::WorkerPanicked)?
        .get(source)
        .and_then(|reader| reader.token());
    let Some(token) = token else {
        return Ok(());
    };
    let shared = gate.lock()?;
    if shared.flights.contains_key(source) || shared.saved.get(source) == Some(&token) {
        return Ok(());
    }
    let Some((generation, checkpoint)) = shared.committed.get(source).cloned() else {
        return Ok(());
    };
    drop(shared);
    if generation.is_empty() || checkpoint.is_empty() {
        return Ok(());
    }
    store.save(
        source,
        &Record {
            generation,
            checkpoint,
            token: token.clone(),
        },
    )?;
    gate.lock()?.saved.insert(source.to_owned(), token);
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
                gate.changed.notify_all();
            }
            Ok(false) => {}
            Err(_) => return,
        }
        if gate.shared.lock().map_or(true, |shared| shared.closed) {
            return;
        }
    }
}

fn capture_keys(capture: &Value) -> Vec<String> {
    capture["observations"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["native_key"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn note_dispatch(shared: &mut Shared, value: &Value) {
    let sequence = shared.sequence;
    if let Some(id) = value["command_id"].as_str() {
        shared.dispatched.insert(id.to_owned(), sequence);
    }
}

fn note_observed(shared: &mut Shared, flight: &Flight) {
    shared.sequence += 1;
    let sequence = shared.sequence;
    for key in &flight.keys {
        shared
            .observed
            .insert(key.clone(), (flight.checkpoint.clone(), sequence));
    }
}

fn produce(
    gate: &Gate,
    readers: &Readers,
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
    let sources: Vec<String> = readers
        .lock()
        .map_err(|_| Error::WorkerPanicked)?
        .keys()
        .cloned()
        .collect();
    if !wait_open(gate)? {
        return Ok(());
    }
    loop {
        for source in &sources {
            if gate.lock()?.flights.contains_key(source) {
                continue;
            }
            ordinal += 1;
            let context = ScanContext {
                checkpoint: format!("rs-{epoch:x}-{ordinal}"),
                scan_id: format!("rs-scan-{epoch:x}-{ordinal}"),
                now: now()?,
            };
            let next = {
                let mut guard = readers.lock().map_err(|_| Error::WorkerPanicked)?;
                let reader = guard.get_mut(source).ok_or(Error::Invalid)?;
                reader.next(&context)
            };
            let mut capture = match next {
                Ok(Some(capture)) => capture,
                Ok(None) => {
                    persist(gate, readers, store, source)?;
                    continue;
                }
                Err(error) => {
                    eprintln!("ab-session: {source} reader: {error}");
                    continue;
                }
            };
            let generation = text(&capture, "generation")?.to_owned();
            let checkpoint = text(&capture, "checkpoint")?.to_owned();
            let predecessor = gate
                .lock()?
                .committed
                .get(source)
                .filter(|(committed_generation, _)| *committed_generation == generation)
                .map(|(_, checkpoint)| checkpoint.clone())
                .unwrap_or_default();
            capture["predecessor"] = Value::String(predecessor);
            capture["t"] = json!("Capture");
            let bytes = frame_bytes(&capture)?;
            let size = u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
            let mut shared = gate.lock()?;
            if size > shared.total {
                return Err(Error::Invalid);
            }
            while shared.credit < size {
                if shared.closed {
                    return Ok(());
                }
                shared = gate
                    .changed
                    .wait_timeout(shared, Duration::from_millis(100))
                    .map_err(|_| Error::WorkerPanicked)?
                    .0;
            }
            shared.credit -= size;
            shared.flights.insert(
                source.clone(),
                Flight {
                    generation,
                    checkpoint,
                    bytes: size,
                    rust: true,
                    keys: capture_keys(&capture),
                },
            );
            drop(shared);
            if let Some(layer) = layer {
                layer.helper(&capture, usize::try_from(size).map_err(|_| Error::Invalid)?)?;
            }
            to_gateway
                .lock()
                .map_err(|_| Error::WorkerPanicked)?
                .write_all(&bytes)?;
        }
        let shared = gate.lock()?;
        if shared.closed {
            return Ok(());
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

fn blob_reply(readers: &Readers, value: &Value) -> Result<Option<Value>, Error> {
    let handle = text(value, "handle")?;
    let mut guard = readers.lock().map_err(|_| Error::WorkerPanicked)?;
    let Some(reader) = guard.values_mut().find(|reader| reader.owns(handle)) else {
        return Ok(None);
    };
    let offset = value["offset"].as_u64();
    let length = value["length"]
        .as_u64()
        .filter(|length| (1..=131_072).contains(length))
        .and_then(|length| usize::try_from(length).ok());
    let reply = match (offset, length) {
        (Some(offset), Some(length)) => reader.blob(handle, offset, length).ok(),
        _ => None,
    };
    Ok(Some(reply.unwrap_or_else(
        || json!({"t":"BlobUnavailable","handle":handle,"reason":"unavailable"}),
    )))
}

fn gateway_frame(
    gate: &Gate,
    readers: &Readers,
    store: Option<&Store>,
    to_gateway: &Mutex<UnixStream>,
    value: &Value,
) -> Result<bool, Error> {
    match value["t"].as_str() {
        Some("BlobRead") => {
            let Some(reply) = blob_reply(readers, value)? else {
                return Ok(true);
            };
            send(
                &mut *to_gateway.lock().map_err(|_| Error::WorkerPanicked)?,
                &reply,
            )?;
            Ok(false)
        }
        Some("Open") => {
            let mut shared = gate.lock()?;
            let credit = value["byte_credit"].as_u64().unwrap_or(0);
            shared.credit = credit;
            shared.total = credit;
            if let Some(checkpoints) = value["checkpoints"].as_object() {
                for (source, checkpoint) in checkpoints {
                    shared.committed.insert(
                        source.clone(),
                        (
                            checkpoint["generation"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            checkpoint["checkpoint"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                        ),
                    );
                }
            }
            if let Some(store) = store {
                let mut guard = readers.lock().map_err(|_| Error::WorkerPanicked)?;
                for (source, reader) in guard.iter_mut() {
                    let Some(record) = store.load(source) else {
                        continue;
                    };
                    let matches = shared
                        .committed
                        .get(source)
                        .is_some_and(|(g, c)| *g == record.generation && *c == record.checkpoint);
                    if matches && reader.resume(&record.token) {
                        shared.saved.insert(source.clone(), record.token);
                    }
                }
            }
            shared.opened = true;
            drop(shared);
            gate.changed.notify_all();
            Ok(true)
        }
        Some("Committed") => {
            let source = text(value, "source")?;
            let mut shared = gate.lock()?;
            let Some(flight) = shared.flights.get(source) else {
                return Ok(true);
            };
            if flight.rust
                && (value["checkpoint"] != flight.checkpoint
                    || value["credit"].as_u64() != Some(flight.bytes))
            {
                return Err(Error::Conflict);
            }
            let flight = shared.flights.remove(source).ok_or(Error::Conflict)?;
            shared.credit = shared.credit.saturating_add(flight.bytes);
            shared.committed.insert(
                source.to_owned(),
                (flight.generation.clone(), flight.checkpoint.clone()),
            );
            drop(shared);
            if flight.rust {
                note_observed(&mut *gate.lock()?, &flight);
            }
            gate.changed.notify_all();
            if flight.rust {
                readers
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .get_mut(source)
                    .ok_or(Error::Conflict)?
                    .commit()?;
                persist(gate, readers, store, source)?;
                return Ok(false);
            }
            Ok(true)
        }
        Some("Dispatch") => {
            note_dispatch(&mut *gate.lock()?, value);
            Ok(true)
        }
        _ => Ok(true),
    }
}

fn bind_outcome(gate: &Gate, value: &mut Value) -> Result<bool, Error> {
    let key = match value["evidence"]["native_key"].as_str() {
        Some(key) if value["evidence"]["capture_checkpoint"].is_string() => key.to_owned(),
        _ => return Ok(false),
    };
    let mut shared = gate.lock()?;
    let after = value["command_id"]
        .as_str()
        .and_then(|id| shared.dispatched.remove(id))
        .unwrap_or(shared.sequence);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some((checkpoint, sequence)) = shared.observed.get(&key)
            && *sequence > after
        {
            value["evidence"]["capture_checkpoint"] = Value::String(checkpoint.clone());
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

fn helper_capture(
    gate: &Gate,
    to_helper: &Mutex<UnixStream>,
    rust: &BTreeSet<String>,
    value: &Value,
    length: usize,
) -> Result<bool, Error> {
    let source = text(value, "source")?;
    let checkpoint = value["checkpoint"].as_str().unwrap_or_default().to_owned();
    let generation = value["generation"].as_str().unwrap_or_default().to_owned();
    let size = u64::try_from(length).map_err(|_| Error::Invalid)?;
    if rust.contains(source) {
        send(
            &mut *to_helper.lock().map_err(|_| Error::WorkerPanicked)?,
            &json!({"t":"Committed","source":source,"checkpoint":checkpoint,"credit":size}),
        )?;
        return Ok(true);
    }
    let mut shared = gate.lock()?;
    shared.credit = shared.credit.saturating_sub(size);
    let previous = shared.flights.insert(
        source.to_owned(),
        Flight {
            generation,
            checkpoint,
            bytes: size,
            rust: false,
            keys: Vec::new(),
        },
    );
    if previous.is_some() {
        return Err(Error::Conflict);
    }
    Ok(false)
}

fn forward_outcome(
    gate: &Gate,
    to_gateway: &Mutex<UnixStream>,
    value: &mut Value,
) -> Result<bool, Error> {
    if !bind_outcome(gate, value)? {
        return Ok(false);
    }
    send(
        &mut *to_gateway.lock().map_err(|_| Error::WorkerPanicked)?,
        value,
    )?;
    Ok(true)
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
        .map(|reader| reader.root().to_path_buf())
        .collect();
    let store = options.state.as_deref().map(Store::open).transpose()?;
    let store = store.as_ref();
    let gate = Gate {
        shared: Mutex::new(Shared::default()),
        changed: Condvar::new(),
    };
    let readers = Mutex::new(readers);
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
                let layered = match layer {
                    Some(layer) => layer.gateway(scope, &value, &to_gateway)?,
                    None => true,
                };
                let pass = gateway_frame(&gate, &readers, store, &to_gateway, &value)?;
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
                let result = produce(&gate, &readers, &to_gateway, layer, store, options.poll);
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
                match value["t"].as_str() {
                    Some("Capture") => {
                        if helper_capture(&gate, &to_helper, &rust, &value, bytes.len())? {
                            continue;
                        }
                    }
                    Some("Outcome") if !rust.is_empty() => {
                        if forward_outcome(&gate, &to_gateway, &mut value)? {
                            if let Some(layer) = layer {
                                layer.helper(&value, bytes.len())?;
                            }
                            continue;
                        }
                    }
                    Some("Ready") => {
                        if let Some(layer) = layer {
                            layer.helper(&value, bytes.len())?;
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
                    layer.helper(&value, bytes.len())?;
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
