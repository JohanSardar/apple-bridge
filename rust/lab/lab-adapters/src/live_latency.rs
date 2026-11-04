use crate::{
    assertions::await_until,
    http::{Client, random_id},
    journey::records,
    labcore::{self, COLLECTION, title, with_run},
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

fn snapshot(client: &Client, tag: &str) -> String {
    let Ok((_, all)) = records(client) else {
        return "read_failed".to_owned();
    };
    let mut identities: Vec<String> = all
        .iter()
        .filter(|r| {
            r["collection"] == "calendar.event"
                && r["data"]["title"]
                    .as_str()
                    .is_some_and(|t| t.starts_with(tag))
        })
        .map(|r| {
            let data = &r["data"];
            format!(
                "{}|{}|{}|{}",
                data["series_id"], data["occurrence_date"], data["start"], data["title"]
            )
        })
        .collect();
    identities.sort();
    identities.join("\n")
}
fn prefixed(watcher: &Watcher, tag: &str) -> Vec<Instant> {
    watcher
        .timed_changes()
        .into_iter()
        .filter(|(change, _)| {
            change["record"]["source"] == "calendar"
                && change["record"]["data"]["title"]
                    .as_str()
                    .is_some_and(|t| t.starts_with(tag))
        })
        .map(|(_, at)| at)
        .collect()
}
fn calendar_delta(
    world: &World,
    clients: &[Client],
    watchers: &[Watcher],
    run: &str,
) -> (bool, String) {
    let tag = format!("[ab-test {run}]");
    let before = snapshot(&clients[0], &tag);
    let ready = before != "read_failed"
        && await_until(5, || clients.iter().all(|c| snapshot(c, &tag) == before));
    let stop = AtomicBool::new(false);
    let mut created = false;
    let mut bytes = 0_u64;
    let mut observed_values: Vec<String> = vec![];
    thread::scope(|scope| {
        let poller = scope.spawn(|| {
            let mut seen = vec![];
            while ready && !stop.load(Ordering::SeqCst) {
                for client in clients {
                    seen.push(snapshot(client, &tag));
                }
                thread::sleep(Duration::from_millis(10));
            }
            seen
        });
        let writer = scope.spawn(|| {
            let result = world
                .control("/fault", &json!({"name": "calendar-part-delay"}))
                .and_then(|_| {
                    world.control(
                        "/calendar",
                        &json!({"action": "calendar", "key": "test-calendar"}),
                    )
                })
                .and_then(|_| {
                    world.control(
                        "/calendar",
                        &json!({"action": "create_many", "key": run, "count": 100, "data": {"title": tag, "notes": "x".repeat(1600), "start": "2026-11-01T10:00:00Z", "end": "2026-11-01T11:00:00Z"}}),
                    )
                });
            (result.is_ok(), result.map_or(0, |v| v["capture_bytes"].as_u64().unwrap_or(0)))
        });
        (created, bytes) = writer.join().unwrap_or((false, 0));
        let after = snapshot(&clients[0], &tag);
        let converged = await_until(5, || clients.iter().all(|c| snapshot(c, &tag) == after));
        stop.store(true, Ordering::SeqCst);
        observed_values = poller.join().unwrap_or_default();
        observed_values.push(if converged { after } else { "diverged".into() });
    });
    let _ = &observed_values;
    let after = observed_values.last().cloned().unwrap_or_default();
    let split_ok = ready
        && after != "diverged"
        && observed_values
            .iter()
            .all(|value| *value == before || *value == after);
    let in_range = created && (190_000..=240_000).contains(&bytes);
    let visible = await_until(5, || {
        watchers.iter().all(|w| prefixed(w, &tag).len() == 100)
    });
    let batches = watchers
        .iter()
        .all(|w| prefixed(w, &tag).into_iter().collect::<BTreeSet<_>>().len() == 1);
    (
        split_ok && in_range && visible && batches,
        format!(
            "ready={ready} split={split_ok} bytes={bytes} created={created} visible={visible} single_batch={batches}"
        ),
    )
}

pub fn changes(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if evidence.scenario != "CORE-LIVE-LATENCY" {
        return Err(Error::Invalid(format!(
            "changes not ported for {}",
            evidence.scenario
        )));
    }
    let count = input["count"].as_u64().unwrap_or(200).max(1);
    let rate = input["per_second"].as_u64().unwrap_or(5).max(1);
    let run_id = random_id("live")?;
    let clients: Vec<Client> = with_run(|run| Ok(run.observed.clients()))?;
    let mut calendar_ok: Option<(bool, String)> = None;
    with_run(|run| {
        run.effects.retain(|key| key != "probe");
        run.stamps.retain(|(key, _)| key != "probe");
        let mut next = Instant::now();
        let world_ref: &World = world;
        for i in 0..count {
            if i == count / 2 && calendar_ok.is_none() {
                let watchers = &run.observers.watchers;
                calendar_ok = Some(calendar_delta(world_ref, &clients, watchers, &run_id));
            }
            labcore::change(world_ref, run, &format!("live-{i:03}"), true)?;
            next += Duration::from_secs(1) / u32::try_from(rate).unwrap_or(1);
            if let Some(delay) = next.checked_duration_since(Instant::now()) {
                thread::sleep(delay);
            }
        }
        Ok(())
    })?;
    let (ok, detail) = calendar_ok.unwrap_or((false, "not run".to_owned()));
    evidence.check(
        "calendar_200kb_delta_atomic_during_reminders_live",
        ok,
        &detail,
    );
    Ok(())
}

fn stats(mut values: Vec<f64>) -> (usize, f64) {
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return (0, 0.0);
    }
    let index = (values.len() * 95).div_ceil(100) - 1;
    (values.len(), values[index])
}

pub fn latency(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if evidence.scenario != "CORE-LIVE-LATENCY" {
        return Err(Error::Invalid(format!(
            "latency not ported for {}",
            evidence.scenario
        )));
    }
    let limit = input["p95_ms"].as_f64().unwrap_or(1000.0);
    let samples = input["samples"].as_u64().unwrap_or(200);
    let rows: Vec<(String, usize, usize, f64)> = with_run(|run| {
        let mut rows = vec![];
        for (index, watcher) in run.observers.watchers.iter().enumerate() {
            let mut values = vec![];
            let mut timeouts = 0;
            for (key, effect) in &run.stamps {
                match watcher.first_seen_title(COLLECTION, &title(key)) {
                    Some(seen) => {
                        values.push(seen.saturating_duration_since(*effect).as_secs_f64() * 1000.0);
                    }
                    None => timeouts += 1,
                }
            }
            let name = if index == 0 {
                "effect_done→gateway_visible".to_owned()
            } else {
                format!("effect_done→replica_visible:replica{}", index - 1)
            };
            let (count, p95) = stats(values);
            rows.push((name, count, timeouts, p95));
        }
        Ok(rows)
    })?;
    for (name, count, timeouts, p95) in rows {
        evidence.check(
            &format!("latency_{name}"),
            u64::try_from(count).unwrap_or(0) >= samples && timeouts == 0 && p95 < limit,
            &format!("{count} samples, {timeouts} timeouts, p95 {p95:.1} ms"),
        );
    }
    labcore::finish(world, evidence)
}
