use crate::{
    Error,
    flight::{Flight, Flights, Refused, typed_rows},
};
use ab_wire::{
    crypto::ipc_proof,
    framing::MAX_FRAME,
    messages::{IpcFrame, Observation},
    native::{
        self, ErrorCode, Frame as Link, Scan, ScanCommitted, ScanFailed, Session, Source,
    },
};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    io::{Read, Write},
    net::Shutdown,
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::Path,
    sync::{
        Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BULK_CHUNK_BYTES: usize = 16_000;
const LIVE_CHUNK_BYTES: usize = 64_000;
const CALENDAR_LIVE_CHUNK_BYTES: usize = 240_000;
const OBSERVATION_LIMIT: usize = 240_000;
const MAX_CREDIT: i64 = 4_194_304;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

struct Origin {
    scan: String,
    last: bool,
}

struct Core {
    flights: Flights<Origin>,
    closed: bool,
}

struct Hub {
    core: Mutex<Core>,
    changed: Condvar,
    gateway: Mutex<UnixStream>,
    link: Mutex<UnixStream>,
    sequence: AtomicU64,
    epoch: u128,
}

impl Hub {
    fn core(&self) -> Result<MutexGuard<'_, Core>, Error> {
        self.core.lock().map_err(|_| Error::WorkerPanicked)
    }

    fn send_gateway(&self, frame: &IpcFrame) -> Result<(), Error> {
        let bytes = frame.encode().map_err(|_| Error::Invalid)?;
        self.write_gateway(&bytes)
    }

    fn write_gateway(&self, bytes: &[u8]) -> Result<(), Error> {
        self.gateway
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .write_all(bytes)?;
        Ok(())
    }

    fn send_link(&self, frame: &Link) -> Result<(), Error> {
        let bytes = frame.encode().map_err(|_| Error::Invalid)?;
        self.link
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .write_all(&bytes)?;
        Ok(())
    }

    fn close(&self) {
        if let Ok(mut core) = self.core.lock() {
            core.closed = true;
        }
        self.changed.notify_all();
        if let Ok(stream) = self.gateway.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Ok(stream) = self.link.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn next_checkpoint(&self) -> String {
        let n = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        format!("ses-{:x}-{n}", self.epoch)
    }
}

pub fn load_secret(path: &Path) -> Result<Vec<u8>, Error> {
    let file = std::fs::symlink_metadata(path)?;
    let parent = path.parent().ok_or(Error::Invalid)?;
    let directory = std::fs::symlink_metadata(parent)?;
    if !file.file_type().is_file()
        || file.mode() & 0o777 != 0o600
        || !directory.file_type().is_dir()
        || directory.mode() & 0o777 != 0o700
        || file.uid() != directory.uid()
    {
        return Err(Error::Invalid);
    }
    let secret = std::fs::read(path)?;
    if secret.len() != 32 {
        return Err(Error::Invalid);
    }
    Ok(secret)
}

fn name(source: Source) -> Result<String, Error> {
    serde_json::to_value(source)?
        .as_str()
        .map(str::to_owned)
        .ok_or(Error::Invalid)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn random_nonce() -> Result<String, Error> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(hex(&bytes))
}

fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0, |a, (l, r)| a | (l ^ r)) == 0
}

fn read_ipc(stream: &mut UnixStream) -> Result<IpcFrame, Error> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header)?;
    let size = usize::try_from(u32::from_be_bytes(header)).map_err(|_| Error::Invalid)?;
    if size == 0 || size > MAX_FRAME {
        return Err(Error::Invalid);
    }
    let mut bytes = vec![0_u8; size];
    stream.read_exact(&mut bytes)?;
    let frame: IpcFrame = serde_json::from_slice(&bytes)?;
    if frame.t.is_empty() {
        return Err(Error::Invalid);
    }
    Ok(frame)
}

