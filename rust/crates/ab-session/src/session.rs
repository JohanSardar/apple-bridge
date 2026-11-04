use crate::{
    Error,
    flight::{Flight, Flights, Refused, typed_rows},
};
use ab_wire::{
    crypto::ipc_proof,
    framing::MAX_FRAME,
    messages::{IpcFrame, Observation},
    native::{self, ErrorCode, Frame as Link, Scan, ScanCommitted, ScanFailed, Session, Source},
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
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod calendar;
mod capture;
mod commands;
mod contacts;
mod messages;
mod messages_advertise;
#[cfg(feature = "rust-capture")]
mod messages_cleanup;
#[cfg(feature = "rust-capture")]
mod messages_local;
mod messages_selftest;
#[cfg(feature = "rust-capture")]
mod messages_ui;

pub fn selftest_messages(link: UnixStream, request: &serde_json::Value) -> Result<(), Error> {
    messages_selftest::serve(link, request)
}

#[cfg(feature = "rust-capture")]
pub fn cleanup_messages(link: UnixStream, request: &serde_json::Value) -> Result<(), Error> {
    messages_selftest::serve_cleanup(link, request)
}

mod notes;
mod owned;
mod reminders;

pub use capture::Attached;

pub const RUST_COMMAND_SOURCES: [&str; 14] = [
    "reminders",
    "calendar",
    "contacts",
    "notes",
    "messages",
    "messages.send",
    "messages.reply",
    "messages.react",
    "messages.edit",
    "messages.unsend",
    "messages.delete_message",
    "messages.mark_read",
    "messages.mark_unread",
    "messages.delete_chat",
];

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub rust_commands: Vec<String>,
}

