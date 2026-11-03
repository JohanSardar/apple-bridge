use crate::{
    phone::read_request,
    rawhttp::{Response, request},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lab_runner::Evidence;
use lab_world::{Error, Result};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const NODE: &str = "node.fixture.ts.net";
const LOCAL: &str = "localhost:7780";

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug)]
struct LocalState {
    config: Vec<u8>,
    etag: u32,
}

fn lease_target(home: &Path) -> Option<String> {
    let lease: Value =
        serde_json::from_slice(&fs::read(home.join("private/remote/lease.json")).ok()?).ok()?;
    lease["target"].as_str().map(str::to_owned)
}

fn dial(mut stream: UnixStream, home: &Path, headers: &[(String, String)]) {
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map_or("", |(_, v)| v.as_str())
    };
    if header("Dial-Host") != NODE
        || header("Dial-Port") != "7780"
        || header("Dial-Network") != "tcp"
    {
        let _ =
            stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 13\r\n\r\ninvalid dial\n");
        return;
    }
    let Some(upstream) = lease_target(home).and_then(|t| TcpStream::connect(t).ok()) else {
        let _ = stream.write_all(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 12\r\n\r\nunavailable\n",
        );
        return;
    };
    if stream
        .write_all(
            b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: ts-dial\r\n\r\n",
        )
        .is_err()
    {
        return;
    }
    let (Ok(mut read_side), Ok(mut write_side), Ok(mut up_read), Ok(mut up_write)) = (
        stream.try_clone(),
        stream.try_clone(),
        upstream.try_clone(),
        upstream.try_clone(),
    ) else {
        return;
    };
    let forward = thread::spawn(move || {
        let _ = std::io::copy(&mut read_side, &mut up_write);
        let _ = up_write.shutdown(Shutdown::Both);
    });
    let _ = std::io::copy(&mut up_read, &mut write_side);
    let _ = stream.shutdown(Shutdown::Both);
    let _ = forward.join();
}

fn serve_local(stream: UnixStream, home: &Path, state: &Mutex<LocalState>) {
    let mut reader = stream;
    let Some((method, path, headers, body)) = read_request(&mut reader) else {
        return;
    };
    if path == "/localapi/v0/dial" {
        dial(reader, home, &headers);
        return;
    }
    let mut state = lock(state);
    let (status, payload): (u16, Vec<u8>) = if method == "GET"
        && path.starts_with("/localapi/v0/status")
    {
        (
            200,
            format!(r#"{{"BackendState":"Running","Self":{{"DNSName":"{NODE}."}}}}"#).into_bytes(),
        )
    } else if path != "/localapi/v0/serve-config" {
        (404, b"404 page not found\n".to_vec())
    } else if method == "POST" {
        let matches = headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("if-match") && *v == state.etag.to_string());
        if !matches {
            (412, Vec::new())
        } else if serde_json::from_slice::<Value>(&body).is_err() {
            (400, Vec::new())
        } else {
            state.config = body;
            state.etag += 1;
            (200, b"{}".to_vec())
        }
    } else {
        (200, state.config.clone())
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nETag: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        state.etag,
        payload.len()
    );
    let _ = reader.write_all(head.as_bytes());
    let _ = reader.write_all(&payload);
}

