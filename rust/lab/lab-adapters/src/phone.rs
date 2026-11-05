#![allow(
    clippy::too_many_lines,
    clippy::format_push_string,
    clippy::assigning_clones
)]
use crate::http::Client;
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, generic_array::GenericArray},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use lab_world::{Error, Result};
use rustls::{ServerConfig, ServerConnection, StreamOwned, pki_types};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
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
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

pub(crate) fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

#[derive(Debug)]
struct LocalState {
    config: Vec<u8>,
    etag: u32,
}

#[derive(Debug)]
pub struct Gateway {
    pub home: PathBuf,
    dir: PathBuf,
    binary: PathBuf,
    env: Vec<(String, String)>,
    child: Option<Child>,
    stop: Arc<AtomicBool>,
    local_path: PathBuf,
    public_address: SocketAddr,
    pub api: Client,
    pub api_address: String,
}

pub(crate) type Request = (String, String, Vec<(String, String)>, Vec<u8>);

pub(crate) fn read_request<R: Read>(stream: &mut R) -> Option<Request> {
    let mut data = Vec::new();
    let mut byte = [0u8; 1];
    while !data.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 || data.len() > 65536 {
            return None;
        }
        data.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&data).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).ok()?;
    Some((method, path, headers, body))
}

pub(crate) fn tls_server(names: &[&str], ca: bool) -> Result<(Arc<ServerConfig>, String)> {
    let mut params =
        rcgen::CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
            .map_err(|e| invalid(e.to_string()))?;
    if ca {
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    }
    let key = rcgen::KeyPair::generate().map_err(|e| invalid(e.to_string()))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| invalid(e.to_string()))?;
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| invalid(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(
                vec![pki_types::CertificateDer::from(cert.der().to_vec())],
                pki_types::PrivateKeyDer::try_from(key.serialize_der())
                    .map_err(|e| invalid(e.to_string()))?,
            )
            .map_err(|e| invalid(e.to_string()))?;
    Ok((Arc::new(config), cert.pem()))
}

