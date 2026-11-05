use super::{
    CONVERSATION, DISPATCH_DEADLINE, POLL_GAP, ROW_LIMIT, SEND, TARGETED, call, code_of, strings,
    validate_agent_policy, validate_resend, verify_route, write_conversation, write_targeted,
};
use crate::{
    messages_policy::normalize,
    session::owned::{Probe, Settled, text},
};
use serde_json::{Value, json};
use std::{thread, time::Instant};

pub(super) fn correlate(part: &Value, route: &Value, rows: &[Value]) -> Value {
    let wanted_chat = text(route, "chat");
    let wanted_handle = text(route, "handle");
    let mut matches: Vec<&Value> = Vec::new();
    for row in rows {
        if !wanted_chat.is_empty() && text(row, "capture_chat_guid") != wanted_chat {
            continue;
        }
        if !wanted_handle.is_empty() {
            let peers = strings(&row["peers"]);
            if peers.len() != 1 || normalize(&peers[0]) != normalize(wanted_handle) {
                continue;
            }
        }
        let same = if text(part, "kind") == "text" {
            row["body"].as_str() == part["text"].as_str()
        } else {
            strings(&row["attachment_names"])
                .iter()
                .any(|name| name == text(part, "name"))
        };
        if same {
            matches.push(row);
        }
    }
    if matches.len() > 1 {
        return json!({"disposition": "indeterminate", "ambiguous": true, "candidate_count": matches.len()});
    }
    if let Some(row) = matches.first() {
        let code = row["error"].as_i64().unwrap_or(0);
        let disposition = if code != 0 {
            "delivery_failed"
        } else if row["is_delivered"] == true {
            "delivered"
        } else {
            "locally_observed"
        };
        return json!({"native_key": row["guid"], "disposition": disposition, "observed": true, "error_code": code});
    }
    let ghosts = rows
        .iter()
        .filter(|row| {
            text(row, "service") == "SMS"
                && text(row, "capture_chat_guid").is_empty()
                && text(row, "text").is_empty()
                && row["has_body"] != true
        })
        .count();
    if ghosts > 0 {
        return json!({"disposition": "delivery_failed", "reason": "empty_unjoined_sms", "candidate_count": ghosts});
    }
    json!({"disposition": "indeterminate"})
}

pub(super) fn aggregate(parts: &[Value], fingerprint: &str, baseline: &Value) -> Value {
    let dispositions: Vec<&str> = parts.iter().map(|part| text(part, "disposition")).collect();
    let disposition =
        if dispositions.contains(&"indeterminate") || dispositions.contains(&"not_dispatched") {
            "indeterminate"
        } else if dispositions.contains(&"delivery_failed") {
            "delivery_failed"
        } else if dispositions.iter().all(|value| *value == "delivered") {
            "delivered"
        } else {
            "locally_observed"
        };
    let native_key = parts
        .iter()
        .find(|part| !text(part, "native_key").is_empty())
        .map_or(Value::Null, |part| part["native_key"].clone());
    let observed = parts.iter().any(|part| part["observed"] == true);
    let mut result = json!({
        "source": "messages",
        "collection": "messages.message",
        "native_key": native_key,
        "observed": observed,
        "disposition": disposition,
        "parts": parts,
        "fingerprint": fingerprint,
        "baseline": baseline,
    });
    if !observed && let Some(reason) = parts.iter().find_map(|part| part["reason"].as_str()) {
        result["reason"] = json!(reason);
    }
    result
}

fn outgoing_rows(probe: &dyn Probe, maximum: i64) -> Result<Vec<Value>, String> {
    let answer = probe.call(
        "messages.rows_after",
        &json!({"maximum": maximum, "limit": ROW_LIMIT}),
    )?;
    Ok(answer["rows"].as_array().cloned().unwrap_or_default())
}

pub(super) fn write(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    if TARGETED.contains(&text(action, "op")) {
        return write_targeted(probe, command_id, action, state);
    }
    if CONVERSATION.contains(&text(action, "op")) {
        return write_conversation(probe, command_id, action, state);
    }
    if text(action, "op") != SEND {
        return Err("unsupported".into());
    }
    write_send(probe, command_id, action, state)
}

