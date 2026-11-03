use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const ZONE: &str = "relay.test";
pub const ADMIN: &str = "synthetic-admin";
pub const SCENARIOS: [&str; 4] = [
    "RELAY-ENROLL",
    "RELAY-PASSTHROUGH",
    "RELAY-LIMITS",
    "RELAY-ROTATE-GATED",
];

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub connections: i64,
    pub bytes_per_second: i64,
    pub monthly_bytes: i64,
}
impl Limits {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            connections: 2,
            bytes_per_second: 1_048_576,
            monthly_bytes: 10_485_760,
        }
    }
    #[must_use]
    pub fn json(self) -> Value {
        json!({
            "connections": self.connections,
            "bytes_per_second": self.bytes_per_second,
            "monthly_bytes": self.monthly_bytes
        })
    }
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}
impl Reply {
    #[must_use]
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

pub fn urandom(count: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; count];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn signing_key() -> Result<SigningKey> {
    let seed: [u8; 32] = urandom(32)?
        .try_into()
        .map_err(|_| Error::Invalid("seed".into()))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn split_http(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let status = head.split(' ').nth(1)?.parse().ok()?;
    let chunked = head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked");
    let body = raw[end + 4..].to_vec();
    if !chunked {
        return Some((status, body));
    }
    let mut out = vec![];
    let mut rest = body.as_slice();
    loop {
        let line_end = rest.windows(2).position(|w| w == b"\r\n")?;
        let size =
            usize::from_str_radix(String::from_utf8_lossy(&rest[..line_end]).trim(), 16).ok()?;
        rest = &rest[line_end + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..)?;
    }
    Some((status, out))
}
pub fn http(
    address: &str,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<&[u8]>,
) -> Result<Reply> {
    let (address, prefix) = address
        .find('/')
        .map_or((address, ""), |at| (&address[..at], &address[at..]));
    let path = format!("{prefix}{path}");
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let auth = bearer
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let length = body
        .map(|data| {
            format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                data.len()
            )
        })
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n{auth}{length}\r\n"
    );
    stream.write_all(request.as_bytes())?;
    if let Some(data) = body {
        stream.write_all(data)?;
    }
    let mut raw = vec![];
    let _ = stream.read_to_end(&mut raw);
    let (status, body) =
        split_http(&raw).ok_or_else(|| Error::Invalid("malformed HTTP reply".into()))?;
    Ok(Reply { status, body })
}

#[derive(Debug)]
pub struct Fixture {
    child: Child,
    pub address: String,
}
impl Fixture {
    pub fn open(binary: &Path, state: &Path, limits: Limits) -> Result<Self> {
        let mut child = Command::new(binary)
            .arg(state)
            .arg(ZONE)
            .arg(limits.json().to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Invalid("fixture stdout".into()))?;
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line)?;
        let address = line.trim().trim_start_matches("http://").to_owned();
        if address.is_empty() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Invalid("fixture did not start".into()));
        }
        Ok(Self { child, address })
    }
    pub fn rpc(&self, action: &str, mut input: Value) -> Result<Value> {
        input["action"] = json!(action);
        let reply = http(
            &self.address,
            "POST",
            "/rpc",
            None,
            Some(input.to_string().as_bytes()),
        )?;
        let value = reply.json();
        if value["ok"].as_bool() == Some(true) {
            Ok(value["value"].clone())
        } else {
            Err(Error::Invalid(format!(
                "fixture: {}",
                value["error"].as_str().unwrap_or("rpc")
            )))
        }
    }
    pub fn device(&self, host: &str) -> Value {
        self.rpc("device", json!({"host": host}))
            .unwrap_or(Value::Null)
    }
    pub fn policy(&self, host: &str, limits: Limits, disabled: bool, revoked: bool) -> bool {
        self.rpc(
            "policy",
            json!({"host": host, "limits": limits.json(), "disabled": disabled, "revoked": revoked}),
        )
        .is_ok()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Default)]