fn serve_local(stream: &mut UnixStream, state: &Mutex<LocalState>) {
    let Some((method, path, headers, body)) = read_request(stream) else {
        return;
    };
    let mut state = lock(state);
    let (status, payload): (u16, Vec<u8>) = if method == "GET"
        && path.starts_with("/localapi/v0/status")
    {
        (
            200,
            br#"{"BackendState":"Running","Self":{"DNSName":"node.fixture.test.","CapMap":{"https":null,"funnel":null,"https://tailscale.com/cap/funnel-ports?ports=443,8443,10000":null}}}"#.to_vec(),
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
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&payload);
}

fn lease_target(home: &Path) -> Option<(String, String)> {
    let lease: Value =
        serde_json::from_slice(&fs::read(home.join("remote/lease.json")).ok()?).ok()?;
    let target = lease["target"].as_str()?.strip_prefix("http://")?;
    let (address, path) = target
        .split_once('/')
        .map_or((target, String::new()), |(a, p)| (a, format!("/{p}")));
    Some((address.to_owned(), path))
}

pub(crate) fn proxy(mut downstream: StreamOwned<ServerConnection, TcpStream>, home: &Path) {
    let Some((method, path, headers, body)) = read_request(&mut downstream) else {
        return;
    };
    let Some((address, prefix)) = lease_target(home) else {
        return;
    };
    let Ok(mut upstream) = TcpStream::connect(&address) else {
        return;
    };
    let mut head = format!(
        "{method} {prefix}{path} HTTP/1.1\r\nHost: node.fixture.test\r\nX-Forwarded-Host: node.fixture.test\r\nX-Forwarded-Proto: https\r\nX-Forwarded-For: 198.51.100.4\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in &headers {
        let lower = name.to_ascii_lowercase();
        if ![
            "host",
            "connection",
            "content-length",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
        ]
        .contains(&lower.as_str())
        {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str("\r\n");
    if upstream.write_all(head.as_bytes()).is_err() || upstream.write_all(&body).is_err() {
        return;
    }
    let mut chunk = [0u8; 8192];
    while let Ok(count) = upstream.read(&mut chunk) {
        if count == 0
            || downstream.write_all(&chunk[..count]).is_err()
            || downstream.flush().is_err()
        {
            break;
        }
    }
    downstream.conn.send_close_notify();
    let _ = downstream.flush();
    let _ = downstream.sock.shutdown(Shutdown::Both);
}

impl Gateway {
    pub fn start(binary: &Path) -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = PathBuf::from(format!(
            "/tmp/abp-{}-{}",
            std::process::id(),
            nanos % 1_000_000_000
        ));
        fs::create_dir_all(&dir)?;
        let home = dir.join("home");
        fs::create_dir_all(&home)?;
        let stop = Arc::new(AtomicBool::new(false));
        let local_path = dir.join("tailscale.sock");
        let listener = UnixListener::bind(&local_path)?;
        let state = Arc::new(Mutex::new(LocalState {
            config: br#"{"unrelated":"preserved"}"#.to_vec(),
            etag: 1,
        }));
        {
            let stop = stop.clone();
            let state = state.clone();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(mut stream) = stream {
                        let state = state.clone();
                        thread::spawn(move || serve_local(&mut stream, &state));
                    }
                }
            });
        }
        let (config, pem) = tls_server(&["node.fixture.test"], true)?;
        let cert_path = dir.join("fixture.pem");
        fs::write(&cert_path, pem)?;
        let public = TcpListener::bind("127.0.0.1:0")?;
        let public_address = public.local_addr()?;
        {
            let stop = stop.clone();
            let home = home.clone();
            thread::spawn(move || {
                for stream in public.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let Ok(connection) = ServerConnection::new(config.clone()) else {
                        continue;
                    };
                    let home = home.clone();
                    thread::spawn(move || proxy(StreamOwned::new(connection, stream), &home));
                }
            });
        }
        let env = vec![
            ("APPLE_BRIDGE_SECRET_STORE".to_owned(), "file".to_owned()),
            (
                "APPLE_BRIDGE_TAILSCALE_SOCKET".to_owned(),
                local_path.to_string_lossy().into_owned(),
            ),
            (
                "APPLE_BRIDGE_REMOTE_PROBE_ADDRESS".to_owned(),
                public_address.to_string(),
            ),
            (
                "APPLE_BRIDGE_RELAY_URL".to_owned(),
                "https://127.0.0.1:1".to_owned(),
            ),
            (
                "SSL_CERT_FILE".to_owned(),
                cert_path.to_string_lossy().into_owned(),
            ),
            (
                "APPLE_BRIDGE_LAB_TLS_ROOT".to_owned(),
                cert_path.to_string_lossy().into_owned(),
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
        ];
        let mut gateway = Self {
            home,
            dir,
            binary: binary.to_owned(),
            env,
            child: None,
            stop,
            local_path,
            public_address,
            api: Client::at("", ""),
            api_address: String::new(),
        };
        gateway.launch()?;
        gateway.command(&["remote", "on", "--via", "tailscale-app"])?;
        gateway.wait(Duration::from_secs(30), "public ingress readiness", |g| {
            fs::read(g.home.join("remote/status.json"))
                .ok()
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .is_some_and(|v| v["state"] == "on")
        })?;
        Ok(gateway)
    }

    fn wait(&mut self, limit: Duration, what: &str, ready: impl Fn(&Self) -> bool) -> Result<()> {
        let until = Instant::now() + limit;
        while Instant::now() < until {
            if let Some(child) = self.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                self.child = None;
                return Err(invalid(format!("gateway exited {status}")));
            }
            if ready(self) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
        Err(invalid(format!("{what} timed out")))
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
        self.wait(Duration::from_secs(15), "gateway readiness", |g| {
            let runtime = g.home.join("runtime.json");
            let fresh = fs::metadata(&runtime)
                .and_then(|m| m.modified())
                .is_ok_and(|m| m >= started);
            let address = fs::read(&runtime)
                .ok()
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .and_then(|v| v["api_address"].as_str().map(str::to_owned));
            fresh
                && address.is_some_and(|a| TcpStream::connect(&a).is_ok())
                && UnixStream::connect(g.home.join("run/peer.sock")).is_ok()
        })?;
        let runtime: Value = serde_json::from_slice(&fs::read(self.home.join("runtime.json"))?)?;
        self.api_address = runtime["api_address"]
            .as_str()
            .ok_or_else(|| invalid("api address"))?
            .to_owned();
        Ok(())
    }

    pub fn command(&self, args: &[&str]) -> Result<String> {
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

    pub fn request(&self, path: &str, input: &Value, token: &str) -> Result<(u16, Value)> {
        let client = Client::at(&self.api_address, token);
        let body = serde_json::to_vec(input)?;
        let reply = client.request(
            "POST",
            path,
            &[("Content-Type", "application/json")],
            Some(&body),
            15,
        )?;
        Ok((
            reply.status,
            serde_json::from_slice(&reply.body).unwrap_or(Value::Null),
        ))
    }

    pub fn invoke(&self, operation: &str, input: &Value, token: &str) -> Result<Value> {
        let (status, value) = self.request(&format!("/v1/op/{operation}"), input, token)?;
        if status == 200 {
            Ok(value)
        } else {
            Err(invalid(format!("{operation}: HTTP {status}")))
        }
    }

    pub fn public(&self, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>)> {
        let (address, prefix) = lease_target(&self.home).ok_or_else(|| invalid("remote lease"))?;
        let client = Client::at(&address, "");
        let reply = client.request(
            "POST",
            &format!("{prefix}{path}"),
            &[
                ("Host", "node.fixture.test"),
                ("X-Forwarded-Host", "node.fixture.test"),
                ("X-Forwarded-Proto", "https"),
                ("X-Forwarded-For", "198.51.100.4"),
                ("Content-Type", "application/json"),
            ],
            Some(body),
            15,
        )?;
        Ok((reply.status, reply.body))
    }

    #[must_use]
    pub fn certificate_path(&self) -> PathBuf {
        self.dir.join("fixture.pem")
    }

    #[must_use]
    pub fn log(&self) -> String {
        fs::read_to_string(self.dir.join("gateway.log")).unwrap_or_default()
    }

    fn stop_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = Command::new("kill").arg(child.id().to_string()).status();
            let until = Instant::now() + Duration::from_secs(10);
            while Instant::now() < until && child.try_wait().ok().flatten().is_none() {
                thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn restart(&mut self) -> Result<()> {
        let lease = self.home.join("remote/lease.json");
        let ingress = lease.exists();
        self.stop_child();
        let started = SystemTime::now();
        self.launch()?;
        if !ingress {
            return Ok(());
        }
        self.command(&["remote", "on", "--via", "tailscale-app"])?;
        let certificate = self.certificate_path();
        let port = self.public_address.port();
        self.wait(Duration::from_secs(30), "ingress readiness", |g| {
            fs::metadata(&lease)
                .and_then(|m| m.modified())
                .is_ok_and(|m| m >= started)
                && Command::new("curl")
                    .args([
                        "--silent",
                        "--output",
                        "/dev/null",
                        "--max-time",
                        "5",
                        "--cacert",
                    ])
                    .arg(&certificate)
                    .args(["--resolve", &format!("node.fixture.test:{port}:127.0.0.1")])
                    .args(["--write-out", "%{http_code}"])
                    .arg(format!("https://node.fixture.test:{port}/"))
                    .output()
                    .is_ok_and(|o| {
                        let code = String::from_utf8_lossy(&o.stdout).into_owned();
                        o.status.success() && code != "502" && code != "503" && g.child.is_some()
                    })
        })
    }

    pub fn agent(&self, level: &str) -> Result<(String, String)> {
        let raw = self.command(&[
            "agent",
            "create",
            "--name",
            &format!("[ab-test] {level}"),
            "--level",
            level,
            "--scope",
            "iphone:write",
        ])?;
        let value: Value = serde_json::from_str(&raw)?;
        Ok((
            value["token"].as_str().unwrap_or_default().to_owned(),
            value["agent"]["id"].as_str().unwrap_or_default().to_owned(),
        ))
    }

    pub fn lab(&self, action: &str, owner: &str, pin: &str) -> Result<Value> {
        self.lab_with(action, owner, pin, "")
    }

    pub fn lab_with(&self, action: &str, owner: &str, pin: &str, digest: &str) -> Result<Value> {
        let body = serde_json::to_vec(
            &json!({"action": action, "owner": owner, "pin": pin, "digest": digest}),
        )?;
        let mut stream = UnixStream::connect(self.home.join("run/blobs-lab.sock"))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        write!(
            stream,
            "POST / HTTP/1.1\r\nHost: lab\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        stream.write_all(&body)?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw)?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, payload) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        if !head.starts_with("HTTP/1.1 200") {
            return Err(invalid(format!(
                "lab fixture {}",
                head.lines().next().unwrap_or_default()
            )));
        }
        Ok(serde_json::from_str(payload).unwrap_or(Value::Null))
    }

    pub fn expire(&self, source: &str, id: &str) -> Result<()> {
        self.lab("phone-expired", source, id).map(|_| ())
    }

    pub fn record(&self, id: &str) -> Result<Value> {
        self.lab("phone-command", id, "")
    }

    pub fn public_address(&self) -> SocketAddr {
        self.public_address
    }

    pub fn close(&mut self) {
        self.stop_child();
        if !self.stop.swap(true, Ordering::SeqCst) {
            let _ = UnixStream::connect(&self.local_path);
            let _ = TcpStream::connect(self.public_address);
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct Phone {
    signing: SigningKey,
    encryption: StaticSecret,
    gateway_key: Vec<u8>,
    pub id: String,
}

impl std::fmt::Debug for Phone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Phone")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = Vec::new();
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            b'=' => break,
            _ => return None,
        };
        bits = (bits << 5) | u32::from(value);
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push(u8::try_from((bits >> count) & 0xff).ok()?);
        }
    }
    Some(out)
}

fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

impl Phone {
    pub fn new() -> Result<Self> {
        Ok(Self {
            signing: SigningKey::from_bytes(&random::<32>()?),
            encryption: StaticSecret::from(random::<32>()?),
            gateway_key: vec![],
            id: String::new(),
        })
    }

    pub fn pair(&mut self, gateway: &Gateway, raw: &str) -> Result<u16> {
        let decoded = raw
            .strip_prefix("ab1-")
            .and_then(base32_decode)
            .ok_or_else(|| invalid("invalid pairing code"))?;
        let code: Value = serde_json::from_slice(&decoded)?;
        let id = code["id"].as_str().ok_or_else(|| invalid("pairing id"))?;
        let secret = STANDARD
            .decode(
                code["secret"]
                    .as_str()
                    .ok_or_else(|| invalid("pairing secret"))?,
            )
            .map_err(|e| invalid(e.to_string()))?;
        let signing = self.signing.verifying_key().to_bytes();
        let encryption = PublicKey::from(&self.encryption).to_bytes();
        let mut message = format!("apple-bridge-iphone-pair/1\0{id}\0").into_bytes();
        message.extend_from_slice(&signing);
        message.extend_from_slice(&encryption);
        let key = Sha256::digest(&secret);
        let mut mac =
            <HmacSha256 as Mac>::new_from_slice(&key).map_err(|e| invalid(e.to_string()))?;
        mac.update(&message);
        let request = json!({
            "id": id,
            "signing_key": b64(&signing),
            "encryption_key": b64(&encryption),
            "proof": b64(&mac.finalize().into_bytes()),
            "signature": b64(&self.signing.sign(&message).to_bytes()),
        });
        let (status, body) = gateway.public("/v1/iphone/pair", &serde_json::to_vec(&request)?)?;
        if status == 200 {
            let result: Value = serde_json::from_slice(&body)?;
            let device = result["id"].as_str().unwrap_or_default();
            let gateway_key = STANDARD
                .decode(result["gateway_key"].as_str().unwrap_or_default())
                .map_err(|e| invalid(e.to_string()))?;
            if code["gateway_key"] != result["gateway_key"] {
                return Err(invalid("pair response key differs from the pairing code"));
            }
            let proof = STANDARD
                .decode(result["pair_proof"].as_str().unwrap_or_default())
                .map_err(|e| invalid(e.to_string()))?;
            let mut response = format!(
                "apple-bridge-iphone-pair-response/1\0{id}\0{}\0{device}\0",
                code["address"].as_str().unwrap_or_default()
            )
            .into_bytes();
            response.extend_from_slice(&signing);
            response.extend_from_slice(&encryption);
            response.extend_from_slice(&gateway_key);
            let mut mac =
                <HmacSha256 as Mac>::new_from_slice(&key).map_err(|e| invalid(e.to_string()))?;
            mac.update(&response);
            mac.verify_slice(&proof)
                .map_err(|_| invalid("pair response proof is invalid"))?;
            device.clone_into(&mut self.id);
            self.gateway_key = gateway_key;
        }
        Ok(status)
    }