fn write_send(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    let interactive = state["interactive"] == true;
    let route = verify_route(probe, action, interactive).map_err(|refusal| code_of(&refusal))?;
    if route != state["destination"] {
        return Err("conflict".into());
    }
    validate_resend(probe, action, &route).map_err(|refusal| code_of(&refusal))?;
    let agent =
        call(probe, "messages.agent_policy", &json!({})).map_err(|refusal| code_of(&refusal))?;
    validate_agent_policy(&agent, action, &route).map_err(|refusal| code_of(&refusal))?;
    if agent["configured"] == true {
        probe.call(
            "messages.mark_policy_used",
            &json!({"command_id": command_id}),
        )?;
    }
    let args = &action["messages"];
    let staged = probe.call(
        "messages.stage",
        &json!({"command_id": command_id, "messages": args}),
    )?;
    if staged["failed"].is_object() {
        return Ok(json!({
            "source": "messages",
            "disposition": "failed_no_effect",
            "observed": false,
            "reason": "staging_failed",
            "error_domain": staged["failed"]["domain"],
            "error_code": staged["failed"]["code"],
            "fingerprint": "",
        }));
    }
    let parts: Vec<Value> = staged["parts"].as_array().cloned().unwrap_or_default();
    let baseline = state["max_row_id"].as_i64().unwrap_or(0);
    let started = Instant::now();
    let mut outcomes: Vec<Value> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if started.elapsed() >= DISPATCH_DEADLINE {
            for (remaining, rest) in parts.iter().enumerate().skip(index) {
                outcomes.push(json!({"index": remaining, "kind": rest["kind"], "disposition": "not_dispatched"}));
            }
            break;
        }
        let current = probe.call("messages.max_row_id", &json!({}))?["value"]
            .as_i64()
            .unwrap_or(0);
        let maximum = baseline.max(current);
        let sent = probe.call(
            "messages.send_part",
            &json!({"part": part, "route": route, "command_id": command_id, "index": index, "count": parts.len()}),
        )?;
        let mut evidence = json!({"disposition": "indeterminate"});
        loop {
            let rows = outgoing_rows(probe, maximum)?;
            evidence = correlate(part, &route, &rows);
            if text(&evidence, "disposition") != "indeterminate" || evidence["ambiguous"] == true {
                break;
            }
            if started.elapsed() >= DISPATCH_DEADLINE {
                break;
            }
            thread::sleep(POLL_GAP);
        }
        let mut value = evidence.clone();
        value["index"] = json!(index);
        value["kind"] = part["kind"].clone();
        value["apple_event_error"] = sent["error"].clone();
        value["max_row_id"] = json!(maximum);
        value["payload_fingerprint"] = probe
            .call("messages.payload_fingerprint", &json!({"messages": part}))?["digest"]
            .clone();
        outcomes.push(value);
        if text(&evidence, "disposition") == "indeterminate" {
            for (remaining, rest) in parts.iter().enumerate().skip(index + 1) {
                outcomes.push(json!({"index": remaining, "kind": rest["kind"], "disposition": "not_dispatched", "reason": "prior_part_indeterminate"}));
            }
            break;
        }
    }
    let result = aggregate(&outcomes, "", state);
    let config = call(probe, "messages.config", &json!({})).map_err(|refusal| code_of(&refusal))?;
    if config["lab_commands"] == true
        && matches!(
            text(&result, "disposition"),
            "locally_observed" | "delivered"
        )
    {
        probe.call(
            "messages.record_proof",
            &json!({"route": route, "parts": parts}),
        )?;
    }
    if result["observed"] == true {
        probe.call("messages.reconcile", &json!({"result": result}))?;
    }
    Ok(result)
}

pub(super) fn settle(operation: &str, written: &Value) -> Option<Settled> {
    if operation != SEND && !TARGETED.contains(&operation) && !CONVERSATION.contains(&operation) {
        return None;
    }
    let disposition = match text(written, "disposition") {
        "locally_observed" => "locally_observed",
        "delivered" => "delivered",
        "delivery_failed" => "delivery_failed",
        "failed_no_effect" => "failed_no_effect",
        _ => "indeterminate",
    };
    Some(Settled {
        disposition,
        evidence: written.clone(),
    })
}

pub(super) fn resolved(probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let saved = &evidence["prepared"];
    if TARGETED.contains(&text(&saved["action"], "op"))
        || CONVERSATION.contains(&text(&saved["action"], "op"))
    {
        if evidence["observed"] != true {
            return None;
        }
        let mut kept = evidence.clone();
        kept["fingerprint"] = saved["request_hash"].clone();
        kept["prepared"] = saved.clone();
        return Some(kept);
    }
    if text(&saved["action"], "op") != SEND {
        return None;
    }
    let state = &saved["native_state"];
    let route = &state["destination"];
    let args = &saved["action"]["messages"];
    let mut requested: Vec<Value> = Vec::new();
    if !text(args, "text").is_empty() {
        requested.push(json!({"kind": "text", "text": args["text"]}));
    }
    for (index, file) in args["files"].as_array().into_iter().flatten().enumerate() {
        requested
            .push(json!({"kind": "attachment", "name": format!("{index}-{}", text(file, "name"))}));
    }
    let mut parts: Vec<Value> = Vec::new();
    for (index, part) in requested.iter().enumerate() {
        let existing = evidence["parts"].as_array().and_then(|items| {
            items
                .iter()
                .find(|item| item["index"].as_u64() == Some(index as u64))
        });
        if let Some(existing) = existing
            && text(existing, "disposition") == "not_dispatched"
        {
            parts.push(existing.clone());
            continue;
        }
        let maximum = existing
            .and_then(|item| item["max_row_id"].as_i64())
            .or_else(|| state["max_row_id"].as_i64())
            .unwrap_or(0);
        let rows = outgoing_rows(probe, maximum).ok()?;
        let mut value = correlate(part, route, &rows);
        value["index"] = json!(index);
        value["kind"] = part["kind"].clone();
        value["max_row_id"] = json!(maximum);
        parts.push(value);
    }
    let mut result = aggregate(&parts, text(saved, "request_hash"), state);
    if result["observed"] != true {
        return None;
    }
    result["prepared"] = saved.clone();
    Some(result)
}
