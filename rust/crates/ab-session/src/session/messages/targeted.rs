use super::{
    DISPATCH_DEADLINE, POLL_GAP, REACTIONS, call, code_of, invalid, known_routes, own_handles,
    route_proof, strings, verify_route,
};
use crate::{
    messages_policy::{ActionWindow, normalize, tagged},
    session::owned::{Probe, Refusal, equal, refuse, text},
};
use serde_json::{Value, json};
use std::{thread, time::Instant};

fn seconds_of(value: &Value) -> Option<f64> {
    let stamp: jiff::Timestamp = value.as_str()?.parse().ok()?;
    #[allow(clippy::cast_precision_loss)]
    Some(stamp.as_second() as f64 + f64::from(stamp.subsec_nanosecond()) / 1e9)
}

fn now_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

pub(super) fn ui_prepared(action: &Value, state: &Value, fingerprint: &str) -> Value {
    json!({"action": action, "native_state": state, "fingerprint": fingerprint})
}

pub(super) fn prepare_targeted(
    probe: &dyn Probe,
    command: &Value,
    interactive: bool,
) -> Result<Value, Refusal> {
    let action = &command["action"];
    let operation = text(action, "op");
    let args = &action["messages"];
    let found = call(
        probe,
        "messages.target",
        &json!({"guid": text(action, "native_key")}),
    )?;
    if found["exists"] != true {
        return Err(refuse("capability_unproven"));
    }
    let target = &found["target"];
    let row = &found["row"];
    if !target["deleted_at"].is_null() {
        return Err(refuse("unsupported"));
    }
    if !(interactive || tagged(text(target, "text"))) {
        return Err(invalid());
    }
    if !interactive && !text(args, "text").is_empty() && !tagged(text(args, "text")) {
        return Err(invalid());
    }
    let mut destination = action.clone();
    destination["list_key"] = row["capture_chat_guid"].clone();
    let route = verify_route(probe, &destination, interactive)?;
    if matches!(operation, "messages.edit" | "messages.unsend") {
        let window = ActionWindow::new(
            text(row, "service"),
            row["is_from_me"] == true,
            row["item_type"].as_i64() == Some(0),
            seconds_of(&target["sent_at"]),
            u32::try_from(target["edit_history"].as_array().map_or(0, Vec::len))
                .unwrap_or(u32::MAX),
        );
        if !window.is_some_and(|window| window.allows(operation == "messages.edit", now_seconds()))
        {
            return Err(refuse("unsupported"));
        }
    }
    for field in ["text", "edit_history", "retracted_parts"] {
        let expected = &command["expected_state"][field];
        if !expected.is_null() && !equal(expected, &target[field]) {
            return Err(refuse("conflict"));
        }
    }
    let own = own_handles(probe)?.ok_or_else(|| refuse("unavailable"))?;
    let members = call(
        probe,
        "messages.chat_members",
        &json!({"chat_guid": row["capture_chat_guid"]}),
    )?;
    let peers = strings(&members["joined"]);
    let self_chat = !peers.is_empty() && peers.iter().all(|peer| own.contains(&normalize(peer)));
    let reactions = call(
        probe,
        "messages.reactions",
        &json!({"guid": row["guid"], "self_chat": self_chat}),
    )?["reactions"]
        .clone();
    let maximum = call(probe, "messages.max_row_id", &json!({}))?["value"].clone();
    let state = json!({
        "self_chat": self_chat,
        "reactions": reactions,
        "route": "accessibility",
        "destination": route,
        "target": target,
        "max_row_id": maximum,
        "candidate_ids": [row["guid"]],
    });
    let config = call(probe, "messages.config", &json!({}))?;
    let proven = config["lab_commands"] == true
        || known_routes(probe)?.contains(&route_proof(&route, operation));
    let available = call(probe, "messages.ui_available", &json!({}))?["available"] == true;
    if !(proven && available) {
        return Err(refuse("capability_unproven"));
    }
    call(
        probe,
        "messages.ui_prepare",
        &json!({"guid": row["guid"], "prepared": ui_prepared(action, &state, text(command, "request_hash"))}),
    )?;
    Ok(state)
}

fn current_target(probe: &dyn Probe, guid: &str) -> Result<Value, String> {
    probe.call("messages.target", &json!({"guid": guid}))
}

