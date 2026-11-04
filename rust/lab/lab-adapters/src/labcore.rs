use crate::{
    assertions::{
        Observed, Observers, await_until, converged, read_all, record_integrity, sse_observers,
        stream_assertions,
    },
    http::{Client, random_id},
    journey::{self, GATEWAY_SCOPE},
    replicas,
    watch::Cursor,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

pub const JOURNEYS: [&str; 7] = [
    "CORE-RESUME",
    "LAB-SSE",
    "LAB-TRUTH-MULTIPLICITY",
    "CORE-BOOT",
    "CORE-DISCONNECT-DETECT",
    "CORE-LIVE-LATENCY",
    "CORE-GATEWAY-DIALS",
];
pub const CALENDAR_SCOPE: &str =
    "calendar:read,calendar:write,reminders:read,reminders:write,system:read";
pub const SCENARIOS: [&str; 9] = [
    "CORE-LIVE-LATENCY",
    "CORE-GATEWAY-DIALS",
    "CORE-BOOT",
    "CORE-DISCONNECT-DETECT",
    "CORE-RESUME",
    "LAB-SSE",
    "LAB-TRUTH-MULTIPLICITY",
    "CORE-STALE-FENCE",
    "LAB-MALFORMED-SCENARIO",
];
pub(crate) const COLLECTION: &str = "reminders.reminder";
pub(crate) fn scope_for(id: &str) -> &'static str {
    if id == "CORE-LIVE-LATENCY" {
        CALENDAR_SCOPE
    } else {
        GATEWAY_SCOPE
    }
}

