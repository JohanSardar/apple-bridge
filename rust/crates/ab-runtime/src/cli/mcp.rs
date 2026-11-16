use super::{Failure, Outcome, failure::Reason, gateway};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::watch,
};

const CALL_WAIT: Duration = Duration::from_secs(50);
const POLL: Duration = Duration::from_millis(400);
const FORWARD_LIMIT: Duration = Duration::from_mins(10);
const NOT_RUNNING: &str =
    "Apple Bridge isn't running on this Mac. Run `bridge up` in a terminal, then try again.";
const WAITING: &str = "Apple Bridge is waiting for you to click Allow on your Mac. Click Allow in the Apple Bridge panel, then try again.";
const DENIED: &str = "You chose Don't Allow on the Mac, so this agent can't use Apple Bridge. Restart the agent to ask again.";
const NO_PANEL: &str = "Apple Bridge couldn't show its Allow panel on this Mac. Run `bridge connectors list` in a terminal to allow this agent.";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    Unasked,
    Waiting,
    Allowed(String),
    Denied,
    Unreachable(&'static str),
}

struct Agent {
    label: String,
    key: String,
}

struct Bridge {
    home: PathBuf,
    http: reqwest::Client,
    address: Mutex<Option<String>>,
    agent: Mutex<Option<Agent>>,
    access: watch::Sender<Access>,
    previewed: Mutex<Option<Vec<Value>>>,
    out: tokio::sync::Mutex<tokio::io::Stdout>,
}

enum Reply {
    Forwarded(Option<Value>),
    Unauthorized,
    Unreachable,
    Problem(String),
}

pub async fn serve(home: &Path) -> Outcome {
    let http = reqwest::Client::builder()
        .timeout(FORWARD_LIMIT)
        .no_proxy()
        .build()
        .map_err(|error| {
            Failure::new(
                Reason::InternalError,
                "Couldn't start the local connection.",
            )
            .row("Detail", error.to_string())
        })?;
    let bridge = Arc::new(Bridge {
        home: home.to_owned(),
        http,
        address: Mutex::new(None),
        agent: Mutex::new(None),
        access: watch::Sender::new(Access::Unasked),
        previewed: Mutex::new(None),
        out: tokio::sync::Mutex::new(tokio::io::stdout()),
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut tasks = tokio::task::JoinSet::new();
    let mut introduced = false;
    while let Some(line) = lines.next_line().await? {
        while tasks.try_join_next().is_some() {}
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            bridge
                .send(&json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}))
                .await;
            continue;
        };
        if message.get("id").is_none_or(Value::is_null) || message.get("method").is_none() {
            continue;
        }
        if message["method"] == "initialize" {
            introduced = true;
            bridge.initialize(&message).await;
            continue;
        }
        if !introduced && message["method"] != "ping" {
            bridge
                .send(&json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"Method not found"}}))
                .await;
            continue;
        }
        let bridge = bridge.clone();
        tasks.spawn(async move { bridge.handle(message).await });
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    Ok(())
}

impl Bridge {
    async fn send(&self, message: &Value) {
        let mut out = self.out.lock().await;
        let mut line = message.to_string();
        line.push('\n');
        let _ = out.write_all(line.as_bytes()).await;
        let _ = out.flush().await;
    }

    async fn initialize(self: &Arc<Self>, message: &Value) {
        let client = &message["params"]["clientInfo"];
        let name = client["title"]
            .as_str()
            .or_else(|| client["name"].as_str())
            .unwrap_or("An agent");
        let agent = identify(name, &launcher());
        let cached = self.cached_token(&agent.key);
        if let Ok(mut slot) = self.agent.lock() {
            *slot = Some(agent);
        }
        if let Some(token) = cached {
            self.access.send_replace(Access::Allowed(token));
        } else {
            self.ask();
        }
        self.send(
            &json!({"jsonrpc":"2.0","id":message["id"],"result":ab_frontdoor::mcp_server_info()}),
        )
        .await;
    }

    async fn handle(self: Arc<Self>, message: Value) {
        let id = message["id"].clone();
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        let reply = match method.as_str() {
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/list" => self.list_tools(&message).await,
            "resources/list" | "resources/templates/list" | "prompts/list" => {
                self.list_other(&message, &method).await
            }
            _ => self.call(&message, &method).await,
        };
        self.send(&reply).await;
    }

