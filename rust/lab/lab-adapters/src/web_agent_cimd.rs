#![allow(clippy::too_many_lines)]
use crate::{
    web_agent_oauth::{Query, agent, authorize, contains, get, oauth, pairs, redirect_query, set},
    web_agents::{owner_session, text},
    webauthn::Owner,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    crypto::ring::default_provider,
    pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    fs,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CALLBACK: &str = "https://connector.example/callback";

fn failed<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Default)]
struct Counters {
    hits: AtomicI64,
    private_hits: AtomicI64,
    private_connections: AtomicI64,
}

struct Resolver {
    ip: String,
    rebinding: bool,
    resolutions: i64,
    connections: i64,
    now: u64,
}

struct Server {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn spawn_server(
    stop: &Arc<AtomicBool>,
    handler: impl Fn(TcpStream) + Send + Sync + 'static,
) -> Result<(String, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let flag = stop.clone();
    let handler = Arc::new(handler);
    let thread = thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((socket, _)) => {
                    let _ = socket.set_nonblocking(false);
                    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
                    let handler = handler.clone();
                    thread::spawn(move || handler(socket));
                }
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
    });
    Ok((address, thread))
}

fn read_head(stream: &mut impl Read) -> Option<String> {
    let mut raw = vec![];
    let mut buffer = [0u8; 4096];
    loop {
        if let Some(end) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&raw[..end]).into_owned());
        }
        let count = stream.read(&mut buffer).ok()?;
        if count == 0 || raw.len() > 65536 {
            return None;
        }
        raw.extend_from_slice(&buffer[..count]);
    }
}

