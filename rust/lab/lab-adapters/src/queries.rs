use crate::http::{Client, random_id};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, time::Instant};

const SCOPE: &str = "messages:read,reminders:read,calendar:read,contacts:read,notes:read,system:read";
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
    evidence.check("echo_equal", values_ok, "result_json equals the query arguments");
    let max = samples.iter().copied().fold(0.0, f64::max);
    evidence.check("latency_budget", max < BUDGET_MS, &format!("max {max:.1} ms"));
    let q = query(&args, 1000)?;
    let rest = client.call("test.echo", &q, 20)?;
    let mcp = client.call_mcp("test.echo", &q, 20)?;
    evidence.check(
        "mcp_parity",
        rest.as_ref().zip(mcp.as_ref()).is_some_and(|(a, b)| a["result_json"] == b["result_json"]),
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
    evidence.check("audit_names_and_sizes_only", audit_ok && found, "audit rows carry names and sizes only");
    let invalid = client.call("test.echo", &query(&args, -1)?, 20)?;
    evidence.check("invalid_rejected", invalid.is_none(), "negative deadline is rejected");
    Ok(())
}
fn blob(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, SCOPE)?;
    let q = query(&json!({"size":5_242_880}), 10_000)?;
    let out = client.call("test.blob", &q, 20)?;
    let blobs = out.as_ref().and_then(|o| o["blobs"].as_array()).filter(|b| b.len() == 1);
    evidence.check("blob_result", blobs.is_some(), "query returned one blob");
    let Some(digest) = blobs.and_then(|b| b[0]["digest"].as_str()) else {
        return Ok(());
    };
    let id = q["id"].as_str().ok_or_else(|| Error::Invalid("query id".into()))?;
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
    evidence.check("blob_scope", denied.status == 403, "missing grant is forbidden");
    Ok(())
}
pub fn queries(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "CORE-QUERY-ROUNDTRIP" => roundtrip(world, evidence),
        "CORE-QUERY-BLOB" => blob(world, evidence),
        other => Err(Error::Invalid(format!("queries variant not ported: {other}"))),
    }
}