pub struct Wire {
    pub marker: Vec<u8>,
    leaked: AtomicBool,
    bytes: AtomicUsize,
    opens: AtomicUsize,
    active: AtomicUsize,
}
impl Wire {
    #[must_use]
    pub fn new(marker: &[u8]) -> Arc<Self> {
        Arc::new(Self {
            marker: marker.to_vec(),
            ..Self::default()
        })
    }
    fn observe(&self, data: &[u8]) {
        if !self.marker.is_empty() && data.windows(self.marker.len()).any(|w| w == self.marker) {
            self.leaked.store(true, Ordering::SeqCst);
        }
        self.bytes.fetch_add(data.len(), Ordering::SeqCst);
    }
    #[must_use]
    pub fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }
    #[must_use]
    pub fn drained(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if self.active.load(Ordering::SeqCst) == 0 {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        false
    }
    #[must_use]
    pub fn ciphertext_only(&self) -> bool {
        self.bytes.load(Ordering::SeqCst) > 0 && !self.leaked.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub enum Tunnel {
    Dial(String),
    Blocked,
}

#[derive(Debug)]
struct Stream {
    socket: TcpStream,
    cancel: Arc<AtomicBool>,
}
type Streams = Arc<Mutex<HashMap<String, Stream>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut raw = vec![];
    let mut buffer = [0u8; 4096];
    loop {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buffer[..n]);
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while raw.len() < end + 4 + length {
                let n = stream.read(&mut buffer).ok()?;
                if n == 0 {
                    return None;
                }
                raw.extend_from_slice(&buffer[..n]);
            }
            let path = head.split(' ').nth(1)?.to_owned();
            return Some((path, raw[end + 4..end + 4 + length].to_vec()));
        }
    }
}

