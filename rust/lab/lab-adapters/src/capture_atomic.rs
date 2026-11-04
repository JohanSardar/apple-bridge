use crate::{
    assertions::{Observed, await_until, converged},
    http::{Client, random_id},
    journey::{observed as command_observed, records},
    replicas,
    watch::Watcher,
};
use jiff::Timestamp;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Instant};

pub const SCENARIOS: [&str; 1] = ["CORE-REPLICA-CAPTURE-ATOMIC"];
pub const SCOPE: &str = "calendar:read,calendar:write,reminders:read,reminders:write,system:read";

pub fn start_world(world: &mut World, count: usize) -> Result<()> {
    world.install_proxy()?;
    world.control("/seed", &json!({"count": 0}))?;
    replicas::start_with(world, count, SCOPE, 0)
}

fn title_of(record: &Value) -> &str {
    record["data"]["title"].as_str().unwrap_or_default()
}
fn signature(phase: &str, client: &Client) -> String {
    let mut values: Vec<String> = records(client)
        .map(|(_, all)| all)
        .unwrap_or_default()
        .iter()
        .filter(|record| {
            let title = title_of(record);
            (phase == "generic"
                && record["collection"] == "reminders.reminder"
                && title.starts_with("[ab-test] seed"))
                || (phase == "generic-byte"
                    && record["collection"] == "reminders.reminder"
                    && title.starts_with("[ab-test atomic-byte]"))
                || (phase == "calendar"
                    && record["collection"] == "calendar.event"
                    && title.starts_with("[ab-test atomic]"))
        })
        .map(|record| {
            format!(
                "{}/{}",
                record["id"].as_str().unwrap_or_default(),
                record["data"]
            )
        })
        .collect();
    values.sort();
    values.join("\n")
}
fn sequence(client: &Client) -> Option<u64> {
    client.cursor().ok().map(|c| c.sequence)
}
fn calendar(world: &World, action: &str, key: &str, data: &Value) -> bool {
    world
        .control(
            "/calendar",
            &json!({"action": action, "key": key, "data": data}),
        )
        .is_ok()
}