    async fn list_tools(self: &Arc<Self>, message: &Value) -> Value {
        if let Access::Allowed(token) = self.current() {
            match self.forward(message, &token).await {
                Reply::Forwarded(Some(reply)) => return reply,
                Reply::Unauthorized => self.forget(),
                Reply::Unreachable | Reply::Forwarded(None) | Reply::Problem(_) => {}
            }
        }
        if self.current() == Access::Unasked {
            self.ask();
        }
        let tools = ab_frontdoor::mcp_tools();
        if let Ok(mut previewed) = self.previewed.lock() {
            *previewed = Some(tools.clone());
        }
        json!({"jsonrpc":"2.0","id":message["id"],"result":{"tools":tools}})
    }

    async fn list_other(&self, message: &Value, method: &str) -> Value {
        if let Access::Allowed(token) = self.current()
            && let Reply::Forwarded(Some(reply)) = self.forward(message, &token).await
        {
            return reply;
        }
        let field = match method {
            "resources/list" => "resources",
            "resources/templates/list" => "resourceTemplates",
            _ => "prompts",
        };
        json!({"jsonrpc":"2.0","id":message["id"],"result":{ field: [] }})
    }

    async fn call(self: &Arc<Self>, message: &Value, method: &str) -> Value {
        for _ in 0..2 {
            let token = match self.allowed().await {
                Ok(token) => token,
                Err(text) => return refusal(message, method, text),
            };
            match self.forward(message, &token).await {
                Reply::Forwarded(Some(reply)) => return reply,
                Reply::Forwarded(None) => {
                    return json!({"jsonrpc":"2.0","id":message["id"],"result":{}});
                }
                Reply::Unauthorized => self.forget(),
                Reply::Unreachable => return refusal(message, method, NOT_RUNNING),
                Reply::Problem(text) => return refusal(message, method, &text),
            }
        }
        refusal(message, method, WAITING)
    }

    fn current(&self) -> Access {
        self.access.borrow().clone()
    }

