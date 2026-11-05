use super::targeted::finish_targeted;
use super::{
    DISPATCH_DEADLINE, POLL_GAP, call, code_of, invalid, known_routes, route_proof, ui_prepared,
    verify_route,
};
use crate::{
    messages_policy::{authorizes, first_tag, tag_at_start, tag_run, tagged},
    session::owned::{Probe, Refusal, refuse, text},
};
use serde_json::{Value, json};
use std::{thread, time::Instant};

fn latest_date(rows: &[Value]) -> f64 {
    rows.iter()
        .filter_map(|row| {
            row["date"]
                .as_str()
                .and_then(|date| date.parse::<f64>().ok())
        })
        .map(|date| if date > 1e12 { date / 1e9 } else { date })
        .fold(0.0, f64::max)
}

fn ledger(probe: &dyn Probe, run: &str, chat: &str) -> Result<Value, Refusal> {
    call(probe, "messages.ledger", &json!({"run": run, "chat": chat}))
}

fn read_state_fixture(
    probe: &dyn Probe,
    chat: &str,
    target: &Value,
    row: i64,
) -> Result<(), Refusal> {
    let body = text(target, "text");
    let tag = tag_at_start(body).ok_or_else(invalid)?;
    let run = tag_run(tag);
    let registration = ledger(probe, run, chat)?;
    if !authorizes(&registration, chat, row, body) {
        return Err(invalid());
    }
    let members = call(probe, "messages.chat_members", &json!({"chat_guid": chat}))?;
    let itself = call(
        probe,
        "messages.is_self_chat",
        &json!({"participants": members["handles"]}),
    )?;
    if itself["self_chat"] != true {
        return Err(invalid());
    }
    let baseline = registration["baseline_max_row_id"].as_i64().unwrap_or(0);
    let foreign = call(
        probe,
        "messages.foreign_activity",
        &json!({"chat": chat, "baseline": baseline, "tag": tag}),
    )?;
    if foreign["foreign_unread"] == true || foreign["foreign_new"] == true {
        return Err(refuse("conflict"));
    }
    Ok(())
}

