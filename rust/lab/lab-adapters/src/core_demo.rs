use crate::{
    assertions::{Observed, await_until, converged, false_deletion_free},
    http::Client,
    journey,
    labcore::CALENDAR_SCOPE,
    replicas,
    robust::automatic_healthy,
    watch::Watcher,
};
use lab_demo::{Options, generate};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const SCENARIOS: [&str; 2] = ["CORE-MESSAGES-SHAPE", "CORE-READMODEL-UPDATE-PERF"];

fn doctor(home: &std::path::Path) -> Option<Value> {
    replicas::cli(home, &["doctor"])
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}
fn tail(world: &World, observed: &Observed, observers: &[Watcher], evidence: &mut Evidence) {
    let drained = world.control("/flush", &json!({})).is_ok();
    evidence.check(
        "recovery_capture_drained_before_convergence",
        drained,
        "final capture drain",
    );
    if !drained {
        return;
    }
    if !await_until(30, || converged(world, observed)) {
        evidence.check("truth_gateway_replica_convergence", false, "not converged");
        return;
    }
    evidence.check(
        "truth_gateway_replica_convergence",
        true,
        "truth, gateway and replicas agree",
    );
    if !automatic_healthy(world, observed, evidence) {
        evidence.check("automatic_sync_guarantees", false, "healthy probe failed");
        return;
    }
    let mut homes = vec![("gateway".to_owned(), world.gateway_home.clone())];
    homes.extend(
        replicas::endpoints()
            .into_iter()
            .map(|e| ("replica".to_owned(), e.home)),
    );
    for (role, home) in homes {
        let valid = doctor(&home).is_some_and(|r| r["ok"] == true && r["exactly_once"] == true);
        evidence.check(
            &format!("sqlite_integrity_{role}"),
            valid,
            "doctor reports ok and exactly once",
        );
    }
    for observer in observers {
        evidence.check(
            "no_false_deletion_in_observed_journal_changes",
            false_deletion_free(world, observed, observer),
            "every observed delete is absent in SimApple truth",
        );
    }
}
fn message_count(world: &World) -> u64 {
    doctor(&world.gateway_home).map_or(0, |report| {
        report["records"]["messages.message"].as_u64().unwrap_or(0)
    })
}
fn messages_shape(world: &mut World, evidence: &mut Evidence) -> bool {
    let seeded = world
        .control("/messages", &json!({"action": "seed", "count": 32}))
        .is_ok()
        && world
            .control(
                "/messages",
                &json!({"action": "incoming", "key": format!("shape{}", std::process::id())}),
            )
            .is_ok();
    if !seeded {
        evidence.check("sim_messages_seeded", false, "seed rejected");
        return false;
    }
    let loaded = crate::demo::load(
        world,
        &generate(Options {
            seed: 1,
            huge: false,
        }),
        600,
    )
    .is_ok();
    if !loaded {
        evidence.check("demo_dataset_loaded", false, "dataset rejected");
        return false;
    }
    if !await_until(30, || message_count(world) >= 5000) {
        evidence.check("demo_dataset_captured", false, "messages not captured");
        return false;
    }
    let Some(report) = doctor(&world.gateway_home) else {
        evidence.check("doctor_reports_messages_shape", false, "doctor failed");
        return false;
    };
    let Some(shape) = report["shape"].as_object() else {
        evidence.check("doctor_reports_messages_shape", false, "no shape");
        return false;
    };
    let drift: BTreeMap<_, _> = shape
        .iter()
        .filter(|(key, _)| !key.ends_with(".title: unknown_field"))
        .collect();
    evidence.check(
        "sim_messages_cover_the_helper_shape",
        drift.is_empty(),
        &format!(
            "{} drifting shape rows: {:?}",
            drift.len(),
            drift.keys().take(20).collect::<Vec<_>>()
        ),
    );
    true
}
pub fn run(world: &mut World, case: &str, evidence: &mut Evidence) -> Result<()> {
    if case == "bench" {
        return crate::readmodel_perf::run(world, evidence);
    }
    journey::seed(world, &json!({"count": 4}), evidence)?;
    replicas::start_with(world, 1, CALENDAR_SCOPE, 0)?;
    let gateway = Client::new(world, CALENDAR_SCOPE)?;
    let observed = Observed::new(gateway, CALENDAR_SCOPE);
    evidence.check(
        "feed_network_loopback",
        true,
        "replicas dial the gateway on loopback",
    );
    if !await_until(15, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "world did not converge");
        return Ok(());
    }
    let mut observers = vec![];
    for client in observed.clients() {
        observers.push(Watcher::start(&client, false)?);
    }
    if !messages_shape(world, evidence) {
        return Ok(());
    }
    tail(world, &observed, &observers, evidence);
    Ok(())
}

pub fn matches(case: &str, scenario: &str) -> bool {
    matches!(
        (case, scenario),
        ("messages-shape", "CORE-MESSAGES-SHAPE") | ("bench", "CORE-READMODEL-UPDATE-PERF")
    )
}
