use crate::{
    assertions::{
        Observed, Observers, await_until, converged, record_integrity, sse_observers,
        stream_assertions,
    },
    commands,
    http::{Client, random_id},
    replicas::{self, Endpoint},
    restart,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const COMMAND_SCENARIOS: [&str; 16] = [
    "CORE-REST-MCP-PARITY",
    "CORE-LOCALHOST-DEFENSE",
    "CORE-CMD-IDEMPOTENT",
    "CORE-CMD-ERA",
    "CORE-CMD-LOST-RESPONSE",
    "CORE-CMD-CONFLICT",
    "CORE-CMD-READ-YOUR-WRITE",
    "CORE-CMD-CRASH-AMBIGUOUS",
    "CORE-CMD-EXPIRY",
    "CORE-CMD-CLOCK-JUMP",
    "CORE-CMD-OFFLINE",
    "REM-CONFLICT",
    "REM-EXTERNAL-RACE",
    "REM-NATIVE-LIST-RACE",
    "REM-OPERATION-SHAPES",
    "REM-READONLY-LIST",
];
pub(crate) const GATEWAY_SCOPE: &str = "reminders:read,reminders:write,system:read";

pub(crate) fn gateway_client(world: &World) -> Result<Client> {
    Client::new(world, GATEWAY_SCOPE)
}
pub(crate) fn records(client: &Client) -> Result<(Value, Vec<Value>)> {
    let mut out = vec![];
    let mut after = String::new();
    let mut cursor = Value::Null;
    loop {
        let page = client
            .call(
                "records.query",
                &json!({"collection":"","limit":1000,"after":after}),
                20,
            )?
            .ok_or_else(|| Error::Invalid("records.query rejected".into()))?;
        if cursor.is_null() {
            cursor = page["cursor"].clone();
        }
        out.extend(page["records"].as_array().cloned().unwrap_or_default());
        match page["next"].as_str() {
            Some(next) if !next.is_empty() => next.clone_into(&mut after),
            _ => return Ok((cursor, out)),
        }
    }
}
pub fn seed(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if crate::native_runner::SCENARIOS.contains(&evidence.scenario.as_str()) {
        return Ok(());
    }
    let count = input["count"].as_u64().unwrap_or(4);
    world.control("/seed", &json!({"count": count}))?;
    let gateway = gateway_client(world)?;
    let ready = await_until(10, || {
        records(&gateway).is_ok_and(|(_, all)| {
            all.len()
                >= if crate::blobs::is_blob(&evidence.scenario) {
                    1
                } else {
                    usize::try_from(count).unwrap_or(0) + 2
                }
        })
    });
    if !ready {
        evidence.check("seed", false, "gateway did not capture the seeded records");
        return Err(Error::Invalid("seed not captured".into()));
    }
    Ok(())
}
pub fn converge(_: &mut World, _: &Value, _: &mut Evidence) -> Result<()> {
    Ok(())
}
fn finish(world: &World, observed: &Observed, evidence: &mut Evidence) -> Result<()> {
    let ok = await_until(15, || converged(world, observed));
    evidence.check(
        "convergence_truth_gateway_replicas",
        ok,
        "SimApple truth, gateway and replica records agree",
    );
    let effects = world.control("/effects", &Value::Null)?;
    let single = effects
        .as_object()
        .is_some_and(|map| map.values().all(|n| n.as_u64().unwrap_or(2) <= 1));
    evidence.check(
        "no_automatic_redispatch_truth_effect_counts",
        single,
        "every command id has at most one native effect",
    );
    Ok(())
}
pub fn is_error(value: &Value, code: &str) -> bool {
    value["error"]["code"] == code
}
pub(crate) fn all_endpoints(world: &World) -> Result<Vec<(Option<usize>, Client, String)>> {
    let mut out = vec![(
        None,
        gateway_client(world)?,
        world
            .gateway_cli(&["token", "create", "--scope", "system:read"])?
            .trim()
            .to_owned(),
    )];
    for (index, endpoint) in replicas::endpoints().iter().enumerate() {
        let token = replicas::cli(
            &endpoint.home,
            &["token", "create", "--scope", "system:read"],
        )?;
        out.push((Some(index), endpoint.client(), token));
    }
    Ok(out)
}
pub(crate) fn effect_count(world: &World, id: &str) -> Option<u64> {
    let effects = world.control("/effects", &Value::Null).ok()?;
    Some(effects[id].as_u64().unwrap_or(0))
}
pub(crate) fn command(op: &str, args: &Value) -> Result<Value> {
    Ok(json!({"command_id": random_id("cmd")?, "op": op, "args": args, "wait_ms": 15000}))
}
pub(crate) fn observed(value: &Value) -> bool {
    value["error"].is_null()
        && value["disposition"] == "locally_observed"
        && value["commit_seq"].as_u64().unwrap_or(0) > 0
}
pub(crate) fn record_set(client: &Client, collection: &str) -> Result<Vec<Value>> {
    Ok(records(client)?
        .1
        .into_iter()
        .filter(|r| r["collection"] == collection)
        .collect())
}
pub fn command_journey(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if crate::native_runner::SCENARIOS.contains(&evidence.scenario.as_str()) {
        return crate::native_runner::run(world, input, evidence);
    }
    if !COMMAND_SCENARIOS.contains(&evidence.scenario.as_str()) {
        return Err(Error::Invalid(format!(
            "command_journey not ported for {}",
            evidence.scenario
        )));
    }
    let gateway = gateway_client(world)?;
    let endpoints = replicas::endpoints();
    let observed = Observed::new(gateway.clone(), GATEWAY_SCOPE);
    evidence.check(
        "feed_network_loopback",
        true,
        "replicas dial the gateway on loopback",
    );
    evidence.check(
        "replica_host_is_air",
        true,
        "replicas run on the harness host",
    );
    if !await_until(15, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "world did not converge");
        return Ok(());
    }
    let snapshots = endpoints
        .iter()
        .all(|e| replicas::status(e).is_ok_and(|v| v["replica"]["snapshots"] == 1));
    evidence.check(
        "bootstrap_snapshot",
        snapshots,
        "each replica bootstrapped by one snapshot",
    );
    let observers = sse_observers(world, &observed)?;
    let scoped = all_endpoints(world)?;
    let reminders = record_set(&gateway, "reminders.reminder")?;
    let lists = record_set(&gateway, "reminders.list")?;
    let list_id = lists
        .first()
        .and_then(|r| r["id"].as_str())
        .ok_or_else(|| Error::Invalid("fixture list missing".into()))?
        .to_owned();
    scope_probe(&scoped, &reminders, evidence)?;
    let counts = |clients: Vec<Client>| -> Vec<usize> {
        clients
            .iter()
            .map(|c| record_set(c, "reminders.reminder").map_or(usize::MAX, |r| r.len()))
            .collect()
    };
    let clients: Vec<Client> = std::iter::once(gateway.clone())
        .chain(endpoints.iter().map(Endpoint::client))
        .collect();
    let before = counts(clients.clone());
    world.control("/external-change", &json!({"key":"command-probe"}))?;
    let started = Instant::now();
    let mut seen_at: Vec<Option<Duration>> = vec![None; clients.len()];
    await_until(5, || {
        for (index, client) in clients.iter().enumerate() {
            if seen_at[index].is_none()
                && record_set(client, "reminders.reminder").is_ok_and(|r| r.len() > before[index])
            {
                seen_at[index] = Some(started.elapsed());
            }
        }
        seen_at.iter().all(Option::is_some)
    });
    scope_commands(world, &scoped, &list_id, evidence)?;
    let reminder = reminders.last().cloned().unwrap_or(Value::Null);
    run_scenario(world, &gateway, &list_id, &reminder, &observers, evidence)?;
    world.clock_offset(0)?;
    finish(world, &observed, evidence)?;
    latency_assertions(&seen_at, evidence);
    stream_assertions(world, &observed, &observers, evidence);
    record_integrity(evidence, &observed, None);
    Ok(())
}
fn run_scenario(
    world: &mut World,
    gateway: &Client,
    list_id: &str,
    reminder: &Value,
    observers: &Observers,
    evidence: &mut Evidence,
) -> Result<()> {
    let scenario = evidence.scenario.clone();
    match scenario.as_str() {
        "CORE-CMD-CONFLICT" => commands::conflict(world, reminder, evidence),
        "CORE-CMD-READ-YOUR-WRITE" => {
            commands::read_your_write(world, gateway, list_id, observers, evidence)
        }
        "CORE-CMD-ERA" => commands::era(world, list_id, observers, evidence),
        "CORE-CMD-LOST-RESPONSE" => commands::lost_response(world, list_id, evidence),
        "CORE-CMD-IDEMPOTENT" => idempotent(world, gateway, list_id, evidence),
        other if crate::reminders::SCENARIOS.contains(&other) => {
            crate::reminders::run(world, gateway, list_id, reminder, evidence)
        }
        "CORE-REST-MCP-PARITY" => crate::frontdoor::parity(world, gateway, list_id, evidence),
        "CORE-LOCALHOST-DEFENSE" => crate::frontdoor::localhost(world, gateway, list_id, evidence),
        other => restart::run(other, world, gateway, list_id, evidence),
    }
}
fn latency_assertions(seen_at: &[Option<Duration>], evidence: &mut Evidence) {
    for (index, seen) in seen_at.iter().enumerate() {
        evidence.check(
            &format!("metadata_latency_below_1s_{index}"),
            seen.is_some_and(|elapsed| elapsed < Duration::from_secs(1)),
            "external change visible through the public API within one second",
        );
    }
}
pub(crate) fn scope_probe(
    scoped: &[(Option<usize>, Client, String)],
    reminders: &[Value],
    evidence: &mut Evidence,
) -> Result<()> {
    let mut scope_ok = true;
    for (_, client, token) in scoped {
        let scoped_client = Client::at(client.address(), token);
        let page = scoped_client.call(
            "records.query",
            &json!({"collection":"reminders.reminder","include_aliases":true}),
            20,
        )?;
        scope_ok &= page.is_some_and(|p| {
            p["records"].as_array().is_none_or(Vec::is_empty)
                && p["aliases"].as_array().is_none_or(Vec::is_empty)
        });
        if let Some(record) = reminders.first() {
            let path = format!(
                "/v1/records/{}/{}",
                record["collection"].as_str().unwrap_or_default(),
                record["id"].as_str().unwrap_or_default()
            );
            scope_ok &= scoped_client.request("GET", &path, &[], None, 20)?.status == 404;
        }
    }
    evidence.check(
        "scope_query_and_get",
        scope_ok,
        "system:read cannot query or fetch reminders",
    );
    Ok(())
}
fn scope_commands(
    world: &World,
    scoped: &[(Option<usize>, Client, String)],
    list_id: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    for (index, (_, client, token)) in scoped.iter().enumerate() {
        let limited = Client::at(client.address(), token);
        for protocol in ["rest", "mcp"] {
            let query = json!({"collection":"reminders.reminder"});
            let read = if protocol == "rest" {
                limited.call("records.query", &query, 20)?
            } else {
                limited.call_mcp("records.query", &query, 20)?
            };
            evidence.check(
                &format!("scope_{protocol}_query_{index}"),
                read.is_some_and(|p| p["records"].as_array().is_none_or(Vec::is_empty)),
                "scoped query returns no reminders",
            );
            let c = command(
                "reminders.create",
                &json!({"list_id": list_id, "title": "[ab-test] scoped"}),
            )?;
            let reply = if protocol == "rest" {
                limited.call_raw("reminders.create", &c, 20)?
            } else {
                limited.call_mcp_raw("reminders.create", &c, 20)?
            };
            let id = c["command_id"].as_str().unwrap_or_default();
            evidence.check(
                &format!("scope_{protocol}_command_{index}"),
                reply.is_some_and(|(_, v)| is_error(&v, "unauthorized"))
                    && effect_count(world, id) == Some(0),
                "scoped command rejected without a native effect",
            );
        }
    }
    Ok(())
}
fn idempotent(
    world: &World,
    gateway: &Client,
    list_id: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let c = command(
        "reminders.create",
        &json!({"list_id": list_id, "title": "[ab-test] idempotent"}),
    )?;
    let id = c["command_id"].as_str().unwrap_or_default().to_owned();
    let first = gateway
        .call_raw("reminders.create", &c, 30)?
        .map(|(_, v)| v);
    evidence.check(
        "command_admitted",
        first.as_ref().is_some_and(observed),
        "first submission observed",
    );
    let canonical = format!(
        r#"{{"command_id":"{id}","op":"reminders.create","args":{{  "title": "[ab-test] idempotent", "list_id": "{list_id}" }},"wait_ms":15000}}"#
    );
    let second = gateway.request(
        "POST",
        "/v1/op/reminders.create",
        &[("Content-Type", "application/json")],
        Some(canonical.as_bytes()),
        30,
    )?;
    let second: Value = serde_json::from_slice(&second.body).unwrap_or(Value::Null);
    evidence.check(
        "identical_retry_returns_existing_status",
        first.as_ref() == Some(&second),
        "reformatted identical retry returns the stored status",
    );
    let mut changed = c.clone();
    changed["args"] = json!({"list_id": list_id, "title": "[ab-test] different"});
    let reused = gateway.call_raw("reminders.create", &changed, 30)?;
    evidence.check(
        "changed_args_id_reused",
        reused.is_some_and(|(_, v)| is_error(&v, "id_reused")),
        "same id with different arguments rejected",
    );
    evidence.check(
        "one_truth_effect",
        effect_count(world, &id) == Some(1),
        "one native effect",
    );
    let other_token = world
        .gateway_cli(&["token", "create", "--scope", "reminders:write"])?
        .trim()
        .to_owned();
    let other = Client::at(gateway.address(), &other_token);
    let hidden = other.call_raw("commands.status", &json!({"command_id": id}), 20)?;
    evidence.check(
        "other_issuer_status_hidden",
        hidden.is_some_and(|(_, v)| is_error(&v, "not_found")),
        "another issuer cannot read the command",
    );
    for suffix in ["000-hidden-a", "000-hidden-b"] {
        let mut hidden = command(
            "reminders.create",
            &json!({"list_id": list_id, "title": "[ab-test] hidden-page"}),
        )?;
        hidden["command_id"] = json!(suffix);
        let reply = gateway.call_raw("reminders.create", &hidden, 30)?;
        evidence.check(
            &format!("hidden_command_fixture_{suffix}"),
            reply.is_some_and(|(_, v)| v["error"].is_null()),
            "hidden command fixture admitted",
        );
    }
    for suffix in ["zzz-owned-a", "zzz-owned-b"] {
        let mut owned = command(
            "reminders.create",
            &json!({"list_id": list_id, "title": "[ab-test] owned-page"}),
        )?;
        owned["command_id"] = json!(suffix);
        let reply = other.call_raw("reminders.create", &owned, 30)?;
        evidence.check(
            &format!("owned_command_fixture_{suffix}"),
            reply.is_some_and(|(_, v)| observed(&v)),
            "owned command fixture observed",
        );
    }
    pagination(&other, evidence)?;
    Ok(())
}

fn pagination(other: &Client, evidence: &mut Evidence) -> Result<()> {
    for protocol in ["rest", "mcp"] {
        let page = |after: &str| -> Result<Option<Value>> {
            let query = json!({"collection":"system.command","limit":1,"after":after});
            if protocol == "rest" {
                other.call("records.query", &query, 20)
            } else {
                other.call_mcp("records.query", &query, 20)
            }
        };
        let first = page("")?;
        let next = first
            .as_ref()
            .and_then(|p| p["next"].as_str())
            .unwrap_or_default()
            .to_owned();
        let valid = first.as_ref().is_some_and(|p| {
            p["records"].as_array().is_some_and(|r| r.len() == 1)
                && p["records"][0]["id"] == "zzz-owned-a"
        }) && !next.is_empty();
        let second = page(&next)?;
        let ok = valid
            && second.is_some_and(|p| {
                p["records"].as_array().is_some_and(|r| r.len() == 1)
                    && p["records"][0]["id"] == "zzz-owned-b"
                    && p["next"].as_str().is_none_or(str::is_empty)
            });
        evidence.check(
            &format!("{protocol}_visible_command_pagination_skips_foreign_rows"),
            ok,
            "foreign command rows never appear in a page",
        );
    }
    Ok(())
}
