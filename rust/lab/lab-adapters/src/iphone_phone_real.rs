#![allow(clippy::too_many_lines)]
use crate::{
    http::Client,
    iphone::{Scratch, project},
    phone::{Gateway, proxy, read_request, tls_server},
    replicas,
    webauthn::Owner,
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use lab_runner::{Evidence, Scenario};
use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer},
    pkcs8::DecodePrivateKey,
};
use rustls::{ServerConnection, StreamOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 7] = [
    "APP-SHELL-REAL",
    "APP-PAIR-REAL",
    "APP-CMD-REAL",
    "APP-NOTIFY-REAL",
    "APP-APPROVE-REAL",
    "APP-LIVE-REAL",
    "APP-INTENTS-REAL",
];
const BUNDLE: &str = "com.johansardar.applebridge.ios";
const PHASE_LIMIT: Duration = Duration::from_secs(210);
const RUN_LIMIT: Duration = Duration::from_secs(800);

#[derive(Debug, Default)]
struct Receipt {
    run: String,
    done: bool,
    checks: BTreeMap<String, bool>,
    counts: BTreeMap<String, i64>,
    digests: BTreeMap<String, String>,
}

impl Receipt {
    fn parse(raw: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(raw).ok()?;
        let map = |name: &str| value[name].as_object().cloned().unwrap_or_default();
        Some(Self {
            run: value["run"].as_str().unwrap_or_default().to_owned(),
            done: value["done"].as_bool().unwrap_or(false),
            checks: map("checks")
                .into_iter()
                .map(|(k, v)| (k, v.as_bool().unwrap_or(false)))
                .collect(),
            counts: map("counts")
                .into_iter()
                .map(|(k, v)| (k, v.as_i64().unwrap_or(0)))
                .collect(),
            digests: map("digests")
                .into_iter()
                .map(|(k, v)| (k, v.as_str().unwrap_or_default().to_owned()))
                .collect(),
        })
    }
}

fn paired(id: &str) -> bool {
    matches!(
        id,
        "APP-PAIR-REAL" | "APP-CMD-REAL" | "APP-NOTIFY-REAL" | "APP-APPROVE-REAL"
    )
}

#[must_use]
pub fn instruction(id: &str) -> &'static str {
    match id {
        "APP-SHELL-REAL" => {
            "tap Capability, inspect its fields and tap Back; tap Diagnostics, inspect its fields and tap Back, then tap Confirm observations"
        }
        "APP-PAIR-REAL" => "tap Allow on Apple Bridge Local Network if prompted",
        "APP-CMD-REAL" => {
            "tap Allow on Apple Bridge Local Network if prompted; keep the phone unlocked"
        }
        "APP-NOTIFY-REAL" => {
            "tap Allow on Notifications if prompted, touch and hold the [ab-test] Notification qualification banner, then tap [ab-test] Confirm"
        }
        "APP-APPROVE-REAL" => {
            "tap Allow on Notifications if prompted, open the [ab-test] approval, tap Approve and complete Face ID"
        }
        "APP-LIVE-REAL" => {
            "view [ab-test] Updated on the Lock Screen and the existing Bridge status widget, then return to Apple Bridge and tap Confirm observations"
        }
        _ => {
            "in Shortcuts tap +, search Check Bridge Status, tap Apple Bridge Check Bridge Status, name it [ab-test] Ping iPhone, tap Done, then tap the shortcut to run it and return to Apple Bridge"
        }
    }
}

#[must_use]
pub fn phase_instruction(id: &str, phase: i64) -> &'static str {
    if id == "APP-INTENTS-REAL" && phase == 2 {
        "in Shortcuts touch and hold [ab-test] Ping iPhone, tap Delete and Delete Shortcut, return to Apple Bridge, then tap Confirm observations"
    } else {
        instruction(id)
    }
}

