use crate::{
    assertions::await_until,
    http::Client,
    journey::{command, effect_count, observed, records},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};

fn submit(client: &Client, command: &Value, mcp: bool) -> Value {
    let op = command["op"].as_str().unwrap_or_default();
    let reply = if mcp {
        client.call_mcp_raw(op, command, 40)
    } else {
        client.call_raw(op, command, 40)
    };
    reply
        .ok()
        .flatten()
        .map_or(Value::Null, |(_, value)| value)
}

fn count(world: &World, command: &Value, expected: u64) -> bool {
    effect_count(world, command["command_id"].as_str().unwrap_or_default()) == Some(expected)
}

fn find_titled(client: &Client, title: &str) -> Option<Value> {
    records(client)
        .ok()?
        .1
        .into_iter()
        .find(|record| record["data"]["title"] == title)
}

fn truth_matches(world: &World, title: &str, op: &str) -> bool {
    let Ok(truth) = world.control("/inspect", &Value::Null) else {
        return false;
    };
    let mut matched = false;
    for observation in truth.as_array().into_iter().flatten() {
        if observation["native_key"].as_str().unwrap_or_default().is_empty()
            || observation["collection"] != "reminders.reminder"
            || observation["data"]["title"] != title
        {
            continue;
        }
        matched = match op {
            "complete" => observation["data"]["completed"] == true,
            "uncomplete" => observation["data"]["completed"] == false,
            "update" => observation["data"]["notes"] == "[ab-test] revised",
            "delete" => observation["kind"] == "absent",
            _ => false,
        };
    }
    matched
}

pub fn parity(world: &World, gateway: &Client, list_id: &str, evidence: &mut Evidence) -> Result<()> {
    let mut results: Vec<String> = vec![];
    for protocol in ["rest", "mcp"] {
        let mcp = protocol == "mcp";
        let title = format!("[ab-test] crud-{protocol}");
        let create = command(
            "reminders.create",
            &json!({"list_id": list_id, "title": title}),
        )?;
        let reply = submit(gateway, &create, mcp);
        let created_ok = observed(&reply);
        evidence.check(
            &format!("{protocol}_create"),
            created_ok,
            "create observed locally",
        );
        if !created_ok {
            continue;
        }
        let mut created = Value::Null;
        let visible = await_until(5, || {
            find_titled(gateway, &title).is_some_and(|record| {
                created = record;
                true
            })
        });
        if !visible {
            evidence.check("created_visible", false, "created record not visible");
            continue;
        }
        for op in ["update", "complete", "uncomplete", "delete"] {
            let mut args = json!({"id": created["id"]});
            if op == "update" {
                args["notes"] = json!("[ab-test] revised");
            }
            let mut c = command(&format!("reminders.{op}"), &args)?;
            c["expected_version"] = created["version"].clone();
            let reply = submit(gateway, &c, mcp);
            evidence.check(
                &format!("{protocol}_{op}"),
                observed(&reply) && count(world, &c, 1),
                "command observed once",
            );
            results.push(reply["disposition"].as_str().unwrap_or_default().to_owned());
            let read = if mcp {
                gateway.call_mcp(
                    "records.get",
                    &json!({"collection": created["collection"], "id": created["id"], "include_deleted": true}),
                    20,
                )
            } else {
                gateway.call(
                    "records.get",
                    &json!({"collection": created["collection"], "id": created["id"], "include_deleted": true}),
                    20,
                )
            };
            match read.ok().flatten().and_then(|page| {
                let list = page["records"].as_array()?;
                (list.len() == 1).then(|| list[0].clone())
            }) {
                Some(record) => created = record,
                None => evidence.check(
                    &format!("{protocol}_read_updated_version"),
                    false,
                    "record not readable",
                ),
            }
            evidence.check(
                &format!("{protocol}_truth_{op}"),
                truth_matches(world, &title, op),
                "SimApple truth reflects the operation",
            );
        }
        let list = command(
            "reminders.create_list",
            &json!({"title": format!("[ab-test] list {protocol}")}),
        )?;
        let reply = submit(gateway, &list, mcp);
        evidence.check(
            &format!("{protocol}_create_list"),
            observed(&reply) && count(world, &list, 1),
            "list created once",
        );
    }
    evidence.check(
        "rest_mcp_disposition_parity",
        results.len() == 8 && results[..4] == results[4..],
        "REST and MCP produced the same dispositions",
    );
    Ok(())
}

pub fn localhost(world: &World, gateway: &Client, list_id: &str, evidence: &mut Evidence) -> Result<()> {
    for protocol in ["rest", "mcp"] {
        for attack in ["no-token", "bad-host", "foreign-origin"] {
            let c = command(
                "reminders.create",
                &json!({"list_id": list_id, "title": "[ab-test] localhost"}),
            )?;
            let body = serde_json::to_vec(&c)?;
            let path = if protocol == "mcp" {
                "/mcp".to_owned()
            } else {
                format!("/v1/op/{}", c["op"].as_str().unwrap_or_default())
            };
            let headers: Vec<(&str, &str)> = match attack {
                "bad-host" => vec![("Host", "attacker.invalid")],
                "foreign-origin" => vec![("Origin", "https://attacker.invalid")],
                _ => vec![],
            };
            let status = gateway
                .request_with(attack != "no-token", "POST", &path, &headers, Some(&body), 20)
                .map(|reply| reply.status);
            let rejected = status.is_ok_and(|code| code == 401 || code == 403);
            evidence.check(
                &format!("{protocol}_{attack}"),
                rejected && count(world, &c, 0),
                "request rejected before any native effect",
            );
        }
    }
    Ok(())
}
