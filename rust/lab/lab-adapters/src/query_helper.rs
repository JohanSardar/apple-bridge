use crate::{
    assertions::await_until,
    http::{Client, random_id},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::time::Instant;

const SCOPE: &str =
    "messages:read,reminders:read,calendar:read,contacts:read,notes:read,system:read";
const BUDGET_MS: f64 = 1000.0;

fn version(client: &Client) -> Result<(Value, Value, f64)> {
    let q = json!({"id": random_id("qry")?, "args": {}, "deadline_ms": 1000});
    let start = Instant::now();
    let out = client
        .call("helper.version", &q, 20)?
        .unwrap_or(Value::Null);
    Ok((q, out, start.elapsed().as_secs_f64() * 1000.0))
}

pub fn real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, SCOPE)?;
    let ready = await_until(15, || {
        version(&client)
            .is_ok_and(|(q, out, _)| out["id"] == q["id"] && !out["result_json"].is_null())
    });
    evidence.check(
        "helper_query_ready",
        ready,
        "helper.version answered within fifteen seconds",
    );
    if !ready {
        return Ok(());
    }
    let mut samples = vec![];
    let (mut results_ok, mut values_ok) = (true, true);
    for _ in 0..10 {
        let (q, out, elapsed) = version(&client)?;
        samples.push(elapsed);
        results_ok &= out["id"] == q["id"] && !out["result_json"].is_null();
        let result = &out["result_json"];
        values_ok &= result["version"].as_str().is_some_and(|v| !v.is_empty())
            && result["helper_build"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
            && result["protocol"] == "ipc/1";
    }
    evidence.check(
        "ten_query_results",
        results_ok && samples.len() == 10,
        "ten helper.version results echo their query ids",
    );
    evidence.check(
        "helper_build_identified",
        values_ok,
        "version, helper build and protocol are reported",
    );
    let max = samples.iter().copied().fold(0.0, f64::max);
    evidence.check(
        "latency_budget",
        max < BUDGET_MS,
        &format!("max {max:.1} ms"),
    );
    Ok(())
}