#[derive(Debug)]
pub struct Callback {
    pub url: String,
    streams: Streams,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Callback {
    pub fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}", listener.local_addr()?);
        let streams: Streams = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, flag) = (streams.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let streams = shared.clone();
                        thread::spawn(move || {
                            let _ = socket.set_nonblocking(false);
                            if let Some((path, body)) = read_request(&mut socket) {
                                let status = if path == "/cancel" {
                                    let key = serde_json::from_slice::<Value>(&body)
                                        .ok()
                                        .and_then(|v| v["Key"].as_str().map(str::to_owned))
                                        .or_else(|| {
                                            serde_json::from_slice::<Value>(&body)
                                                .ok()
                                                .and_then(|v| v["key"].as_str().map(str::to_owned))
                                        })
                                        .unwrap_or_default();
                                    if let Some(stream) = lock(&streams).get(&key) {
                                        stream.cancel.store(true, Ordering::SeqCst);
                                        let _ = stream.socket.shutdown(Shutdown::Both);
                                    }
                                    "204 No Content"
                                } else {
                                    "404 Not Found"
                                };
                                let _ = socket.write_all(
                                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                                        .as_bytes(),
                                );
                            }
                        });
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Ok(Self {
            url,
            streams,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Callback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for stream in lock(&self.streams).values() {
            stream.cancel.store(true, Ordering::SeqCst);
            let _ = stream.socket.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn pump(mut from: TcpStream, mut to: TcpStream, wire: &Wire) {
    let mut buffer = [0u8; 16384];
    while let Ok(n) = from.read(&mut buffer) {
        if n == 0 {
            break;
        }
        wire.observe(&buffer[..n]);
        if to.write_all(&buffer[..n]).is_err() {
            break;
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

fn bridge(socket: TcpStream, tunnel: &Tunnel, wire: &Arc<Wire>, cancel: &Arc<AtomicBool>) {
    match tunnel {
        Tunnel::Dial(address) => {
            let Ok(up) = TcpStream::connect(address) else {
                return;
            };
            wire.opens.fetch_add(1, Ordering::SeqCst);
            wire.active.fetch_add(1, Ordering::SeqCst);
            let (Ok(socket_read), Ok(up_read)) = (socket.try_clone(), up.try_clone()) else {
                wire.active.fetch_sub(1, Ordering::SeqCst);
                return;
            };
            let upstream = wire.clone();
            let forward = thread::spawn(move || pump(socket_read, up, &upstream));
            pump(up_read, socket, wire);
            let _ = forward.join();
            wire.active.fetch_sub(1, Ordering::SeqCst);
        }
        Tunnel::Blocked => {
            wire.opens.fetch_add(1, Ordering::SeqCst);
            wire.active.fetch_add(1, Ordering::SeqCst);
            while !cancel.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(2));
            }
            let _ = socket.shutdown(Shutdown::Both);
            wire.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[derive(Debug)]
pub struct Detach {
    stop: Arc<AtomicBool>,
    keys: Arc<Mutex<Vec<String>>>,
    streams: Streams,
    thread: Option<JoinHandle<()>>,
}
impl Detach {
    pub fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for key in lock(&self.keys).iter() {
            if let Some(stream) = lock(&self.streams).get(key) {
                stream.cancel.store(true, Ordering::SeqCst);
                let _ = stream.socket.shutdown(Shutdown::Both);
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Detach {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Debug)]
pub struct Lab {
    pub fixture: Arc<Fixture>,
    callback: Callback,
    id: u64,
    pub api: String,
    pub address: String,
    closed: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    serve: Option<JoinHandle<()>>,
    keep: Mutex<Vec<Detach>>,
}
impl Lab {
    pub fn start(fixture: &Arc<Fixture>) -> Result<Self> {
        Self::start_with(fixture, ADMIN)
    }
    pub fn start_with(fixture: &Arc<Fixture>, admin: &str) -> Result<Self> {
        let callback = Callback::start()?;
        let id = fixture
            .rpc("new", json!({"callback": callback.url, "admin": admin}))?
            .as_u64()
            .ok_or_else(|| Error::Invalid("server id".into()))?;
        let api_id = fixture
            .rpc("handler", json!({"id": id, "admin": admin}))?
            .as_u64()
            .ok_or_else(|| Error::Invalid("handler id".into()))?;
        let upstream = fixture
            .rpc("listen", json!({"id": id}))?
            .as_str()
            .ok_or_else(|| Error::Invalid("listen".into()))?
            .to_owned();
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?.to_string();
        let closed = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let (flag, done) = (closed.clone(), exited.clone());
        let serve = thread::spawn(move || {
            let mut sockets: Vec<TcpStream> = vec![];
            let mut workers = vec![];
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        let _ = socket.set_nonblocking(false);
                        let Ok(up) = TcpStream::connect(&upstream) else {
                            continue;
                        };
                        let (Ok(a), Ok(b), Ok(c), Ok(d)) = (
                            socket.try_clone(),
                            up.try_clone(),
                            socket.try_clone(),
                            up.try_clone(),
                        ) else {
                            continue;
                        };
                        sockets.push(a);
                        sockets.push(b);
                        let plain = Wire::new(&[]);
                        let reverse = plain.clone();
                        workers.push(thread::spawn(move || pump(c, d, &plain)));
                        workers.push(thread::spawn(move || pump(up, socket, &reverse)));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
            drop(listener);
            for socket in &sockets {
                let _ = socket.shutdown(Shutdown::Both);
            }
            done.store(true, Ordering::SeqCst);
            for worker in workers {
                let _ = worker.join();
            }
        });
        Ok(Self {
            fixture: fixture.clone(),
            callback,
            id,
            api: format!("{}/api/{api_id}", fixture.address),
            address,
            closed,
            exited,
            serve: Some(serve),
            keep: Mutex::new(vec![]),
        })
    }
    pub fn close_listener(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
    #[must_use]
    pub fn serve_exited(&self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.exited.load(Ordering::SeqCst) {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        self.exited.load(Ordering::SeqCst)
    }
    pub fn close(&mut self) {
        for mut detach in lock(&self.keep).drain(..) {
            detach.finish();
        }
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.fixture.rpc("close", json!({"id": self.id}));
        let _ = self.fixture.rpc("stop", json!({"id": self.id}));
        if let Some(serve) = self.serve.take() {
            let _ = serve.join();
        }
    }
    pub fn disconnect(&self, host: &str) {
        let _ = self
            .fixture
            .rpc("disconnect", json!({"id": self.id, "host": host}));
    }
    pub fn handler(&self, admin: &str) -> Result<String> {
        let id = self
            .fixture
            .rpc("handler", json!({"id": self.id, "admin": admin}))?
            .as_u64()
            .ok_or_else(|| Error::Invalid("handler id".into()))?;
        Ok(format!("{}/api/{id}", self.fixture.address))
    }
    pub fn attach_kept(
        &self,
        host: &str,
        token: &str,
        tunnel: Tunnel,
        wire: &Arc<Wire>,
    ) -> Result<()> {
        let detach = self.attach(host, token, tunnel, wire)?;
        lock(&self.keep).push(detach);
        Ok(())
    }
    pub fn attach(
        &self,
        host: &str,
        token: &str,
        tunnel: Tunnel,
        wire: &Arc<Wire>,
    ) -> Result<Detach> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        self.fixture.rpc(
            "attach",
            json!({
                "id": self.id,
                "host": host,
                "token": token,
                "address": listener.local_addr()?.to_string(),
                "callback": self.callback.url
            }),
        )?;
        let stop = Arc::new(AtomicBool::new(false));
        let keys: Arc<Mutex<Vec<String>>> = Arc::default();
        let streams = self.callback.streams.clone();
        let (flag, recorded, registry, wire) =
            (stop.clone(), keys.clone(), streams.clone(), wire.clone());
        let thread = thread::spawn(move || {
            let mut workers = vec![];
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, peer)) => {
                        let _ = socket.set_nonblocking(false);
                        let key = peer.to_string();
                        let cancel = Arc::new(AtomicBool::new(false));
                        let Ok(clone) = socket.try_clone() else {
                            continue;
                        };
                        lock(&registry).insert(
                            key.clone(),
                            Stream {
                                socket: clone,
                                cancel: cancel.clone(),
                            },
                        );
                        lock(&recorded).push(key.clone());
                        let (tunnel, wire, registry) =
                            (tunnel.clone(), wire.clone(), registry.clone());
                        workers.push(thread::spawn(move || {
                            bridge(socket, &tunnel, &wire, &cancel);
                            lock(&registry).remove(&key);
                        }));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
            drop(listener);
            for worker in workers {
                let _ = worker.join();
            }
        });
        Ok(Detach {
            stop,
            keys,
            streams,
            thread: Some(thread),
        })
    }
}
impl Drop for Lab {
    fn drop(&mut self) {
        self.close();
    }
}

pub fn post(base: &str, path: &str, input: &Value, bearer: Option<&str>) -> (u16, Value) {
    http(
        base,
        "POST",
        path,
        bearer,
        Some(input.to_string().as_bytes()),
    )
    .map_or((0, Value::Null), |r| {
        let value = r.json();
        (r.status, value)
    })
}
pub fn get(base: &str, path: &str) -> (u16, Value) {
    http(base, "GET", path, None, None).map_or((0, Value::Null), |r| {
        let value = r.json();
        (r.status, value)
    })
}

pub fn enroll_with(base: &str, key: &SigningKey, host: &str, path: &str) -> (Value, u16) {
    let (_, challenge) = get(base, "/challenge");
    let nonce = challenge["nonce"].as_str().unwrap_or_default().to_owned();
    let message = format!("apple-bridge-relay-enroll\n{ZONE}\n{nonce}");
    let signature = key.sign(message.as_bytes()).to_bytes();
    let (status, value) = post(
        base,
        path,
        &json!({
            "public_key": b64(key.verifying_key().as_bytes()),
            "nonce": nonce,
            "signature": b64(&signature),
            "hostname": host
        }),
        None,
    );
    (value, status)
}
pub fn enroll(base: &str, key: &SigningKey, host: &str) -> (Value, u16) {
    enroll_with(base, key, host, "/enroll")
}

#[derive(Debug)]
pub struct Workspace {
    pub directory: PathBuf,
}
impl Workspace {
    pub fn create(world: &World, name: &str) -> Result<Self> {
        let directory = world.directory.join(name);
        fs::create_dir_all(&directory)?;
        Ok(Self { directory })
    }
}
pub fn fixture_binary() -> Result<PathBuf> {
    std::env::var_os("AB_RELAY_FIXTURE")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Invalid("AB_RELAY_FIXTURE is required for relay scenarios".into()))
}

pub fn cred(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

pub fn relay_service(world: &mut World, _input: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if crate::relay_more::SCENARIOS.contains(&id.as_str()) {
        return crate::relay_more::run(world, evidence);
    }
    let certificates = matches!(id.as_str(), "RELAY-CT-INDEPENDENT" | "RELAY-CAA-ACCOUNT");
    let binary = if certificates {
        PathBuf::new()
    } else {
        fixture_binary()?
    };
    let workspace = Workspace::create(world, &format!("relay-{}", id.to_ascii_lowercase()))?;
    match id.as_str() {
        "RELAY-ENROLL" | "RELAY-PASSTHROUGH" | "RELAY-LIMITS" => {
            crate::relay_service::run(&id, &binary, &workspace.directory, evidence)
        }
        "RELAY-ROTATE-GATED" => crate::relay_gated::run(&binary, &workspace.directory, evidence),
        "RELAY-CT-INDEPENDENT" | "RELAY-CAA-ACCOUNT" => {
            crate::relay_certs::run(&id, &workspace.directory, evidence)
        }
        _ => Err(Error::Invalid(format!("relay scenario not ported: {id}"))),
    }
}