fn every_message_tagged(
    probe: &dyn Probe,
    rows: &[Value],
    needle: Option<&str>,
) -> Result<bool, Refusal> {
    for item in rows {
        let found = call(probe, "messages.target", &json!({"guid": item["guid"]}))?;
        let body = text(&found["target"], "text");
        let ok = found["exists"] == true
            && needle.map_or_else(|| first_tag(body).is_some(), |tag| body.contains(tag));
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn prepare_conversation(
    probe: &dyn Probe,
    command: &Value,
    interactive: bool,
) -> Result<Value, Refusal> {
    let action = &command["action"];
    let operation = text(action, "op");
    let chat = text(action, "native_key");
    let unproven = || refuse("capability_unproven");
    let conversation = call(probe, "messages.conversation", &json!({"chat_guid": chat}))?;
    if conversation["exists"] != true {
        return Err(unproven());
    }
    let listed = call(
        probe,
        "messages.conversation_messages",
        &json!({"chat_guid": chat}),
    )?;
    let rows = listed["rows"].as_array().cloned().unwrap_or_default();
    let last = rows.first().ok_or_else(unproven)?;
    let found = call(probe, "messages.target", &json!({"guid": last["guid"]}))?;
    if found["exists"] != true {
        return Err(unproven());
    }
    let target = found["target"].clone();
    if !interactive {
        if !tagged(text(&target, "text")) {
            return Err(invalid());
        }
        if matches!(operation, "messages.mark_read" | "messages.mark_unread") {
            read_state_fixture(
                probe,
                chat,
                &target,
                found["row"]["row_position"].as_i64().unwrap_or(0),
            )?;
        } else if !every_message_tagged(probe, &rows, None)? {
            return Err(refuse("unsupported"));
        }
    }
    let mut destination = action.clone();
    destination["list_key"] = json!(chat);
    let route = verify_route(probe, &destination, interactive)?;
    if operation == "messages.delete_chat" && !interactive {
        let body = text(&target, "text");
        let tag = first_tag(body).ok_or_else(invalid)?;
        let run = tag_run(tag);
        let registration = ledger(probe, run, chat)?;
        let created_after = registration["baseline_max_chat_row_id"]
            .as_i64()
            .is_some_and(|baseline| conversation["row_position"].as_i64().unwrap_or(0) > baseline);
        if registration["run_id"] != run
            || text(&registration, "created_chat_key") != chat
            || registration["created_by_run"] != true
            || !created_after
        {
            return Err(refuse("unsupported"));
        }
        if !every_message_tagged(probe, &rows, Some(tag))? {
            return Err(invalid());
        }
    }
    let config = call(probe, "messages.config", &json!({}))?;
    let proven = config["lab_commands"] == true
        || known_routes(probe)?.contains(&route_proof(&route, operation));
    let available = call(probe, "messages.ui_available", &json!({}))?["available"] == true;
    if !(proven && available) {
        return Err(unproven());
    }
    let state = json!({
        "interactive": interactive,
        "conversation_members": rows.iter().map(|row| row["guid"].clone()).collect::<Vec<_>>(),
        "conversation_target": true,
        "message_guid": last["guid"],
        "target": target,
        "destination": route,
        "route": "accessibility",
        "last_read_timestamp": conversation["last_read_message_timestamp"],
        "latest_message_date": latest_date(&rows).to_string(),
        "candidate_ids": [chat],
    });
    if operation == "messages.delete_chat" {
        call(
            probe,
            "messages.ui_activate_validate",
            &json!({"prepared": ui_prepared(action, &state, text(command, "request_hash"))}),
        )?;
    }
    Ok(state)
}

pub(super) fn write_conversation(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    let operation = text(action, "op");
    let chat = text(action, "native_key");
    let mut destination = action.clone();
    destination["list_key"] = json!(chat);
    let route = verify_route(probe, &destination, state["interactive"] == true)
        .map_err(|refusal| code_of(&refusal))?;
    if route != state["destination"] {
        return Err("conflict".into());
    }
    let current = probe.call("messages.conversation", &json!({"chat_guid": chat}))?;
    if current["exists"] != true
        || current["last_read_message_timestamp"] != state["last_read_timestamp"]
    {
        return Err("conflict".into());
    }
    let listed = probe.call(
        "messages.conversation_messages",
        &json!({"chat_guid": chat}),
    )?;
    let rows = listed["rows"].as_array().cloned().unwrap_or_default();
    let guids: Vec<Value> = rows.iter().map(|row| row["guid"].clone()).collect();
    if json!(guids) != state["conversation_members"]
        || latest_date(&rows).to_string() != text(state, "latest_message_date")
    {
        return Err("conflict".into());
    }
    let ui = probe.call(
        "messages.ui_perform",
        &json!({"prepared": ui_prepared(action, state, ""), "command_id": command_id}),
    )?["ui"]
        .clone();
    let started = Instant::now();
    loop {
        let now = probe.call("messages.conversation", &json!({"chat_guid": chat}))?;
        let recoverable = probe.call("messages.chat_recoverable", &json!({"chat_guid": chat}))?["recoverable"]
            == true;
        let missing = now["exists"] != true;
        let observed = if operation == "messages.delete_chat" {
            missing || recoverable
        } else {
            !missing && (now["unread"] == true) == (operation == "messages.mark_unread")
        };
        if observed {
            let mut evidence = ui.as_object().cloned().map_or(json!({}), Value::Object);
            evidence["source"] = json!("messages");
            evidence["collection"] = json!("messages.chat");
            evidence["native_key"] = json!(chat);
            evidence["observed"] = json!(true);
            evidence["absent"] = json!(missing);
            evidence["disposition"] = json!("locally_observed");
            return finish_targeted(probe, state, operation, evidence);
        }
        if started.elapsed() >= DISPATCH_DEADLINE {
            break;
        }
        thread::sleep(POLL_GAP);
    }
    Ok(
        json!({"source": "messages", "observed": false, "disposition": "indeterminate", "reason": "conversation_effect_unproven", "ui": ui}),
    )
}