pub fn capture_atomic(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if !SCENARIOS.contains(&evidence.scenario.as_str()) {
        return Err(Error::Invalid(format!(
            "capture_atomic not ported for {}",
            evidence.scenario
        )));
    }
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let Some(endpoint) = replicas::endpoints().first().cloned() else {
        return Err(Error::Invalid("no replica".into()));
    };
    let replica = endpoint.client();
    if !await_until(10, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "");
        return Ok(());
    }
    if !calendar(world, "calendar", "test-calendar", &Value::Null)
        || !calendar(
            world,
            "create",
            "atomic-series",
            &json!({"title": "[ab-test atomic] original", "start": "2026-11-01", "end": "2026-11-02", "all_day": true, "recurrence": [{"frequency": "daily", "interval": 1, "count": 3}]}),
        )
    {
        evidence.check("calendar_fixture", false, "");
        return Ok(());
    }
    if !await_until(10, || converged(world, &observed)) {
        evidence.check("calendar_convergence", false, "");
        return Ok(());
    }
    for phase in ["generic", "generic-byte", "calendar"] {
        if !await_until(10, || sequence(&gateway) == sequence(&replica)) {
            evidence.check(&format!("{phase}_caught_up"), false, "");
            return Ok(());
        }
        let replica_before = sequence(&replica).unwrap_or(0);
        world.partition()?;
        let goal = match phase {
            "generic-byte" => 1,
            "calendar" => 124,
            _ => 127,
        };
        let mut aligned = false;
        for n in 0..256 {
            let head = sequence(&gateway).unwrap_or(0);
            if head.wrapping_sub(replica_before) % 128 == goal {
                aligned = true;
                break;
            }
            let mut padding = json!({"key": format!("padding-{phase}-{n}")});
            if phase == "generic-byte" {
                padding["data"] =
                    json!({"title": "[ab-test] byte padding", "notes": "p".repeat(100_000)});
            }
            if world.control("/external-change", &padding).is_err()
                || world.control("/flush", &json!({})).is_err()
            {
                break;
            }
        }
        evidence.check(
            &format!("{phase}_capture_crosses_batch_boundary"),
            aligned,
            "",
        );
        if !aligned {
            return Ok(());
        }
        let initial = signature(phase, &replica);
        let before = sequence(&gateway).unwrap_or(0);
        let Ok(mut watcher) = Watcher::start(&replica, false) else {
            evidence.check(&format!("{phase}_watch"), false, "");
            return Ok(());
        };
        let mut failed = false;
        match phase {
            "generic" => failed = world.control("/seed", &json!({"count": 3})).is_err(),
            "generic-byte" => {
                let mut observations = vec![];
                for n in 0..3 {
                    let key = format!("atomic-byte-{n}");
                    let data = json!({"title": format!("[ab-test atomic-byte] {key}"), "notes": "b".repeat(60_000)});
                    if world
                        .control(
                            "/external-change",
                            &json!({"key": key, "data": data, "omit_capture": true}),
                        )
                        .is_err()
                    {
                        failed = true;
                        break;
                    }
                    observations.push(json!({"kind": "upsert", "collection": "reminders.reminder", "native_key": key, "data": data, "observed_at": Timestamp::now().to_string()}));
                }
                if !failed {
                    failed = world
                        .control(
                            "/capture",
                            &json!({"source": "reminders", "observations": observations}),
                        )
                        .is_err();
                }
            }
            _ => {
                let target = records(&gateway)
                    .map(|(_, all)| all)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|record| {
                        record["collection"] == "calendar.event"
                            && record["data"]["start"] == "2026-11-02"
                    })
                    .unwrap_or(Value::Null);
                let command = json!({
                    "command_id": random_id("cmd")?,
                    "expected_version": target["version"],
                    "wait_ms": 15000,
                    "args": {"id": target["id"], "span": "future", "start": "2026-11-04", "end": "2026-11-05"}
                });
                let outcome = gateway.call_raw("calendar.update_event", &command, 30)?;
                evidence.check(
                    "calendar_split_observed",
                    outcome.is_some_and(|(_, value)| command_observed(&value)),
                    "",
                );
            }
        }
        if failed || world.control("/flush", &json!({})).is_err() {
            watcher.close();
            evidence.check(&format!("{phase}_capture"), false, "");
            return Ok(());
        }
        let through = sequence(&gateway).unwrap_or(0);
        let finalized = signature(phase, &gateway);
        let mut reads_atomic = true;
        world.heal()?;
        let caught = await_until(10, || {
            let value = signature(phase, &replica);
            reads_atomic = reads_atomic && (value == initial || value == finalized);
            watcher.last().sequence >= through
        });
        let mut batches: BTreeSet<Instant> = BTreeSet::new();
        for (change, at) in watcher.timed_changes() {
            let collection = change["record"]["collection"].as_str().unwrap_or_default();
            if change["sequence"].as_u64().unwrap_or(0) <= before
                || collection == "commands.command"
            {
                continue;
            }
            let wanted = if phase == "calendar" {
                "calendar.event"
            } else {
                "reminders.reminder"
            };
            if collection == wanted {
                batches.insert(at);
            }
        }
        watcher.close();
        evidence.check(
            &format!("{phase}_capture_never_partial_in_replica_live_delta"),
            caught && batches.len() == 1,
            &format!(
                "capture_batches={} after={before} through={through} replica_before={replica_before}",
                batches.len()
            ),
        );
        evidence.check(
            &format!("{phase}_replica_reads_never_partial"),
            reads_atomic,
            "",
        );
        evidence.check(
            &format!("{phase}_replica_converges"),
            converged(world, &observed),
            "",
        );
    }
    crate::labcore::oversized_resets(world, evidence);
    Ok(())
}