pub(super) fn write_targeted(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    let operation = text(action, "op");
    let args = &action["messages"];
    let native = text(action, "native_key");
    let before = current_target(probe, native)?;
    if before["exists"] != true {
        return Err("conflict".into());
    }
    let current = before["target"].clone();
    for field in [
        "text",
        "is_from_me",
        "edit_history",
        "retracted_parts",
        "retracted_at",
        "deleted_at",
    ] {
        if !equal(&current[field], &state["target"][field]) {
            return Err("conflict".into());
        }
    }
    let reaction_type = if operation == "messages.react" {
        let index = REACTIONS
            .iter()
            .position(|kind| *kind == text(args, "reaction"))
            .ok_or("invalid")?;
        Some((if args["remove"] == true { 3000 } else { 2000 }) + index)
    } else {
        None
    };
    let ui = match probe.call(
        "messages.ui_perform",
        &json!({"prepared": ui_prepared(action, state, ""), "command_id": command_id}),
    ) {
        Ok(answer) => answer["ui"].clone(),
        Err(code) => json!({"target_verified": false, "ui_error": code}),
    };
    let base = |extra: Value| {
        let mut evidence = ui.as_object().cloned().map_or(json!({}), Value::Object);
        for (key, value) in extra.as_object().into_iter().flatten() {
            evidence[key] = value.clone();
        }
        evidence
    };
    let started = Instant::now();
    loop {
        let seen = current_target(probe, native)?;
        let found = observe_targeted(probe, action, state, &current, &seen, reaction_type)?;
        if let Some(mut found) = found {
            found["source"] = json!("messages");
            found["observed"] = json!(true);
            found["disposition"] = json!("locally_observed");
            found["parts"] = json!([]);
            let result = base(found);
            return finish_targeted(probe, state, operation, result);
        }
        if started.elapsed() >= DISPATCH_DEADLINE {
            break;
        }
        thread::sleep(POLL_GAP);
    }
    Ok(base(
        json!({"source": "messages", "observed": false, "disposition": "indeterminate"}),
    ))
}

fn observe_targeted(
    probe: &dyn Probe,
    action: &Value,
    state: &Value,
    current: &Value,
    seen: &Value,
    reaction_type: Option<usize>,
) -> Result<Option<Value>, String> {
    let operation = text(action, "op");
    let args = &action["messages"];
    let native = text(action, "native_key");
    let data = if seen["exists"] == true {
        seen["target"].clone()
    } else {
        Value::Null
    };
    let message = |key: &Value, absent: bool| json!({"native_key": key, "collection": "messages.message", "absent": absent});
    Ok(match operation {
        "messages.edit" => (equal(&data["text"], &args["text"])
            && data["edit_history"].as_array().map_or(0, Vec::len)
                > current["edit_history"].as_array().map_or(0, Vec::len))
        .then(|| message(&json!(native), false)),
        "messages.unsend" => (!data["retracted_at"].is_null()
            || data["retracted_parts"]
                .as_array()
                .is_some_and(|parts| !parts.is_empty()))
        .then(|| message(&json!(native), false)),
        "messages.delete_message" => (seen["exists"] != true || !data["deleted_at"].is_null())
            .then(|| message(&json!(native), seen["exists"] != true)),
        "messages.reply" => {
            let replies = probe.call(
                "messages.replies",
                &json!({"guid": native, "text": text(args, "text"), "max_row_id": state["max_row_id"]}),
            )?;
            let guids = strings(&replies["guids"]);
            (guids.len() == 1).then(|| message(&json!(guids[0]), false))
        }
        "messages.react" => {
            let rows = probe.call(
                "messages.reaction_rows",
                &json!({"guid": native, "self_chat": state["self_chat"], "type": reaction_type}),
            )?;
            let known = state["reactions"].as_array().cloned().unwrap_or_default();
            let fresh: Vec<&Value> = rows["reactions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| {
                    !known.iter().any(|old| equal(old, item))
                        && (text(args, "reaction") != "emoji"
                            || args["remove"] == true
                            || item["emoji"] == args["emoji"])
                })
                .collect();
            (fresh.len() == 1).then(|| json!({"native_key": fresh[0]["guid"], "collection": "messages.reaction", "absent": false}))
        }
        _ => None,
    })
}

pub(super) fn finish_targeted(
    probe: &dyn Probe,
    state: &Value,
    operation: &str,
    result: Value,
) -> Result<Value, String> {
    let config = call(probe, "messages.config", &json!({})).map_err(|refusal| code_of(&refusal))?;
    if config["lab_commands"] == true {
        probe.call(
            "messages.record_proof",
            &json!({"route": state["destination"], "parts": [{"kind": operation}]}),
        )?;
    }
    probe.call("messages.reconcile", &json!({"result": result}))?;
    Ok(result)
}