    async fn allowed(self: &Arc<Self>) -> Result<String, &'static str> {
        if matches!(self.current(), Access::Unasked | Access::Unreachable(_)) {
            self.ask();
        }
        let mut changes = self.access.subscribe();
        let settled = tokio::time::timeout(
            CALL_WAIT,
            changes.wait_for(|access| !matches!(access, Access::Waiting | Access::Unasked)),
        )
        .await;
        match settled {
            Err(_) => Err(WAITING),
            Ok(Err(_)) => Err(NOT_RUNNING),
            Ok(Ok(access)) => match access.clone() {
                Access::Allowed(token) => Ok(token),
                Access::Denied => Err(DENIED),
                Access::Unreachable(text) => Err(text),
                Access::Waiting | Access::Unasked => Err(WAITING),
            },
        }
    }

    fn ask(self: &Arc<Self>) {
        let mut started = false;
        self.access.send_if_modified(|access| {
            if matches!(access, Access::Unasked | Access::Unreachable(_)) {
                *access = Access::Waiting;
                started = true;
                return true;
            }
            false
        });
        if !started {
            return;
        }
        let bridge = self.clone();
        tokio::spawn(async move {
            let outcome = bridge.request_approval().await;
            if let Access::Allowed(token) = &outcome {
                bridge.remember(token);
            }
            let allowed = matches!(outcome, Access::Allowed(_));
            bridge.access.send_replace(outcome);
            if allowed {
                bridge.refresh_tools().await;
            }
        });
    }

    async fn request_approval(&self) -> Access {
        let label = self
            .agent
            .lock()
            .ok()
            .and_then(|agent| agent.as_ref().map(|agent| agent.label.clone()))
            .unwrap_or_else(|| "An agent".to_owned());
        if self.address().await.is_none() {
            return Access::Unreachable(NOT_RUNNING);
        }
        let Ok(ticket) = crate::connector::ask_for_agent(&self.home, &label).await else {
            return Access::Unreachable(NO_PANEL);
        };
        let (Some(id), Some(secret)) = (ticket["id"].as_str(), ticket["secret"].as_str()) else {
            return Access::Unreachable(NO_PANEL);
        };
        eprintln!("Apple Bridge: click Allow on your Mac to let {label} connect.");
        loop {
            tokio::time::sleep(POLL).await;
            let Ok(answer) = crate::connector::agent_answer(&self.home, id, secret).await else {
                return Access::Unasked;
            };
            match answer["state"].as_str() {
                Some("allowed") => {
                    return answer["token"]
                        .as_str()
                        .filter(|token| !token.is_empty())
                        .map_or(Access::Unreachable(NO_PANEL), |token| {
                            Access::Allowed(token.to_owned())
                        });
                }
                Some("denied") => return Access::Denied,
                _ => {}
            }
        }
    }

    async fn refresh_tools(&self) {
        let Some(previewed) = self.previewed.lock().ok().and_then(|p| p.clone()) else {
            return;
        };
        let Access::Allowed(token) = self.current() else {
            return;
        };
        let probe = json!({"jsonrpc":"2.0","id":"bridge-tools","method":"tools/list"});
        if let Reply::Forwarded(Some(reply)) = self.forward(&probe, &token).await
            && reply["result"]["tools"].as_array() != Some(&previewed)
        {
            self.send(&json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}))
                .await;
        }
    }

    async fn address(&self) -> Option<String> {
        if let Some(address) = self.address.lock().ok().and_then(|a| a.clone()) {
            return Some(address);
        }
        let address = gateway::running_address(&self.home).await.ok()?;
        if let Ok(mut slot) = self.address.lock() {
            *slot = Some(address.clone());
        }
        Some(address)
    }

    async fn forward(&self, message: &Value, token: &str) -> Reply {
        let Some(address) = self.address().await else {
            return Reply::Unreachable;
        };
        let sent = self
            .http
            .post(gateway::mcp_address(&address))
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(message.to_string())
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(error) => {
                if error.is_connect()
                    && let Ok(mut slot) = self.address.lock()
                {
                    *slot = None;
                }
                return Reply::Unreachable;
            }
        };
        let status = response.status();
        let streamed = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let Ok(body) = response.bytes().await else {
            return Reply::Unreachable;
        };
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Reply::Unauthorized;
        }
        if status == reqwest::StatusCode::ACCEPTED || body.is_empty() {
            return Reply::Forwarded(None);
        }
        let parsed = if streamed {
            String::from_utf8_lossy(&body)
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
                .find(|value| value.get("id") == message.get("id"))
        } else {
            serde_json::from_slice::<Value>(&body).ok()
        };
        match parsed {
            Some(reply) if reply.get("jsonrpc").is_some() => Reply::Forwarded(Some(reply)),
            Some(problem) => Reply::Problem(
                problem["error"]["message"]
                    .as_str()
                    .unwrap_or("Apple Bridge refused the request.")
                    .to_owned(),
            ),
            None => Reply::Problem(format!("Apple Bridge answered with HTTP {status}.")),
        }
    }

    fn store(&self) -> PathBuf {
        self.home.join("local-agents")
    }

    fn cached_token(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.store().join(key))
            .ok()
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty())
    }

    fn remember(&self, token: &str) {
        let Some(key) = self.key() else {
            return;
        };
        let store = self.store();
        if std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&store)
            .is_err()
        {
            return;
        }
        let temporary = store.join(format!(".{key}.{}", std::process::id()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .and_then(|mut file| std::io::Write::write_all(&mut file, token.as_bytes()));
        if written.is_ok() {
            let _ = std::fs::rename(&temporary, store.join(&key));
        } else {
            let _ = std::fs::remove_file(&temporary);
        }
    }

    fn forget(&self) {
        if let Some(key) = self.key() {
            let _ = std::fs::remove_file(self.store().join(key));
        }
        self.access.send_replace(Access::Unasked);
    }

    fn key(&self) -> Option<String> {
        self.agent
            .lock()
            .ok()
            .and_then(|agent| agent.as_ref().map(|agent| agent.key.clone()))
    }
}

fn refusal(message: &Value, method: &str, text: &str) -> Value {
    if method == "tools/call" {
        return json!({"jsonrpc":"2.0","id":message["id"],"result":{"content":[{"type":"text","text":text}],"isError":true}});
    }
    json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32001,"message":text}})
}

fn identify(client: &str, launcher: &Option<(String, String)>) -> Agent {
    let client: String = client
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned();
    let client = if client.is_empty() {
        "An agent".to_owned()
    } else {
        client
    };
    let label = match launcher {
        Some((_, app))
            if !app.to_lowercase().contains(&client.to_lowercase())
                && !client.to_lowercase().contains(&app.to_lowercase()) =>
        {
            format!("{client} in {app}")
        }
        _ => client.clone(),
    };
    let label: String = label.chars().take(60).collect();
    let bundle = launcher.as_ref().map_or("", |(bundle, _)| bundle.as_str());
    let digest = Sha256::digest(format!("{client}\0{bundle}").as_bytes());
    Agent {
        label,
        key: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
    }
}

fn launcher() -> Option<(String, String)> {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..12 {
        if pid <= 1 {
            return None;
        }
        let output = std::process::Command::new("/bin/ps")
            .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let (parent, command) = text.split_once(char::is_whitespace)?;
        if let Some(end) = command.find(".app/") {
            let bundle = &command[..end + 4];
            let app = Path::new(bundle)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())?;
            if app != "Apple Bridge" {
                return Some((bundle.to_owned(), app));
            }
        }
        pid = parent.trim().parse().ok()?;
    }
    None
}