fn respond(stream: &mut impl Write, status: &str, headers: &[(&str, &str)], body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    let _ = write!(
        head,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(value) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(value);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn request_line(head: &str) -> (String, String, Vec<(String, String)>) {
    let target = head
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (percent_decode(key), percent_decode(value)))
        .collect();
    let host = head
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default().to_owned();
    (path.to_owned(), host, params)
}

fn metadata_response(path: &str, host: &str) -> (String, Vec<(String, String)>, Vec<u8>) {
    if path == "/private-redirect" {
        return (
            "302 Found".into(),
            vec![("Location".into(), "https://private.example/client".into())],
            vec![],
        );
    }
    let mut id = format!("https://client.example{path}");
    if host == "private.example" {
        id = "https://client.example/private-redirect".into();
    }
    if path == "/mismatch" {
        id = "https://other.example/client".into();
    }
    let mut redirects = vec![CALLBACK.to_owned()];
    match path {
        "/malformed-redirect" => redirects = vec!["%".into()],
        "/unsupported-redirect" => redirects = vec!["ftp://connector.example/callback".into()],
        "/mixed-redirects" => redirects.push("%".into()),
        _ => {}
    }
    let mut body = vec![];
    if path == "/oversize" {
        body.extend(std::iter::repeat_n(b' ', 5121));
    }
    body.extend(
        serde_json::to_vec(&json!({
            "client_id": id,
            "client_name": "Bridge Test metadata",
            "redirect_uris": redirects
        }))
        .unwrap_or_default(),
    );
    body.push(b'\n');
    (
        "200 OK".into(),
        vec![
            ("Cache-Control".into(), "max-age=60".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body,
    )
}

fn issue_chain() -> Result<(String, Vec<u8>, Vec<u8>)> {
    let authority_key = KeyPair::generate().map_err(failed)?;
    let mut authority_params = CertificateParams::new(Vec::<String>::new()).map_err(failed)?;
    authority_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    authority_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let authority = authority_params
        .self_signed(&authority_key)
        .map_err(failed)?;
    let issuer = Issuer::new(authority_params, authority_key);
    let leaf_key = KeyPair::generate().map_err(failed)?;
    let leaf = CertificateParams::new(vec![
        "client.example".to_owned(),
        "private.example".to_owned(),
    ])
    .map_err(failed)?
    .signed_by(&leaf_key, &issuer)
    .map_err(failed)?;
    Ok((
        authority.pem(),
        leaf.der().to_vec(),
        leaf_key.serialize_der(),
    ))
}

fn metadata_server(
    stop: &Arc<AtomicBool>,
    counters: &Arc<Counters>,
    leaf: Vec<u8>,
    key: Vec<u8>,
) -> Result<Server> {
    let config = Arc::new(
        ServerConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(failed)?
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.into()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )
            .map_err(failed)?,
    );
    let counters = counters.clone();
    let flag = stop.clone();
    let (address, thread) = spawn_server(stop, move |socket| {
        let Ok(connection) = ServerConnection::new(config.clone()) else {
            return;
        };
        let mut stream = StreamOwned::new(connection, socket);
        let Some(head) = read_head(&mut stream) else {
            return;
        };
        let (path, host, _) = request_line(&head);
        counters.hits.fetch_add(1, Ordering::SeqCst);
        if host == "private.example" {
            counters.private_hits.fetch_add(1, Ordering::SeqCst);
        }
        if path == "/slow" {
            let deadline = Instant::now() + Duration::from_secs(4);
            while Instant::now() < deadline {
                if flag.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        let (status, headers, body) = metadata_response(&path, &host);
        let headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        respond(&mut stream, &status, &headers, &body);
        stream.conn.send_close_notify();
        let _ = stream.flush();
        let _ = stream.sock.shutdown(Shutdown::Both);
    })?;
    Ok(Server {
        address,
        stop: stop.clone(),
        thread: Some(thread),
    })
}

fn control_server(
    stop: &Arc<AtomicBool>,
    counters: &Arc<Counters>,
    state: &Arc<Mutex<Resolver>>,
    metadata: &str,
) -> Result<Server> {
    let counters = counters.clone();
    let state = state.clone();
    let metadata = metadata.to_owned();
    let (address, thread) = spawn_server(stop, move |mut socket| {
        let Some(head) = read_head(&mut socket) else {
            return;
        };
        let (path, _, params) = request_line(&head);
        let host = get(&params, "host");
        let mut state = lock(&state);
        let body = match path.as_str() {
            "/resolve" => {
                state.resolutions += 1;
                let value =
                    if host == "private.example" || (state.rebinding && state.resolutions > 1) {
                        "127.0.0.1".to_owned()
                    } else {
                        state.ip.clone()
                    };
                json!([value])
            }
            "/dial" => {
                state.connections += 1;
                if host.contains("private.example") || host.contains("127.0.0.1") {
                    counters.private_connections.fetch_add(1, Ordering::SeqCst);
                }
                json!(metadata)
            }
            "/clock" => json!(state.now),
            _ => Value::Null,
        };
        drop(state);
        respond(
            &mut socket,
            "200 OK",
            &[("Content-Type", "application/json")],
            body.to_string().as_bytes(),
        );
        let _ = socket.shutdown(Shutdown::Both);
    })?;
    Ok(Server {
        address,
        stop: stop.clone(),
        thread: Some(thread),
    })
}

struct Probe<'a> {
    owner: &'a Owner,
    query: Query,
}

impl Probe<'_> {
    fn request(&self, method: &str) -> (u16, String) {
        thread::sleep(Duration::from_millis(110));
        let reply = authorize(
            self.owner,
            method,
            &self.query,
            Some(&json!({"decision": "approve", "level": "read", "scopes": ["messages:read"]})),
        );
        (
            reply.status,
            String::from_utf8_lossy(&reply.body).into_owned(),
        )
    }

    fn fetch(&mut self, id: &str, redirect: &str) -> u16 {
        set(&mut self.query, "client_id", id);
        set(&mut self.query, "redirect_uri", redirect);
        self.request("GET").0
    }
}

pub fn cimd(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("authority", false, "agent list");
        return Ok(());
    }
    let Ok(owner) = owner_session(world, None) else {
        evidence.check("owner_session", false, "owner enrollment");
        return Ok(());
    };
    let origin = owner.origin.clone();
    let reply = agent(
        &origin,
        "",
        "GET",
        "/.well-known/oauth-authorization-server",
        None,
        None,
    );
    evidence.check(
        "metadata_documents_advertised",
        reply.status == 200 && contains(&reply, "\"client_id_metadata_document_supported\":true"),
        "",
    );
    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(Counters::default());
    let Ok((authority, leaf, key)) = issue_chain() else {
        evidence.check("fixture_certificate", false, "");
        return Ok(());
    };
    let Ok(metadata) = metadata_server(&stop, &counters, leaf, key) else {
        evidence.check("fixture_tls", false, "");
        return Ok(());
    };
    let resolver = Arc::new(Mutex::new(Resolver {
        ip: "8.8.8.8".into(),
        rebinding: false,
        resolutions: 0,
        connections: 0,
        now: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()),
    }));
    let Ok(control) = control_server(&stop, &counters, &resolver, &metadata.address) else {
        evidence.check("fixture_control", false, "");
        return Ok(());
    };
    let ca = world.directory.join("cimd-ca.pem");
    if fs::write(&ca, authority).is_err() {
        evidence.check("fixture_ca", false, "");
        return Ok(());
    }
    let restarted = world.configure_gateway(
        vec![
            (
                "APPLE_BRIDGE_LAB_CIMD".to_owned(),
                format!("http://{}", control.address),
            ),
            (
                "APPLE_BRIDGE_LAB_CIMD_CA".to_owned(),
                ca.to_string_lossy().into_owned(),
            ),
        ],
        vec![],
    );
    if restarted.is_err() {
        evidence.check("fixture_restart", false, "");
        return Ok(());
    }
    let Ok(owner) = owner_session(world, None) else {
        evidence.check("fixture_owner", false, "");
        return Ok(());
    };
    let origin = owner.origin.clone();
    let result = exercise(world, &owner, &origin, &counters, &resolver, evidence);
    stop.store(true, Ordering::SeqCst);
    result
}

