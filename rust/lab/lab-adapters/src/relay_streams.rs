#![allow(clippy::too_many_lines)]
use crate::{
    relay::{
        ADMIN, Fixture, Lab, Limits, b64, cred, enroll, fixture_binary, get, signing_key, urandom,
    },
    relay_tls::{Session, connect},
};
use ed25519_dalek::{Signer, SigningKey};
use lab_relay::{Ca, client_config};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    crypto::ring::default_provider,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tungstenite::{Message, client::IntoClientRequest};

const WINDOW: u32 = 262_144;
const POLL: Duration = Duration::from_millis(20);

fn invalid<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn frame(cmd: u8, id: u32, data: &[u8]) -> Vec<u8> {
    let mut out = vec![2, cmd];
    out.extend_from_slice(&u16::try_from(data.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(data);
    out
}
fn timed_out(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(io)
        if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

pub(crate) struct Identity {
    pub(crate) host: String,
    pub(crate) credential: String,
}

pub(crate) fn proof_json(
    api: &str,
    key: &SigningKey,
    who: &Identity,
    action: &str,
) -> Option<Value> {
    let (_, challenge) = get(api, "/challenge");
    let nonce = challenge["nonce"].as_str()?.to_owned();
    let message = format!(
        "apple-bridge-relay/1\n{action}\n{}\n{}\n{nonce}",
        who.host, who.credential
    );
    Some(json!({
        "hostname": who.host,
        "credential": who.credential,
        "nonce": nonce,
        "signature": b64(&key.sign(message.as_bytes()).to_bytes()),
    }))
}

fn tunnel_once(
    api: &str,
    key: &SigningKey,
    who: &Identity,
    origin: &str,
    stop: &AtomicBool,
) -> Option<()> {
    let proof = proof_json(api, key, who, "tunnel")?;
    let (hostport, prefix) = api
        .find('/')
        .map_or((api, ""), |at| (&api[..at], &api[at..]));
    let mut request = format!("ws://{hostport}{prefix}/tunnel")
        .into_client_request()
        .ok()?;
    request.headers_mut().insert(
        "X-Bridge-Proof",
        b64(proof.to_string().as_bytes()).parse().ok()?,
    );
    let tcp = TcpStream::connect(hostport).ok()?;
    tcp.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    tcp.set_write_timeout(Some(Duration::from_secs(5))).ok()?;
    let (mut socket, _) = tungstenite::client::client(request, tcp).ok()?;
    socket.get_mut().set_read_timeout(Some(POLL)).ok()?;
    let (out, queue) = mpsc::channel::<Vec<u8>>();
    let mut streams: HashMap<u32, (TcpStream, u32)> = HashMap::new();
    let mut pending: Vec<u8> = vec![];
    let mut beat = Instant::now();
    let finished = loop {
        if stop.load(Ordering::SeqCst) {
            break Some(());
        }
        match socket.read() {
            Ok(Message::Binary(bytes)) => pending.extend_from_slice(&bytes),
            Ok(Message::Close(_)) => break None,
            Ok(_) => {}
            Err(error) if timed_out(&error) => {}
            Err(_) => break None,
        }
        let mut offset = 0;
        while pending.len() - offset >= 8 {
            let head = &pending[offset..offset + 8];
            let length = usize::from(u16::from_le_bytes([head[2], head[3]]));
            if pending.len() - offset < 8 + length {
                break;
            }
            if head[0] != 2 {
                return None;
            }
            let id = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
            let data = pending[offset + 8..offset + 8 + length].to_vec();
            match head[1] {
                0 => {
                    if let Ok(local) = TcpStream::connect(origin) {
                        let _ = local.set_nodelay(true);
                        if let Ok(mut reader) = local.try_clone() {
                            let sender = out.clone();
                            thread::spawn(move || {
                                let mut buffer = [0u8; 8192];
                                while let Ok(n) = reader.read(&mut buffer) {
                                    if n == 0 || sender.send(frame(2, id, &buffer[..n])).is_err() {
                                        break;
                                    }
                                }
                                let _ = sender.send(frame(1, id, &[]));
                            });
                            streams.insert(id, (local, 0));
                        }
                    } else {
                        let _ = out.send(frame(1, id, &[]));
                    }
                }
                1 => {
                    if let Some((local, _)) = streams.get(&id) {
                        let _ = local.shutdown(Shutdown::Write);
                    }
                }
                2 => {
                    if let Some((local, consumed)) = streams.get_mut(&id) {
                        let _ = local.write_all(&data);
                        *consumed = consumed.wrapping_add(u32::try_from(data.len()).unwrap_or(0));
                        let mut update = consumed.to_le_bytes().to_vec();
                        update.extend_from_slice(&WINDOW.to_le_bytes());
                        let _ = out.send(frame(4, id, &update));
                    }
                }
                _ => {}
            }
            offset += 8 + length;
        }
        pending.drain(..offset);
        while let Ok(bytes) = queue.try_recv() {
            if socket.send(Message::binary(bytes)).is_err() {
                break;
            }
        }
        if beat.elapsed() >= Duration::from_secs(1) {
            beat = Instant::now();
            if socket.send(Message::binary(frame(3, 0, &[]))).is_err() {
                break None;
            }
        }
    };
    for (local, _) in streams.values() {
        let _ = local.shutdown(Shutdown::Both);
    }
    finished
}

struct TunnelClient {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl std::fmt::Debug for TunnelClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelClient").finish_non_exhaustive()
    }
}
impl TunnelClient {
    fn start(api: String, key: SigningKey, who: Identity, origin: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            let mut delay = Duration::from_millis(100);
            while !flag.load(Ordering::SeqCst) {
                let started = Instant::now();
                let _ = tunnel_once(&api, &key, &who, &origin, &flag);
                if started.elapsed() > Duration::from_secs(3) {
                    delay = Duration::from_millis(100);
                }
                let until = Instant::now() + delay;
                while Instant::now() < until && !flag.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(10));
                }
                if delay < Duration::from_secs(1) {
                    delay *= 2;
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for TunnelClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Replay<S> {
    prefix: Cursor<Vec<u8>>,
    inner: S,
}
impl<S: Read> Read for Replay<S> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.prefix.read(buffer)?;
        if n > 0 {
            return Ok(n);
        }
        self.inner.read(buffer)
    }
}
impl<S: Write> Write for Replay<S> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buffer)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

type Served = StreamOwned<ServerConnection, TcpStream>;

fn read_head(stream: &mut Served) -> Option<(Vec<u8>, usize)> {
    let mut raw = vec![];
    let mut buffer = [0u8; 4096];
    loop {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 || raw.len() > 16_384 {
            return None;
        }
        raw.extend_from_slice(&buffer[..n]);
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            return Some((raw, end + 4));
        }
    }
}

