use crate::{
    assertions::{Observed, await_until, converged},
    journey::records,
    labcore::{self, change, with_run},
    replicas,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};

const REMINDERS: &str = "reminders.reminder";

pub fn disconnect(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let observed = with_run(|run| Ok(run.observed.clone()))?;
    let replica = observed
        .replicas
        .first()
        .cloned()
        .ok_or_else(|| Error::Invalid("replica".into()))?;
    if !await_until(5, || converged(world, &observed)) {
        evidence.check("pre_disconnect_convergence", false, "world not converged");
        return Err(Error::Invalid("pre_disconnect_convergence".into()));
    }
    let Ok((_, before)) = records(&replica.client()) else {
        evidence.check("reads_before_disconnect", false, "read failed");
        return Err(Error::Invalid("reads_before_disconnect".into()));
    };
    world.blackhole()?;
    if !await_until(15, || {
        replicas::status(&replica).is_ok_and(|v| v["replica"]["state"] == "disconnected")
    }) {
        evidence.check(
            "disconnected_within_15_seconds",
            false,
            "blackhole not detected",
        );
        return Err(Error::Invalid("disconnected_within_15_seconds".into()));
    }
    let after = records(&replica.client()).map(|(_, r)| r);
    if after.as_ref().map_or(true, |after| *after != before) {
        evidence.check("local_reads_survive_disconnect", false, "reads changed");
        return Err(Error::Invalid("local_reads_survive_disconnect".into()));
    }
    evidence.check(
        "silent_blackhole_detected_within_15_seconds_reads_continue",
        true,
        "silent blackhole detected, local reads continued",
    );
    world.heal()?;
    with_run(|run| change(world, run, "reconnected", true))
}
pub fn source_ready(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    labcore::settle(world, evidence)?;
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
    labcore::settle(world, evidence)?;
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
    labcore::settle(world, evidence)?;
    let observed = with_run(|run| Ok(run.observed.clone()))?;
    let ok = capture_safety_ok(world, &observed).unwrap_or(false);
    evidence.check(
        "capture_stale_noop_and_absence_protection",
        ok,
        "stale captures and repeated absences are no-ops",
    );
    Ok(())
}
