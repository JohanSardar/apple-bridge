use crate::{
    http::{Client, random_id},
    vision,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

const SCOPE: &str =
    "messages:read,reminders:read,calendar:read,contacts:read,notes:read,system:read";
const BUDGET_MS: f64 = 200.0;

fn query(args: &Value, deadline: i64) -> Result<Value> {
    Ok(json!({"id": random_id("qry")?, "args": args, "deadline_ms": deadline}))
}
fn roundtrip(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, SCOPE)?;
    let args = json!({"value":"lab"});
    let mut samples = vec![];
    let mut results_ok = true;
    let mut values_ok = true;
    for _ in 0..10 {
        let q = query(&args, 1000)?;
        let start = Instant::now();
        let out = client.call("test.echo", &q, 20)?;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        results_ok &= out.as_ref().is_some_and(|o| o["id"] == q["id"]);
        values_ok &= out.as_ref().is_some_and(|o| o["result_json"] == args);
    }
    evidence.check(
        "ten_query_results",
        results_ok && samples.len() == 10,
        "ten REST results echo their query ids",
    );
    evidence.check(
        "echo_equal",
        values_ok,
        "result_json equals the query arguments",
    );
    let max = samples.iter().copied().fold(0.0, f64::max);
    evidence.check(
        "latency_budget",
        max < BUDGET_MS,
        &format!("max {max:.1} ms"),
    );
    let q = query(&args, 1000)?;
    let rest = client.call("test.echo", &q, 20)?;
    let mcp = client.call_mcp("test.echo", &q, 20)?;
    evidence.check(
        "mcp_parity",
        rest.as_ref()
            .zip(mcp.as_ref())
            .is_some_and(|(a, b)| a["result_json"] == b["result_json"]),
        "REST and MCP result_json agree",
    );
    let audit = fs::read_to_string(world.gateway_home.join("commands-audit.jsonl")).ok();
    let mut found = false;
    let mut audit_ok = audit.is_some();
    for row in audit.iter().flat_map(|text| text.lines()) {
        let Ok(row) = serde_json::from_str::<Value>(row) else {
            continue;
        };
        if row["operation"] == "test.echo" {
            found = true;
            audit_ok &= row.as_object().is_some_and(|m| m.len() == 4)
                && !row["argument_bytes"].is_null()
                && !row["result_bytes"].is_null()
                && !row["blob_bytes"].is_null();
        }
    }
    evidence.check(
        "audit_names_and_sizes_only",
        audit_ok && found,
        "audit rows carry names and sizes only",
    );
    let invalid = client.call("test.echo", &query(&args, -1)?, 20)?;
    evidence.check(
        "invalid_rejected",
        invalid.is_none(),
        "negative deadline is rejected",
    );
    Ok(())
}
fn blob(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, SCOPE)?;
    let q = query(&json!({"size":5_242_880}), 10_000)?;
    let out = client.call("test.blob", &q, 20)?;
    let blobs = out
        .as_ref()
        .and_then(|o| o["blobs"].as_array())
        .filter(|b| b.len() == 1);
    evidence.check("blob_result", blobs.is_some(), "query returned one blob");
    let Some(digest) = blobs.and_then(|b| b[0]["digest"].as_str()) else {
        return Ok(());
    };
    let id = q["id"]
        .as_str()
        .ok_or_else(|| Error::Invalid("query id".into()))?;
    let path = format!("/v1/blobs/{digest}?record={id}");
    let full = client.request("GET", &path, &[], None, 30)?;
    evidence.check(
        "digest_matches",
        full.status == 200
            && full.body.len() == 5_242_880
            && hex::encode(Sha256::digest(&full.body)) == digest,
        "full read matches the advertised digest",
    );
    let part = client.request("GET", &path, &[("Range", "bytes=11-30")], None, 30)?;
    evidence.check(
        "range_read",
        part.status == 206 && part.body.len() == 20 && part.body[0] == 11,
        "range read returns twenty bytes from offset eleven",
    );
    let scoped = Client::new(world, "calendar:read")?;
    let denied = scoped.request("GET", &path, &[], None, 30)?;
    evidence.check(
        "blob_scope",
        denied.status == 403,
        "missing grant is forbidden",
    );
    Ok(())
}
fn error_code(raw: Option<&(u16, Value)>) -> Option<String> {
    raw?.1["error"]["code"].as_str().map(str::to_owned)
}
fn agent_token(world: &World, name: &str, level: &str, scope: &str) -> Result<Option<String>> {
    let Ok(raw) = world.gateway_cli(&[
        "agent", "create", "--name", name, "--level", level, "--scope", scope,
    ]) else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(raw.trim())?;
    Ok(value["token"]
        .as_str()
        .filter(|t| !t.is_empty() && !value["agent"]["id"].as_str().unwrap_or("").is_empty())
        .map(str::to_owned))
}
fn scope(world: &World, evidence: &mut Evidence) -> Result<()> {
    let q = query(&json!({"value":"lab"}), 1000)?;
    let scoped = Client::new(world, "calendar:read")?;
    evidence.check("scoped_token", true, "scoped token created");
    let denied = scoped.call_raw("test.echo", &q, 20)?;
    evidence.check(
        "missing_grant_rejected",
        denied.as_ref().is_some_and(|(status, _)| *status == 403)
            || error_code(denied.as_ref()).as_deref() == Some("forbidden"),
        "missing grant is forbidden",
    );
    let client = Client::new(world, SCOPE)?;
    evidence.check(
        "read_allowed",
        client.call("test.echo", &q, 20)?.is_some(),
        "granted scope reads",
    );
    let listed = world.gateway_cli(&["agent", "list"]).is_ok();
    evidence.check("agent_authority", listed, "agent list succeeds");
    if !listed {
        return Ok(());
    }
    let read = agent_token(world, "Bridge Test query read", "read", "reminders:read")?;
    evidence.check("read_agent_created", read.is_some(), "read agent created");
    let allowed = match &read {
        Some(token) => Client::with_token(world, token)?
            .call("test.echo", &q, 20)?
            .is_some(),
        None => false,
    };
    evidence.check("read_agent_allowed", allowed, "read agent queries");
    let approve = agent_token(
        world,
        "Bridge Test query approve",
        "approve",
        "reminders:write",
    )?;
    evidence.check(
        "approve_agent_created",
        approve.is_some(),
        "approve agent created",
    );
    let pending = match &approve {
        Some(token) => Client::with_token(world, token)?.call_raw(
            "reminders.create_list",
            &json!({"command_id": random_id("cmd")?, "args": {"title": "[ab-test query] approval"}}),
            20,
        )?,
        None => None,
    };
    evidence.check(
        "effect_requires_approval",
        pending
            .as_ref()
            .is_some_and(|(status, v)| *status == 200 && v["state"] == "awaiting_approval"),
        "write effect waits for approval",
    );
    Ok(())
}
fn effect(world: &World, id: &str) -> Option<u64> {
    world.control("/effects", &Value::Null).ok()?[id].as_u64()
}
fn await_effect(world: &World, id: &str, want: u64, limit: Duration) -> bool {
    let end = Instant::now() + limit;
    loop {
        if effect(world, id) == Some(want) {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn cancellation(
    world: &World,
    client: &Client,
    mcp: bool,
    prefix: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let q = query(&json!({"ms":5000}), 10_000)?;
    let id = q["id"]
        .as_str()
        .ok_or_else(|| Error::Invalid("query id".into()))?
        .to_owned();
    let mut child = client.spawn_call("test.sleep", &q, mcp)?;
    let name = |n: &str| format!("{prefix}{n}");
    evidence.check(
        &name("helper_work_started"),
        await_effect(world, &id, 1, Duration::from_secs(2)),
        "helper work started",
    );
    let _ = child.kill();
    let started = Instant::now();
    let _ = child.wait();
    evidence.check(
        &name("client_cancelled"),
        started.elapsed() < Duration::from_secs(1),
        "client call cancelled",
    );
    evidence.check(
        &name("helper_work_stopped"),
        await_effect(world, &id, 0, Duration::from_secs(1)),
        "helper work stopped",
    );
    Ok(())
}
fn deadline(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, SCOPE)?;
    let q = query(&json!({"ms":5000}), 100)?;
    let start = Instant::now();
    let out = client.call_raw("test.sleep", &q, 20)?;
    evidence.check(
        "deadline_bounded",
        error_code(out.as_ref()).as_deref() == Some("deadline")
            && start.elapsed() < Duration::from_millis(200),
        "deadline error within 200 ms",
    );
    cancellation(world, &client, false, "rest_", evidence)?;
    cancellation(world, &client, true, "mcp_", evidence)
}
pub fn queries(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "CORE-QUERY-ROUNDTRIP" => roundtrip(world, evidence),
        "CORE-QUERY-BLOB" => blob(world, evidence),
        "CORE-QUERY-SCOPE" => scope(world, evidence),
        "CORE-QUERY-DEADLINE" => deadline(world, evidence),
        other if vision::owns(other) => vision::run(world, evidence),
        other => Err(Error::Invalid(format!(
            "queries variant not ported: {other}"
        ))),
    }
}
