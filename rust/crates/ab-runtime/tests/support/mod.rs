#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, Output, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

pub type Check = Result<(), Box<dyn std::error::Error>>;
pub type Log = Arc<Mutex<Vec<(String, Value)>>>;
pub type Reply = (u16, Vec<(&'static str, String)>, Value);

pub fn bridge() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bridge"));
    command
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .env_remove("APPLE_BRIDGE_HOME")
        .env_remove("APPLE_BRIDGE_LEGACY_JSON")
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .stdin(Stdio::null());
    for variable in [
        "AI_AGENT",
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CODEX_SANDBOX",
        "CODEX_THREAD_ID",
        "CURSOR_AGENT",
        "GEMINI_CLI",
        "CLINE_ACTIVE",
        "OPENCODE",
        "AMP_AGENT",
    ] {
        command.env_remove(variable);
    }
    command
}

pub struct Terminal {
    host: Child,
    output: BufReader<ChildStdout>,
    slave: File,
}

impl Terminal {
    pub fn open() -> Result<Self, Box<dyn std::error::Error>> {
        let mut host = Command::new("python3")
            .args([
                "-c",
                "import os,pty,select,sys
master,slave=pty.openpty()
print(os.ttyname(slave),flush=True)
while True:
 ready=select.select([master,0],[],[])[0]
 if master in ready:
  sys.stdout.buffer.write(os.read(master,65536));sys.stdout.buffer.flush()
 elif 0 in ready:
  break",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let mut output = BufReader::new(host.stdout.take().ok_or("terminal output")?);
        let mut path = String::new();
        output.read_line(&mut path)?;
        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.trim())?;
        Ok(Self {
            host,
            output,
            slave,
        })
    }

    pub fn attach(&self, command: &mut Command) -> std::io::Result<()> {
        command
            .env_remove("SSH_CONNECTION")
            .env_remove("SSH_TTY")
            .stdin(self.slave.try_clone()?)
            .stderr(self.slave.try_clone()?);
        Ok(())
    }

    pub fn read(mut self) -> std::io::Result<String> {
        drop(self.host.stdin.take());
        let mut text = String::new();
        self.output.read_to_string(&mut text)?;
        self.host.wait()?;
        Ok(text)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.host.try_wait().ok().flatten().is_none() {
            let _ = self.host.kill();
            let _ = self.host.wait();
        }
    }
}

pub fn run(home: &Path, arguments: &[&str]) -> Result<Output, std::io::Error> {
    bridge().args(arguments).arg("--home").arg(home).output()
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn error_document(output: &Output) -> Result<Value, Box<dyn std::error::Error>> {
    let stderr = text(&output.stderr);
    let line = stderr
        .lines()
        .rev()
        .find(|line| line.starts_with('{'))
        .ok_or_else(|| format!("no JSON error on stderr: {stderr}"))?;
    Ok(serde_json::from_str(line)?)
}

pub fn short_home() -> Result<tempfile::TempDir, std::io::Error> {
    tempfile::tempdir_in("/tmp")
}

pub fn seeded_home(home: &Path) -> Check {
    drop(ab_state::State::<ab_state::Gateway>::open(home)?);
    Ok(())
}

pub fn gateway_home(home: &Path) -> Check {
    seeded_home(home)?;
    let path = home.join("runtime.json");
    std::fs::write(
        &path,
        json!({"role":"gateway","home":home,"peer_address":"","api_address":"","socket":""})
            .to_string(),
    )?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn record(collection: &str, id: &str) -> Value {
    json!({"collection": collection, "id": id, "data": {"title": format!("item {id}"), "date": "2026-11-01T00:00:00Z"}})
}

#[allow(clippy::too_many_lines)]
pub fn scripted_gateway(
    home: &Path,
    handler: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static,
) -> Result<(std::fs::File, Log), Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let log: Log = Arc::default();
    let seen = log.clone();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let seen = seen.clone();
            let handler = handler.clone();
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 8192];
                let mut body_start = None;
                while body_start.is_none() {
                    let Ok(read) = stream.read(&mut buffer) else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    body_start = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|at| at + 4);
                }
                let start = body_start.unwrap_or(request.len());
                let head = String::from_utf8_lossy(&request[..start]).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while request.len() < start + length {
                    let Ok(read) = stream.read(&mut buffer) else {
                        return;
                    };
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let respond = |stream: &mut std::net::TcpStream,
                               status: u16,
                               headers: &[(&str, String)],
                               body: &str| {
                    let mut extra = String::new();
                    for (name, value) in headers {
                        extra.push_str(name);
                        extra.push_str(": ");
                        extra.push_str(value);
                        extra.push_str("\r\n");
                    }
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                };
                if head.starts_with("GET /v1/health") {
                    respond(&mut stream, 200, &[], "{}");
                    return;
                }
                let operation = head
                    .split_whitespace()
                    .nth(1)
                    .and_then(|path| path.strip_prefix("/v1/op/"))
                    .unwrap_or_default()
                    .to_owned();
                let body: Value = serde_json::from_slice(&request[start..]).unwrap_or(Value::Null);
                if let Ok(mut log) = seen.lock() {
                    log.push((operation.clone(), body.clone()));
                }
                let (status, headers, reply) = handler(&operation, &body);
                if status == 0 {
                    return;
                }
                let body = match &reply {
                    Value::String(raw) if raw.starts_with("raw:") => raw[4..].to_owned(),
                    other => other.to_string(),
                };
                respond(&mut stream, status, &headers, &body);
            });
        }
    });
    std::fs::write(
        home.join("runtime.json"),
        json!({"role": "gateway", "home": home, "peer_address": "127.0.0.1:1", "api_address": address, "socket": ""}).to_string(),
    )?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join("lock"))?;
    lock.try_lock()?;
    Ok((lock, log))
}