pub trait Clock: Send + Sync {
    fn elapsed(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemClock(std::time::Instant);

impl SystemClock {
    #[must_use]
    pub fn start() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

const BULK_CHUNK_BYTES: usize = 16_000;
const LIVE_CHUNK_BYTES: usize = 64_000;
const CALENDAR_LIVE_CHUNK_BYTES: usize = 240_000;
const OBSERVATION_LIMIT: usize = 240_000;
const MAX_CREDIT: i64 = 4_194_304;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

enum Origin {
    Helper { scan: String, last: bool },
    Reader(capture::ReaderFlight),
}

struct Core {
    flights: Flights<Origin>,
    commands: commands::Commands,
    capture: capture::State,
    closed: bool,
}

struct Hub {
    core: Mutex<Core>,
    changed: Condvar,
    gateway: Mutex<UnixStream>,
    link: Mutex<UnixStream>,
    sequence: AtomicU64,
    epoch: u128,
    fence: u64,
    clock: Arc<dyn Clock>,
    capture: Option<capture::Capture>,
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

fn gateway_frame<'scope>(
    hub: &'scope Hub,
    scope: &'scope thread::Scope<'scope, '_>,
    frame: &IpcFrame,
) -> Result<bool, Error> {
    match frame.t.as_str() {
        "Ping" => hub.send_gateway(&IpcFrame {
            t: "Pong".into(),
            ..IpcFrame::default()
        })?,
        "Pong" => {}
        "Committed" => committed(hub, frame)?,
        "BlobRead" if hub.capture_blob(frame)? => {}
        "Query" if hub.capture_handles_query(&frame.op) => {
            let frame = frame.clone();
            scope.spawn(move || {
                if let Err(error) = hub.capture_query(&frame) {
                    eprintln!("ab-session: query failed: {error}");
                }
            });
        }
        "Query" | "QueryCancel" | "Prepare" | "Dispatch" | "Resolve" | "BlobRead" => {
            hub.gateway_command(scope, frame)?;
        }
        "Close" => return Ok(false),
        _ => return Err(Error::Invalid),
    }
    Ok(true)
}

fn committed(hub: &Hub, frame: &IpcFrame) -> Result<(), Error> {
    let source = name(frame.source.ok_or(Error::Invalid)?)?;
    let reader_last = {
        let core = hub.core()?;
        let flight = core
            .flights
            .get(&source)
            .filter(|flight| flight.matches(&frame.checkpoint, u64::try_from(frame.credit).ok()))
            .ok_or(Error::Invalid)?;
        matches!(&flight.origin, Origin::Reader(origin) if origin.last)
    };
    let token = if reader_last {
        hub.capture_commit(&source)?
    } else {
        None
    };
    let flight = hub
        .core()?
        .flights
        .complete(&source, true, facts(&source))
        .ok_or(Error::Conflict)?;
    hub.changed.notify_all();
    match &flight.origin {
        Origin::Reader(_) => hub.capture_committed(&source, &flight, token)?,
        Origin::Helper { scan, last: true } => {
            hub.send_link(&Link::ScanCommitted(ScanCommitted {
                id: scan.clone(),
                source: frame.source.ok_or(Error::Invalid)?,
                checkpoint: flight.checkpoint.clone(),
            }))?;
        }
        Origin::Helper { .. } => {}
    }
    Ok(())
}

fn facts(source: &str) -> bool {
    matches!(source, "calendar" | "contacts" | "reminders")
}

fn limit(scan: &Scan) -> usize {
    match (scan.live, scan.source) {
        (true, Source::Calendar) => CALENDAR_LIVE_CHUNK_BYTES,
        (true, _) => LIVE_CHUNK_BYTES,
        (false, _) => BULK_CHUNK_BYTES,
    }
}

fn split(scan: &Scan) -> Result<Vec<Vec<Observation>>, Error> {
    let limit = if scan.whole { usize::MAX } else { limit(scan) };
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
            Origin::Helper {
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
    let Ok(Link::Ready(mut ready)) = native::read_frame(link) else {
        return Err(Error::Invalid);
    };
    messages_advertise::apply(&mut ready.coverage, &mut ready.capabilities);
    gateway.write_all(&ready.into_ipc().encode().map_err(|_| Error::Invalid)?)?;
    gateway.set_read_timeout(None)?;
    link.set_read_timeout(None)?;
    Ok((opened, fence))
}

pub fn serve(
    gateway: UnixStream,
    link: UnixStream,
    secret: &[u8],
    home: &Path,
) -> Result<(), Error> {
    serve_with(gateway, link, secret, home, Arc::new(SystemClock::start()))
}

pub fn serve_with(
    gateway: UnixStream,
    link: UnixStream,
    secret: &[u8],
    home: &Path,
    clock: Arc<dyn Clock>,
) -> Result<(), Error> {
    serve_configured(gateway, link, secret, home, clock, &Options::default())
}

pub fn serve_configured(
    gateway: UnixStream,
    link: UnixStream,
    secret: &[u8],
    home: &Path,
    clock: Arc<dyn Clock>,
    options: &Options,
) -> Result<(), Error> {
    serve_attached(gateway, link, secret, home, clock, options, None)
}

#[allow(clippy::too_many_lines)]
pub fn serve_attached(
    gateway: UnixStream,
    link: UnixStream,
    secret: &[u8],
    home: &Path,
    clock: Arc<dyn Clock>,
    options: &Options,
    attached: Option<Attached>,
) -> Result<(), Error> {
    if options
        .rust_commands
        .iter()
        .any(|source| !RUST_COMMAND_SOURCES.contains(&source.as_str()))
    {
        return Err(Error::Invalid);
    }
    let mut gateway_read = gateway.try_clone()?;
    let mut link_read = link.try_clone()?;
    let (opened, fence) = match start(&mut gateway_read, &mut link_read, secret) {
        Ok(started) => started,
        Err(error) => {
            let _ = gateway.shutdown(Shutdown::Both);
            let _ = link.shutdown(Shutdown::Both);
            return Err(error);
        }
    };
    let committed = committed_map(&opened)?;
    let capture = capture::Capture::attach(attached)?;
    let mut capture_state = capture::State::default();
    if let Some(capture) = &capture {
        capture.open(&mut capture_state, &committed)?;
    }
    let mut flights = Flights::default();
    flights.open(
        u64::try_from(opened.byte_credit).map_err(|_| Error::Invalid)?,
        committed,
    );
    let commands = commands::Commands::open(home, fence, &options.rust_commands)?;
    let unresolved = commands.unresolved();
    let hub = Hub {
        core: Mutex::new(Core {
            flights,
            commands,
            capture: capture_state,
            closed: false,
        }),
        changed: Condvar::new(),
        gateway: Mutex::new(gateway),
        link: Mutex::new(link),
        clock,
        capture,
        fence,
        sequence: AtomicU64::new(0),
        epoch: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Invalid)?
            .as_nanos(),
    };
    for outcome in unresolved {
        let frame: IpcFrame = serde_json::from_value(outcome)?;
        hub.send_gateway(&frame)?;
    }
    thread::scope(|scope| {
        let upstream = scope.spawn(|| -> Result<(), Error> {
            let result = (|| loop {
                let frame = match read_ipc(&mut gateway_read) {
                    Ok(frame) => frame,
                    Err(Error::Io(_)) => return Ok(()),
                    Err(error) => return Err(error),
                };
                if !gateway_frame(&hub, scope, &frame)? {
                    return Ok(());
                }
            })();
            hub.close();
            result
        });
        let producer = scope.spawn(|| {
            let result = hub.produce();
            if let Err(error) = &result {
                eprintln!("ab-session: capture ended: {error}");
                hub.close();
            }
            result
        });
        let watcher = scope.spawn(|| hub.watch());
        let result = (|| -> Result<(), Error> {
            loop {
                let frame = match native::read_frame(&mut link_read) {
                    Ok(frame) => frame,
                    Err(ErrorCode::Unavailable) => return Ok(()),
                    Err(_) => return Err(Error::Invalid),
                };
                match frame {
                    Link::SourceState(mut state) => {
                        messages_advertise::apply(&mut state.coverage, &mut state.capabilities);
                        hub.send_gateway(&state.into_ipc())?;
                    }
                    Link::Scan(scan) => {
                        let hub = &hub;
                        let source = name(scan.source)?;
                        scope.spawn(move || {
                            let done = if hub.capture_owns(&source) {
                                hub.capture_scan(&scan, &source)
                            } else {
                                run_scan(hub, &scan)
                            };
                            if let Err(error) = done {
                                scan_failed(hub, &scan.id, &error);
                            }
                        });
                    }
                    Link::Prepared(prepared) => hub.prepared(prepared)?,
                    Link::Rejected(rejection) => hub.rejected_by_helper(rejection)?,
                    Link::Outcome(outcome) => {
                        if let Some((active, outcome)) = hub.outcome_from_helper(outcome)? {
                            let hub = &hub;
                            scope.spawn(move || {
                                if let Err(error) = hub.settle(&active, outcome) {
                                    eprintln!("ab-session: settle failed: {error}");
                                    hub.close();
                                }
                            });
                        }
                    }
                    Link::QueryResult(result) => {
                        hub.finish_query(&result.id)?;
                        hub.send_gateway(&result.into_ipc())?;
                    }
                    Link::QueryError(failure) => {
                        hub.finish_query(&failure.id)?;
                        hub.send_gateway(&failure.into_ipc())?;
                    }
                    Link::PrimitiveResult(answer) => {
                        let value = serde_json::from_str(answer.result.get())?;
                        hub.finish_primitive(&answer.id, Ok(value))?;
                    }
                    Link::PrimitiveError(failure) => {
                        hub.finish_primitive(&failure.id, Err(failure.code))?;
                    }
                    Link::BlobChunk(chunk) => hub.send_gateway(&chunk.into_ipc())?,
                    Link::BlobUnavailable(missing) => hub.send_gateway(&missing.into_ipc())?,
                    Link::Close => return Ok(()),
                    _ => return Err(Error::Invalid),
                }
            }
        })();
        hub.close();
        let upstream = upstream.join().map_err(|_| Error::WorkerPanicked)?;
        let producer = producer.join().map_err(|_| Error::WorkerPanicked)?;
        watcher.join().map_err(|_| Error::WorkerPanicked)?;
        result.and(upstream).and(producer)
    })
}
