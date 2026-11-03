use crate::http::Client;
use lab_world::{Error, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: String,
    pub revision: u64,
    pub sequence: u64,
}
impl Cursor {
    #[must_use]
    pub fn from_value(value: &Value) -> Self {
        Self {
            epoch: value["epoch"].as_str().unwrap_or_default().to_owned(),
            revision: value["subscription_revision"].as_u64().unwrap_or(0),
            sequence: value["sequence"].as_u64().unwrap_or(0),
        }
    }
    fn event_id(&self) -> String {
        let raw = json!({
            "epoch": self.epoch,
            "subscription_revision": self.revision,
            "sequence": self.sequence
        })
        .to_string();
        base64url(raw.as_bytes())
    }
}
fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(TABLE[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

#[derive(Debug)]
struct State {
    last: Cursor,
    initial_sequence: u64,
    invalid: bool,
    filtered: bool,
    connected: bool,
    resets: usize,
    changes: Vec<(Value, Instant)>,
}
#[derive(Debug)]
pub struct Watcher {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    thread: Option<JoinHandle<()>>,
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn full_snapshot(client: &Client) -> Result<(Cursor, BTreeMap<String, Value>)> {
    let mut records = BTreeMap::new();
    let mut after = String::new();
    let mut cursor: Option<Cursor> = None;
    loop {
        let page = client
            .call(
                "records.query",
                &json!({"collection":"","limit":128,"after":after,"include_deleted":true}),
                20,
            )?
            .ok_or_else(|| Error::Invalid("snapshot rejected".into()))?;
        let current = Cursor::from_value(&page["cursor"]);
        match &cursor {
            None => cursor = Some(current),
            Some(first) if *first != current => {
                return Err(Error::Invalid("snapshot cursor moved".into()));
            }
            Some(_) => {}
        }
        for record in page["records"].as_array().into_iter().flatten() {
            records.insert(record_key(record), record.clone());
        }
        match page["next"].as_str() {
            Some(next) if !next.is_empty() => next.clone_into(&mut after),
            _ => break,
        }
    }
    Ok((cursor.unwrap_or(Cursor::from_value(&Value::Null)), records))
}
#[must_use]
pub fn record_key(record: &Value) -> String {
    format!(
        "{}/{}",
        record["collection"].as_str().unwrap_or_default(),
        record["id"].as_str().unwrap_or_default()
    )
}
fn reset_snapshot(client: &Client, state: &Mutex<State>) {
    let Ok((cursor, records)) = full_snapshot(client) else {
        return;
    };
    let mut state = lock(state);
    if cursor == state.last {
        state.invalid = true;
        return;
    }
    let at = Instant::now();
    for record in records.into_values() {
        if state.filtered && record["source"] != "system" {
            state.invalid = true;
        }
        let op = if record["deleted"] == true {
            "delete"
        } else {
            "upsert"
        };
        let change = json!({"sequence": cursor.sequence, "op": op, "record": record});
        state.changes.push((change, at));
    }
    state.resets += 1;
    state.initial_sequence = cursor.sequence;
    state.last = cursor;
}
fn apply_batch(state: &Mutex<State>, batch: &Value) {
    let at = Instant::now();
    let mut state = lock(state);
    let after = batch["after"].as_u64().unwrap_or(0);
    let through = batch["through"].as_u64().unwrap_or(0);
    if batch["partial"] == true
        || batch["epoch"].as_str().unwrap_or_default() != state.last.epoch
        || batch["subscription_revision"].as_u64().unwrap_or(0) != state.last.revision
        || after != state.last.sequence
        || through < after
    {
        state.invalid = true;
    }
    let mut previous = after;
    for change in batch["changes"].as_array().into_iter().flatten() {
        let sequence = change["sequence"].as_u64().unwrap_or(0);
        if sequence <= previous || sequence > through {
            state.invalid = true;
        }
        previous = sequence;
        if state.filtered && change["record"]["source"] != "system" {
            state.invalid = true;
        }
        state.changes.push((change.clone(), at));
    }
    state.last.sequence = through;
}
fn stream(
    client: &Client,
    state: &Mutex<State>,
    child_slot: &Mutex<Option<Child>>,
    stop: &AtomicBool,
) {
    let after = lock(state).last.clone();
    let mut command = Command::new("curl");
    command
        .args(["--silent", "--no-buffer", "--include", "--max-time", "600"])
        .args(["-H", &format!("Authorization: Bearer {}", client.token())])
        .arg(format!(
            "http://{}/v1/changes?after={}",
            client.address(),
            after.event_id()
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    *lock(child_slot) = Some(child);
    let mut reset = false;
    let mut status = 0u16;
    for line in BufReader::new(stdout).lines() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(line) = line else { break };
        if line.starts_with("HTTP/") {
            status = line
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse().ok())
                .unwrap_or(0);
            if status == 200 {
                lock(state).connected = true;
            }
            continue;
        }
        if line == "event: reset" {
            reset = true;
            continue;
        }
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        lock(state).connected = true;
        if reset {
            reset_snapshot(client, state);
            break;
        }
        if let Ok(batch) = serde_json::from_str::<Value>(data) {
            apply_batch(state, &batch);
        } else {
            lock(state).invalid = true;
            break;
        }
    }
    if let Some(mut child) = lock(child_slot).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if status == 409 {
        reset_snapshot(client, state);
    }
}
impl Watcher {
    pub fn start(client: &Client, filtered: bool) -> Result<Self> {
        let page = client
            .call("records.query", &json!({"collection":"","limit":1}), 20)?
            .ok_or_else(|| Error::Invalid("watch cursor rejected".into()))?;
        let cursor = Cursor::from_value(&page["cursor"]);
        let state = Arc::new(Mutex::new(State {
            initial_sequence: cursor.sequence,
            last: cursor,
            invalid: false,
            filtered,
            connected: false,
            resets: 0,
            changes: vec![],
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let worker = {
            let (client, state, stop, child) =
                (client.clone(), state.clone(), stop.clone(), child.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    stream(&client, &state, &child, &stop);
                    thread::sleep(Duration::from_millis(100));
                }
            })
        };
        let watcher = Self {
            state,
            stop,
            child,
            thread: Some(worker),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !lock(&watcher.state).connected {
            if Instant::now() >= deadline {
                return Err(Error::Invalid("sse observer did not connect".into()));
            }
            thread::sleep(Duration::from_millis(25));
        }
        Ok(watcher)
    }
    #[must_use]
    pub fn valid(&self) -> bool {
        !lock(&self.state).invalid
    }
    #[must_use]
    pub fn advanced(&self) -> bool {
        let state = lock(&self.state);
        state.last.sequence > state.initial_sequence
    }
    #[must_use]
    pub fn last(&self) -> Cursor {
        lock(&self.state).last.clone()
    }
    #[must_use]
    pub fn resets(&self) -> usize {
        lock(&self.state).resets
    }
    #[must_use]
    pub fn deletes(&self) -> Vec<Value> {
        lock(&self.state)
            .changes
            .iter()
            .filter(|(change, _)| change["op"] == "delete")
            .map(|(change, _)| change["record"].clone())
            .collect()
    }
    #[must_use]
    pub fn first_seen_title(&self, collection: &str, title: &str) -> Option<Instant> {
        lock(&self.state)
            .changes
            .iter()
            .find(|(change, _)| {
                change["op"] != "delete"
                    && change["record"]["collection"] == collection
                    && change["record"]["data"]["title"] == title
            })
            .map(|(_, at)| *at)
    }
    #[must_use]
    pub fn first_seen_sequence(&self, collection: &str, title: &str) -> Option<u64> {
        lock(&self.state)
            .changes
            .iter()
            .find(|(change, _)| {
                change["op"] != "delete"
                    && change["record"]["collection"] == collection
                    && change["record"]["data"]["title"] == title
            })
            .and_then(|(change, _)| change["sequence"].as_u64())
    }
    #[must_use]
    pub fn first_seen_record(&self, matches: impl Fn(&Value) -> bool) -> Option<Instant> {
        lock(&self.state)
            .changes
            .iter()
            .find(|(change, _)| change["op"] != "delete" && matches(&change["record"]))
            .map(|(_, at)| *at)
    }
    #[must_use]
    pub fn upserts_of(&self, collection: &str) -> (usize, Option<Instant>) {
        let state = lock(&self.state);
        let mut last: Option<Instant> = None;
        let mut count = 0;
        for (change, at) in &state.changes {
            if change["op"] != "delete" && change["record"]["collection"] == collection {
                count += 1;
                last = Some(last.map_or(*at, |seen| seen.max(*at)));
            }
        }
        (count, last)
    }
    #[must_use]
    pub fn data_for_title(&self, collection: &str, title: &str) -> Vec<Value> {
        lock(&self.state)
            .changes
            .iter()
            .filter(|(change, _)| {
                change["op"] != "delete"
                    && change["record"]["collection"] == collection
                    && change["record"]["data"]["title"] == title
            })
            .map(|(change, _)| change["record"]["data"].clone())
            .collect()
    }
    pub fn close(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(mut child) = lock(&self.child).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.close();
    }
}