#[must_use]
pub fn requirements(id: &str) -> &'static [&'static str] {
    match id {
        "APP-SHELL-REAL" => &["capability_fields_observed", "diagnostics_observed"],
        "APP-PAIR-REAL" => &["signed_pair_status", "identity_restored"],
        "APP-CMD-REAL" => &[
            "ready",
            "signed_result_uploaded",
            "apns_wake",
            "identity_restored",
        ],
        "APP-NOTIFY-REAL" => &[
            "notification_authorized",
            "ready",
            "signed_result_uploaded",
            "identity_restored",
        ],
        "APP-APPROVE-REAL" => &[
            "notification_authorized",
            "ready",
            "face_id_succeeded",
            "signed_result_uploaded",
            "identity_restored",
            "connector_prompt_code_and_scopes",
            "connector_face_id_succeeded",
            "connector_decision_uploaded",
        ],
        "APP-LIVE-REAL" => &[
            "activities_authorized",
            "activity_started",
            "activity_updated",
            "widget_shared_status",
            "live_and_widget_observed",
            "activity_ended",
        ],
        "APP-INTENTS-REAL" => &[
            "saved_shortcut_invoked",
            "shortcut_pong",
            "shortcut_cleaned_up",
        ],
        _ => &["unsupported_scenario"],
    }
}

fn announce(text: &str) {
    let text = text.replace(['\n', '\r'], " ");
    eprintln!("Automation step: {text}");
    if let Ok(path) = env::var("AB_IPHONE_AUTOMATION_STEPS")
        && let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "{} {text}", jiff::Zoned::now().strftime("%H:%M"));
    }
}

fn hex_random() -> Result<String, String> {
    let bytes = crate::phone::random::<12>().map_err(|e| e.to_string())?;
    Ok(hex::encode(bytes))
}

#[derive(Debug)]
struct PhoneLock(Child);