fn exercise(
    world: &mut World,
    owner: &Owner,
    origin: &str,
    counters: &Counters,
    resolver: &Mutex<Resolver>,
    evidence: &mut Evidence,
) -> Result<()> {
    let verifier = "a".repeat(43);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let resource = format!("{origin}/mcp");
    let mut probe = Probe {
        owner,
        query: pairs(&[
            ("client_id", "https://client.example/good"),
            ("response_type", "code"),
            ("redirect_uri", CALLBACK),
            ("scope", "messages:read"),
            ("resource", &resource),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("format", "json"),
        ]),
    };
    let hits = || counters.hits.load(Ordering::SeqCst);
    for id in [
        "https://client.example/client#",
        "https://client.example/client#fragment",
        "https://client.example",
        "https://client.example/",
        "https://client.example/a/./client",
        "https://client.example/a/../client",
        "https://client.example/a/%2e/client",
        "https://client.example/a/%2E%2e/client",
    ] {
        let before = hits();
        let code = probe.fetch(id, CALLBACK);
        evidence.check(
            &format!("invalid_client_id_{id}"),
            code == 400 && hits() == before,
            "",
        );
    }
    for variant in [
        "malformed-redirect",
        "unsupported-redirect",
        "mixed-redirects",
    ] {
        let redirect = match variant {
            "malformed-redirect" => "%",
            "unsupported-redirect" => "ftp://connector.example/callback",
            _ => CALLBACK,
        };
        let id = format!("https://client.example/{variant}");
        let code = probe.fetch(&id, redirect);
        let before = hits();
        let again = probe.fetch(&id, redirect);
        evidence.check(
            &format!("invalid_document_{variant}"),
            code == 400 && again == 400 && hits() == before + 1,
            "",
        );
    }
    for ip in [
        "0.1.2.3",
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.0.1",
        "169.254.1.1",
        "100.64.0.1",
        "::1",
        "fc00::1",
        "fe80::1",
        "::ffff:127.0.0.1",
    ] {
        ip.clone_into(&mut lock(resolver).ip);
        let before = hits();
        let code = probe.fetch("https://client.example/good", CALLBACK);
        evidence.check(
            &format!("blocked_ip_{ip}"),
            code == 400 && hits() == before,
            "",
        );
    }
    {
        let mut state = lock(resolver);
        "8.8.8.8".clone_into(&mut state.ip);
        state.rebinding = true;
        state.resolutions = 0;
        state.connections = 0;
    }
    let code = probe.fetch("https://client.example/rebinding", CALLBACK);
    {
        let mut state = lock(resolver);
        evidence.check(
            "dns_rebinding_refused_before_connection",
            code == 400 && state.resolutions == 2 && state.connections == 0,
            "",
        );
        state.rebinding = false;
    }
    let code = probe.fetch("https://client.example/cached", CALLBACK);
    let before = hits();
    let cached = probe.fetch("https://client.example/cached", CALLBACK);
    evidence.check(
        "cache_control_reuses_document",
        code == 200 && cached == 200 && hits() == before,
        "",
    );
    lock(resolver).now += 61;
    let cached = probe.fetch("https://client.example/cached", CALLBACK);
    evidence.check(
        "cache_control_expires_document",
        cached == 200 && hits() == before + 1,
        "",
    );
    let agents_before = world.gateway_cli(&["agent", "list"])?;
    for variant in [
        "good",
        "mismatch",
        "oversize",
        "slow",
        "private-redirect",
        "private",
        "bad-redirect",
        "malformed-redirect",
        "unsupported-redirect",
        "mixed-redirects",
    ] {
        let id = if variant == "private" {
            "https://private.example/client".to_owned()
        } else {
            format!("https://client.example/{variant}")
        };
        set(&mut probe.query, "client_id", &id);
        let redirect = match variant {
            "bad-redirect" => "https://attacker.example/callback",
            "malformed-redirect" => "%",
            "unsupported-redirect" => "ftp://connector.example/callback",
            _ => CALLBACK,
        };
        set(&mut probe.query, "redirect_uri", redirect);
        let (status, _) = probe.request("GET");
        if variant == "private-redirect" {
            evidence.check(
                "private_redirect_no_connection",
                counters.private_hits.load(Ordering::SeqCst) == 0
                    && counters.private_connections.load(Ordering::SeqCst) == 0,
                "",
            );
        }
        if variant == "good" {
            evidence.check("metadata_valid_consent", status == 200, "");
        } else {
            for method in ["GET", "POST"] {
                let reply = authorize(
                    owner,
                    method,
                    &probe.query,
                    Some(
                        &json!({"decision": "approve", "level": "read", "scopes": ["messages:read"]}),
                    ),
                );
                evidence.check(
                    &format!("reject_{variant}_{method}"),
                    reply.status == 400
                        && crate::web_agent_oauth::header(&reply, "Location").is_empty()
                        && crate::web_agent_oauth::header(&reply, "Content-Type")
                            .starts_with("text/html")
                        && contains(&reply, "<h1>This connection request isn't valid</h1>"),
                    &format!(
                        "status={} body={}",
                        reply.status,
                        String::from_utf8_lossy(&reply.body)
                    ),
                );
            }
        }
    }
    evidence.check(
        "invalid_metadata_never_grants_an_agent",
        world.gateway_cli(&["agent", "list"])? == agents_before,
        "",
    );
    set(&mut probe.query, "client_id", "https://client.example/good");
    set(&mut probe.query, "redirect_uri", CALLBACK);
    thread::sleep(Duration::from_millis(110));
    let reply = authorize(
        owner,
        "POST",
        &probe.query,
        Some(&json!({"decision": "approve", "level": "read", "scopes": ["messages:read"]})),
    );
    let granted = redirect_query(&reply);
    let code = get(&granted, "code");
    evidence.check(
        "metadata_owner_consent",
        reply.status == 200 && !code.is_empty(),
        "",
    );
    if code.is_empty() {
        return Ok(());
    }
    let form: Query = pairs(&[
        ("grant_type", "authorization_code"),
        ("client_id", &get(&probe.query, "client_id")),
        ("code", &code),
        ("redirect_uri", &get(&probe.query, "redirect_uri")),
        ("code_verifier", &verifier),
        ("resource", &resource),
    ]);
    let reply = agent(origin, "", "POST", "/oauth/token", None, Some(&form));
    evidence.check(
        "metadata_token_issued",
        reply.status == 200 && !text(&reply.json(), "access_token").is_empty(),
        "",
    );
    thread::sleep(Duration::from_secs(1));
    oauth(world, owner, evidence)
}
