#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    http::{Client, random_id},
    replicas,
    web_agent_activity::activity_rows,
    web_agents::{create_agent, owner_session, post, text},
    webauthn::Owner,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, thread, time::Duration};

const BODY: &str = "owner-approved agent message";

fn effect(world: &World, id: &str) -> u64 {
    world
        .control("/effects", &Value::Null)
        .map_or(u64::MAX, |effects| effects[id].as_u64().unwrap_or(0))
}

fn status(world: &World, token: &str, id: &str) -> Value {
    post(
        world,
        token,
        "/v1/op/commands.status",
        &json!({"command_id": id}),
    )
    .map_or(Value::Null, |(_, row)| row)
}

fn call(world: &World, token: &str, command: &Value) -> (u16, Value) {
    post(world, token, "/v1/op/messages.send", command).unwrap_or((0, Value::Null))
}

fn command(chat: &str, expires: Option<String>) -> Result<Value> {
    let mut value = json!({
        "command_id": random_id("cmd")?,
        "op": "messages.send",
        "args": {"chat_id": chat, "text": format!("[ab-test levels] {BODY}")},
        "wait_ms": 15000
    });
    if let Some(expires) = expires {
        value["expires_at"] = json!(expires);
    }
    Ok(value)
}

fn id_of(command: &Value) -> String {
    text(command, "command_id").to_owned()
}

fn pending_over_mcp(world: &World, token: &str, pending: &Value, evidence: &mut Evidence) {
    let Ok(client) = Client::with_token(world, token) else {
        return;
    };
    let record = client.call_mcp_raw("messages.send", pending, 40);
    evidence.check(
        "mcp_wait_reports_awaiting_approval",
        record.is_ok_and(|r| {
            r.is_some_and(|(_, row)| {
                row["state"] == "awaiting_approval" && row["disposition"] == "awaiting_approval"
            })
        }),
        "MCP wait reports awaiting approval",
    );
    let record = client.call_mcp_raw(
        "commands.status",
        &json!({"command_id": id_of(pending)}),
        40,
    );
    evidence.check(
        "mcp_status_reports_awaiting_approval",
        record.is_ok_and(|r| r.is_some_and(|(_, row)| row["state"] == "awaiting_approval")),
        "MCP status reports awaiting approval",
    );
}

fn replica_boundary(
    world: &World,
    chat: &str,
    make: &dyn Fn() -> Result<Value>,
    evidence: &mut Evidence,
) -> Result<()> {
    let Some(replica) = replicas::endpoints().into_iter().next() else {
        evidence.check("replica_agent_authority", false, "no replica");
        return Ok(());
    };
    if replicas::cli(&replica.home, &["agent", "list"]).is_err() {
        evidence.check("replica_agent_authority", false, "replica agent list");
        return Ok(());
    }
    let created = replicas::cli(
        &replica.home,
        &[
            "agent",
            "create",
            "--name",
            "Bridge Test replica",
            "--level",
            "approve",
            "--scope",
            "messages:write",
        ],
    )
    .ok()
    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let agent = created
        .as_ref()
        .map(|c| text(&c["agent"], "id").to_owned())
        .unwrap_or_default();
    let token = created
        .as_ref()
        .map(|c| text(c, "token").to_owned())
        .unwrap_or_default();
    let client = replica.client();
    let reader = Client::at(client.address(), &token);
    let read = await_until(5, || {
        reader
            .request(
                "POST",
                "/v1/op/messages.chats",
                &[("Content-Type", "application/json")],
                Some(b"{}"),
                15,
            )
            .is_ok_and(|r| r.status == 200 && String::from_utf8_lossy(&r.body).contains(chat))
    });
    evidence.check(
        "replica_agent_local_read",
        created.is_some() && read,
        "replica agent reads its local copy",
    );
    let sent = make()?;
    let reply = reader.request(
        "POST",
        "/v1/op/messages.send",
        &[("Content-Type", "application/json")],
        Some(&serde_json::to_vec(&sent)?),
        40,
    )?;
    let pending: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
    evidence.check(
        "replica_agent_queues_gateway_approval_without_effect",
        reply.status == 200
            && pending["state"] == "awaiting_approval"
            && effect(world, &id_of(&sent)) == 0,
        "replica queues approval on the gateway without effect",
    );
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("gateway_activity_authority", false, "agent list");
        return Ok(());
    }
    let owner = owner_session(world, None);
    let replica_id = fs::read(replica.home.join("credentials.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .map(|c| text(&c["enrollment"], "replica_id").to_owned())
        .unwrap_or_default();
    if replica_id.is_empty() {
        evidence.check("gateway_activity_enrollment", false, "replica identity");
        return Ok(());
    }
    let activity = activity_rows(&world.gateway_home, &format!("{replica_id}/{agent}"), "");
    evidence.check(
        "replica_delegation_logged_on_gateway",
        owner.is_ok() && activity.is_ok_and(|a| a.to_string().contains(&id_of(&sent))),
        "replica delegation logged on the gateway",
    );
    Ok(())
}

fn level_activity(world: &World, expected: &BTreeMap<String, &str>, evidence: &mut Evidence) {
    let Ok(listed) = activity_rows(&world.gateway_home, "", "") else {
        evidence.check("level_activity_available", false, "activity readable");
        return;
    };
    let raw = listed.to_string();
    let found = listed["activity"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            let id = text(row, "id");
            let id = id.rsplit(':').next().unwrap_or(id);
            expected
                .get(id)
                .is_some_and(|state| *state == text(row, "state"))
                && row["count"] == 1
                && !text(row, "argument_hash").is_empty()
        })
        .count();
    evidence.check(
        "each_level_command_logged_individually",
        found == expected.len(),
        "every level command logged once",
    );
    evidence.check(
        "level_activity_text_redacted",
        !raw.contains(BODY),
        "message text not logged",
    );
}