impl PhoneLock {
    fn acquire() -> Result<Self, String> {
        let mut child = Command::new("lockf")
            .args([
                "-k",
                "-t",
                "0",
                "/tmp/ab-iphone.lock",
                "sh",
                "-c",
                "printf 'locked\\n'; cat >/dev/null",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or("lock output")?;
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if line == "locked\n" {
            Ok(Self(child))
        } else {
            let _ = child.kill();
            Err("phone lock held elsewhere".into())
        }
    }
}

impl Drop for PhoneLock {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}

fn device_script(device: &str, check: bool) -> bool {
    let mut args = vec!["lab/iphone-real/device.py", device];
    if check {
        args.push("--check");
    }
    project("python3", &args).is_ok_and(|o| o.status.success())
}

#[derive(Debug)]
struct Device {
    id: String,
    offline: Duration,
}

impl Device {
    fn run(&mut self, args: &[&str]) -> bool {
        loop {
            let ok = Command::new("xcrun")
                .args(["devicectl", "--timeout", "15"])
                .args(args)
                .output()
                .is_ok_and(|o| o.status.success());
            if ok || device_script(&self.id, true) {
                if ok
                    && args.contains(&"launch")
                    && env::var_os("AB_WDA_URL").is_some()
                    && !project("python3", &["lab/iphone-real/activate.py", BUNDLE])
                        .is_ok_and(|o| o.status.success())
                {
                    return false;
                }
                return ok;
            }
            let start = Instant::now();
            let reconnected = device_script(&self.id, false);
            self.offline += start.elapsed();
            if !reconnected {
                return false;
            }
        }
    }

    fn launch(&mut self, extra: &[&str]) -> bool {
        let mut args = vec![
            "device",
            "process",
            "launch",
            "--device",
            self.id.as_str(),
            "--terminate-existing",
            BUNDLE,
        ];
        args.extend_from_slice(extra);
        let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.run(&refs)
    }

    fn copy_to(&mut self, source: &Path, destination: &str) -> bool {
        let id = self.id.clone();
        let source = source.to_string_lossy().into_owned();
        self.run(&[
            "device",
            "copy",
            "to",
            "--device",
            &id,
            "--source",
            &source,
            "--destination",
            destination,
            "--domain-type",
            "appDataContainer",
            "--domain-identifier",
            BUNDLE,
        ])
    }

    fn receipt(&mut self, source: &str, destination: &Path) -> Option<Receipt> {
        let _ = fs::remove_file(destination);
        let id = self.id.clone();
        let target = destination.to_string_lossy().into_owned();
        let copied = self.run(&[
            "device",
            "copy",
            "from",
            "--device",
            &id,
            "--source",
            source,
            "--destination",
            &target,
            "--domain-type",
            "appDataContainer",
            "--domain-identifier",
            BUNDLE,
        ]);
        if !copied {
            return None;
        }
        Receipt::parse(&fs::read(destination).ok()?)
    }
}

#[derive(Debug)]
struct Apns {
    key: SigningKey,
    key_id: String,
    team: String,
}

impl Apns {
    fn load() -> Result<Self, String> {
        let path = env::var("AB_REAL_APNS_KEY").map_err(|_| "set AB_REAL_APNS_KEY")?;
        let key_id = env::var("AB_REAL_APNS_KEY_ID").map_err(|_| "set AB_REAL_APNS_KEY_ID")?;
        let team = env::var("AB_REAL_APNS_TEAM_ID").map_err(|_| "set AB_REAL_APNS_TEAM_ID")?;
        let pem = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = STANDARD.decode(body).map_err(|e| e.to_string())?;
        let key = SigningKey::from_pkcs8_der(&der).map_err(|e| e.to_string())?;
        Ok(Self { key, key_id, team })
    }

    fn token(&self) -> String {
        let header = URL_SAFE_NO_PAD.encode(
            json!({"alg": "ES256", "kid": self.key_id})
                .to_string()
                .as_bytes(),
        );
        let claims = URL_SAFE_NO_PAD.encode(
            json!({"iss": self.team, "iat": jiff::Timestamp::now().as_second()})
                .to_string()
                .as_bytes(),
        );
        let message = format!("{header}.{claims}");
        let signature: Signature = self.key.sign(message.as_bytes());
        format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
    }

    fn send(&self, device: &str, packet: &Value, expires: i64) -> bool {
        if !(32..=512).contains(&device.len()) || !device.bytes().all(|b| b.is_ascii_hexdigit()) {
            return false;
        }
        let live = packet.get("live_activity").filter(|v| v.is_object());
        let body = live.map_or_else(
            || json!({"aps": {"content-available": 1}, "bridge": packet}),
            |live| json!({"aps": live}),
        );
        let (topic, kind, priority) = if live.is_some() {
            (
                format!("{BUNDLE}.push-type.liveactivity"),
                "liveactivity",
                "10",
            )
        } else {
            (BUNDLE.to_owned(), "background", "5")
        };
        let Ok(mut child) = Command::new("curl")
            .args(["--silent", "--http2", "--max-time", "20", "-o", "/dev/null"])
            .args(["-w", "%{http_code}", "-X", "POST"])
            .args(["-H", &format!("authorization: bearer {}", self.token())])
            .args(["-H", &format!("apns-topic: {topic}")])
            .args(["-H", &format!("apns-push-type: {kind}")])
            .args(["-H", &format!("apns-priority: {priority}")])
            .args(["-H", &format!("apns-expiration: {expires}")])
            .args(["--data-binary", "@-"])
            .arg(format!(
                "https://api.sandbox.push.apple.com/3/device/{device}"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
        else {
            return false;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(body.to_string().as_bytes());
        }
        child.wait_with_output().is_ok_and(|o| o.stdout == b"200")
    }
}

const RELAY_CREDENTIAL: &str = "ab-test-real-relay-credential";

#[derive(Debug)]
struct ApnsRelay {
    stop: Arc<AtomicBool>,
    address: String,
    accepted: Arc<AtomicI64>,
    failed: Arc<AtomicI64>,
}

impl ApnsRelay {
    fn start(gateway: &mut Gateway, apns: Apns) -> Result<Self, String> {
        let (config, pem) = tls_server(&["127.0.0.1"], false).map_err(|e| e.to_string())?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let address = listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicI64::new(0));
        let failed = Arc::new(AtomicI64::new(0));
        let apns = Arc::new(apns);
        {
            let (stop, accepted, failed) = (stop.clone(), accepted.clone(), failed.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let Ok(connection) = ServerConnection::new(config.clone()) else {
                        continue;
                    };
                    let (apns, accepted, failed) = (apns.clone(), accepted.clone(), failed.clone());
                    thread::spawn(move || {
                        let mut stream = StreamOwned::new(connection, stream);
                        let Some((method, path, headers, body)) = read_request(&mut stream) else {
                            return;
                        };
                        let bearer = headers
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                            .map(|(_, v)| v.trim_start_matches("Bearer ").to_owned());
                        let status = if method != "POST" || path != "/iphone/push" {
                            404
                        } else if bearer.as_deref() != Some(RELAY_CREDENTIAL) {
                            403
                        } else if let Ok(push) = serde_json::from_slice::<Value>(&body) {
                            let token = push["token"].as_str().unwrap_or_default();
                            let expires = push["expires"].as_i64().unwrap_or(0);
                            if apns.send(token, &push["packet"], expires) {
                                accepted.fetch_add(1, Ordering::SeqCst);
                                204
                            } else {
                                failed.fetch_add(1, Ordering::SeqCst);
                                502
                            }
                        } else {
                            400
                        };
                        let _ = stream.write_all(
                            format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                                .as_bytes(),
                        );
                        stream.conn.send_close_notify();
                        let _ = stream.flush();
                    });
                }
            });
        }
        let seed = crate::phone::random::<32>().map_err(|e| e.to_string())?;
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        fs::OpenOptions::new()
            .append(true)
            .open(gateway.certificate_path())
            .and_then(|mut file| file.write_all(pem.as_bytes()))
            .map_err(|e| e.to_string())?;
        fs::create_dir_all(gateway.home.join("remote")).map_err(|e| e.to_string())?;
        let relay = json!({
            "key": STANDARD.encode(signing.to_keypair_bytes()),
            "endpoint": format!("https://{address}"),
            "credential": {"hostname": "example.test", "credential": RELAY_CREDENTIAL},
        });
        fs::write(
            gateway.home.join("remote/relay.json"),
            serde_json::to_vec(&relay).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        gateway.restart().map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            address,
            accepted,
            failed,
        })
    }
}

