use crate::{
    assertions::{Observed, await_until, converged, read_all},
    http::Client,
    replicas,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const SCOPE: &str = "messages:read,system:read";
const MESSAGES: &str = "messages";

fn status(world: &World) -> Option<(u64, bool)> {
    let value = world.control("/messages-status", &Value::Null).ok()?;
    Some((
        value["capture_count"].as_u64()?,
        value["bulk_blocked"].as_bool()?,
    ))
}
fn background(world: &World, route: &str, input: &Value) -> Result<Child> {
    let child = Command::new("curl")
        .args(["--silent", "--fail", "--max-time", "120", "--unix-socket"])
        .arg(world.directory.join("sim.sock"))
        .args(["-H", "Content-Type: application/json", "--data"])
        .arg(serde_json::to_string(input)?)
        .arg(format!("http://sim{route}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(child)
}
fn file(world: &World, name: &str) -> Option<Value> {
    serde_json::from_slice(&fs::read(world.directory.join(name)).ok()?).ok()
}
fn gateway_checkpoint(world: &World) -> Option<String> {
    let database = Connection::open_with_flags(
        world.gateway_home.join("state.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    database.busy_timeout(Duration::from_secs(5)).ok()?;
    database
        .query_row(
            "SELECT checkpoint FROM checkpoints WHERE source='messages'",
            [],
            |row| row.get(0),
        )
        .ok()
}
fn flag(progress: &Value, field: &str) -> bool {
    progress[field][MESSAGES].as_bool().unwrap_or(false)
}
fn number(value: &Value, field: &str) -> u64 {
    value[field][MESSAGES].as_u64().unwrap_or(0)
}
fn wait_exit(child: &mut Child, seconds: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if child.try_wait().is_ok_and(|status| status.is_some()) {
            return true;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn run(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    replicas::start_with(world, 1, SCOPE, 0)?;
    let gateway = Client::new(world, SCOPE)?;
    let observed = Observed::new(gateway.clone(), SCOPE);
    let flush = |world: &World| world.control_for("/flush", &json!({}), 30).is_ok();
    let initial = flush(world);
    evidence.check("initial_captures_durable", initial, "");
    if !initial {
        return Ok(());
    }
    let Some((before, _)) = status(world) else {
        evidence.check("initial_capture_count", false, "");
        return Ok(());
    };
    if world
        .control(
            "/messages",
            &json!({"action": "bulk-pause", "key": "rm-message-000128"}),
        )
        .is_err()
    {
        evidence.check("pause_middle_capture_chunk", false, "");
        return Ok(());
    }
    let mut seed = background(
        world,
        "/readmodels",
        &json!({"action": "seed", "count": 512}),
    )?;
    let paused = await_until(30, || {
        status(world).is_some_and(|(count, blocked)| blocked && count >= before + 2)
    });
    evidence.check("multiple_chunks_acknowledged_before_pause", paused, "");
    if !paused {
        let _ = seed.kill();
        return Ok(());
    }
    let checkpoint = gateway_checkpoint(world);
    let persisted = file(world, "rust-sim-progress.json");
    let desired = file(world, "rust-sim-state.json");
    let marker = match (&persisted, &desired) {
        (Some(persisted), Some(desired)) => {
            flag(persisted, "in_progress")
                && number(persisted, "committed") < number(desired, "revision")
        }
        _ => false,
    };
    evidence.check(
        "durable_start_marker_retains_unfinished_revision",
        marker && checkpoint.is_some(),
        "",
    );
    let unforced = match (&persisted, &checkpoint) {
        (Some(persisted), Some(gateway)) => {
            !gateway.is_empty()
                && persisted["checkpoints"][MESSAGES].as_str() != Some(gateway.as_str())
        }
        _ => false,
    };
    evidence.check(
        "intermediate_acks_do_not_force_progress_fsync",
        unforced,
        "",
    );
    restart_and_verify(world, evidence, &gateway, &observed, desired.as_ref(), seed);
    Ok(())
}

fn restart_and_verify(
    world: &mut World,
    evidence: &mut Evidence,
    gateway: &Client,
    observed: &Observed,
    desired: Option<&Value>,
    mut seed: Child,
) {
    let flush = |world: &World| world.control_for("/flush", &json!({}), 30).is_ok();
    let restarted = world.restart_peer().is_ok();
    evidence.check("helper_restarts_from_durable_start", restarted, "");
    if !restarted {
        let _ = seed.kill();
        return;
    }
    let finished = wait_exit(&mut seed, 5);
    if !finished {
        evidence.check("interrupted_request_finishes", false, "");
        return;
    }
    let replayed = await_until(30, || flush(world));
    evidence.check("replayed_capture_finishes", replayed, "");
    if !replayed {
        return;
    }
    evidence.check(
        "gateway_and_replica_equal_source_truth",
        await_until(15, || converged(world, observed)),
        "",
    );
    let records = read_all(gateway, false, false);
    let (mut messages, mut chats, mut versions) = (0, 0, records.is_ok());
    for record in records.iter().flat_map(|read| &read.records) {
        match record["collection"].as_str() {
            Some("messages.message") => messages += 1,
            Some("messages.chat") => chats += 1,
            _ => {}
        }
        if record["collection"] != "system.source" && record["version"] != 1 {
            versions = false;
        }
    }
    evidence.check(
        "all_512_messages_and_500_chats_retained",
        messages == 512 && chats == 500,
        &format!("{messages} messages and {chats} chats"),
    );
    evidence.check("replay_does_not_duplicate_record_versions", versions, "");
    let checkpoint = gateway_checkpoint(world);
    let persisted = file(world, "rust-sim-progress.json");
    let durable = match (&persisted, desired, &checkpoint) {
        (Some(persisted), Some(desired), Some(gateway)) => {
            !flag(persisted, "in_progress")
                && persisted["checkpoints"][MESSAGES].as_str() == Some(gateway.as_str())
                && number(persisted, "committed") >= number(desired, "revision")
        }
        _ => false,
    };
    evidence.check("final_checkpoint_and_revision_are_durable", durable, "");
}