pub fn levels(world: &mut World, owner: &Owner, evidence: &mut Evidence) -> Result<()> {
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("authority", false, "agent list");
        return Ok(());
    }
    let create = |world: &World, evidence: &mut Evidence, level: &str| -> String {
        let created = create_agent(
            world,
            &format!("Bridge Test {level}"),
            level,
            "messages:write",
        );
        evidence.check(
            &format!("create_{level}"),
            created.is_some(),
            "agent created",
        );
        created.map(|(_, token, _)| token).unwrap_or_default()
    };
    evidence.check(
        "message_fixture",
        world
            .control("/messages", &json!({"action": "seed", "count": 0}))
            .is_ok(),
        "message fixture",
    );
    let mut chat = None;
    let seen = await_until(5, || {
        chat = world
            .query("records.query", &json!({"collection": "messages.chat"}))
            .ok()
            .and_then(|r| r["records"].as_array().cloned())
            .filter(|records| records.len() == 1);
        chat.is_some()
    });
    let Some(chat) = chat.filter(|_| seen) else {
        evidence.check("chat_visible", false, "chat visible");
        return Ok(());
    };
    let chat = text(&chat[0], "id").to_owned();
    let make = || command(&chat, None);
    let read_token = create(world, evidence, "read");
    let read_command = make()?;
    let (code, _) = call(world, &read_token, &read_command);
    evidence.check("read_command_rejected", code == 401, "read level refused");
    evidence.check(
        "read_no_effect",
        effect(world, &id_of(&read_command)) == 0,
        "no effect",
    );
    let approve_token = create(world, evidence, "");
    let pending = make()?;
    let (code, row) = call(world, &approve_token, &pending);
    evidence.check(
        "default_awaiting_approval",
        code == 200
            && row["state"] == "awaiting_approval"
            && row["disposition"] == "awaiting_approval",
        "default level awaits approval",
    );
    evidence.check(
        "approval_no_effect_before_consent",
        effect(world, &id_of(&pending)) == 0,
        "no effect before consent",
    );
    let (_, retry) = call(world, &approve_token, &pending);
    evidence.check(
        "pending_idempotent",
        retry["history"].as_array().map(Vec::len) == Some(1) && retry["state"] == row["state"],
        "retry returns the same pending command",
    );
    pending_over_mcp(world, &approve_token, &pending, evidence);
    let mut changed = pending.clone();
    changed["args"]["text"] =
        json!(text(&pending["args"], "text").replacen("owner-approved", "changed", 1));
    let (code, _) = call(world, &approve_token, &changed);
    evidence.check(
        "pending_changed_id_conflicts",
        code == 409,
        "changed body under the same id conflicts",
    );
    let listed = owner.request("GET", "/approvals", "application/json", b"");
    evidence.check(
        "owner_receives_full_text",
        listed.is_ok_and(|r| {
            let page = String::from_utf8_lossy(&r.body).into_owned();
            r.status == 200 && page.contains(BODY) && page.contains("argument_hash")
        }),
        "owner sees the full text",
    );
    let agent = Client::with_token(world, &approve_token)?;
    let foreign = agent.request("GET", "/web/v1/approvals", &[], None, 15);
    evidence.check(
        "agent_cannot_approve",
        foreign.is_ok_and(|r| r.status == 404),
        "agent cannot reach approvals",
    );
    let decided = owner.decide(&id_of(&pending), "approve");
    evidence.check(
        "owner_approves",
        decided.is_ok_and(|(code, _)| code == 200),
        "owner approves",
    );
    let mut row = Value::Null;
    let observed = await_until(5, || {
        row = status(world, &approve_token, &id_of(&pending));
        row["state"] == "observed"
    });
    evidence.check(
        "approved_sim_send_observed",
        observed && row["commit_seq"].as_u64().unwrap_or(0) > 0,
        "approved send observed",
    );
    evidence.check(
        "approved_executes_once",
        effect(world, &id_of(&pending)) == 1,
        "executed once",
    );
    let again = owner.decide(&id_of(&pending), "approve");
    evidence.check(
        "approval_retry_no_redispatch",
        again.is_ok_and(|(code, _)| code == 200) && effect(world, &id_of(&pending)) == 1,
        "approval retry does not redispatch",
    );
    let rejected = make()?;
    call(world, &approve_token, &rejected);
    let decided = owner.decide(&id_of(&rejected), "reject");
    let row = status(world, &approve_token, &id_of(&rejected));
    evidence.check(
        "owner_reject_final_no_effect",
        decided.is_ok_and(|(code, _)| code == 200)
            && row["state"] == "rejected_by_owner"
            && row["disposition"] == "rejected_by_owner"
            && effect(world, &id_of(&rejected)) == 0,
        "owner rejection is final",
    );
    let expiring = make()?;
    call(world, &approve_token, &expiring);
    evidence.check(
        "advance_approval_clock",
        world.clock_offset(86_401_000).is_ok(),
        "clock advanced",
    );
    let (_, row) = call(world, &approve_token, &expiring);
    evidence.check(
        "retry_expires_unanswered_approval",
        row["state"] == "expired",
        "retry expires the approval",
    );
    let row = status(world, &approve_token, &id_of(&expiring));
    evidence.check(
        "approval_expires_24h",
        row["state"] == "expired" && effect(world, &id_of(&expiring)) == 0,
        "approval expires after 24 hours",
    );
    let _ = fs::remove_file(world.gateway_home.join("clock-offset"));
    let soon = jiff::Timestamp::now() + jiff::SignedDuration::from_secs(3);
    let durable = command(&chat, Some(soon.to_string()))?;
    call(world, &approve_token, &durable);
    let restarted = world.restart_gateway().is_ok();
    evidence.check(
        "pending_survives_gateway_restart",
        restarted
            && status(world, &approve_token, &id_of(&durable))["state"] == "awaiting_approval",
        "pending approval survives restart",
    );
    let wait = soon.as_millisecond() + 100 - jiff::Timestamp::now().as_millisecond();
    if wait > 0 {
        thread::sleep(Duration::from_millis(u64::try_from(wait).unwrap_or(0)));
    }
    let approved_at = jiff::Timestamp::now();
    let decided = owner.decide(&id_of(&durable), "approve");
    evidence.check(
        "execution_expiry_starts_at_approval",
        decided.is_ok_and(|(code, _)| code == 200),
        "approval after the request expiry",
    );
    let mut row = Value::Null;
    let observed = await_until(5, || {
        row = status(world, &approve_token, &id_of(&durable));
        row["state"] == "observed"
    });
    let expires = row["expires_at"]
        .as_str()
        .and_then(|s| s.parse::<jiff::Timestamp>().ok());
    evidence.check(
        "expired_original_request_executes_after_approval",
        observed && expires.is_some_and(|e| e > approved_at && e > soon),
        "expired request executes after approval",
    );
    replica_boundary(world, &chat, &make, evidence)?;
    let full_token = create(world, evidence, "full");
    let full = make()?;
    let (code, row) = call(world, &full_token, &full);
    evidence.check(
        "full_executes_directly",
        code == 200 && row["state"] == "observed",
        "full level executes directly",
    );
    let expected = BTreeMap::from([
        (id_of(&read_command), "rejected"),
        (id_of(&pending), "observed"),
        (id_of(&rejected), "rejected_by_owner"),
        (id_of(&expiring), "expired"),
        (id_of(&durable), "observed"),
        (id_of(&full), "observed"),
    ]);
    level_activity(world, &expected, evidence);
    Ok(())
}