fn authenticate(gateway: &mut UnixStream, secret: &[u8]) -> Result<(), Error> {
    let challenge = read_ipc(gateway)?;
    if challenge.t != "Challenge" || challenge.nonce.len() != 64 {
        return Err(Error::Invalid);
    }
    let mine = random_nonce()?;
    let proof = |role: &str| -> Result<String, Error> {
        Ok(hex(
            &ipc_proof(secret, role, &challenge.nonce, &mine).map_err(|_| Error::Invalid)?
        ))
    };
    let frame = IpcFrame {
        t: "Authenticate".into(),
        nonce: mine.clone(),
        proof: proof("helper")?,
        ..IpcFrame::default()
    };
    gateway.write_all(&frame.encode().map_err(|_| Error::Invalid)?)?;
    let answer = read_ipc(gateway)?;
    if answer.t != "Authenticated" || !same(answer.proof.as_bytes(), proof("gateway")?.as_bytes()) {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn open(frame: &IpcFrame) -> Result<(u64, u64), Error> {
    if frame.t != "Open"
        || frame.protocol != "ipc/1"
        || frame.owner_fence == 0
        || frame.byte_credit <= 0
        || frame.byte_credit > MAX_CREDIT
    {
        return Err(Error::Invalid);
    }
    Ok((
        frame.owner_fence,
        u64::try_from(frame.byte_credit).map_err(|_| Error::Invalid)?,
    ))
}

fn committed_map(frame: &IpcFrame) -> Result<BTreeMap<String, (String, String)>, Error> {
    Ok(Session::from_open(frame)
        .map_err(|_| Error::Invalid)?
        .checkpoints
        .into_iter()
        .map(|(source, checkpoint)| (source, (checkpoint.generation, checkpoint.checkpoint)))
        .collect())
}

fn rejected(command_id: &str, code: &str) -> Result<IpcFrame, Error> {
    let error =
        RawValue::from_string(serde_json::json!({"code": code, "message": code}).to_string())?;
    Ok(IpcFrame {
        t: "Rejected".into(),
        command_id: command_id.into(),
        disposition: "rejected_before_dispatch".into(),
        error: Some(error),
        ..IpcFrame::default()
    })
}

fn gateway_frame(hub: &Hub, frame: &IpcFrame) -> Result<bool, Error> {
    match frame.t.as_str() {
        "Ping" => hub.send_gateway(&IpcFrame {
            t: "Pong".into(),
            ..IpcFrame::default()
        })?,
        "Pong" | "QueryCancel" => {}
        "Committed" => committed(hub, frame)?,
        "Query" => hub.send_gateway(&IpcFrame {
            t: "QueryError".into(),
            id: frame.id.clone(),
            code: "unsupported".into(),
            ..IpcFrame::default()
        })?,
        "Prepare" => hub.send_gateway(&rejected(&frame.command_id, "unsupported")?)?,
        "Dispatch" => hub.send_gateway(&rejected(&frame.command_id, "conflict")?)?,
        "Resolve" => hub.send_gateway(&rejected(&frame.command_id, "not_found")?)?,
        "BlobRead" => hub.send_gateway(&IpcFrame {
            t: "BlobUnavailable".into(),
            handle: frame.handle.clone(),
            reason: "unavailable".into(),
            ..IpcFrame::default()
        })?,
        "Close" => return Ok(false),
        _ => return Err(Error::Invalid),
    }
    Ok(true)
}

fn committed(hub: &Hub, frame: &IpcFrame) -> Result<(), Error> {
    let source = name(frame.source.ok_or(Error::Invalid)?)?;
    let flight = {
        let mut core = hub.core()?;
        let matches = core.flights.get(&source).is_some_and(|flight| {
            flight.matches(&frame.checkpoint, u64::try_from(frame.credit).ok())
        });
        if !matches {
            return Err(Error::Invalid);
        }
        core.flights
            .complete(&source, true, false)
            .ok_or(Error::Conflict)?
    };
    hub.changed.notify_all();
    if flight.origin.last {
        hub.send_link(&Link::ScanCommitted(ScanCommitted {
            id: flight.origin.scan,
            source: frame.source.ok_or(Error::Invalid)?,
            checkpoint: flight.checkpoint,
        }))?;
    }
    Ok(())
}

fn limit(scan: &Scan) -> usize {
    match (scan.live, scan.source) {
        (true, Source::Calendar) => CALENDAR_LIVE_CHUNK_BYTES,
        (true, _) => LIVE_CHUNK_BYTES,
        (false, _) => BULK_CHUNK_BYTES,
    }
}

fn split(scan: &Scan) -> Result<Vec<Vec<Observation>>, Error> {
    let limit = limit(scan);
    let ordered = scan
        .observations
        .iter()
        .filter(|row| row.kind != "absent")
        .chain(scan.observations.iter().filter(|row| row.kind == "absent"));
    let mut groups: Vec<Vec<Observation>> = vec![Vec::new()];
    let mut bytes = 0;
    for row in ordered {
        let size = serde_json::to_vec(row)?.len() + 1;
        if size >= OBSERVATION_LIMIT {
            return Err(Error::Invalid);
        }
        if bytes + size > limit && groups.last().is_some_and(|group| !group.is_empty()) {
            groups.push(Vec::new());
            bytes = 0;
        }
        if let Some(group) = groups.last_mut() {
            group.push(row.clone());
        }
        bytes += size;
    }
    Ok(groups)
}

enum Step {
    Sent,
    Closed,
}

fn send_chunk(
    hub: &Hub,
    scan: &Scan,
    source: &str,
    chunk: &[Observation],
    checkpoint: &str,
    last: bool,
) -> Result<Step, Error> {
    loop {
        let predecessor = {
            let mut core = hub.core()?;
            while core.flights.contains(source) {
                if core.closed {
                    return Ok(Step::Closed);
                }
                core = hub
                    .changed
                    .wait_timeout(core, Duration::from_millis(100))
                    .map_err(|_| Error::WorkerPanicked)?
                    .0;
            }
            core.flights.predecessor(source, &scan.generation)
        };
        let capture = scan
            .capture(chunk.to_vec(), predecessor.clone(), checkpoint.to_owned())
            .map_err(|_| Error::Invalid)?;
        let bytes = IpcFrame {
            t: "Capture".into(),
            capture: Some(capture),
            ..IpcFrame::default()
        }
        .encode()
        .map_err(|_| Error::Invalid)?;
        let size = u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
        let flight = Flight::new(
            scan.generation.clone(),
            checkpoint.to_owned(),
            size,
            typed_rows(chunk),
            Origin {
                scan: scan.id.clone(),
                last,
            },
        );
        let mut core = hub.core()?;
        if core.closed {
            return Ok(Step::Closed);
        }
        if core.flights.predecessor(source, &scan.generation) != predecessor {
            continue;
        }
        match core.flights.reserve(source, flight) {
            Ok(()) => {
                drop(core);
                hub.write_gateway(&bytes)?;
                return Ok(Step::Sent);
            }
            Err(Refused::TooLarge) => return Err(Error::Invalid),
            Err(Refused::Busy) => {}
            Err(Refused::Credit) => {
                drop(
                    hub.changed
                        .wait_timeout(core, Duration::from_millis(100))
                        .map_err(|_| Error::WorkerPanicked)?,
                );
            }
        }
    }
}

fn wait_free(hub: &Hub, source: &str) -> Result<bool, Error> {
    let mut core = hub.core()?;
    while core.flights.contains(source) {
        if core.closed {
            return Ok(false);
        }
        core = hub
            .changed
            .wait_timeout(core, Duration::from_millis(100))
            .map_err(|_| Error::WorkerPanicked)?
            .0;
    }
    Ok(true)
}

fn run_scan(hub: &Hub, scan: &Scan) -> Result<(), Error> {
    let source = name(scan.source)?;
    let chunks = split(scan)?;
    let final_index = chunks.len() - 1;
    let checkpoint = hub.next_checkpoint();
    for (index, chunk) in chunks.iter().enumerate() {
        let last = index == final_index;
        let chunk_checkpoint = if last {
            checkpoint.clone()
        } else {
            format!("{checkpoint}-{index}")
        };
        if matches!(
            send_chunk(hub, scan, &source, chunk, &chunk_checkpoint, last)?,
            Step::Closed
        ) {
            return Ok(());
        }
        if !last && !wait_free(hub, &source)? {
            return Ok(());
        }
    }
    Ok(())
}

fn scan_failed(hub: &Hub, id: &str, error: &Error) {
    let code = match error {
        Error::Invalid => "invalid",
        _ => "unavailable",
    };
    let _ = hub.send_link(&Link::ScanFailed(ScanFailed {
        id: id.to_owned(),
        code: code.into(),
    }));
}

fn start(
    gateway: &mut UnixStream,
    link: &mut UnixStream,
    secret: &[u8],
) -> Result<(IpcFrame, u64), Error> {
    gateway.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    link.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    authenticate(gateway, secret)?;
    let opened = read_ipc(gateway)?;
    let (fence, _) = open(&opened)?;
    link.write_all(
        &Link::Hello { owner_fence: fence }
            .encode()
            .map_err(|_| Error::Invalid)?,
    )?;
    if !matches!(native::read_frame(link), Ok(Link::Welcome)) {
        return Err(Error::Invalid);
    }
    link.write_all(
        &Link::Session(Session::from_open(&opened).map_err(|_| Error::Invalid)?)
            .encode()
            .map_err(|_| Error::Invalid)?,
    )?;
    let Ok(Link::Ready(ready)) = native::read_frame(link) else {
        return Err(Error::Invalid);
    };
    gateway.write_all(&ready.into_ipc().encode().map_err(|_| Error::Invalid)?)?;
    gateway.set_read_timeout(None)?;
    link.set_read_timeout(None)?;
    Ok((opened, fence))
}

pub fn serve(gateway: UnixStream, link: UnixStream, secret: &[u8]) -> Result<(), Error> {
    let mut gateway_read = gateway.try_clone()?;
    let mut link_read = link.try_clone()?;
    let (opened, _) = match start(&mut gateway_read, &mut link_read, secret) {
        Ok(started) => started,
        Err(error) => {
            let _ = gateway.shutdown(Shutdown::Both);
            let _ = link.shutdown(Shutdown::Both);
            return Err(error);
        }
    };
    let mut flights = Flights::default();
    flights.open(
        u64::try_from(opened.byte_credit).map_err(|_| Error::Invalid)?,
        committed_map(&opened)?,
    );
    let hub = Hub {
        core: Mutex::new(Core {
            flights,
            closed: false,
        }),
        changed: Condvar::new(),
        gateway: Mutex::new(gateway),
        link: Mutex::new(link),
        sequence: AtomicU64::new(0),
        epoch: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Invalid)?
            .as_nanos(),
    };
    thread::scope(|scope| {
        let upstream = scope.spawn(|| -> Result<(), Error> {
            let result = (|| loop {
                let frame = match read_ipc(&mut gateway_read) {
                    Ok(frame) => frame,
                    Err(Error::Io(_)) => return Ok(()),
                    Err(error) => return Err(error),
                };
                if !gateway_frame(&hub, &frame)? {
                    return Ok(());
                }
            })();
            hub.close();
            result
        });
        let result = (|| -> Result<(), Error> {
            loop {
                let frame = match native::read_frame(&mut link_read) {
                    Ok(frame) => frame,
                    Err(ErrorCode::Unavailable) => return Ok(()),
                    Err(_) => return Err(Error::Invalid),
                };
                match frame {
                    Link::SourceState(state) => hub.send_gateway(&state.into_ipc())?,
                    Link::Scan(scan) => {
                        let hub = &hub;
                        scope.spawn(move || {
                            if let Err(error) = run_scan(hub, &scan) {
                                scan_failed(hub, &scan.id, &error);
                            }
                        });
                    }
                    Link::Close => return Ok(()),
                    _ => return Err(Error::Invalid),
                }
            }
        })();
        hub.close();
        let upstream = upstream.join().map_err(|_| Error::WorkerPanicked)?;
        result.and(upstream)
    })
}
