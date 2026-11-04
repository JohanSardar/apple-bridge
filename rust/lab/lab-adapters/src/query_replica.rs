use crate::{
    http::{Client, random_id},
    labcore::CALENDAR_SCOPE,
    queries::{cancellation, echo_checks},
    replicas,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub fn start_world(world: &World, count: usize) -> Result<()> {
    replicas::start_with(world, count, CALENDAR_SCOPE, 0)
}

fn long(client: &Client) -> Result<(bool, Duration)> {
    let q = json!({"id": random_id("qry")?, "args": {"ms": 25000}, "deadline_ms": 30000});
    let start = Instant::now();
    let out = client.call("test.sleep", &q, 40)?;
    let elapsed = start.elapsed();
    Ok((
        out.is_some_and(|o| o["id"] == q["id"])
            && elapsed > Duration::from_secs(20)
            && elapsed < Duration::from_secs(30),
        elapsed,
    ))
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let endpoint = replicas::endpoints()
        .first()
        .cloned()
        .ok_or_else(|| Error::Invalid("no replica".into()))?;
    let client = endpoint.client();
    echo_checks(&client, evidence)?;
    let (ok, elapsed) = long(&client)?;
    evidence.check(
        "replica_work_exceeds_twenty_seconds",
        ok,
        &format!("{} ms", elapsed.as_millis()),
    );
    let feed = replicas::spawn_dialed(world, CALENDAR_SCOPE)
        .and_then(|_| replicas::wait_connected(15))
        .ok()
        .and_then(|()| replicas::endpoints().get(1).cloned());
    evidence.check(
        "query_feed_world_started",
        feed.is_some(),
        "a replica dialed by the gateway is connected",
    );
    if let Some(feed) = feed {
        let (ok, elapsed) = long(&feed.client())?;
        evidence.check(
            "feed_work_exceeds_twenty_seconds",
            ok,
            &format!("{} ms", elapsed.as_millis()),
        );
    }
    let q = json!({"id": random_id("qry")?, "args": {"ms": 5000}, "deadline_ms": 100});
    let start = Instant::now();
    let out = client.call_raw("test.sleep", &q, 20)?;
    evidence.check(
        "replica_deadline_bounded",
        out.as_ref()
            .is_some_and(|(_, v): &(u16, Value)| v["error"]["code"] == "deadline")
            && start.elapsed() < Duration::from_millis(200),
        "deadline error within 200 ms",
    );
    cancellation(world, &client, false, "", evidence)?;
    world.stop_gateway();
    let q = json!({"id": random_id("qry")?, "args": {"value": "lab"}, "deadline_ms": 1000});
    let start = Instant::now();
    let out = client.call_raw("test.echo", &q, 20)?;
    evidence.check(
        "offline_unavailable",
        out.is_some_and(|(_, v)| v["error"]["code"] == "unavailable")
            && start.elapsed() < Duration::from_secs(1),
        "a replica answers unavailable while the gateway is down",
    );
    Ok(())
}