pub fn requests(log: &Log, operation: &str) -> Vec<Value> {
    log.lock()
        .map(|log| {
            log.iter()
                .filter(|(name, _)| name == operation)
                .map(|(_, body)| body.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub fn ok(body: Value) -> Reply {
    (200, Vec::new(), body)
}

pub fn search_pages(page_size: usize, pages: usize) -> impl Fn(&str, &Value) -> Reply {
    move |operation, body| {
        if operation != "search" {
            return ok(json!({}));
        }
        let page: usize = body["after"]
            .as_str()
            .and_then(|token| token.strip_prefix('p'))
            .and_then(|number| number.parse().ok())
            .unwrap_or(0);
        let asked = body["limit"]
            .as_u64()
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(10);
        let records: Vec<Value> = (0..asked.min(page_size))
            .map(|index| record("notes.note", &format!("{page}-{index}")))
            .collect();
        let next = (page + 1 < pages).then(|| format!("p{}", page + 1));
        ok(json!({"records": records, "cursor": null, "next": next}))
    }
}

pub fn fake_program(directory: &Path, name: &str, script: &str) -> Result<PathBuf, std::io::Error> {
    let path = directory.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

pub fn with_path(directory: &Path) -> String {
    format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

pub fn answer_remote_requests(home: PathBuf, link: &'static str, url: &'static str) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut answered = String::new();
        for _ in 0..600 {
            std::thread::sleep(Duration::from_millis(20));
            let Ok(config) = ab_remote::state::read::<ab_remote::state::Config>(&home, "config")
            else {
                continue;
            };
            if config.id.is_empty() || config.id == answered {
                continue;
            }
            std::thread::sleep(Duration::from_millis(100));
            let _ = runtime.block_on(ab_remote::onboarding::send_link(&home, &config.id, link));
            std::thread::sleep(Duration::from_millis(200));
            let status = json!({"id": config.id, "state": "on", "url": url, "last_error": ""});
            let _ = std::fs::write(home.join("remote/status.json"), status.to_string());
            answered = config.id;
        }
    });
}