pub(crate) struct Run {
    pub(crate) observed: Observed,
    pub(crate) observers: Observers,
    pub(crate) effects: Vec<String>,
    pub(crate) stamps: Vec<(String, Instant)>,
    pub(crate) partition_effects: Vec<String>,
    pub(crate) settled: bool,
}
impl Run {
    pub(crate) fn forget_effect(&mut self, key: &str) {
        self.effects.retain(|k| k != key);
        self.stamps.retain(|(k, _)| k != key);
    }
    pub(crate) fn latencies(&self) -> Vec<(String, Vec<f64>, usize)> {
        self.observers
            .watchers
            .iter()
            .enumerate()
            .map(|(index, watcher)| {
                let name = if index == 0 {
                    "effect_done→gateway_visible".to_owned()
                } else {
                    format!("effect_done→replica_visible:replica{}", index - 1)
                };
                let mut values = vec![];
                let mut timeouts = 0;
                for key in &self.effects {
                    match (
                        watcher.first_seen_title(COLLECTION, &title(key)),
                        self.stamps.iter().find(|(k, _)| k == key).map(|(_, at)| at),
                    ) {
                        (Some(at), Some(effect)) => {
                            values
                                .push(at.saturating_duration_since(*effect).as_secs_f64() * 1000.0);
                        }
                        _ => timeouts += 1,
                    }
                }
                (name, values, timeouts)
            })
            .collect()
    }
}
static RUN: Mutex<Option<Run>> = Mutex::new(None);
static STALE: Mutex<Option<Value>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(crate) fn title(key: &str) -> String {
    format!("[ab-test] {key}")
}
pub(crate) fn change(world: &World, run: &mut Run, key: &str, measured: bool) -> Result<()> {
    world.control("/external-change", &json!({"key": key}))?;
    if measured {
        run.effects.push(key.to_owned());
        run.stamps.push((key.to_owned(), Instant::now()));
    } else {
        run.partition_effects.push(key.to_owned());
    }
    Ok(())
}
pub fn start_world(world: &mut World, id: &str, replicas: usize) -> Result<()> {
    if id == "CORE-GATEWAY-DIALS" {
        return crate::core_dials::start(world, replicas);
    }
    if id == "CORE-RESUME" || id == "CORE-DISCONNECT-DETECT" {
        world.install_proxy()?;
    }
    replicas::start_with(world, replicas, scope_for(id), 0)
}
pub fn seed(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    journey::seed(world, input, evidence)?;
    if !JOURNEYS.contains(&evidence.scenario.as_str()) {
        return Ok(());
    }
    evidence.check(
        &format!(
            "feed_network_{}",
            crate::core_bulk::network(&evidence.scenario)
        ),
        true,
        "replicas dial the gateway on loopback",
    );
    evidence.check(
        "replica_host_is_air",
        true,
        "replicas run on the harness host",
    );
    let scope = scope_for(&evidence.scenario);
    let gateway = Client::new(world, scope)?;
    let observed = Observed::new(gateway.clone(), scope);
    if !await_until(15, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "world did not converge");
        return Err(Error::Invalid("initial convergence".into()));
    }
    let bootstrapped = observed
        .replicas
        .iter()
        .all(|e| replicas::status(e).is_ok_and(|v| v["replica"]["snapshots"] == 1));
    evidence.check(
        "bootstrap_snapshot",
        bootstrapped,
        "each replica bootstrapped by one snapshot",
    );
    if !bootstrapped {
        return Err(Error::Invalid("bootstrap snapshot".into()));
    }
    let observers = sse_observers(world, &observed)?;
    let scoped = journey::all_endpoints(world)?;
    let reminders = journey::record_set(&gateway, COLLECTION)?;
    journey::scope_probe(&scoped, &reminders, evidence)?;
    let mut run = Run {
        observed,
        observers,
        effects: vec![],
        stamps: vec![],
        partition_effects: vec![],
        settled: false,
    };
    change(world, &mut run, "probe", true)?;
    *lock(&RUN) = Some(run);
    Ok(())
}
pub(crate) fn with_run<T>(body: impl FnOnce(&mut Run) -> Result<T>) -> Result<T> {
    let mut guard = lock(&RUN);
    let run = guard
        .as_mut()
        .ok_or_else(|| Error::Invalid("scenario prologue did not run".into()))?;
    body(run)
}
fn status_of(endpoint: &replicas::Endpoint) -> Value {
    replicas::status(endpoint).map_or(Value::Null, |v| v["replica"].clone())
}
pub fn connect(_: &mut World, _: &Value, _: &mut Evidence) -> Result<()> {
    Ok(())
}
pub fn partition(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    with_run(|run| {
        let seen = await_until(5, || {
            run.observers
                .watchers
                .get(1)
                .is_some_and(|w| w.first_seen_title(COLLECTION, &title("probe")).is_some())
        });
        if !seen {
            evidence.check("pre_partition_visible", false, "probe not visible");
            return Err(Error::Invalid("pre_partition_visible".into()));
        }
        world.partition()?;
        let seconds = input["seconds"].as_u64().unwrap_or(30);
        let count = input["changes"].as_u64().unwrap_or(30).max(1);
        for i in 0..count {
            change(world, run, &format!("offline-{i:03}"), false)?;
            thread::sleep(Duration::from_millis(seconds * 1000 / count));
        }
        Ok(())
    })
}
pub fn heal(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    with_run(|run| {
        world.heal()?;
        let connected = await_until(15, || {
            run.observed
                .replicas
                .first()
                .is_some_and(|e| status_of(e)["state"] == "connected")
        });
        if !connected {
            evidence.check("reconnect", false, "replica did not reconnect");
            return Err(Error::Invalid("reconnect".into()));
        }
        change(world, run, "healed", true)?;
        if evidence.scenario == "CORE-GATEWAY-DIALS" {
            run.forget_effect("healed");
        }
        Ok(())
    })
}
fn count_titled(values: &[Value], wanted: &str) -> usize {
    values
        .iter()
        .filter(|v| v["data"]["title"] == wanted || v["title"] == wanted)
        .count()
}
pub fn truth_multiplicity(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let wanted = "[ab-test] duplicate";
    for key in ["duplicate-a", "duplicate-b"] {
        let sent = world.control(
            "/external-change",
            &json!({
                "key": key,
                "data": {"title": wanted, "completed": false},
                "omit_capture": key == "duplicate-b"
            }),
        );
        if sent.is_err() {
            evidence.check("multiplicity_fixture", false, "fixture change rejected");
            return Ok(());
        }
    }
    if world.control("/flush", &json!({})).is_err() {
        evidence.check("multiplicity_flush", false, "flush failed");
        return Ok(());
    }
    let truth = world.control("/inspect", &Value::Null);
    let gateway = with_run(|run| read_all(&run.observed.gateway, false, false))?;
    let truth_count = truth
        .as_ref()
        .ok()
        .and_then(Value::as_array)
        .map_or(0, |all| count_titled(all, wanted));
    let gateway_count = count_titled(&gateway.records, wanted);
    let fixture = truth.is_ok() && truth_count == 2 && gateway_count == 1;
    evidence.check(
        "multiplicity_fault_two_truth_one_gateway",
        fixture,
        &format!("truth={truth_count} gateway={gateway_count}"),
    );
    let diverged = with_run(|run| Ok(!converged(world, &run.observed)))?;
    evidence.check(
        "truth_multiplicity_mismatch_detected",
        fixture && diverged,
        "a duplicate in SimApple truth that the gateway collapsed fails convergence",
    );
    *lock(&RUN) = None;
    Ok(())
}
pub(crate) fn settle(world: &World, evidence: &mut Evidence) -> Result<()> {
    with_run(|run| {
        if run.settled {
            return Ok(());
        }
        run.settled = true;
        if !await_until(15, || converged(world, &run.observed)) {
            evidence.check("convergence", false, "world did not converge");
            return Err(Error::Invalid("convergence".into()));
        }
        evidence.check(
            "convergence_truth_gateway_replicas",
            true,
            "SimApple truth, gateway and replicas agree",
        );
        let complete = await_until(10, || {
            run.effects.iter().all(|key| {
                run.observers
                    .watchers
                    .iter()
                    .all(|w| w.first_seen_title(COLLECTION, &title(key)).is_some())
            })
        });
        evidence.check(
            "latency_observations_complete",
            complete,
            "every effect reached every endpoint",
        );
        Ok(())
    })
}
pub fn observer_resets(world: &World) -> usize {
    let homes = std::iter::once(world.gateway_home.clone())
        .chain(replicas::endpoints().into_iter().map(|e| e.home));
    homes
        .map(|home| {
            fs::read(home.join("lab-capture-observer-resets")).map_or(0, |raw| {
                raw.split(|b| *b == b'\n').count().saturating_sub(1)
            })
        })
        .sum()
}
pub fn oversized_resets(world: &World, evidence: &mut Evidence) {
    let resets = observer_resets(world);
    evidence.check(
        "no_oversized_capture_observer_resets",
        resets == 0,
        &format!("capture observer resets: {resets}"),
    );
}
pub fn finish_hook(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if lock(&RUN).is_none() {
        return Ok(());
    }
    let settled = settle(world, evidence);
    let finished = finish(world, evidence);
    *lock(&RUN) = None;
    settled.and(finished)
}
pub(crate) fn finish(world: &World, evidence: &mut Evidence) -> Result<()> {
    with_run(|run| {
        if !await_until(15, || converged(world, &run.observed)) {
            evidence.check(
                "post_assertion_convergence",
                false,
                "world did not converge",
            );
        }
        record_integrity(evidence, &run.observed, None);
        stream_assertions(world, &run.observed, &run.observers, evidence);
        if !run.partition_effects.is_empty() {
            for (index, watcher) in run.observers.watchers.iter().enumerate().skip(1) {
                let complete = run
                    .partition_effects
                    .iter()
                    .all(|key| watcher.first_seen_title(COLLECTION, &title(key)).is_some());
                evidence.check(
                    &format!("partition_observations_complete_{index}"),
                    complete,
                    "every offline change reached the replica",
                );
            }
        }
        Ok(())
    })?;
    *lock(&RUN) = None;
    if matches!(
        evidence.scenario.as_str(),
        "CORE-RESUME" | "CORE-DISCONNECT-DETECT" | "CORE-LIVE-LATENCY" | "CORE-GATEWAY-DIALS"
    ) {
        oversized_resets(world, evidence);
    }
    Ok(())
}
pub fn converge(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "LAB-TRUTH-MULTIPLICITY" => Ok(()),
        "CORE-RESUME"
        | "LAB-SSE"
        | "CORE-BOOT"
        | "CORE-DISCONNECT-DETECT"
        | "CORE-LIVE-LATENCY"
        | "CORE-GATEWAY-DIALS" => settle(world, evidence),
        _ => journey::converge(world, input, evidence),
    }
}
pub fn resumed(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    settle(world, evidence)?;
    let endpoints = with_run(|run| Ok(run.observed.replicas.clone()))?;
    for endpoint in &endpoints {
        let status = status_of(endpoint);
        evidence.check(
            "resume_without_snapshot",
            status["mode"] == "resume"
                && status["resumes"].as_u64().unwrap_or(0) >= 1
                && status["snapshots"] == 1,
            "the replica resumed from its cursor with its original snapshot",
        );
    }
    if evidence.scenario == "CORE-GATEWAY-DIALS" {
        return Ok(());
    }
    finish(world, evidence)
}
fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = vec![];
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.trim_end_matches('=').bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}
fn event_cursor(text: &str) -> Option<Cursor> {
    let raw = base64url_decode(text)?;
    let value: Value = serde_json::from_slice(&raw).ok()?;
    Some(Cursor::from_value(&value))
}
fn open(client: &Client, last_event_id: &str, seconds: u64) -> Result<std::process::Child> {
    Ok(Command::new("curl")
        .args(["--silent", "--no-buffer", "--include", "--max-time"])
        .arg(seconds.to_string())
        .args(["-H", &format!("Authorization: Bearer {}", client.token())])
        .args(["-H", &format!("Last-Event-ID: {last_event_id}")])
        .arg(format!("http://{}/v1/changes", client.address()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?)
}
fn http_status(line: &str) -> Option<u16> {
    line.strip_prefix("HTTP/")
        .and_then(|rest| rest.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
}
fn stale_rejected(client: &Client, cursor: &Cursor) -> bool {
    let stale = [
        Cursor {
            epoch: "prior-epoch".into(),
            revision: cursor.revision,
            sequence: cursor.sequence,
        },
        Cursor {
            epoch: cursor.epoch.clone(),
            revision: cursor.revision + 1,
            sequence: cursor.sequence,
        },
    ];
    stale.iter().all(|c| {
        client
            .request(
                "GET",
                "/v1/changes",
                &[("Last-Event-ID", &c.event_id())],
                None,
                5,
            )
            .is_ok_and(|reply| reply.status == 409)
    })
}
fn sse_resume(world: &World, client: &Client) -> bool {
    let Ok(cursor) = client.cursor() else {
        return false;
    };
    if !stale_rejected(client, &cursor) {
        return false;
    }
    let Ok(mut child) = open(client, &cursor.event_id(), 5) else {
        return false;
    };
    let Some(stdout) = child.stdout.take() else {
        return false;
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut status = 0;
    for line in lines.by_ref() {
        let Ok(line) = line else { break };
        if let Some(code) = http_status(&line) {
            status = code;
        }
        if line.is_empty() {
            break;
        }
    }
    let Ok(key) = random_id("sse") else {
        return false;
    };
    let ok = status == 200
        && world
            .control("/external-change", &json!({"key": key}))
            .is_ok()
        && {
            let wanted = title(&key);
            let mut id = None;
            let mut found = false;
            let mut after = cursor.sequence;
            for line in lines.map_while(std::result::Result::ok) {
                if let Some(text) = line.strip_prefix("id: ") {
                    id = event_cursor(text);
                }
                if let Some(data) = line.strip_prefix("data: ") {
                    let Ok(batch) = serde_json::from_str::<Value>(data) else {
                        break;
                    };
                    let expected = Cursor {
                        epoch: batch["epoch"].as_str().unwrap_or_default().to_owned(),
                        revision: batch["subscription_revision"].as_u64().unwrap_or(0),
                        sequence: batch["through"].as_u64().unwrap_or(0),
                    };
                    if id.as_ref() != Some(&expected) || batch["after"].as_u64() != Some(after) {
                        break;
                    }
                    after = expected.sequence;
                    if batch["changes"].as_array().is_some_and(|all| {
                        all.iter()
                            .any(|c| c["record"]["data"]["title"] == wanted.as_str())
                    }) {
                        found = true;
                        break;
                    }
                }
            }
            found
        };
    let _ = child.kill();
    let _ = child.wait();
    ok
}
fn sse_idle(world: &World, client: &Client) -> bool {
    let Ok(mut child) = open(client, "", 23) else {
        return false;
    };
    let Some(stdout) = child.stdout.take() else {
        return false;
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut status = 0;
    for line in lines.by_ref() {
        let Ok(line) = line else { break };
        if let Some(code) = http_status(&line) {
            status = code;
        }
        if line.is_empty() {
            break;
        }
    }
    let Ok(key) = random_id("idle") else {
        return false;
    };
    let mut ok = false;
    if status == 200
        && world
            .control("/external-change", &json!({"key": key}))
            .is_ok()
    {
        let start = Instant::now();
        let mut heartbeats = 0;
        for line in lines.map_while(std::result::Result::ok) {
            if line == ": heartbeat" {
                heartbeats += 1;
                if heartbeats >= 3 && start.elapsed() >= Duration::from_secs(14) {
                    ok = true;
                    break;
                }
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    ok
}
pub fn sse_resume_all(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    settle(world, evidence)?;
    let clients = with_run(|run| Ok(run.observed.clients()))?;
    for (index, client) in clients.iter().enumerate() {
        evidence.check(
            &format!("sse_full_cursor_resume_{index}"),
            sse_resume(world, client),
            "stale cursors reset, the exact cursor resumes with contiguous batches",
        );
    }
    Ok(())
}
pub fn sse_idle_all(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    settle(world, evidence)?;
    let clients = with_run(|run| Ok(run.observed.clients()))?;
    for (index, client) in clients.iter().enumerate() {
        evidence.check(
            &format!("sse_idle_heartbeats_{index}"),
            sse_idle(world, client),
            "an idle stream sends heartbeats for at least fourteen seconds",
        );
    }
    finish(world, evidence)
}
pub fn stale_frame(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    world.control(
        "/external-change",
        &json!({"key": "stale-fence-baseline", "collection": COLLECTION, "data": {"title": "[ab-test] fence baseline", "completed": false}}),
    )?;
    world.control("/flush", &json!({}))?;
    let outcome = world.control(
        "/stale-frame",
        &json!({"key": "stale-fence-record", "collection": COLLECTION, "data": {"title": "[ab-test] frame on superseded connection", "completed": false}}),
    );
    if outcome.is_err() {
        evidence.check("action", false, "stale frame control failed");
    }
    *lock(&STALE) = outcome.ok();
    Ok(())
}
pub fn stale_fence_rejected(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let outcome = lock(&STALE).take().unwrap_or(Value::Null);
    let collection = outcome["collection"].as_str().unwrap_or_default();
    let visible = world.query(
        "records.query",
        &json!({"collection": collection, "limit": 1000}),
    )?["records"]
        .as_array()
        .is_some_and(|records| records.iter().any(|r| r["data"] == outcome["data"]));
    let stale = outcome["stale_fence"].as_u64().unwrap_or(0);
    let current = outcome["current_fence"].as_u64().unwrap_or(0);
    let superseded = stale > 0 && stale < current;
    let reply = outcome["reply"].as_str().unwrap_or_default();
    evidence.check(
        "stale_fence_rejected",
        superseded
            && outcome["key"].as_str().is_some_and(|k| !k.is_empty())
            && reply != "committed"
            && !visible,
        &format!(
            "superseded={superseded} stale_reply={reply} visible_through_public_query={visible}"
        ),
    );
    Ok(())
}
fn expected_kind(action: &str, key: &str) -> Option<&'static str> {
    match (action, key) {
        ("world", "replicas")
        | ("seed", "count")
        | ("partition", "seconds" | "changes")
        | ("changes", "count" | "per_second")
        | ("latency", "p95_ms" | "samples")
        | ("blobs", "rows") => Some("integer"),
        ("world", "gateway_dials") | ("resumed", "without_snapshot") => Some("boolean"),
        ("load_rejected", "path") | ("robustness" | "blobs", "case") => Some("string"),
        ("robustness", "paths") => Some("array"),
        _ => None,
    }
}
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "mapping",
    }
}
fn validate(scenario: &Value) -> std::result::Result<(), String> {
    for field in ["id", "feature"] {
        if scenario[field].as_str().is_none_or(str::is_empty) {
            return Err("scenario identity missing".into());
        }
    }
    for phase in ["given", "when", "then", "cleanup"] {
        for (index, step) in scenario[phase].as_array().into_iter().flatten().enumerate() {
            let Some(step) = step.as_object().filter(|s| s.len() == 1) else {
                return Err(format!("{phase}[{index}]: expected exactly one action"));
            };
            for (action, options) in step {
                let Some(options) = options.as_object() else {
                    return Err(format!(
                        "{phase}[{index}].{action}: expected options mapping"
                    ));
                };
                for (key, value) in options {
                    if let Some(kind) = expected_kind(action, key) {
                        if kind_of(value) != kind {
                            return Err(format!(
                                "{phase}[{index}].{action}.{key}: expected {kind}"
                            ));
                        }
                        if value.as_i64().is_some_and(|n| {
                            n < 0
                                || (n == 0
                                    && kind == "integer"
                                    && !(phase == "given"
                                        && (action == "seed" || key == "replicas")))
                        }) {
                            return Err(format!(
                                "{phase}[{index}].{action}.{key}: expected positive integer"
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn load_rejected(_: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let path = input["path"]
        .as_str()
        .ok_or_else(|| Error::Invalid("load_rejected.path required".into()))?;
    let error = fs::read(Path::new(path))
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_saphyr::from_slice::<Value>(&bytes).map_err(|e| e.to_string()))
        .and_then(|value| validate(&value));
    evidence.check(
        "malformed_scenario_rejected",
        error.as_ref().is_err_and(|e| e.contains("p95_ms")),
        &error.err().unwrap_or_default(),
    );
    Ok(())
}
