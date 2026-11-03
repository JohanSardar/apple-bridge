use crate::{
    Error,
    frame::{MAX_FRAME, read_frame, send, text},
    readers::{CaptureReader, ScanContext},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Condvar, Mutex, MutexGuard},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
struct Flight {
    generation: String,
    checkpoint: String,
    bytes: u64,
    rust: bool,
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

fn frame_bytes(capture: &Value) -> Result<Vec<u8>, Error> {
    let mut frame = capture.clone();
    frame["t"] = json!("Capture");
    let body = ab_native::native_json::record_data(&frame)
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

fn produce(
    gate: &Gate,
    readers: &Mutex<BTreeMap<String, Box<dyn CaptureReader>>>,
    to_gateway: &Mutex<UnixStream>,
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
            let mut capture = {
                let mut readers = readers.lock().map_err(|_| Error::WorkerPanicked)?;
                let reader = readers.get_mut(source).ok_or(Error::Invalid)?;
                match reader.next(&context) {
                    Ok(Some(capture)) => capture,
                    Ok(None) => continue,
                    Err(error) => {
                        eprintln!("ab-session: {source} reader: {error}");
                        continue;
                    }
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
                },
            );
            drop(shared);
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

fn gateway_frame(
    gate: &Gate,
    readers: &Mutex<BTreeMap<String, Box<dyn CaptureReader>>>,
    value: &Value,
) -> Result<bool, Error> {
    match value["t"].as_str() {
        Some("Open") => {
            let mut shared = gate.lock()?;
            let credit = value["byte_credit"].as_u64().ok_or(Error::Invalid)?;
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
            gate.changed.notify_all();
            if flight.rust {
                readers
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .get_mut(source)
                    .ok_or(Error::Conflict)?
                    .commit()?;
                return Ok(false);
            }
            Ok(true)
        }
        _ => Ok(true),
    }
}

pub fn relay(
    gateway: UnixStream,
    helper: UnixStream,
    readers: BTreeMap<String, Box<dyn CaptureReader>>,
    poll: Duration,
) -> Result<(), Error> {
    let rust: BTreeSet<String> = readers.keys().cloned().collect();
    let gate = Gate {
        shared: Mutex::new(Shared::default()),
        changed: Condvar::new(),
    };
    let readers = Mutex::new(readers);
    let to_gateway = Mutex::new(gateway.try_clone()?);
    let to_helper = Mutex::new(helper.try_clone()?);
    let mut gateway_read = gateway;
    let mut helper_read = helper;
    let gateway_shutdown = gateway_read.try_clone()?;
    let helper_shutdown = helper_read.try_clone()?;
    thread::scope(|scope| {
        let upstream = scope.spawn(|| -> Result<(), Error> {
            let result = (|| loop {
                let (bytes, value) = read_frame(&mut gateway_read)?;
                if gateway_frame(&gate, &readers, &value)? {
                    to_helper
                        .lock()
                        .map_err(|_| Error::WorkerPanicked)?
                        .write_all(&bytes)?;
                }
            })();
            gate.close();
            let _ = helper_shutdown.shutdown(Shutdown::Both);
            let _ = gateway_shutdown.shutdown(Shutdown::Both);
            result
        });
        let producer = scope.spawn(|| -> Result<(), Error> {
            let result = produce(&gate, &readers, &to_gateway, poll);
            gate.close();
            if result.is_err() {
                let _ = helper_shutdown.shutdown(Shutdown::Both);
                let _ = gateway_shutdown.shutdown(Shutdown::Both);
            }
            result
        });
        let result = (|| -> Result<(), Error> {
            loop {
                let (bytes, value) = read_frame(&mut helper_read)?;
                match value["t"].as_str() {
                    Some("Capture") => {
                        let source = text(&value, "source")?;
                        let checkpoint =
                            value["checkpoint"].as_str().unwrap_or_default().to_owned();
                        let generation =
                            value["generation"].as_str().unwrap_or_default().to_owned();
                        let size = u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
                        if rust.contains(source) {
                            send(
                                &mut *to_helper.lock().map_err(|_| Error::WorkerPanicked)?,
                                &json!({"t":"Committed","source":source,"checkpoint":checkpoint,"credit":size}),
                            )?;
                            continue;
                        }
                        let mut shared = gate.lock()?;
                        shared.credit = shared.credit.saturating_sub(size);
                        if shared
                            .flights
                            .insert(
                                source.to_owned(),
                                Flight {
                                    generation,
                                    checkpoint,
                                    bytes: size,
                                    rust: false,
                                },
                            )
                            .is_some()
                        {
                            return Err(Error::Conflict);
                        }
                    }
                    Some("Ready") => {
                        to_gateway
                            .lock()
                            .map_err(|_| Error::WorkerPanicked)?
                            .write_all(&bytes)?;
                        gate.lock()?.ready = true;
                        gate.changed.notify_all();
                        continue;
                    }
                    _ => {}
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
        let upstream = upstream.join().map_err(|_| Error::WorkerPanicked)?;
        let producer = producer.join().map_err(|_| Error::WorkerPanicked)?;
        result.and(upstream).and(producer)
    })
}