fn respond(stream: &mut Served, status: &str, extra: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle(mut stream: Served, sessions: &Mutex<HashSet<String>>, stop: &AtomicBool) {
    let Some((raw, end)) = read_head(&mut stream) else {
        return;
    };
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.lines();
    let mut request = lines.next().unwrap_or_default().split(' ');
    let (method, path) = (
        request.next().unwrap_or_default().to_owned(),
        request.next().unwrap_or_default().to_owned(),
    );
    let header = |name: &str| {
        lines
            .clone()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.trim()
                    .eq_ignore_ascii_case(name)
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default()
    };
    let length: usize = header("content-length").parse().unwrap_or(0);
    let mut body = raw[end..].to_vec();
    while body.len() < length {
        let mut buffer = [0u8; 4096];
        let Ok(n) = stream.read(&mut buffer) else {
            return;
        };
        if n == 0 {
            return;
        }
        body.extend_from_slice(&buffer[..n]);
    }
    let session = header("mcp-session-id");
    match (method.as_str(), path.as_str()) {
        ("GET", "/sse") => {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            );
            while !stop.load(Ordering::SeqCst) {
                if stream.write_all(b"data: bridge\n\n").is_err() || stream.flush().is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        ("GET", "/ws") if header("upgrade").eq_ignore_ascii_case("websocket") => {
            let replay = Replay {
                prefix: Cursor::new(raw),
                inner: stream,
            };
            if let Ok(mut socket) = tungstenite::accept(replay)
                && let Ok(message) = socket.read()
            {
                let _ = socket.send(message);
                let _ = socket.flush();
                thread::sleep(Duration::from_millis(200));
            }
        }
        ("POST", "/mcp") => {
            let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let method = value["method"].as_str().unwrap_or_default();
            if method == "initialize" {
                let id = hex::encode(urandom(16).unwrap_or_default());
                lock(sessions).insert(id.clone());
                let reply = json!({"jsonrpc": "2.0", "id": value["id"], "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": {"name": "relay-lab", "version": "1"}
                }});
                respond(
                    &mut stream,
                    "200 OK",
                    &format!("Content-Type: application/json\r\nMcp-Session-Id: {id}\r\n"),
                    reply.to_string().as_bytes(),
                );
            } else if !lock(sessions).contains(&session) {
                respond(&mut stream, "404 Not Found", "", b"unknown session");
            } else if method.starts_with("notifications/") {
                respond(&mut stream, "202 Accepted", "", b"");
            } else {
                let reply = json!({"jsonrpc": "2.0", "id": value["id"], "result": {}});
                respond(
                    &mut stream,
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    reply.to_string().as_bytes(),
                );
            }
        }
        ("GET", "/") => respond(&mut stream, "200 OK", "", b"alias-origin"),
        _ => respond(&mut stream, "405 Method Not Allowed", "", b""),
    }
}

pub(crate) struct Origin {
    pub(crate) address: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl std::fmt::Debug for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Origin").finish_non_exhaustive()
    }
}
impl Drop for Origin {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn origin(ca: &Ca, host: &str) -> Result<Origin> {
    let (chain, key) = ca.leaf_for(host).map_err(invalid)?;
    let chain = CertificateDer::pem_slice_iter(chain.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(invalid)?;
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes()).map_err(invalid)?;
    let config = Arc::new(
        ServerConfig::builder_with_provider(Arc::new(default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(invalid)?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(invalid)?,
    );
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let sessions: Arc<Mutex<HashSet<String>>> = Arc::default();
    let thread = thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((tcp, _)) => {
                    let (config, sessions, flag) = (config.clone(), sessions.clone(), flag.clone());
                    thread::spawn(move || {
                        let _ = tcp.set_nonblocking(false);
                        let _ = tcp.set_read_timeout(Some(Duration::from_secs(30)));
                        let _ = tcp.set_write_timeout(Some(Duration::from_secs(5)));
                        if let Ok(connection) = ServerConnection::new(config) {
                            handle(StreamOwned::new(connection, tcp), &sessions, &flag);
                        }
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
    });
    Ok(Origin {
        address,
        stop,
        thread: Some(thread),
    })
}

pub(crate) fn read_reply(session: &mut Session) -> Option<(u16, String, Vec<u8>)> {
    let mut raw = vec![];
    let mut byte = [0u8; 1];
    while !raw.ends_with(b"\r\n\r\n") {
        if session.read(&mut byte).ok()? == 0 {
            return None;
        }
        raw.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&raw).into_owned();
    let status = head.split(' ').nth(1)?.parse().ok()?;
    let lower = head.to_ascii_lowercase();
    let length: usize = lower
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    session.read_exact(&mut body).ok()?;
    Some((status, head, body))
}

pub(crate) fn exchange(
    config: &Arc<rustls::ClientConfig>,
    address: &str,
    host: &str,
    method: &str,
    path: &str,
    extra: &str,
    body: &str,
) -> Option<(u16, String, Vec<u8>)> {
    let mut session = connect(config, address, host, 10).ok()?;
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    session.write_all(request.as_bytes()).ok()?;
    session.flush().ok()?;
    read_reply(&mut session)
}

fn rpc_post(
    config: &Arc<rustls::ClientConfig>,
    address: &str,
    host: &str,
    session: &str,
    body: &Value,
) -> Option<(u16, String, Vec<u8>)> {
    let extra = if session.is_empty() {
        String::new()
    } else {
        format!("Mcp-Session-Id: {session}\r\n")
    };
    exchange(
        config,
        address,
        host,
        "POST",
        "/mcp",
        &extra,
        &body.to_string(),
    )
}

fn ping(config: &Arc<rustls::ClientConfig>, address: &str, host: &str, session: &str) -> bool {
    rpc_post(
        config,
        address,
        host,
        session,
        &json!({"jsonrpc": "2.0", "id": 9, "method": "ping"}),
    )
    .is_some_and(|(status, _, body)| {
        status == 200 && String::from_utf8_lossy(&body).contains("result")
    })
}

fn open_stream(
    config: &Arc<rustls::ClientConfig>,
    address: &str,
    host: &str,
    seconds: u64,
) -> Option<Session> {
    let mut session = connect(config, address, host, seconds).ok()?;
    let request = format!("GET /sse HTTP/1.1\r\nHost: {host}\r\nAccept: text/event-stream\r\n\r\n");
    session.write_all(request.as_bytes()).ok()?;
    session.flush().ok()?;
    let mut raw = vec![];
    let mut byte = [0u8; 1];
    while !raw.ends_with(b"\r\n\r\n") {
        if session.read(&mut byte).ok()? == 0 {
            return None;
        }
        raw.push(byte[0]);
    }
    String::from_utf8_lossy(&raw)
        .starts_with("HTTP/1.1 200")
        .then_some(session)
}

fn session_id(head: &str) -> String {
    head.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("mcp-session-id")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default()
}

fn websocket_echo(config: &Arc<rustls::ClientConfig>, address: &str, host: &str) -> bool {
    let Ok(session) = connect(config, address, host, 10) else {
        return false;
    };
    let Ok(request) = format!("wss://{host}/ws").into_client_request() else {
        return false;
    };
    let Ok((mut socket, _)) = tungstenite::client::client(request, session) else {
        return false;
    };
    let sent = socket.send(Message::text("echo")).is_ok();
    sent && matches!(socket.read(), Ok(Message::Text(text)) if text.as_str() == "echo")
}

pub fn run(world: &mut World, e: &mut Evidence) -> Result<()> {
    let directory = world.directory.join("relay-streams");
    std::fs::create_dir_all(&directory)?;
    let binary = fixture_binary()?;
    let limits = Limits::standard();
    let store = Arc::new(Fixture::open(
        &binary,
        &directory.join("relay.json"),
        limits,
    )?);
    let lab = Lab::start_with(&store, ADMIN)?;
    let (first, second) = (signing_key()?, signing_key()?);
    let (a, code1) = enroll(&lab.api, &first, "");
    let (b, code2) = enroll(&lab.api, &second, "");
    e.check(
        "two_distinct_device_enrollments",
        code1 == 200 && code2 == 200 && cred(&a, "hostname") != cred(&b, "hostname"),
        "",
    );
    if code1 != 200 || code2 != 200 {
        return Ok(());
    }
    let host = cred(&a, "hostname");
    let budget = store.policy(
        &host,
        Limits {
            connections: 8,
            ..limits
        },
        false,
        false,
    );
    if !budget {
        e.check("stream_connection_budget", false, "");
        return Ok(());
    }
    let ca = Ca::new().map_err(invalid)?;
    let config = client_config(ca.roots().map_err(invalid)?).map_err(invalid)?;
    let served = origin(&ca, &host)?;
    let _tunnel = TunnelClient::start(
        lab.api.clone(),
        first.clone(),
        Identity {
            host: host.clone(),
            credential: cred(&a, "credential"),
        },
        served.address.clone(),
    );
    let seconds: u64 = if std::env::var("AB_LAB_RELAY_STREAM_SECONDS").as_deref() == Ok("4") {
        4
    } else {
        600
    };
    let until = Instant::now() + Duration::from_secs(5);
    let mut stream = None;
    while stream.is_none() && Instant::now() < until {
        stream = open_stream(&config, &lab.address, &host, 30);
        if stream.is_none() {
            thread::sleep(Duration::from_millis(50));
        }
    }
    let Some(mut stream) = stream else {
        e.check("outbound_tunnel_tls", false, "");
        return Ok(());
    };
    let started = Instant::now();
    let mut event = [0u8; 14];
    let mut survived = true;
    while started.elapsed() < Duration::from_secs(seconds) && survived {
        survived = stream.read_exact(&mut event).is_ok();
    }
    e.check("sse_survives_idle_budget", survived, "");
    drop(stream);
    e.check(
        "websocket_duplex",
        websocket_echo(&config, &lab.address, &host),
        "",
    );
    let opened = rpc_post(
        &config,
        &lab.address,
        &host,
        "",
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "relay-client", "version": "1"}
        }}),
    );
    let session = opened
        .as_ref()
        .map(|(_, head, _)| session_id(head))
        .unwrap_or_default();
    let initialized = !session.is_empty()
        && rpc_post(
            &config,
            &lab.address,
            &host,
            &session,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .is_some_and(|(status, _, _)| status == 202);
    e.check(
        "mcp_streamable_http_session",
        initialized && ping(&config, &lab.address, &host, &session),
        "",
    );
    let active = open_stream(&config, &lab.address, &host, 30);
    if active.is_none() {
        e.check("restart_active_stream", false, "");
        return Ok(());
    }
    let restarted = Instant::now();
    lab.restart()?;
    drop(active);
    let mut reconnected = false;
    while restarted.elapsed() < Duration::from_secs(5) {
        if exchange(&config, &lab.address, &host, "GET", "/mcp", "", "").is_some() {
            reconnected = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    e.check("relay_listener_restart_reconnect_under_5s", reconnected, "");
    e.check(
        "mcp_session_resumes_after_relay_restart",
        initialized && ping(&config, &lab.address, &host, &session),
        "",
    );
    let verify = |proof: &Value| {
        store
            .rpc("verify", json!({"proof": proof, "proof_action": "tunnel"}))
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    };
    let proof = proof_json(
        &lab.api,
        &first,
        &Identity {
            host: host.clone(),
            credential: cred(&a, "credential"),
        },
        "tunnel",
    );
    e.check(
        "signed_proof_single_use",
        proof.is_some_and(|proof| verify(&proof) && !verify(&proof)),
        "",
    );
    Ok(())
}