impl Drop for ApnsRelay {
    fn drop(&mut self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            let _ = std::net::TcpStream::connect(&self.address);
        }
    }
}

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut bits, mut count) = (0u32, 0u32);
    for byte in bytes {
        bits = (bits << 8) | u32::from(*byte);
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(char::from(BASE32[((bits >> count) & 31) as usize]));
        }
    }
    if count > 0 {
        out.push(char::from(BASE32[((bits << (5 - count)) & 31) as usize]));
    }
    out
}

fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut bits, mut count) = (0u32, 0u32);
    for c in text.bytes() {
        let value = BASE32.iter().position(|b| *b == c)?;
        bits = (bits << 5) | u32::try_from(value).ok()?;
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push(u8::try_from((bits >> count) & 0xff).ok()?);
        }
    }
    Some(out)
}

fn lan_address() -> Result<String, String> {
    if let Ok(host) = env::var("AB_REAL_HOST")
        && !host.is_empty()
    {
        return Ok(host);
    }
    let output = Command::new("ipconfig")
        .args(["getifaddr", "en0"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[derive(Debug)]
struct Lan {
    stop: Arc<AtomicBool>,
    port: u16,
    fingerprint: String,
}

impl Lan {
    fn start(gateway: &Gateway, host: &str) -> Result<Self, String> {
        let (config, pem) = tls_server(&[host], false).map_err(|e| e.to_string())?;
        let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = STANDARD.decode(body).map_err(|e| e.to_string())?;
        let listener = TcpListener::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let home = gateway.home.clone();
        {
            let stop = stop.clone();
            thread::spawn(move || {
                for stream in listener.incoming() {
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
        Ok(Self {
            stop,
            port,
            fingerprint: hex::encode(Sha256::digest(der)),
        })
    }

    fn pair_code(&self, gateway: &Gateway, host: &str) -> Result<String, String> {
        let raw = gateway
            .command(&["pair", "--iphone", "--via-remote", "--expires-in", "10m"])
            .map_err(|e| e.to_string())?;
        let line = raw.lines().next().unwrap_or_default();
        let decoded = line
            .strip_prefix("ab1-")
            .and_then(base32_decode)
            .ok_or("pairing code")?;
        let mut code: Value = serde_json::from_slice(&decoded).map_err(|e| e.to_string())?;
        code["address"] = json!(format!("https://{host}:{}", self.port));
        code["fingerprint"] = json!(self.fingerprint);
        code["remote"] = json!(false);
        Ok(format!("ab1-{}", base32(code.to_string().as_bytes())))
    }
}

impl Drop for Lan {
    fn drop(&mut self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
        }
    }
}

#[derive(Debug)]
struct Connector {
    owner: Owner,
    client_id: String,
    verifier: String,
    pending: String,
    code: String,
}

const REDIRECT: &str = "https://connector.example/callback";

impl Connector {
    fn create(gateway: &Gateway) -> Result<Self, String> {
        let owner = Owner::new(gateway).map_err(|e| e.to_string())?;
        let registered = owner
            .request(
                "POST",
                "/oauth/register",
                "application/json",
                json!({
                    "client_name": "[ab-test] iPhone connection",
                    "redirect_uris": [REDIRECT],
                    "grant_types": ["authorization_code", "refresh_token"],
                })
                .to_string()
                .as_bytes(),
            )
            .map_err(|e| e.to_string())?;
        let client_id = registered.json()["client_id"]
            .as_str()
            .ok_or("client registration")?
            .to_owned();
        let secret = crate::phone::random::<32>().map_err(|e| e.to_string())?;
        let verifier = URL_SAFE_NO_PAD.encode(secret);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let resource = format!("{}/mcp", owner.origin);
        let query = [
            ("client_id", client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", resource.as_str()),
            ("scope", "messages:read"),
            ("state", "ab-test"),
        ]
        .iter()
        .map(|(k, v)| format!("{k}={}", percent(v)))
        .collect::<Vec<_>>()
        .join("&");
        let pending = owner
            .request(
                "POST",
                &format!("/oauth/pending?{query}"),
                "application/json",
                b"{}",
            )
            .map_err(|e| e.to_string())?;
        let body = pending.json();
        let id = body["id"].as_str().ok_or("connector pending")?.to_owned();
        let code = body["code"].as_str().unwrap_or_default().to_owned();
        if pending.status != 201 || !id.starts_with("connector:") {
            return Err("connector request".into());
        }
        Ok(Self {
            owner,
            client_id,
            verifier,
            pending: id,
            code,
        })
    }

    fn qualified(&self, gateway: &Gateway) -> bool {
        let Ok(polled) = self.owner.request(
            "GET",
            &format!("/oauth/pending/{}", percent(&self.pending)),
            "application/json",
            b"",
        ) else {
            return false;
        };
        let redirect = polled.json()["redirect"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let Some((_, query)) = redirect.split_once('?') else {
            return false;
        };
        let value = |name: &str| {
            query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_owned())
        };
        let (Some(state), Some(code)) = (value("state"), value("code")) else {
            return false;
        };
        if state != "ab-test" {
            return false;
        }
        let resource = format!("{}/mcp", self.owner.origin);
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", self.client_id.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", self.verifier.as_str()),
            ("resource", resource.as_str()),
        ]
        .iter()
        .map(|(k, v)| format!("{k}={}", percent(v)))
        .collect::<Vec<_>>()
        .join("&");
        let Ok(exchanged) = self.owner.request(
            "POST",
            "/oauth/token",
            "application/x-www-form-urlencoded",
            form.as_bytes(),
        ) else {
            return false;
        };
        let Some(token) = exchanged.json()["access_token"].as_str().map(str::to_owned) else {
            return false;
        };
        let client = Client::at(&gateway.api_address, &token);
        client
            .request(
                "POST",
                "/v1/op/messages.search",
                &[("Content-Type", "application/json")],
                Some(br#"{"query":"ab-test"}"#),
                15,
            )
            .is_ok_and(|r| r.status != 401 && r.status != 403)
    }
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn phone_device_id(gateway: &Gateway) -> Option<String> {
    fn find(value: &Value) -> Option<String> {
        match value {
            Value::Object(map) => {
                if map.get("kind").and_then(Value::as_str) == Some("iphone")
                    && let Some(id) = map.get("id").and_then(Value::as_str)
                {
                    return Some(id.to_owned());
                }
                map.values().find_map(find)
            }
            Value::Array(items) => items.iter().find_map(find),
            _ => None,
        }
    }
    let raw = fs::read(gateway.home.join("credentials.json")).ok()?;
    find(&serde_json::from_slice(&raw).ok()?)
}

fn evidence_for(scenario: &Scenario) -> Evidence {
    Evidence {
        scenario: scenario.id.clone(),
        passed: true,
        assertions: vec![],
    }
}

#[must_use]
pub fn run(scenario: &Scenario) -> Option<Evidence> {
    if !SCENARIOS.contains(&scenario.id.as_str()) {
        return None;
    }
    let mut evidence = evidence_for(scenario);
    drive(&scenario.id, &mut evidence);
    Some(evidence)
}

fn drive(id: &str, evidence: &mut Evidence) {
    let Ok(device) = env::var("AB_REAL_DEVICE") else {
        evidence.check("physical_device_selected", false, "");
        return;
    };
    if env::var("APPLE_BRIDGE_SECRET_STORE").as_deref() != Ok("file") {
        evidence.check("isolated_file_secret_store", false, "");
        return;
    }
    if !device_script(&device, false) {
        evidence.check("physical_device_available", false, "");
        return;
    }
    let lock = PhoneLock::acquire();
    if let Err(error) = &lock {
        evidence.check("phone_lock", false, error);
        return;
    }
    let dir = Scratch::new("real-phone");
    let Ok(dir) = dir else {
        evidence.check("temporary_home", false, "");
        return;
    };
    let Ok(run) = hex_random() else {
        evidence.check("run_nonce", false, "");
        return;
    };
    let command_id = format!("cmd_ab_test_{run}");
    let mut request = json!({"run": run, "scenario": id, "command": command_id});
    let mut gateway: Option<Gateway> = None;
    let mut relay: Option<ApnsRelay> = None;
    let mut lan: Option<Lan> = None;
    if paired(id) {
        let binary = match replicas::binary() {
            Ok(binary) => binary,
            Err(error) => {
                evidence.check("authority", false, &error.to_string());
                return;
            }
        };
        let mut started = match Gateway::start(binary) {
            Ok(started) => started,
            Err(error) => {
                evidence.check("authority", false, &error.to_string());
                return;
            }
        };
        let host = match lan_address() {
            Ok(host) if !host.is_empty() => host,
            _ => {
                evidence.check("reachable_host", false, "");
                return;
            }
        };
        match Lan::start(&started, &host)
            .and_then(|lan| lan.pair_code(&started, &host).map(|c| (lan, c)))
        {
            Ok((forward, code)) => {
                request["code"] = json!(code);
                lan = Some(forward);
            }
            Err(error) => {
                evidence.check("pair_code", false, &error);
                return;
            }
        }
        if id != "APP-PAIR-REAL" {
            match Apns::load().and_then(|apns| ApnsRelay::start(&mut started, apns)) {
                Ok(started_relay) => relay = Some(started_relay),
                Err(error) => {
                    evidence.check("apns_credentials", false, &error);
                    return;
                }
            }
        }
        gateway = Some(started);
    }
    let mut phone = Device {
        id: device,
        offline: Duration::ZERO,
    };
    let source = dir.path().join("request.json");
    if fs::write(&source, request.to_string()).is_err() {
        evidence.check("request_file", false, "");
        return;
    }
    if !phone.copy_to(&source, "Documents/ab-real-request.json") {
        evidence.check("copy_request", false, "");
        restore(&mut phone, &run, dir.path(), evidence);
        return;
    }
    if !phone.launch(&["--ab-real"]) {
        evidence.check("launch_probe", false, "");
        restore(&mut phone, &run, dir.path(), evidence);
        return;
    }
    announce(instruction(id));
    let mut state = Loop::default();
    let mut connector: Option<Connector> = None;
    let start = Instant::now();
    let mut phase_start = Instant::now();
    let mut phase_offline = phone.offline;
    while start.elapsed() < RUN_LIMIT + phone.offline {
        if phase_start
            .elapsed()
            .saturating_sub(phone.offline.saturating_sub(phase_offline))
            > PHASE_LIMIT
        {
            break;
        }
        if let Some(current) = phone.receipt(
            "Documents/ab-real-result.json",
            &dir.path().join("receipt.json"),
        ) && current.run == run
        {
            let phase = current.counts.get("phase").copied().unwrap_or(0);
            if phase != state.phase {
                state.phase = phase;
                phase_start = Instant::now();
                phase_offline = phone.offline;
                announce(phase_instruction(id, phase));
            }
            if id == "APP-APPROVE-REAL"
                && phase == 3
                && connector.is_none()
                && let Some(gateway) = &gateway
            {
                match Connector::create(gateway) {
                    Ok(created) => {
                        announce(&format!(
                            "open [ab-test] iPhone connection, match Code {}, tap Allow, and complete Face ID",
                            created.code
                        ));
                        connector = Some(created);
                    }
                    Err(error) => {
                        evidence.check("connector_request_created", false, &error);
                        restore(&mut phone, &run, dir.path(), evidence);
                        return;
                    }
                }
            }
            if current.checks.get("ready") == Some(&true)
                && !state.submitted
                && let Some(gateway) = &gateway
                && id != "APP-PAIR-REAL"
            {
                match submit(gateway, id, &command_id) {
                    Ok(()) => state.submitted = true,
                    Err(error) => {
                        evidence.check("admit_phone_command", false, &error);
                        restore(&mut phone, &run, dir.path(), evidence);
                        return;
                    }
                }
            }
            let done = current.done;
            state.receipt = current;
            if done {
                break;
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
    let receipt = &state.receipt;
    evidence.check(
        "fresh_completed_phone_receipt",
        receipt.run == run && receipt.done,
        "",
    );
    let check = |name: &str| receipt.checks.get(name) == Some(&true);
    evidence.check("state_restored", check("state_restored"), "");
    evidence.check("resources_cleaned", check("resources_cleaned"), "");
    for name in requirements(id) {
        evidence.check(name, check(name), "");
    }
    for (name, passed) in &receipt.checks {
        evidence.check(name, *passed, "");
    }
    let mut metrics = json!({});
    for (name, count) in &receipt.counts {
        metrics[name] = json!(count);
    }
    for (name, digest) in &receipt.digests {
        metrics[format!("{name}_digest")] = json!(digest);
    }
    if paired(id) {
        evidence.check("signed_pair_status", check("signed_pair_status"), "");
        evidence.check("identity_restored", check("identity_restored"), "");
    }
    if let Some(gateway) = &gateway
        && matches!(id, "APP-CMD-REAL" | "APP-NOTIFY-REAL" | "APP-APPROVE-REAL")
    {
        let record = gateway.record(&command_id);
        evidence.check(
            "gateway_observed_signed_result",
            record.as_ref().is_ok_and(|r| r["state"] == "observed"),
            "",
        );
        let payload = record
            .as_ref()
            .ok()
            .and_then(|r| crate::iphone_command::evidence_result(&r["evidence"]))
            .map_or(Value::Null, |r| r["data"].clone());
        if id == "APP-NOTIFY-REAL" {
            evidence.check(
                "notification_action_observed",
                payload["action_id"] == "confirmed",
                "",
            );
        } else {
            evidence.check("phone_pong_observed", payload["pong"] == true, "");
        }
        if id == "APP-APPROVE-REAL" {
            evidence.check(
                "connector_oauth_token_authenticated",
                connector.as_ref().is_some_and(|c| c.qualified(gateway)),
                "",
            );
            evidence.check("face_id_succeeded", check("face_id_succeeded"), "");
        }
        let accepted = relay
            .as_ref()
            .map_or(0, |r| r.accepted.load(Ordering::SeqCst));
        let failed = relay
            .as_ref()
            .map_or(0, |r| r.failed.load(Ordering::SeqCst));
        evidence.check("apns_accepted", accepted > 0, "");
        metrics["apns_accepted_count"] = json!(accepted);
        metrics["apns_failure_count"] = json!(failed);
    }
    evidence.check("run_metrics", true, &metrics.to_string());
    restore(&mut phone, &run, dir.path(), evidence);
    drop(lan);
    drop(lock);
}

#[derive(Debug, Default)]
struct Loop {
    phase: i64,
    submitted: bool,
    receipt: Receipt,
}

fn submit(gateway: &Gateway, id: &str, command_id: &str) -> Result<(), String> {
    let device = phone_device_id(gateway).ok_or("paired device count")?;
    let mut args = json!({"device_id": device});
    let mut op = "iphone.ping";
    if id == "APP-NOTIFY-REAL" {
        op = "iphone.notify";
        args["title"] = json!("[ab-test] Notification qualification");
        args["body"] = json!("[ab-test] Confirm receipt");
        args["buttons"] = json!([{"id": "confirmed", "title": "[ab-test] Confirm"}]);
    }
    let token = if id == "APP-APPROVE-REAL" {
        gateway.agent("approve").map_err(|e| e.to_string())?.0
    } else {
        gateway
            .command(&["token", "create", "--scope", "iphone:write"])
            .map_err(|e| e.to_string())?
    };
    let command = json!({"command_id": command_id, "op": op, "args": args, "wait_ms": 0});
    gateway
        .invoke(op, &command, &token)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn restore(phone: &mut Device, run: &str, dir: &Path, evidence: &mut Evidence) {
    let mut restored = false;
    if phone.launch(&["--ab-real-restore", run]) {
        for _ in 0..10 {
            if let Some(value) = phone.receipt(
                "Documents/ab-real-acknowledged.json",
                &dir.join("restored.json"),
            ) && value.run == run
                && value.done
                && value.checks.get("state_restored") == Some(&true)
                && value.checks.get("resources_cleaned") == Some(&true)
            {
                restored = true;
                break;
            }
            thread::sleep(Duration::from_secs(1));
        }
    }
    evidence.check("restoration_receipt", restored, "");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_round_trips_pairing_codes() {
        let samples: [&[u8]; 3] = [b"", b"{\"id\":1}", b"abcdefghij"];
        for sample in samples {
            assert_eq!(base32_decode(&base32(sample)).as_deref(), Some(sample));
        }
    }

    #[test]
    fn requirements_cover_every_scenario() {
        for id in SCENARIOS {
            assert_ne!(requirements(id), ["unsupported_scenario"]);
            assert_ne!(instruction(id), "");
        }
        assert_ne!(
            phase_instruction("APP-INTENTS-REAL", 2),
            phase_instruction("APP-INTENTS-REAL", 1)
        );
    }

    #[test]
    fn only_pairing_scenarios_start_a_gateway() {
        assert!(paired("APP-APPROVE-REAL"));
        assert!(!paired("APP-LIVE-REAL"));
    }

    #[test]
    fn receipt_parses_phone_result() {
        let raw = br#"{"run":"r","done":true,"checks":{"a":true,"b":false},"counts":{"phase":2},"digests":{"x":"y"}}"#;
        let Some(receipt) = Receipt::parse(raw) else {
            panic!("receipt")
        };
        assert_eq!(receipt.run, "r");
        assert!(receipt.done);
        assert_eq!(receipt.checks.get("b"), Some(&false));
        assert_eq!(receipt.counts.get("phase"), Some(&2));
        assert!(Receipt::parse(b"not json").is_none());
    }

    #[test]
    fn percent_encodes_reserved_characters() {
        assert_eq!(percent("a b/c:d"), "a%20b%2Fc%3Ad");
    }

    #[test]
    fn missing_device_fails_closed() {
        let mut evidence = Evidence {
            scenario: "APP-SHELL-REAL".into(),
            passed: true,
            assertions: vec![],
        };
        if env::var_os("AB_REAL_DEVICE").is_none() {
            drive("APP-SHELL-REAL", &mut evidence);
            assert!(!evidence.passed);
            assert_eq!(evidence.assertions[0].name, "physical_device_selected");
        }
    }
}
