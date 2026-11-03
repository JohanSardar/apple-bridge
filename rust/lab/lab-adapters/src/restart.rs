use crate::{
    assertions::{Observed, await_until, converged},
    http::{Client, random_id},
    journey::{GATEWAY_SCOPE, effect_count, is_error, observed},
    replicas,
};
use jiff::{SignedDuration, Timestamp};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    thread,
    time::{Duration, Instant},
};

fn create(list_id: &str, title: &str) -> Result<Value> {
    Ok(json!({
        "command_id": random_id("cmd")?,
        "op": "reminders.create",
        "args": {"list_id": list_id, "title": format!("[ab-test] {title}")},
        "wait_ms": 15000
    }))
}
fn expiring(mut command: Value, after: SignedDuration) -> Value {
    let at = Timestamp::now()
        .checked_add(after)
        .unwrap_or(Timestamp::now());
    command["expires_at"] = json!(at.to_string());
    command
}
fn id(command: &Value) -> String {
    command["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
fn submit(client: &Client, command: &Value) -> Option<Value> {
    client
        .call_raw("reminders.create", command, 30)
        .ok()
        .flatten()
        .map(|(_, value)| value)
}
fn status(client: &Client, command: &Value, resolve: bool) -> Option<Value> {
    client
        .call_raw(
            "commands.status",
            &json!({"command_id": id(command), "resolve": resolve}),
            30,
        )
        .ok()
        .flatten()
        .map(|(_, value)| value)
}
fn disposition(value: Option<&Value>, expected: &str) -> bool {
    value.is_some_and(|v| v["error"].is_null() && v["disposition"] == expected)
}
fn fault(world: &World, name: &str, milliseconds: u64) -> bool {
    world
        .control("/fault", &json!({"name": name, "ms": milliseconds}))
        .is_ok()
}
fn count(world: &World, command: &Value, expected: u64) -> bool {
    effect_count(world, &id(command)) == Some(expected)
}
fn settled(world: &World, gateway: &Client) -> bool {
    let observed = Observed::new(gateway.clone(), GATEWAY_SCOPE);
    await_until(10, || converged(world, &observed))
}
fn crash_ambiguous(
    world: &mut World,
    gateway: &Client,
    list_id: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let c = create(list_id, "crash")?;
    evidence.check(
        "crash_fault_armed",
        fault(world, "crash-after-effect", 0),
        "crash-after-effect armed",
    );
    let first = submit(gateway, &c);
    evidence.check(
        "crash_returns_indeterminate",
        disposition(first.as_ref(), "indeterminate"),
        "a helper crash after the effect is indeterminate",
    );
    evidence.check(
        "helper_restarted",
        world.restart_peer().is_ok(),
        "helper process replaced",
    );
    evidence.check(
        "recovered_capture",
        settled(world, gateway),
        "truth and gateway converge after the restart",
    );
    let retry = submit(gateway, &c);
    evidence.check(
        "retry_stays_indeterminate",
        disposition(retry.as_ref(), "indeterminate"),
        "retry does not strengthen the result",
    );
    let resolved = status(gateway, &c, true);
    evidence.check(
        "resolve_strengthens",
        resolved.as_ref().is_some_and(observed),
        "resolve finds the observed record",
    );
    evidence.check(
        "resolve_never_redispatches",
        count(world, &c, 1),
        "one native effect",
    );
    let c = create(list_id, "ambiguous")?;
    fault(world, "ambiguous-outcome", 0);
    let first = submit(gateway, &c);
    evidence.check(
        "ambiguous_truthful",
        disposition(first.as_ref(), "indeterminate"),
        "ambiguous outcome reported as indeterminate",
    );
    let resolved = status(gateway, &c, true);
    evidence.check(
        "ambiguous_resolves",
        resolved.as_ref().is_some_and(observed) && count(world, &c, 1),
        "resolve observes the single effect",
    );
    Ok(())
}
fn expiry(
    world: &mut World,
    gateway: &Client,
    list_id: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let c = expiring(create(list_id, "expiry")?, SignedDuration::from_millis(150));
    fault(world, "pause", 400);
    let first = submit(gateway, &c);
    evidence.check(
        "paused_command_expires",
        disposition(first.as_ref(), "expired"),
        "a paused command expires",
    );
    evidence.check("expired_no_effect", count(world, &c, 0), "no native effect");
    thread::sleep(Duration::from_millis(450));
    let retry = submit(gateway, &c);
    evidence.check(
        "expired_retry_never_dispatches",
        disposition(retry.as_ref(), "expired") && count(world, &c, 0),
        "retry stays expired with no effect",
    );
    let mut c = expiring(
        create(list_id, "restart-expiry")?,
        SignedDuration::from_millis(200),
    );
    c["wait_ms"] = json!(0);
    fault(world, "pause", 500);
    let admitted = submit(gateway, &c);
    let early = admitted.as_ref().is_some_and(|v| {
        v["error"].is_null() && (v["state"] == "admitted" || v["state"] == "prepared")
    });
    evidence.check(
        "zero_wait_returns_admitted",
        early,
        &format!(
            "zero wait returns before dispatch: {}",
            admitted.map_or_else(|| "no reply".into(), |v| v.to_string())
        ),
    );
    world.stop_gateway();
    thread::sleep(Duration::from_millis(300));
    evidence.check(
        "restart_gateway",
        world.restart_gateway().is_ok(),
        "gateway restarted",
    );
    let checked = status(gateway, &c, false);
    evidence.check(
        "restart_rechecks_expiry",
        disposition(checked.as_ref(), "expired") && count(world, &c, 0),
        "expiry rechecked after restart",
    );
    Ok(())
}
fn clock_jump(
    world: &World,
    gateway: &Client,
    list_id: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    for offset in [3_600_000_i64, -3_600_000] {
        world.clock_offset(0)?;
        let c = expiring(
            create(list_id, &format!("clock-{offset}"))?,
            SignedDuration::from_millis(600),
        );
        fault(world, "pause", 200);
        let worker = {
            let (client, command) = (gateway.clone(), c.clone());
            thread::spawn(move || submit(&client, &command))
        };
        thread::sleep(Duration::from_millis(100));
        world.clock_offset(offset)?;
        let result = worker.join().ok().flatten();
        evidence.check(
            &format!("wall_jump_{offset}_preserves_monotonic_deadline"),
            result.as_ref().is_some_and(observed) && count(world, &c, 1),
            "a wall clock jump does not move the monotonic deadline",
        );
        world.clock_offset(0)?;
    }
    for ttl in [-3_600_000_i64, 7_200_000] {
        let c = expiring(create(list_id, "skew")?, SignedDuration::from_millis(ttl));
        let reply = submit(gateway, &c);
        evidence.check(
            "skewed_expiry_rejected",
            reply.is_some_and(|v| is_error(&v, "expired")) && count(world, &c, 0),
            "an expiry outside the allowed skew is rejected",
        );
    }
    Ok(())
}
fn offline(
    world: &mut World,
    gateway: &Client,
    list_id: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let endpoints = replicas::endpoints();
    let c = create(list_id, "helper-offline")?;
    world.stop_helper();
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    let reply = submit(gateway, &c);
    evidence.check(
        "helper_unavailable_fast",
        reply.is_some_and(|v| is_error(&v, "unavailable"))
            && started.elapsed() < Duration::from_secs(1),
        "helper loss answers unavailable within one second",
    );
    evidence.check(
        "helper_restarted",
        world.restart_peer().is_ok(),
        "helper process replaced",
    );
    world.stop_gateway();
    thread::sleep(Duration::from_millis(200));
    let remote = endpoints.first().map(|endpoint| {
        replicas::cli(
            &endpoint.home,
            &["token", "create", "--scope", GATEWAY_SCOPE],
        )
        .map(|token| Client::at(&endpoint.address, &token))
    });
    let started = Instant::now();
    let reply = match remote {
        Some(Ok(client)) => submit(&client, &c),
        _ => None,
    };
    evidence.check(
        "replica_unavailable_fast",
        reply.is_some_and(|v| is_error(&v, "unavailable"))
            && started.elapsed() < Duration::from_secs(2),
        "a replica answers unavailable while the gateway is down",
    );
    evidence.check(
        "gateway_restarted",
        world.restart_gateway().is_ok(),
        "gateway restarted",
    );
    evidence.check(
        "no_offline_effect",
        settled(world, gateway) && count(world, &c, 0),
        "no command executed while offline",
    );
    Ok(())
}
pub fn run(
    scenario: &str,
    world: &mut World,
    gateway: &Client,
    list_id: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    match scenario {
        "CORE-CMD-CRASH-AMBIGUOUS" => crash_ambiguous(world, gateway, list_id, evidence),
        "CORE-CMD-EXPIRY" => expiry(world, gateway, list_id, evidence),
        "CORE-CMD-CLOCK-JUMP" => clock_jump(world, gateway, list_id, evidence),
        "CORE-CMD-OFFLINE" => offline(world, gateway, list_id, evidence),
        _ => Err(lab_world::Error::Invalid(format!("restart case not ported: {scenario}"))),
    }
}