    pub fn request(
        &self,
        gateway: &Gateway,
        path: &str,
        payload: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>)> {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut message = format!("apple-bridge-iphone/1\0{path}\0{at}\0").into_bytes();
        message.extend_from_slice(payload.unwrap_or_default());
        let body = json!({
            "device_id": self.id,
            "at": at,
            "payload": payload.map(b64),
            "signature": b64(&self.signing.sign(&message).to_bytes()),
        });
        gateway.public(path, &serde_json::to_vec(&body)?)
    }

    pub fn open(&self, packet: &Value) -> Result<Value> {
        let decode = |name: &str| {
            STANDARD
                .decode(packet[name].as_str().unwrap_or_default())
                .map_err(|e| invalid(e.to_string()))
        };
        let ephemeral = decode("ephemeral_key")?;
        let sealed = decode("box")?;
        let signature = decode("signature")?;
        let mut message = b"apple-bridge-iphone-command/1\0".to_vec();
        message.extend_from_slice(&ephemeral);
        message.extend_from_slice(&sealed);
        let verifying = <[u8; 32]>::try_from(self.gateway_key.as_slice())
            .ok()
            .and_then(|b| VerifyingKey::from_bytes(&b).ok())
            .ok_or_else(|| invalid("gateway key"))?;
        let signature =
            ed25519_dalek::Signature::from_slice(&signature).map_err(|e| invalid(e.to_string()))?;
        if sealed.len() < 28 || verifying.verify(&message, &signature).is_err() {
            return Err(invalid("invalid phone packet"));
        }
        let public = PublicKey::from(
            <[u8; 32]>::try_from(ephemeral.as_slice()).map_err(|_| invalid("ephemeral key"))?,
        );
        let shared = self.encryption.diffie_hellman(&public);
        let mut extract = <HmacSha256 as Mac>::new_from_slice(b"apple-bridge-iphone/1")
            .map_err(|e| invalid(e.to_string()))?;
        extract.update(shared.as_bytes());
        let mut expand = <HmacSha256 as Mac>::new_from_slice(&extract.finalize().into_bytes())
            .map_err(|e| invalid(e.to_string()))?;
        expand.update(self.id.as_bytes());
        expand.update(&[1]);
        let key = expand.finalize().into_bytes();
        let cipher = Aes256Gcm::new(GenericArray::from_slice(&key));
        let plain = cipher
            .decrypt(GenericArray::from_slice(&sealed[..12]), &sealed[12..])
            .map_err(|_| invalid("packet decrypt"))?;
        Ok(serde_json::from_slice(&plain)?)
    }

    pub fn pending(&self, gateway: &Gateway) -> Result<(u16, Vec<Value>)> {
        let (status, body) = self.request(gateway, "/v1/iphone/pending", None)?;
        Ok((status, serde_json::from_slice(&body).unwrap_or_default()))
    }

    pub fn post(&self, gateway: &Gateway, path: &str, payload: &Value) -> Result<(u16, Vec<u8>)> {
        self.request(gateway, path, Some(&serde_json::to_vec(payload)?))
    }
}

pub fn pair(gateway: &Gateway) -> Result<(Phone, String)> {
    let output = gateway.command(&["pair", "--iphone", "--via-remote"])?;
    let code = output.split('\n').next().unwrap_or_default().to_owned();
    let mut phone = Phone::new()?;
    let status = phone.pair(gateway, &code)?;
    if status != 200 {
        return Err(invalid(format!("phone pairing HTTP {status}")));
    }
    let token = gateway.command(&["token", "create", "--scope", "iphone:write"])?;
    Ok((phone, token))
}