struct Private {
    dir: PathBuf,
    home: PathBuf,
    binary: PathBuf,
    env: Vec<(String, String)>,
    socket: PathBuf,
    child: Option<Child>,
    stop: Arc<AtomicBool>,
}
impl Private {
    fn start(binary: &Path, expired: bool, label: &str) -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = PathBuf::from(format!(
            "/tmp/abq-{label}-{}-{}",
            std::process::id(),
            nanos % 1_000_000
        ));
        let home = dir.join("home");
        fs::create_dir_all(&home)?;
        let socket = dir.join("tailscale.sock");
        let listener = UnixListener::bind(&socket)?;
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(LocalState {
            config: br#"{"unrelated":"preserved"}"#.to_vec(),
            etag: 1,
        }));
        {
            let (stop, state, home) = (stop.clone(), state, home.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        let (state, home) = (state.clone(), home.clone());
                        thread::spawn(move || serve_local(stream, &home, &state));
                    }
                }
            });
        }
        let probe = TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string();
        let mut env = vec![
            ("APPLE_BRIDGE_SECRET_STORE".to_owned(), "file".to_owned()),
            (
                "APPLE_BRIDGE_TAILSCALE_SOCKET".to_owned(),
                socket.to_string_lossy().into_owned(),
            ),
            ("APPLE_BRIDGE_REMOTE_PROBE_ADDRESS".to_owned(), probe),
            (
                "APPLE_BRIDGE_RELAY_URL".to_owned(),
                "https://127.0.0.1:1".to_owned(),
            ),
            (
                "APPLE_BRIDGE_LAB_PROBE_DURATION".to_owned(),
                "1s".to_owned(),
            ),
            (
                "APPLE_BRIDGE_LAB_REMOTE_DIAGNOSTICS".to_owned(),
                home.join("remote-errors.log")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("APPLE_BRIDGE_LAB_PRIVATE_FEED".to_owned(), "1".to_owned()),
        ];
        if expired {
            env.push(("APPLE_BRIDGE_LAB_PRIVATE_TTL".to_owned(), "0".to_owned()));
        }
        let mut private = Self {
            dir,
            home,
            binary: binary.to_owned(),
            env,
            socket,
            child: None,
            stop,
        };
        private.launch()?;
        private.command(&["remote", "on", "--via", "private"])?;
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let on = fs::read(private.home.join("remote/status.json"))
                .ok()
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .is_some_and(|v| v["state"] == "on");
            if on {
                return Ok(private);
            }
            if Instant::now() >= until {
                return Err(invalid("public ingress readiness timed out"));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn launch(&mut self) -> Result<()> {
        let started = SystemTime::now();
        let log = fs::File::create(self.dir.join("gateway.log"))?;
        let child = Command::new(&self.binary)
            .args(["start", "--role", "gateway", "--home"])
            .arg(&self.home)
            .args(["--api-addr", "127.0.0.1:0", "--peer-addr", "127.0.0.1:0"])
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdout(log.try_clone()?)
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()?;
        self.child = Some(child);
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(child) = self.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                self.child = None;
                return Err(invalid(format!("gateway exited {status}")));
            }
            let runtime = self.home.join("runtime.json");
            let fresh = fs::metadata(&runtime)
                .and_then(|m| m.modified())
                .is_ok_and(|m| m >= started);
            let address = fs::read(&runtime)
                .ok()
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .and_then(|v| v["api_address"].as_str().map(str::to_owned));
            if fresh
                && address.is_some_and(|a| TcpStream::connect(&a).is_ok())
                && UnixStream::connect(self.home.join("run/peer.sock")).is_ok()
            {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(invalid("gateway readiness timed out"));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn command(&self, args: &[&str]) -> Result<String> {
        let output = Command::new(&self.binary)
            .args(args)
            .arg("--home")
            .arg(&self.home)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .output()?;
        if !output.status.success() {
            return Err(invalid(format!(
                "gateway {} exited {}: {}",
                args.first().copied().unwrap_or_default(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
    fn invitation(&self) -> Option<(String, Value)> {
        let raw = fs::read(self.home.join("remote/status.json")).ok()?;
        let status: Value = serde_json::from_slice(&raw).ok()?;
        let code = status["message"]
            .as_str()?
            .strip_prefix("bridge connect ")?
            .to_owned();
        let decoded = URL_SAFE_NO_PAD.decode(&code).ok()?;
        Some((code, serde_json::from_slice(&decoded).ok()?))
    }
}
impl Drop for Private {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(mut child) = self.child.take() {
            let _ = Command::new("kill").arg(child.id().to_string()).status();
            let until = Instant::now() + Duration::from_secs(10);
            while Instant::now() < until && child.try_wait().ok().flatten().is_none() {
                thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = UnixStream::connect(&self.socket);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn get(address: &str, path: &str, token: &str) -> Option<Response> {
    let bearer = format!("Bearer {token}");
    let mut headers = vec![("Host", LOCAL)];
    if !token.is_empty() {
        headers.push(("Authorization", &bearer));
    }
    request(address, "GET", path, &headers, None, 3).ok()
}

fn mcp_tools(address: &str, token: &str) -> bool {
    let bearer = format!("Bearer {token}");
    let post = |body: &Value, session: Option<&str>| {
        let mut headers = vec![
            ("Host", LOCAL),
            ("Authorization", bearer.as_str()),
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", "2025-06-18"),
        ];
        if let Some(session) = session {
            headers.push(("Mcp-Session-Id", session));
        }
        request(
            address,
            "POST",
            "/mcp",
            &headers,
            Some(body.to_string().as_bytes()),
            5,
        )
        .ok()
    };
    let Some(init) = post(
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"rust-private","version":"1"}}}),
        None,
    ) else {
        return false;
    };
    if init.status != 200 {
        return false;
    }
    let session = init.header("Mcp-Session-Id").map(str::to_owned);
    let _ = post(
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        session.as_deref(),
    );
    post(
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        session.as_deref(),
    )
    .is_some_and(|r| r.status == 200 && r.text().contains("\"tools\""))
}

fn field(value: &Value, name: &str) -> String {
    value
        .as_object()
        .and_then(|map| {
            map.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| v.as_str())
        })
        .unwrap_or_default()
        .to_owned()
}

fn contains(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|w| w == needle)
}

fn files_private(home: &Path) -> bool {
    fn walk(directory: &Path) -> bool {
        let Ok(entries) = fs::read_dir(directory) else {
            return true;
        };
        entries.flatten().all(|entry| {
            let path = entry.path();
            if path.is_dir() {
                walk(&path)
            } else {
                let data = fs::read(&path).unwrap_or_default();
                !contains(&data, b"auth-secret-token") && !contains(&data, b"feature-secret-token")
            }
        })
    }
    walk(home)
}

fn wait_output(file: &Path, needle: &str, seconds: u64) -> bool {
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        if fs::read_to_string(file).is_ok_and(|text| text.contains(needle)) {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn free_address() -> Result<String> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.to_string())
}

#[allow(clippy::too_many_lines)]
pub fn run(binary: &Path, evidence: &mut Evidence) -> Result<()> {
    let gateway = match Private::start(binary, false, "on") {
        Ok(gateway) => gateway,
        Err(error) => {
            evidence.check("private_on", false, &error.to_string());
            return Ok(());
        }
    };
    let status = gateway.command(&["remote", "status"]);
    evidence.check("status_cli", status.is_ok(), "");
    let status: Value = status
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null);
    evidence.check(
        "status_on_private",
        status["state"] == "on" && status["backend"] == "private",
        "",
    );
    let invitation = gateway.invitation();
    let (code, secret) = invitation.map_or((String::new(), String::new()), |(code, invite)| {
        (
            code,
            if field(&invite, "host") == NODE {
                field(&invite, "secret")
            } else {
                String::new()
            },
        )
    });
    evidence.check("private_invitation_valid", secret.len() == 43, "");
    let target = lease_target(&gateway.home).unwrap_or_default();
    evidence.check(
        "session_required",
        get(&target, "/mcp", "").is_some_and(|r| r.status == 403),
        "",
    );
    let address = free_address()?;
    let output = gateway.dir.join("connect.out");
    let sink = fs::File::create(&output)?;
    let connect = Command::new(binary)
        .args(["connect", &code, "--home"])
        .arg(gateway.dir.join("connect"))
        .args(["--listen", &address])
        .env(
            "APPLE_BRIDGE_TAILSCALE_SOCKET",
            gateway.socket.to_string_lossy().as_ref(),
        )
        .stdout(sink.try_clone()?)
        .stderr(sink)
        .stdin(Stdio::null())
        .spawn();
    evidence.check("connect_started", connect.is_ok(), "");
    let Ok(mut connect) = connect else {
        return Ok(());
    };
    evidence.check(
        "invitation_redeemed",
        wait_output(&output, "http://localhost:7780", 5),
        "",
    );
    evidence.check(
        "session_reaches_bridge_challenge",
        get(&address, "/mcp", "").is_some_and(|r| {
            r.status == 401
                && r.header("WWW-Authenticate")
                    .is_some_and(|v| v.contains("Bearer"))
        }),
        "",
    );
    let reuse = request(
        &target,
        "POST",
        "/_bridge/private/invite",
        &[
            ("Host", LOCAL),
            ("Authorization", &format!("Bearer {secret}")),
        ],
        None,
        3,
    );
    evidence.check(
        "invitation_reuse_rejected",
        reuse.is_ok_and(|r| r.status == 403),
        "",
    );
    let token = gateway.command(&["token", "create", "--scope", "system:read"]);
    evidence.check("api_token", token.is_ok(), "");
    let token = token.unwrap_or_default();
    evidence.check("private_mcp_tools_list", mcp_tools(&address, &token), "");
    let feed = get(&address, "/v1/feed", &token);
    evidence.check(
        "private_feed_dispatch",
        feed.is_some_and(|r| r.status == 200 && r.text().contains("private-feed")),
        "",
    );
    evidence.check(
        "private_transport_retains_authentication",
        get(&address, "/", "").is_some_and(|r| r.status == 401),
        "",
    );
    let _ = connect.kill();
    let _ = connect.wait();
    let persisted = {
        fn found(directory: &Path) -> bool {
            fs::read_dir(directory)
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| {
                    let path = entry.path();
                    if path.is_dir() {
                        found(&path)
                    } else {
                        let mut text = String::new();
                        let _ = fs::File::open(&path).and_then(|mut f| f.read_to_string(&mut text));
                        text.contains("tailscaled.state")
                            || entry
                                .file_name()
                                .to_string_lossy()
                                .contains("tailscaled.state")
                    }
                })
        }
        found(&gateway.dir.join("connect"))
    };
    evidence.check("connect_has_no_persisted_identity", !persisted, "");
    match Private::start(binary, true, "exp") {
        Ok(expired) => {
            let code = expired
                .invitation()
                .map(|(code, _)| code)
                .unwrap_or_default();
            let output = Command::new(binary)
                .args(["connect", &code, "--home"])
                .arg(expired.dir.join("connect"))
                .args(["--listen", "127.0.0.1:0"])
                .env(
                    "APPLE_BRIDGE_TAILSCALE_SOCKET",
                    expired.socket.to_string_lossy().as_ref(),
                )
                .output();
            evidence.check(
                "expired_invitation_rejected",
                output.is_ok_and(|o| {
                    !o.status.success()
                        && format!(
                            "{}{}",
                            String::from_utf8_lossy(&o.stdout),
                            String::from_utf8_lossy(&o.stderr)
                        )
                        .contains("private invitation rejected")
                }),
                "",
            );
        }
        Err(_) => evidence.check("expired_invitation_rejected", false, ""),
    }
    evidence.check(
        "approval_tokens_absent_from_status_and_logs",
        files_private(&gateway.home),
        "",
    );
    Ok(())
}
