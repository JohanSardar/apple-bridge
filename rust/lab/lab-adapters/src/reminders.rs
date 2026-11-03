use crate::{
    assertions::{Visibility, gateway_aliases, truth_records},
    http::Client,
    journey::{command, effect_count, is_error, observed, records},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};

pub const SCENARIOS: [&str; 5] = [
    "REM-CONFLICT",
    "REM-EXTERNAL-RACE",
    "REM-NATIVE-LIST-RACE",
    "REM-OPERATION-SHAPES",
    "REM-READONLY-LIST",
];

fn id_of(command: &Value) -> String {
    command["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
fn count(world: &World, command: &Value, expected: u64) -> bool {
    effect_count(world, &id_of(command)) == Some(expected)
}
fn versioned(op: &str, args: &Value, version: u64) -> Result<Value> {
    let mut c = command(op, args)?;
    if version > 0 {
        c["expected_version"] = json!(version);
    }
    Ok(c)
}
fn submit(client: &Client, command: &Value, mcp: bool) -> Result<Value> {
    let op = command["op"].as_str().unwrap_or_default();
    let reply = if mcp {
        client.call_mcp_raw(op, command, 40)?
    } else {
        client.call_raw(op, command, 40)?
    };
    Ok(reply.map_or(Value::Null, |(_, value)| value))
}
fn title_of(record: &Value) -> &str {
    record["data"]["title"].as_str().unwrap_or_default()
}
fn find_title(client: &Client, title: &str) -> Option<Value> {
    records(client)
        .ok()?
        .1
        .into_iter()
        .find(|r| title_of(r) == title)
}
fn flush(world: &World) {
    let _ = world.control("/flush", &Value::Null);
}
fn external_list(world: &World, key: &str, title: &str, writable: bool) -> bool {
    world
        .control(
            "/external-change",
            &json!({"key": key, "collection": "reminders.list", "data": {"title": title, "writable": writable}}),
        )
        .is_ok()
}
fn conflict_current_truth(world: &World, gateway: &Client, before: &Value, reply: &Value) -> bool {
    let current = &reply["error"]["current"];
    if reply["error"]["code"] != "conflict"
        || current.is_null()
        || current["id"] != before["id"]
        || current["version"].as_u64().unwrap_or(0) <= before["version"].as_u64().unwrap_or(0)
    {
        return false;
    }
    let (Ok(all), Ok(truth), Ok(aliases)) = (
        records(gateway),
        world.control("/inspect", &Value::Null),
        gateway_aliases(gateway),
    ) else {
        return false;
    };
    let visibility = Visibility::from_scope(crate::journey::GATEWAY_SCOPE);
    let Ok(expected) = truth_records(&truth, &aliases, &visibility) else {
        return false;
    };
    let Some(record) = all.1.iter().find(|r| r["id"] == before["id"]) else {
        return false;
    };
    record == current
        && expected
            .iter()
            .find(|r| r["id"] == record["id"])
            .is_some_and(|native| native["data"] == record["data"])
}

pub fn run(
    world: &World,
    gateway: &Client,
    list_id: &str,
    reminder: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    match evidence.scenario.as_str() {
        "REM-CONFLICT" => conflict(world, gateway, reminder, evidence),
        "REM-OPERATION-SHAPES" => shapes(world, gateway, list_id, reminder, evidence),
        "REM-NATIVE-LIST-RACE" => list_race(world, gateway, reminder, evidence),
        "REM-EXTERNAL-RACE" => external_race(world, gateway, reminder, evidence),
        _ => readonly(world, gateway, list_id, reminder, evidence),
    }
}

fn conflict(
    world: &World,
    gateway: &Client,
    reminder: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let version = reminder["version"].as_u64().unwrap_or(0);
    let mut c = versioned(
        "reminders.update",
        &json!({"id": reminder["id"], "title": "[ab-test] first edit"}),
        version,
    )?;
    let first = submit(gateway, &c, false)?;
    evidence.check(
        "first_version_wins",
        observed(&first),
        "the first update at the fixture version is observed",
    );
    c["command_id"] = json!(crate::http::random_id("cmd")?);
    let second = submit(gateway, &c, false)?;
    evidence.check(
        "stale_version_conflict_with_current",
        is_error(&second, "conflict")
            && second["error"]["current"]["version"].as_u64().unwrap_or(0) > version,
        "a stale version conflicts and carries the current record",
    );
    evidence.check(
        "loser_has_no_effect",
        count(world, &c, 0),
        "the rejected update has no native effect",
    );
    Ok(())
}

fn shapes(
    world: &World,
    gateway: &Client,
    list_id: &str,
    reminder: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let version = reminder["version"].as_u64().unwrap_or(0);
    for protocol in ["rest", "mcp"] {
        for op in ["create", "create_list", "complete", "uncomplete", "delete"] {
            let c = versioned(
                &format!("reminders.{op}"),
                &json!({"id": reminder["id"], "list_id": list_id, "title": "[ab-test] forbidden overwrite"}),
                version,
            )?;
            let reply = submit(gateway, &c, protocol == "mcp")?;
            evidence.check(
                &format!("{protocol}_{op}_rejects_foreign_fields"),
                is_error(&reply, "invalid") && count(world, &c, 0),
                "foreign fields rejected before any native effect",
            );
        }
    }
    Ok(())
}

fn list_race(
    world: &World,
    gateway: &Client,
    reminder: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let fixture = external_list(world, "other-list", "[ab-test] Other List", true);
    flush(world);
    evidence.check("other_list_fixture", fixture, "second list injected");
    let c = versioned(
        "reminders.update",
        &json!({"id": reminder["id"], "title": "[ab-test] stale list edit"}),
        reminder["version"].as_u64().unwrap_or(0),
    )?;
    let _ = world.control(
        "/fault",
        &json!({"name": "external-move-before-prepare", "ms": 0}),
    );
    let reply = submit(gateway, &c, false)?;
    evidence.check(
        "native_list_move_conflicts",
        is_error(&reply, "conflict") && count(world, &c, 0),
        "a list move before prepare conflicts without an effect",
    );
    evidence.check(
        "native_list_conflict_current_equals_truth",
        conflict_current_truth(world, gateway, reminder, &reply),
        "the conflict current record equals native truth",
    );
    Ok(())
}

fn external_race(
    world: &World,
    gateway: &Client,
    reminder: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let c = versioned(
        "reminders.update",
        &json!({"id": reminder["id"], "title": "[ab-test] should not overwrite"}),
        reminder["version"].as_u64().unwrap_or(0),
    )?;
    let _ = world.control(
        "/fault",
        &json!({"name": "external-edit-between-prepare-and-dispatch", "ms": 0}),
    );
    let reply = submit(gateway, &c, false)?;
    evidence.check(
        "external_race_rejects",
        is_error(&reply, "conflict") && count(world, &c, 0),
        "an edit between prepare and dispatch conflicts without an effect",
    );
    evidence.check(
        "external_race_current_equals_truth",
        conflict_current_truth(world, gateway, reminder, &reply),
        "the conflict current record equals native truth",
    );
    evidence.check(
        "external_edit_preserved",
        find_title(gateway, "[ab-test] external race").is_some(),
        "the external edit survives",
    );
    Ok(())
}

fn readonly(
    world: &World,
    gateway: &Client,
    list_id: &str,
    reminder: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let fixture = external_list(world, "list", "Bridge Test", false);
    flush(world);
    evidence.check("readonly_fixture", fixture, "list made read-only");
    let c = command(
        "reminders.create",
        &json!({"list_id": list_id, "title": "[ab-test] readonly"}),
    )?;
    let reply = submit(gateway, &c, false)?;
    evidence.check(
        "readonly_rejected_before_effect",
        is_error(&reply, "unsupported") && count(world, &c, 0),
        "a read-only list rejects before any effect",
    );
    external_list(world, "other-list", "[ab-test] Writable Destination", true);
    flush(world);
    let destination = find_title(gateway, "[ab-test] Writable Destination")
        .and_then(|r| r["id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let c = versioned(
        "reminders.update",
        &json!({"id": reminder["id"], "list_id": destination}),
        reminder["version"].as_u64().unwrap_or(0),
    )?;
    let reply = submit(gateway, &c, false)?;
    evidence.check(
        "readonly_source_cannot_be_bypassed_by_move",
        is_error(&reply, "unsupported") && count(world, &c, 0),
        "moving out of a read-only list is rejected",
    );
    Ok(())
}
