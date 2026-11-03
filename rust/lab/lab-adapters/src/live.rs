#![allow(clippy::too_many_lines)]
use crate::phone::{Gateway, read_request, tls_server};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::SigningKey;
use lab_world::{Error, Result};
use rustls::{ServerConnection, StreamOwned};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const CREDENTIAL: &str = "ab-test-relay-credential";

#[derive(Debug, Default)]
struct Seen {
    events: Vec<String>,
    valid: bool,
}

#[derive(Debug)]
pub struct LiveSink {
    seen: Arc<Mutex<Seen>>,
    packets: Mutex<mpsc::Receiver<Value>>,
    stop: Arc<AtomicBool>,
    address: String,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn live_valid(token: &str, aps: &Value) -> (String, bool) {
    let Some(event) = aps["event"].as_str() else {
        return (String::new(), false);
    };
    let expected = if event == "start" {
        "a".repeat(64)
    } else {
        "b".repeat(64)
    };
    let mut valid = token == expected && aps["content-state"].is_object();
    if event == "start" {
        valid &= aps["attributes-type"] == "AgentTaskAttributes";
    }
    (event.to_owned(), valid)
}

fn handle(
    request: &crate::phone::Request,
    seen: &Mutex<Seen>,
    packets: &mpsc::Sender<Value>,
) -> u16 {
    let (method, path, headers, body) = request;
    if method != "POST" || path != "/iphone/push" {
        return 404;
    }
    let bearer = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.trim_start_matches("Bearer ").to_owned());
    if bearer.as_deref() != Some(CREDENTIAL) {
        return 403;
    }
    let Ok(push) = serde_json::from_slice::<Value>(body) else {
        return 400;
    };
    let now = jiff::Timestamp::now().as_second();
    let expires = push["expires"].as_i64().unwrap_or(0);
    let field = |name: &str| {
        push["packet"][name]
            .as_str()
            .and_then(|t| STANDARD.decode(t).ok())
            .unwrap_or_default()
    };
    if expires <= now
        || expires > now + 600
        || field("ephemeral_key").len() != 32
        || field("signature").len() != 64
        || field("box").len() < 28
        || field("box").len() > 2800
    {
        return 400;
    }
    let token = push["token"].as_str().unwrap_or_default();
    let live = &push["packet"]["live_activity"];
    if live.is_object() {
        let (event, valid) = live_valid(token, live);
        let mut seen = lock(seen);
        seen.valid &= valid;
        seen.events.push(event);
        return 204;
    }
    let mut packet = push["packet"].clone();
    if let Some(map) = packet.as_object_mut() {
        map.remove("live_activity");
    }
    if packets.send(packet).is_err() {
        return 502;
    }
    204
}

impl LiveSink {
    pub fn start(gateway: &mut Gateway) -> Result<Self> {
        let (config, pem) = tls_server(&["127.0.0.1"], false)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let seen = Arc::new(Mutex::new(Seen {
            events: vec![],
            valid: true,
        }));
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        {
            let seen = seen.clone();
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
                    let seen = seen.clone();
                    let sender = sender.clone();
                    thread::spawn(move || {
                        let mut stream = StreamOwned::new(connection, stream);
                        let Some(request) = read_request(&mut stream) else {
                            return;
                        };
                        let status = handle(&request, &seen, &sender);
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
        let signing = SigningKey::from_bytes(&crate::phone::random::<32>()?);
        fs::OpenOptions::new()
            .append(true)
            .open(gateway.certificate_path())?
            .write_all(pem.as_bytes())?;
        fs::create_dir_all(gateway.home.join("remote"))?;
        let relay = json!({"key": STANDARD.encode(signing.to_keypair_bytes()), "endpoint": format!("https://{address}"), "credential": {"hostname": "example.test", "credential": CREDENTIAL}});
        fs::write(
            gateway.home.join("remote/relay.json"),
            serde_json::to_vec(&relay)?,
        )?;
        gateway.restart()?;
        Ok(Self {
            seen,
            packets: Mutex::new(receiver),
            stop,
            address,
        })
    }

    pub fn wait(&self, expected: &str) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            let (events, valid) = {
                let seen = lock(&self.seen);
                (seen.events.join(","), seen.valid)
            };
            if events == expected {
                thread::sleep(Duration::from_millis(150));
                let seen = lock(&self.seen);
                return seen.valid && seen.events.join(",") == expected;
            }
            if !valid {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
        false
    }

    pub fn detail(&self) -> String {
        let seen = lock(&self.seen);
        format!("events={:?} valid={}", seen.events, seen.valid)
    }

    pub fn next(&self) -> Result<Value> {
        lock(&self.packets)
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| invalid("push timeout"))
    }
}

impl Drop for LiveSink {
    fn drop(&mut self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            let _ = std::net::TcpStream::connect(&self.address);
        }
    }
}
