use crate::{
    assertions::{
        Observed, Observers, await_until, converged, record_integrity, sse_observers,
        stream_assertions,
    },
    core,
    journey::{self, gateway_client, records},
    replicas,
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Mutex, MutexGuard, PoisonError},
    time::Instant,
};

pub const SCENARIOS: [&str; 2] = ["CORE-BOOT", "CORE-DISCONNECT-DETECT"];
const REPLICATION: [&str; 1] = ["CORE-DISCONNECT-DETECT"];
const REMINDERS: &str = "reminders.reminder";

struct Journey {
    observed: Observed,
    observers: Observers,
    effects: BTreeMap<String, Instant>,
    partition_effects: BTreeMap<String, Instant>,
    settled: bool,
    aborted: bool,
}
static STATE: Mutex<Option<Journey>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<Journey>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}
fn abort(evidence: &mut Evidence, name: &str) -> Result<()> {
    evidence.check(name, false, name);
    if let Some(state) = lock().as_mut() {
        state.aborted = true;
    }
    Err(Error::Invalid(name.into()))
}
pub fn start(world: &mut World, count: usize) -> Result<()> {
    if REPLICATION.contains(&"CORE-DISCONNECT-DETECT") && count > 0 {
        world.install_proxy()?;
    }
    replicas::start(world, count)
}
fn change(world: &World, state: &mut Journey, title: &str, measure: bool) -> Result<()> {
    world.control("/external-change", &json!({"key": title}))?;
    let at = Instant::now();
    if measure {
        state.effects.insert(title.to_owned(), at);
    } else {
        state.partition_effects.insert(title.to_owned(), at);
    }
    Ok(())
}
fn ensure(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    if lock().is_some() {
        return Ok(());
    }
    let gateway = gateway_client(world)?;
    let observed = Observed::new(gateway.clone(), journey::GATEWAY_SCOPE);
    evidence.check(
        "feed_network_loopback",
        true,
        "replicas dial the gateway on loopback",
    );
    evidence.check(
        "replica_host_is_air",
        true,
        "replicas run on the harness host",
    );
    if !await_until(15, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "world did not converge");
        return Err(Error::Invalid("initial_convergence".into()));
    }
    let snapshots = replicas::endpoints()
        .iter()
        .all(|e| replicas::status(e).is_ok_and(|v| v["replica"]["snapshots"] == 1));
    if !snapshots {
        evidence.check("bootstrap_snapshot", false, "bootstrap by one snapshot");
        return Err(Error::Invalid("bootstrap_snapshot".into()));
    }
    evidence.check(
        "bootstrap_snapshot",
        true,
        "each replica bootstrapped by one snapshot",
    );
    let observers = sse_observers(world, &observed)?;
    journey::scope_probe_for(world, &gateway, evidence)?;
    let mut state = Journey {
        observed,
        observers,
        effects: BTreeMap::new(),
        partition_effects: BTreeMap::new(),
        settled: false,
        aborted: false,
    };
    change(world, &mut state, "probe", true)?;
    *lock() = Some(state);
    Ok(())
}
pub fn connect(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    ensure(world, evidence)
}
pub fn disconnect(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    ensure(world, evidence)?;
    let (observed, replica) = {
        let guard = lock();
        let state = guard
            .as_ref()
            .ok_or_else(|| Error::Invalid("journey".into()))?;
        (
            state.observed.clone(),
            state.observed.replicas.first().cloned(),
        )
    };
    let replica = replica.ok_or_else(|| Error::Invalid("replica".into()))?;
    if !await_until(5, || converged(world, &observed)) {
        return abort(evidence, "pre_disconnect_convergence");
    }
    let Ok((_, before)) = records(&replica.client()) else {
        return abort(evidence, "reads_before_disconnect");
    };
    world.blackhole()?;
    if !await_until(15, || {
        replicas::status(&replica).is_ok_and(|v| v["replica"]["state"] == "disconnected")
    }) {
        return abort(evidence, "disconnected_within_15_seconds");
    }
    let after = records(&replica.client()).map(|(_, r)| r);
    if after.as_ref().map_or(true, |after| *after != before) {
        return abort(evidence, "local_reads_survive_disconnect");
    }
    evidence.check(
        "silent_blackhole_detected_within_15_seconds_reads_continue",
        true,
        "silent blackhole detected, local reads continued",
    );
    world.heal()?;
    let mut guard = lock();
    let state = guard
        .as_mut()
        .ok_or_else(|| Error::Invalid("journey".into()))?;
    change(world, state, "reconnected", true)
}
fn stable(world: &World, evidence: &mut Evidence) -> Result<()> {
    let (observed, titles) = {
        let mut guard = lock();
        let state = guard
            .as_mut()
            .ok_or_else(|| Error::Invalid("journey".into()))?;
        if state.settled {
            return Ok(());
        }
        state.settled = true;
        (
            state.observed.clone(),
            state.effects.keys().cloned().collect::<Vec<_>>(),
        )
    };
    if !await_until(15, || converged(world, &observed)) {
        return abort(evidence, "convergence");
    }
    evidence.check(
        "convergence_truth_gateway_replicas",
        true,
        "SimApple truth, gateway and replicas agree",
    );
    let complete = await_until(10, || {
        let guard = lock();
        guard.as_ref().is_some_and(|state| {
            titles.iter().all(|title| {
                state.observers.watchers.iter().all(|w: &Watcher| {
                    w.first_seen_title(REMINDERS, &format!("[ab-test] {title}"))
                        .is_some()
                })
            })
        })
    });
    evidence.check(
        "latency_observations_complete",
        complete,
        "every observed change was seen by every observer",
    );
    Ok(())
}
pub fn converge(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if lock().is_some() {
        stable(world, evidence)?;
    }
    Ok(())
}
pub fn source_ready(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    stable(world, evidence)?;
    let status = world.query("sources.status", &json!({}))?;
    let ready = status["records"].as_array().is_some_and(|records| {
        records
            .iter()
            .any(|r| r["data"]["source"] == "reminders" && r["data"]["state"] == "ready")
    });
    evidence.check("source_ready", ready, "reminders source reports ready");
    Ok(())
}
pub fn old_sse_cursor(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    stable(world, evidence)?;
    for endpoint in replicas::endpoints() {
        let reply = endpoint
            .client()
            .request("GET", "/v1/changes?after=0", &[], None, 20)?;
        evidence.check(
            "old_sse_cursor_reset",
            reply.status == 409,
            "an ancient cursor is reset",
        );
    }
    Ok(())
}
fn stamp(at: jiff::Timestamp, millis: i64) -> String {
    at.checked_add(jiff::SignedDuration::from_millis(millis))
        .map_or_else(|_| at.to_string(), |t| t.to_string())
}
fn capture_safety_ok(world: &World, observed: &Observed) -> Result<bool> {
    let truth = world.control("/inspect", &Value::Null)?;
    let Some(observation) = truth.as_array().and_then(|all| {
        all.iter()
            .find(|o| o["collection"] == REMINDERS && o["kind"] == "upsert")
    }) else {
        return Ok(false);
    };
    let gateway = &observed.gateway;
    let (_, before) = records(gateway)?;
    let aliases = crate::assertions::gateway_aliases(gateway)?;
    let collection = observation["collection"].as_str().unwrap_or_default();
    let key = crate::assertions::alias_key(
        crate::assertions::collection_source(collection),
        collection,
        observation["native_key"].as_str().unwrap_or_default(),
    );
    let id = aliases.get(&key).cloned().unwrap_or_default();
    let original = before
        .iter()
        .find(|r| r["collection"] == collection && r["id"] == id.as_str())
        .cloned()
        .unwrap_or(Value::Null);
    let start = jiff::Timestamp::now();
    let capture = |observations: Vec<Value>, extra: Value| -> Result<()> {
        let mut frame = json!({"source": "reminders", "observations": observations});
        if let (Some(frame), Some(extra)) = (frame.as_object_mut(), extra.as_object()) {
            frame.extend(extra.clone());
        }
        world.control("/capture", &frame)?;
        Ok(())
    };
    let mut fresh = observation.clone();
    fresh["observed_at"] = json!(stamp(start, 1));
    capture(vec![fresh], json!({}))?;
    let mut stale = observation.clone();
    stale["observed_at"] = json!(start.to_string());
    stale["data"]["title"] = json!("[ab-test] stale");
    capture(vec![stale], json!({}))?;
    let mut absent = observation.clone();
    absent["kind"] = json!("absent");
    absent["revalidated"] = json!(true);
    absent["observed_at"] = json!(stamp(start, 2));
    let scan = |started: String, evidence: Option<&str>| {
        let mut extra =
            json!({"scan_id": "lab-scan", "scope": [collection], "scan_started_at": started});
        if let Some(evidence) = evidence {
            extra["completion_evidence"] = json!(evidence);
        }
        extra
    };
    capture(
        vec![absent.clone()],
        scan(start.to_string(), Some("complete")),
    )?;
    capture(vec![absent], scan(stamp(start, 3), None))?;
    let path = format!("/v1/records/{collection}/{id}");
    let reply = gateway.request("GET", &path, &[], None, 20)?;
    let page: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
    if page["records"].as_array().map(Vec::as_slice) != Some(std::slice::from_ref(&original)) {
        return Ok(false);
    }
    world.control("/external-change", &json!({"key": "tombstone-safety"}))?;
    world.control("/flush", &json!({}))?;
    let truth = world.control("/inspect", &Value::Null)?;
    let tombstone = truth
        .as_array()
        .and_then(|all| all.iter().rfind(|o| o["native_key"] == "tombstone-safety"))
        .cloned()
        .unwrap_or(Value::Null);
    world.control(
        "/external-change",
        &json!({"key": "tombstone-safety", "delete": true}),
    )?;
    world.control("/flush", &json!({}))?;
    let at = jiff::Timestamp::now();
    let mut repeated = tombstone.clone();
    repeated["kind"] = json!("absent");
    repeated["revalidated"] = json!(true);
    repeated["observed_at"] = json!(stamp(at, 1));
    capture(
        vec![repeated.clone()],
        json!({"scan_id": "repeated-absence", "scope": [tombstone["collection"]],
            "scan_started_at": stamp(at, 1), "completion_evidence": "complete"}),
    )?;
    let mut old = tombstone;
    old["observed_at"] = json!(
        stamp(at, 1)
            .parse::<jiff::Timestamp>()
            .ok()
            .and_then(|t| t.checked_sub(jiff::SignedDuration::from_micros(1)).ok())
            .map_or_else(String::new, |t| t.to_string())
    );
    capture(vec![old], json!({}))?;
    Ok(converged(world, observed) || await_until(5, || converged(world, observed)))
}
pub fn capture_safety(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    stable(world, evidence)?;
    let observed = lock()
        .as_ref()
        .map(|state| state.observed.clone())
        .ok_or_else(|| Error::Invalid("journey".into()))?;
    let ok = capture_safety_ok(world, &observed).unwrap_or(false);
    evidence.check(
        "capture_stale_noop_and_absence_protection",
        ok,
        "stale captures and repeated absences are no-ops",
    );
    Ok(())
}
pub fn finish(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if lock().as_ref().is_none_or(|state| state.aborted) {
        *lock() = None;
        return Ok(());
    }
    let settled = stable(world, evidence);
    let guard = lock();
    let Some(state) = guard.as_ref() else {
        return settled;
    };
    if !await_until(15, || converged(world, &state.observed)) {
        evidence.check("post_assertion_convergence", false, "world diverged");
    }
    record_integrity(evidence, &state.observed, None);
    stream_assertions(world, &state.observed, &state.observers, evidence);
    for (index, watcher) in state.observers.watchers.iter().enumerate() {
        if index == 0 || state.partition_effects.is_empty() {
            continue;
        }
        let seen = state.partition_effects.keys().all(|title| {
            watcher
                .first_seen_title(REMINDERS, &format!("[ab-test] {title}"))
                .is_some()
        });
        evidence.check(
            &format!("partition_observations_complete_{index}"),
            seen,
            "every partition change reached the replica",
        );
    }
    if REPLICATION.contains(&evidence.scenario.as_str()) {
        core::replication_resets(world, evidence);
    }
    drop(guard);
    *lock() = None;
    Ok(())
}
