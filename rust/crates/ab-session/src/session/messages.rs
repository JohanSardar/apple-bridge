use super::owned::{Driver, Probe, Refusal, Settled, equal, native_failure, refuse, text};
use crate::messages_policy::{
    ActionWindow, Failure, Origin, Refusal as Policy, authorizes, check_with, first_tag, normalize,
    read_state_matches, tag_at_start, tag_run, tagged,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    thread,
    time::{Duration, Instant},
};

const SEND: &str = "messages.send";
const TARGETED: [&str; 5] = [
    "messages.reply",
    "messages.react",
    "messages.edit",
    "messages.unsend",
    "messages.delete_message",
];
const CONVERSATION: [&str; 3] = [
    "messages.mark_read",
    "messages.mark_unread",
    "messages.delete_chat",
];
const REACTIONS: [&str; 7] = [
    "love",
    "like",
    "dislike",
    "laugh",
    "emphasize",
    "question",
    "emoji",
];
const SAFETY_MESSAGE: &str = "untagged text requires a local owner token and verified own recipients; other sends require [ab-test <id>]";

fn invalid() -> Refusal {
    refuse("invalid")
}

fn safety() -> Refusal {
    Refusal {
        code: "invalid".into(),
        key: None,
        message: Some(SAFETY_MESSAGE.into()),
    }
}

fn call(probe: &dyn Probe, operation: &str, args: &Value) -> Result<Value, Refusal> {
    probe
        .call(operation, args)
        .map_err(|code| native_failure(&code))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn origin_of(command: &Value) -> Result<Origin, Refusal> {
    let principal = text(command, "principal");
    let origin = match text(command, "origin") {
        "" => Origin::Gated,
        "owner_self" => {
            if !principal.starts_with("token:")
                || principal.chars().count() <= 6
                || text(&command["action"], "op") != SEND
            {
                return Err(invalid());
            }
            Origin::OwnerSelf
        }
        "interactive" => {
            let parts: Vec<&str> = principal.split('/').collect();
            let paired = parts.len() == 2
                && parts[0].starts_with("replica:")
                && parts[0].chars().count() > 8
                && !principal.contains(['\r', '\n']);
            let delegated = paired
                && ((parts[1].starts_with("device:") && parts[1].chars().count() > 7)
                    || (parts[1].starts_with("agent:") && parts[1].chars().count() > 6));
            if !(principal.starts_with("device:") || principal.starts_with("agent:") || delegated) {
                return Err(invalid());
            }
            Origin::Interactive
        }
        _ => return Err(invalid()),
    };
    let action_origin = &command["action"]["origin"];
    if !action_origin.is_null() && action_origin != &command["origin"] {
        return Err(invalid());
    }
    Ok(origin)
}

fn own_handles(probe: &dyn Probe) -> Result<Option<BTreeSet<String>>, Refusal> {
    let answer = call(probe, "messages.own_handles", &json!({}))?;
    Ok(answer["handles"]
        .is_array()
        .then(|| strings(&answer["handles"]).into_iter().collect()))
}

fn recipients(probe: &dyn Probe, action: &Value) -> Result<Vec<String>, Refusal> {
    let chat = text(action, "list_key");
    if chat.is_empty() {
        return Ok(strings(&action["messages"]["handles"]));
    }
    let members = call(probe, "messages.chat_members", &json!({"chat_guid": chat}))?;
    Ok(members["handles"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default())
}

pub(super) fn policy_stage(
    probe: &dyn Probe,
    origin: Origin,
    action: &Value,
) -> Result<bool, Refusal> {
    let args = &action["messages"];
    let files = args["files"].as_array().map_or(0, Vec::len);
    check_with(
        origin,
        text(args, "text"),
        files,
        || own_handles(probe),
        || recipients(probe, action),
    )
    .map_err(|failure| match failure {
        Failure::Policy(Policy::TextTagRequired) => safety(),
        Failure::Policy(Policy::Invalid) => invalid(),
        Failure::Source(refusal) => refusal,
    })
}

fn route_proof(route: &Value, kind: &str) -> String {
    let joined = format!(
        "{}|{}|{}|{}",
        text(route, "kind"),
        text(route, "chat"),
        text(route, "handle"),
        text(route, "service")
    );
    let digest = Sha256::digest(joined.as_bytes());
    format!("{digest:x}:{kind}")
}

fn known_routes(probe: &dyn Probe) -> Result<BTreeSet<String>, Refusal> {
    let answer = call(probe, "messages.route_proofs", &json!({}))?;
    Ok(strings(&answer["routes"]).into_iter().collect())
}

fn approved(probe: &dyn Probe) -> Result<BTreeSet<String>, Refusal> {
    let answer = call(probe, "messages.approved_handles", &json!({}))?;
    Ok(strings(&answer["handles"]).into_iter().collect())
}

fn verify_route(probe: &dyn Probe, action: &Value, interactive: bool) -> Result<Value, Refusal> {
    let args = &action["messages"];
    let own = if interactive {
        BTreeSet::new()
    } else {
        approved(probe)?
    };
    if !interactive && own.is_empty() {
        return Err(invalid());
    }
    let owned = |handle: &str| own.contains(&normalize(handle));
    let chat = text(action, "list_key");
    if !chat.is_empty() {
        let members = call(probe, "messages.chat_members", &json!({"chat_guid": chat}))?;
        let rows = strings(&members["joined"]);
        let route = json!({"kind": "chat_id_raw", "chat": chat});
        let known = known_routes(probe)?;
        let proven = if text(action, "op") == SEND {
            ["text", "attachment"]
                .iter()
                .any(|kind| known.contains(&route_proof(&route, kind)))
        } else {
            known.contains(&route_proof(&route, text(action, "op")))
        };
        if rows.is_empty() || !(interactive || rows.iter().all(|row| owned(row)) || proven) {
            return Err(invalid());
        }
        return Ok(route);
    }
    let handles = strings(&args["handles"]);
    if handles.len() != 1
        || !(interactive || handles.iter().all(|handle| owned(handle)))
        || text(args, "service") != "iMessage"
    {
        return Err(refuse("unsupported"));
    }
    let direct = call(probe, "messages.direct_chats", &json!({}))?;
    let wanted = normalize(&handles[0]);
    let found = direct["chats"].as_array().and_then(|chats| {
        chats
            .iter()
            .find(|chat| normalize(text(chat, "id")) == wanted)
    });
    Ok(match found {
        Some(chat) => json!({"kind": "chat_id_raw", "chat": chat["guid"]}),
        None => json!({"kind": "imessage_buddy", "handle": handles[0], "service": "iMessage"}),
    })
}

fn validate_resend(probe: &dyn Probe, action: &Value, route: &Value) -> Result<(), Refusal> {
    let native = text(&action["messages"], "resend_of");
    if native.is_empty() {
        return Ok(());
    }
    let failed = call(probe, "messages.message", &json!({"guid": native}))?;
    let valid = failed["exists"] == true
        && failed["is_from_me"] == true
        && failed["error"].as_i64().is_some_and(|code| code != 0)
        && !text(route, "chat").is_empty()
        && text(&failed, "capture_chat_guid") == text(route, "chat");
    if valid {
        Ok(())
    } else {
        Err(refuse("conflict"))
    }
}

fn validate_agent_policy(agent: &Value, action: &Value, route: &Value) -> Result<(), Refusal> {
    if agent["configured"] != true {
        return Ok(());
    }
    let policy = &agent["policy"];
    let args = &action["messages"];
    let permitted = args["variant"].is_null()
        && text(action, "op") == SEND
        && text(route, "kind") == "chat_id_raw"
        && text(route, "chat") == text(policy, "chat_guid")
        && text(args, "text") == text(policy, "text")
        && args["files"].as_array().is_none_or(Vec::is_empty)
        && agent["used"] != true;
    if permitted { Ok(()) } else { Err(invalid()) }
}

fn file_names_valid(files: &Value) -> bool {
    files.as_array().is_none_or(|files| {
        files.iter().all(|file| {
            let name = text(file, "name");
            !name.is_empty()
                && !name.contains('/')
                && name != "."
                && name != ".."
                && text(file, "data").is_empty()
        })
    })
}

pub(super) fn prepare(probe: &dyn Probe, command: &Value) -> Result<Value, Refusal> {
    let action = &command["action"];
    let origin = origin_of(command)?;
    let interactive = origin == Origin::Interactive;
    let agent = call(probe, "messages.agent_policy", &json!({}))?;
    if agent["configured"] == true && text(action, "op") != SEND {
        return Err(invalid());
    }
    if TARGETED.contains(&text(action, "op")) {
        return prepare_targeted(probe, command, interactive);
    }
    if CONVERSATION.contains(&text(action, "op")) {
        return prepare_conversation(probe, command, interactive);
    }
    if text(action, "op") != SEND {
        return Err(refuse("unsupported"));
    }
    let args = &action["messages"];
    let owner_self = policy_stage(probe, origin, action)?;
    if !file_names_valid(&args["files"]) {
        return Err(invalid());
    }
    let route = verify_route(probe, action, interactive)?;
    validate_resend(probe, action, &route)?;
    validate_agent_policy(&agent, action, &route)?;
    let config = call(probe, "messages.config", &json!({}))?;
    if config["lab_commands"] != true {
        let known = known_routes(probe)?;
        let mut kinds = Vec::new();
        if !text(args, "text").is_empty() {
            kinds.push("text");
        }
        if args["files"]
            .as_array()
            .is_some_and(|files| !files.is_empty())
        {
            kinds.push("attachment");
        }
        if !kinds
            .iter()
            .all(|kind| known.contains(&route_proof(&route, kind)))
        {
            return Err(refuse("capability_unproven"));
        }
    }
    let maximum = call(probe, "messages.max_row_id", &json!({}))?["value"].clone();
    let fingerprint = call(
        probe,
        "messages.payload_fingerprint",
        &json!({"messages": args}),
    )?["digest"]
        .clone();
    Ok(json!({
        "owner_self": owner_self,
        "interactive": interactive,
        "route": route["kind"],
        "destination": route,
        "max_row_id": maximum,
        "payload_fingerprint": fingerprint,
        "command_id": command["command_id"],
        "expires_at": command["expires_at"],
        "candidate_ids": [maximum.to_string(), text(&route, "chat")],
    }))
}

pub(super) fn validate(probe: &dyn Probe, command: &Value, state: &Value) -> Result<(), Refusal> {
    if state["owner_self"] != true {
        return Ok(());
    }
    let action = &command["action"];
    let own = own_handles(probe)?;
    let members = recipients(probe, action)?;
    if crate::messages_policy::owns_recipients(own.as_ref(), &members) {
        Ok(())
    } else {
        Err(safety())
    }
}

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

const DISPATCH_DEADLINE: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(10)
};
const POLL_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(50)
};
const ROW_LIMIT: u32 = 512;

fn code_of(refusal: &Refusal) -> String {
    refusal.code.clone()
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
    let route = verify_route(probe, action, interactive).map_err(|r| code_of(&r))?;
    if route != state["destination"] {
        return Err("conflict".into());
    }
    validate_resend(probe, action, &route).map_err(|r| code_of(&r))?;
    let agent = call(probe, "messages.agent_policy", &json!({})).map_err(|r| code_of(&r))?;
    validate_agent_policy(&agent, action, &route).map_err(|r| code_of(&r))?;
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
    let config = call(probe, "messages.config", &json!({})).map_err(|r| code_of(&r))?;
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

fn reconcile_args(found: &Value, _operation: &str) -> Value {
    json!({"result": found})
}

fn prepare_unsupported(
    _probe: &dyn Probe,
    _action: &Value,
    _expected: &Value,
) -> Result<Value, Refusal> {
    Err(refuse("unsupported"))
}

fn validate_send(probe: &dyn Probe, action: &Value, state: &Value) -> Result<(), Refusal> {
    if text(action, "op") == "messages.delete_chat" {
        call(
            probe,
            "messages.ui_validate",
            &json!({"prepared": ui_prepared(action, state, "")}),
        )?;
        return Ok(());
    }
    if CONVERSATION.contains(&text(action, "op")) {
        return Ok(());
    }
    if TARGETED.contains(&text(action, "op")) {
        call(
            probe,
            "messages.ui_validate",
            &json!({"prepared": ui_prepared(action, state, "")}),
        )?;
        return Ok(());
    }
    validate(probe, &json!({"action": action}), state)
}

pub(super) const DRIVER: Driver = Driver {
    name: "messages",
    prepare: prepare_unsupported,
    prepare_command: Some(prepare),
    validate: validate_send,
    write,
    settle,
    resolved,
    reconcile_args,
};

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

fn ui_prepared(action: &Value, state: &Value, fingerprint: &str) -> Value {
    json!({"action": action, "native_state": state, "fingerprint": fingerprint})
}

fn prepare_targeted(
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

fn write_targeted(
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

fn finish_targeted(
    probe: &dyn Probe,
    state: &Value,
    operation: &str,
    result: Value,
) -> Result<Value, String> {
    let config = call(probe, "messages.config", &json!({})).map_err(|r| code_of(&r))?;
    if config["lab_commands"] == true {
        probe.call(
            "messages.record_proof",
            &json!({"route": state["destination"], "parts": [{"kind": operation}]}),
        )?;
    }
    probe.call("messages.reconcile", &json!({"result": result}))?;
    Ok(result)
}

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

fn prepare_conversation(
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

fn write_conversation(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    let operation = text(action, "op");
    let chat = text(action, "native_key");
    let mut destination = action.clone();
    destination["list_key"] = json!(chat);
    let route =
        verify_route(probe, &destination, state["interactive"] == true).map_err(|r| code_of(&r))?;
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
    let latest: f64 = text(state, "latest_message_date").parse().unwrap_or(0.0);
    let started = Instant::now();
    loop {
        let now = probe.call("messages.conversation", &json!({"chat_guid": chat}))?;
        let recoverable = probe.call("messages.chat_recoverable", &json!({"chat_guid": chat}))?["recoverable"]
            == true;
        let missing = now["exists"] != true;
        let stamp: f64 = text(&now, "last_read_message_timestamp")
            .parse()
            .unwrap_or(0.0);
        let observed = if operation == "messages.delete_chat" {
            missing || recoverable
        } else {
            !missing && read_state_matches(stamp, latest, operation == "messages.mark_unread")
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};

    #[allow(clippy::struct_excessive_bools)]
    struct Fake {
        own: Option<Vec<String>>,
        approved: Vec<String>,
        chats: BTreeMap<String, Vec<Value>>,
        direct: Vec<Value>,
        proofs: Vec<String>,
        lab: bool,
        messages: BTreeMap<String, Value>,
        agent: Value,
        calls: RefCell<Vec<String>>,
        rows: RefCell<Vec<Vec<Value>>>,
        stage: Value,
        policy_used_fails: bool,
        send_delay: Duration,
        targets: RefCell<BTreeMap<String, Value>>,
        effects: RefCell<Vec<(String, Value)>>,
        reaction_state: Vec<Value>,
        reaction_hits: RefCell<Vec<Value>>,
        reply_guids: RefCell<Vec<String>>,
        ui_available: bool,
        ui_prepare_fails: bool,
        ui_perform_fails: bool,
        conversation: RefCell<Value>,
        conversation_rows: Vec<Value>,
        conversation_effect: RefCell<Option<Value>>,
        ledger: Option<Value>,
        self_chat: bool,
        foreign_activity: Value,
        recoverable: RefCell<bool>,
        activate_fails: bool,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                own: Some(vec!["+15550100100".into()]),
                approved: vec!["+15550100100".into()],
                chats: BTreeMap::new(),
                direct: Vec::new(),
                proofs: Vec::new(),
                lab: true,
                messages: BTreeMap::new(),
                agent: json!({"configured": false}),
                calls: RefCell::new(Vec::new()),
                rows: RefCell::new(Vec::new()),
                stage: json!({"parts": [{"kind": "text", "text": "hello"}]}),
                policy_used_fails: false,
                send_delay: Duration::ZERO,
                targets: RefCell::new(BTreeMap::new()),
                effects: RefCell::new(Vec::new()),
                reaction_state: Vec::new(),
                reaction_hits: RefCell::new(Vec::new()),
                reply_guids: RefCell::new(Vec::new()),
                ui_available: true,
                ui_prepare_fails: false,
                ui_perform_fails: false,
                conversation: RefCell::new(json!({"exists": false})),
                conversation_rows: Vec::new(),
                conversation_effect: RefCell::new(None),
                ledger: None,
                self_chat: true,
                foreign_activity: json!({"foreign_unread": false, "foreign_new": false}),
                recoverable: RefCell::new(false),
                activate_fails: false,
            }
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push(operation.into());
            Ok(match operation {
                "messages.own_handles" => json!({"handles": self.own}),
                "messages.approved_handles" => {
                    if self.approved.is_empty() && self.own.is_none() {
                        return Err("unavailable".into());
                    }
                    json!({"handles": self.approved})
                }
                "messages.chat_members" => {
                    let all = self
                        .chats
                        .get(text(args, "chat_guid"))
                        .cloned()
                        .unwrap_or_default();
                    let joined: Vec<Value> = all
                        .iter()
                        .filter(|handle| !handle.is_null())
                        .cloned()
                        .collect();
                    json!({"handles": all, "joined": joined})
                }
                "messages.direct_chats" => json!({"chats": self.direct}),
                "messages.route_proofs" => json!({"routes": self.proofs}),
                "messages.message" => self
                    .messages
                    .get(text(args, "guid"))
                    .cloned()
                    .unwrap_or(json!({"exists": false})),
                "messages.agent_policy" => self.agent.clone(),
                "messages.config" => json!({"lab_commands": self.lab}),
                "messages.max_row_id" => json!({"value": 42}),
                "messages.payload_fingerprint" => json!({"digest": "fp"}),
                "messages.rows_after" => {
                    let mut queue = self.rows.borrow_mut();
                    let rows = if queue.len() > 1 {
                        queue.remove(0)
                    } else {
                        queue.first().cloned().unwrap_or_default()
                    };
                    json!({"rows": rows})
                }
                "messages.stage" => self.stage.clone(),
                "messages.target" => self
                    .targets
                    .borrow()
                    .get(text(args, "guid"))
                    .cloned()
                    .unwrap_or(json!({"exists": false})),
                "messages.reactions" => json!({"reactions": self.reaction_state}),
                "messages.reaction_rows" => {
                    json!({"reactions": self.reaction_hits.borrow().clone()})
                }
                "messages.replies" => json!({"guids": self.reply_guids.borrow().clone()}),
                "messages.ui_available" => json!({"available": self.ui_available}),
                "messages.ui_prepare" => {
                    if self.ui_prepare_fails {
                        return Err("unavailable".into());
                    }
                    json!({})
                }
                "messages.ui_validate" => json!({}),
                "messages.conversation" => self.conversation.borrow().clone(),
                "messages.conversation_messages" => json!({"rows": self.conversation_rows}),
                "messages.ledger" => {
                    return self.ledger.clone().ok_or_else(|| "unavailable".to_owned());
                }
                "messages.is_self_chat" => json!({"self_chat": self.self_chat}),
                "messages.foreign_activity" => self.foreign_activity.clone(),
                "messages.chat_recoverable" => json!({"recoverable": *self.recoverable.borrow()}),
                "messages.ui_activate_validate" => {
                    if self.activate_fails {
                        return Err("unavailable".into());
                    }
                    json!({})
                }
                "messages.ui_perform" => {
                    if self.ui_perform_fails {
                        return Err("unavailable".into());
                    }
                    if let Some(next) = self.conversation_effect.borrow_mut().take() {
                        *self.conversation.borrow_mut() = next;
                    }
                    for (guid, replacement) in self.effects.borrow_mut().drain(..) {
                        self.targets.borrow_mut().insert(guid, replacement);
                    }
                    json!({"ui": {"target_verified": true}})
                }
                "messages.send_part" => {
                    thread::sleep(self.send_delay);
                    json!({"error": 0})
                }
                "messages.mark_policy_used" => {
                    if self.policy_used_fails {
                        return Err("unavailable".into());
                    }
                    json!({})
                }
                _ => Value::Null,
            })
        }
    }

    fn command(origin: &str, principal: &str, messages: &Value) -> Value {
        json!({"command_id": "c1", "principal": principal, "origin": origin, "expires_at": "2099-01-01T00:00:00Z",
            "action": {"op": "messages.send", "messages": messages}})
    }

    fn direct(text_body: &str) -> Value {
        command(
            "interactive",
            "device:abc",
            &json!({"text": text_body, "handles": ["+15550100100"], "service": "iMessage", "files": []}),
        )
    }

    #[test]
    fn the_send_policy_stage_matches_every_shared_vector() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../design/spec/messages-send-policy-vectors.json"
        );
        let vectors: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let list = vectors["send"].as_array().unwrap();
        assert!(list.len() >= 20);
        for vector in list {
            let name = vector["name"].as_str().unwrap();
            let (origin, principal) = match vector["origin"].as_str().unwrap() {
                "interactive" => ("interactive", "device:abc"),
                "owner_self" => ("owner_self", "token:abcdefg"),
                _ => ("", "device:abc"),
            };
            let files: Vec<Value> = (0..vector["files"].as_u64().unwrap())
                .map(|_| json!({"name": "f.txt", "data": ""}))
                .collect();
            let messages = json!({"text": vector["text"], "files": files, "handles": vector["recipients"], "service": "iMessage"});
            let cmd = command(origin, principal, &messages);
            let mut probe = Fake::new();
            probe.own = vector["own"]
                .as_array()
                .map(|own| own.iter().map(|v| v.as_str().unwrap().to_owned()).collect());
            let parsed = origin_of(&cmd).unwrap();
            let result = policy_stage(&probe, parsed, &cmd["action"]);
            match vector["expect"].as_str().unwrap() {
                "allow" => {
                    let owner = result.unwrap_or_else(|refusal| panic!("{name}: {refusal:?}"));
                    if let Some(expected) = vector["owner_self"].as_bool() {
                        assert_eq!(owner, expected, "{name}");
                    }
                }
                "text_tag_required" => {
                    let refusal = result.unwrap_err();
                    assert_eq!(
                        (refusal.code.as_str(), refusal.message.as_deref()),
                        ("invalid", Some(SAFETY_MESSAGE)),
                        "{name}"
                    );
                }
                _ => {
                    let refusal = result.unwrap_err();
                    assert_eq!(
                        (refusal.code.as_str(), refusal.message),
                        ("invalid", None),
                        "{name}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_chat_recipient_check_reads_the_chat_and_never_the_handles_list() {
        let mut probe = Fake::new();
        probe
            .chats
            .insert("chat-self".into(), vec![json!("+15550100100")]);
        probe.chats.insert(
            "chat-mixed".into(),
            vec![json!("+15550100100"), json!("+15559990000")],
        );
        probe
            .chats
            .insert("chat-null".into(), vec![json!("+15550100100"), Value::Null]);
        for (chat, allowed) in [
            ("chat-self", true),
            ("chat-mixed", false),
            ("chat-null", false),
            ("chat-none", false),
        ] {
            let mut cmd = command(
                "owner_self",
                "token:abcdefg",
                &json!({"text": "note", "handles": ["+15550100100"], "files": []}),
            );
            cmd["action"]["list_key"] = json!(chat);
            let result = policy_stage(&probe, Origin::OwnerSelf, &cmd["action"]);
            assert_eq!(result.is_ok(), allowed, "{chat}");
        }
    }

    #[test]
    fn origins_follow_the_command_origin_rules() {
        let messages = json!({"text": "x"});
        assert_eq!(
            origin_of(&command("", "anything", &messages)).unwrap(),
            Origin::Gated
        );
        assert_eq!(
            origin_of(&command("owner_self", "token:abcdefg", &messages)).unwrap(),
            Origin::OwnerSelf
        );
        for bad in ["token:", "device:abc", "tok:abcdefg"] {
            assert!(
                origin_of(&command("owner_self", bad, &messages)).is_err(),
                "{bad}"
            );
        }
        assert!(origin_of(&command("owner_self", "token:a", &messages)).is_ok());
        let mut other_op = command("owner_self", "token:abcdefg", &messages);
        other_op["action"]["op"] = json!("messages.edit");
        assert!(origin_of(&other_op).is_err());
        for good in [
            "device:abc",
            "agent:bot",
            "replica:abcdefg/device:xyzabc",
            "replica:abcdefg/agent:botbot",
        ] {
            assert_eq!(
                origin_of(&command("interactive", good, &messages)).unwrap(),
                Origin::Interactive,
                "{good}"
            );
        }
        for bad in [
            "owner",
            "replica:/device:xyzabc",
            "replica:abcdefg/device:",
            "replica:abcdefg/agent:",
            "replica:abcdefg/other:xyzabc",
            "replica:abcdefg/device:xyzabc/x",
            "replica:abcdefg\n/device:xyzabc",
        ] {
            assert!(
                origin_of(&command("interactive", bad, &messages)).is_err(),
                "{bad}"
            );
        }
        assert!(origin_of(&command("other", "device:abc", &messages)).is_err());
        let mut mismatch = command("interactive", "device:abc", &messages);
        mismatch["action"]["origin"] = json!("owner_self");
        assert!(origin_of(&mismatch).is_err());
        let mut same = command("interactive", "device:abc", &messages);
        same["action"]["origin"] = json!("interactive");
        assert!(origin_of(&same).is_ok());
    }

    #[test]
    fn an_interactive_send_prepares_with_the_baseline_state() {
        let state = prepare(&Fake::new(), &direct("hello")).unwrap();
        assert_eq!(state["interactive"], true);
        assert_eq!(state["owner_self"], false);
        assert_eq!(state["max_row_id"], 42);
        assert_eq!(state["payload_fingerprint"], "fp");
        assert_eq!(state["destination"]["kind"], "imessage_buddy");
        assert_eq!(state["candidate_ids"], json!(["42", ""]));
        let mut probe = Fake::new();
        probe.direct = vec![json!({"guid": "chat-1", "id": "+1 (555) 010-0100"})];
        let state = prepare(&probe, &direct("hello")).unwrap();
        assert_eq!(
            state["destination"],
            json!({"kind": "chat_id_raw", "chat": "chat-1"})
        );
        assert_eq!(state["candidate_ids"], json!(["42", "chat-1"]));
    }

    #[test]
    fn the_self_test_sends_pass_prepare_as_a_gated_selftest_principal() {
        let selftest = |messages: Value| {
            let mut value = command("", "selftest", &messages);
            value["action"]["native_key"] = json!("");
            value["action"]["list_key"] = json!("chat-1");
            value
        };
        let text = selftest(
            json!({"handles": ["+15550100100"], "service": "iMessage", "text": "[ab-test selftest_a] self-test"}),
        );
        let mut probe = Fake::new();
        probe
            .chats
            .insert("chat-1".into(), vec![json!("+15550100100")]);
        assert!(prepare(&probe, &text).is_ok());
        let file = selftest(json!({
            "handles": ["+15550100100"], "service": "iMessage", "text": "",
            "files": [{"name": "ab-selftest_a.txt", "path": "/tmp/x", "digest": "d"}]
        }));
        assert!(prepare(&probe, &file).is_ok());
        let untagged = selftest(
            json!({"handles": ["+15550100100"], "service": "iMessage", "text": "self-test"}),
        );
        assert_eq!(
            prepare(&probe, &untagged).unwrap_err().code,
            "invalid"
        );
    }

    #[test]
    fn a_gated_send_must_stay_inside_the_approved_handles() {
        let tagged_text = "[ab-test x] hi";
        let gated = |handles: Value| {
            command(
                "",
                "device:abc",
                &json!({"text": tagged_text, "handles": handles, "service": "iMessage", "files": []}),
            )
        };
        assert!(prepare(&Fake::new(), &gated(json!(["+15550100100"]))).is_ok());
        assert_eq!(
            prepare(&Fake::new(), &gated(json!(["+15559990000"])))
                .unwrap_err()
                .code,
            "unsupported"
        );
        assert_eq!(
            prepare(
                &Fake::new(),
                &gated(json!(["+15550100100", "+15550100100"]))
            )
            .unwrap_err()
            .code,
            "unsupported"
        );
        let mut empty = Fake::new();
        empty.approved = Vec::new();
        empty.own = Some(Vec::new());
        assert_eq!(
            prepare(&empty, &gated(json!(["+15550100100"])))
                .unwrap_err()
                .code,
            "invalid"
        );
        let mut unknown = Fake::new();
        unknown.approved = Vec::new();
        unknown.own = None;
        assert_eq!(
            prepare(&unknown, &gated(json!(["+15550100100"])))
                .unwrap_err()
                .code,
            "unavailable"
        );
        let sms = command(
            "",
            "device:abc",
            &json!({"text": tagged_text, "handles": ["+15550100100"], "service": "SMS", "files": []}),
        );
        assert_eq!(prepare(&Fake::new(), &sms).unwrap_err().code, "unsupported");
    }

    #[test]
    fn a_gated_send_to_a_chat_needs_own_members_or_a_proven_route() {
        let mut cmd = command(
            "",
            "device:abc",
            &json!({"text": "[ab-test x] hi", "files": []}),
        );
        cmd["action"]["list_key"] = json!("chat-foreign");
        let mut probe = Fake::new();
        probe
            .chats
            .insert("chat-foreign".into(), vec![json!("+15559990000")]);
        probe
            .chats
            .insert("chat-own".into(), vec![json!("+15550100100")]);
        assert_eq!(prepare(&probe, &cmd).unwrap_err().code, "invalid");
        probe.proofs = vec![route_proof(
            &json!({"kind": "chat_id_raw", "chat": "chat-foreign"}),
            "text",
        )];
        assert!(prepare(&probe, &cmd).is_ok());
        probe.proofs = vec![route_proof(
            &json!({"kind": "chat_id_raw", "chat": "chat-foreign"}),
            "edit",
        )];
        assert_eq!(prepare(&probe, &cmd).unwrap_err().code, "invalid");
        cmd["action"]["list_key"] = json!("chat-own");
        assert!(prepare(&probe, &cmd).is_ok());
        cmd["action"]["list_key"] = json!("chat-empty");
        assert_eq!(prepare(&probe, &cmd).unwrap_err().code, "invalid");
    }

    #[test]
    fn outside_the_lab_every_part_of_the_send_needs_a_proven_route() {
        let mut probe = Fake::new();
        probe.lab = false;
        probe.direct = vec![json!({"guid": "chat-1", "id": "+15550100100"})];
        let route = json!({"kind": "chat_id_raw", "chat": "chat-1"});
        let with_file = command(
            "interactive",
            "device:abc",
            &json!({"text": "hi", "handles": ["+15550100100"], "service": "iMessage", "files": [{"name": "a.txt", "data": ""}]}),
        );
        assert_eq!(
            prepare(&probe, &direct("hi")).unwrap_err().code,
            "capability_unproven"
        );
        probe.proofs = vec![route_proof(&route, "text")];
        assert!(prepare(&probe, &direct("hi")).is_ok());
        assert_eq!(
            prepare(&probe, &with_file).unwrap_err().code,
            "capability_unproven"
        );
        probe.proofs.push(route_proof(&route, "attachment"));
        assert!(prepare(&probe, &with_file).is_ok());
        probe.proofs = vec![route_proof(&route, "attachment")];
        assert_eq!(
            prepare(&probe, &direct("hi")).unwrap_err().code,
            "capability_unproven"
        );
        probe.proofs = vec![route_proof(
            &json!({"kind": "chat_id_raw", "chat": "chat-2"}),
            "text",
        )];
        assert_eq!(
            prepare(&probe, &direct("hi")).unwrap_err().code,
            "capability_unproven"
        );
    }

    #[test]
    fn attachment_names_must_be_plain_file_names_without_inline_data() {
        for (name, data, ok) in [
            ("a.txt", "", true),
            ("", "", false),
            (".", "", false),
            ("..", "", false),
            ("a/b.txt", "", false),
            ("a.txt/", "", false),
            ("a.txt", "QUJD", false),
        ] {
            let cmd = command(
                "interactive",
                "device:abc",
                &json!({"handles": ["+15550100100"], "service": "iMessage", "files": [{"name": name, "data": data}]}),
            );
            assert_eq!(prepare(&Fake::new(), &cmd).is_ok(), ok, "{name:?}");
        }
        let nothing = command(
            "interactive",
            "device:abc",
            &json!({"text": "", "handles": ["+15550100100"], "service": "iMessage", "files": []}),
        );
        assert_eq!(prepare(&Fake::new(), &nothing).unwrap_err().code, "invalid");
    }

    #[test]
    fn a_resend_must_name_a_failed_message_of_the_same_chat() {
        let mut probe = Fake::new();
        probe.direct = vec![json!({"guid": "chat-1", "id": "+15550100100"})];
        let resend = |guid: &str| {
            command(
                "interactive",
                "device:abc",
                &json!({"text": "hi", "resend_of": guid, "handles": ["+15550100100"], "service": "iMessage", "files": []}),
            )
        };
        let failed =
            json!({"exists": true, "is_from_me": true, "error": 22, "capture_chat_guid": "chat-1"});
        probe.messages.insert("g-ok".into(), failed.clone());
        for (guid, extra) in [
            ("g-from-them", json!({"is_from_me": false})),
            ("g-no-error", json!({"error": 0})),
            ("g-other-chat", json!({"capture_chat_guid": "chat-2"})),
        ] {
            let mut variant = failed.clone();
            for (key, value) in extra.as_object().unwrap() {
                variant[key] = value.clone();
            }
            probe.messages.insert(guid.into(), variant);
        }
        assert!(prepare(&probe, &resend("g-ok")).is_ok());
        for guid in ["g-from-them", "g-no-error", "g-other-chat", "g-missing"] {
            assert_eq!(
                prepare(&probe, &resend(guid)).unwrap_err().code,
                "conflict",
                "{guid}"
            );
        }
        let buddy = Fake::new();
        let mut buddy_failed = failed;
        buddy_failed["capture_chat_guid"] = json!("");
        let mut buddy = buddy;
        buddy.messages.insert("g-ok".into(), buddy_failed);
        assert_eq!(
            prepare(&buddy, &resend("g-ok")).unwrap_err().code,
            "conflict"
        );
    }

    #[test]
    fn the_agent_policy_admits_exactly_one_pinned_message() {
        let mut probe = Fake::new();
        probe.direct = vec![json!({"guid": "chat-1", "id": "+15550100100"})];
        probe.agent = json!({"configured": true, "policy": {"chat_guid": "chat-1", "text": "pinned"}, "used": false});
        assert!(prepare(&probe, &direct("pinned")).is_ok());
        assert_eq!(
            prepare(&probe, &direct("different")).unwrap_err().code,
            "invalid"
        );
        let mut variant = direct("pinned");
        variant["action"]["messages"]["variant"] = json!("effect");
        assert_eq!(prepare(&probe, &variant).unwrap_err().code, "invalid");
        let mut files = direct("pinned");
        files["action"]["messages"]["files"] = json!([{"name": "a.txt", "data": ""}]);
        assert_eq!(prepare(&probe, &files).unwrap_err().code, "invalid");
        probe.agent["used"] = json!(true);
        assert_eq!(
            prepare(&probe, &direct("pinned")).unwrap_err().code,
            "invalid"
        );
        probe.agent["used"] = json!(false);
        probe.direct = vec![json!({"guid": "chat-2", "id": "+15550100100"})];
        assert_eq!(
            prepare(&probe, &direct("pinned")).unwrap_err().code,
            "invalid"
        );
        probe.direct = Vec::new();
        assert_eq!(
            prepare(&probe, &direct("pinned")).unwrap_err().code,
            "invalid"
        );
        let mut edit = direct("pinned");
        edit["action"]["op"] = json!("messages.edit");
        assert_eq!(prepare(&probe, &edit).unwrap_err().code, "invalid");
    }

    #[test]
    fn operations_other_than_send_are_not_ported_yet() {
        let mut chat_op = direct("hi");
        chat_op["action"]["op"] = json!("messages.explode");
        assert_eq!(
            prepare(&Fake::new(), &chat_op).unwrap_err().code,
            "unsupported"
        );
    }

    #[test]
    fn dispatch_validation_rechecks_the_own_recipients_of_an_owner_send() {
        let note = command(
            "owner_self",
            "token:abcdefg",
            &json!({"text": "note", "handles": ["+15550100100"], "service": "iMessage", "files": []}),
        );
        let probe = Fake::new();
        let state = prepare(&probe, &note).unwrap();
        assert_eq!(state["owner_self"], true);
        assert!(validate(&probe, &note, &state).is_ok());
        let mut changed = Fake::new();
        changed.own = Some(vec!["+15557770000".into()]);
        let refusal = validate(&changed, &note, &state).unwrap_err();
        assert_eq!(
            (refusal.code.as_str(), refusal.message.as_deref()),
            ("invalid", Some(SAFETY_MESSAGE))
        );
        let mut unknown = Fake::new();
        unknown.own = None;
        assert!(validate(&unknown, &note, &state).is_err());
        assert!(validate(&unknown, &note, &json!({"owner_self": false})).is_ok());
    }

    fn row(guid: &str, chat: &str, body: &str) -> Value {
        json!({"guid": guid, "capture_chat_guid": chat, "service": "iMessage", "error": 0, "is_delivered": false,
            "text": body, "body": body, "has_body": false, "peers": ["+15550100100"], "attachment_names": []})
    }

    fn text_part(body: &str) -> Value {
        json!({"kind": "text", "text": body})
    }

    #[test]
    fn one_matching_row_is_observed_and_its_state_picks_the_disposition() {
        let route = json!({"kind": "chat_id_raw", "chat": "chat-1"});
        let plain = row("g1", "chat-1", "hello");
        let found = correlate(&text_part("hello"), &route, std::slice::from_ref(&plain));
        assert_eq!(
            (found["disposition"].as_str(), found["native_key"].as_str()),
            (Some("locally_observed"), Some("g1"))
        );
        let mut delivered = plain.clone();
        delivered["is_delivered"] = json!(true);
        assert_eq!(
            correlate(&text_part("hello"), &route, &[delivered.clone()])["disposition"],
            "delivered"
        );
        delivered["error"] = json!(22);
        let failed = correlate(&text_part("hello"), &route, &[delivered]);
        assert_eq!(
            (
                failed["disposition"].as_str(),
                failed["error_code"].as_i64()
            ),
            (Some("delivery_failed"), Some(22))
        );
    }

    #[test]
    fn rows_of_another_chat_text_or_peer_never_match() {
        let route = json!({"kind": "chat_id_raw", "chat": "chat-1"});
        assert_eq!(
            correlate(&text_part("hello"), &route, &[row("g1", "chat-2", "hello")])["disposition"],
            "indeterminate"
        );
        assert_eq!(
            correlate(&text_part("hello"), &route, &[row("g1", "chat-1", "other")])["disposition"],
            "indeterminate"
        );
        let buddy =
            json!({"kind": "imessage_buddy", "handle": "+1 (555) 010-0100", "service": "iMessage"});
        assert_eq!(
            correlate(&text_part("hello"), &buddy, &[row("g1", "chat-1", "hello")])["disposition"],
            "locally_observed"
        );
        let mut group = row("g1", "chat-1", "hello");
        group["peers"] = json!(["+15550100100", "+15559990000"]);
        assert_eq!(
            correlate(&text_part("hello"), &buddy, &[group])["disposition"],
            "indeterminate"
        );
        let mut elsewhere = row("g1", "chat-1", "hello");
        elsewhere["peers"] = json!(["+15559990000"]);
        assert_eq!(
            correlate(&text_part("hello"), &buddy, &[elsewhere])["disposition"],
            "indeterminate"
        );
    }

    #[test]
    fn two_matches_are_ambiguous_and_never_pick_one() {
        let route = json!({"kind": "chat_id_raw", "chat": "chat-1"});
        let found = correlate(
            &text_part("hello"),
            &route,
            &[row("g1", "chat-1", "hello"), row("g2", "chat-1", "hello")],
        );
        assert_eq!(
            (
                found["disposition"].as_str(),
                found["ambiguous"].as_bool(),
                found["candidate_count"].as_u64()
            ),
            (Some("indeterminate"), Some(true), Some(2))
        );
        assert!(found["native_key"].is_null());
    }

    #[test]
    fn attachments_match_by_name_and_empty_unjoined_sms_rows_are_failures() {
        let route = json!({"kind": "chat_id_raw", "chat": "chat-1"});
        let mut with_file = row("g1", "chat-1", "");
        with_file["attachment_names"] = json!(["0-a.txt"]);
        let part = json!({"kind": "attachment", "name": "0-a.txt"});
        assert_eq!(
            correlate(&part, &route, std::slice::from_ref(&with_file))["disposition"],
            "locally_observed"
        );
        assert_eq!(
            correlate(
                &json!({"kind": "attachment", "name": "1-b.txt"}),
                &route,
                &[with_file]
            )["disposition"],
            "indeterminate"
        );
        let ghost = json!({"guid": "gx", "capture_chat_guid": "", "service": "SMS", "error": 0, "text": "", "has_body": false, "peers": [], "attachment_names": []});
        let failed = correlate(&text_part("hello"), &route, std::slice::from_ref(&ghost));
        assert_eq!(
            (
                failed["disposition"].as_str(),
                failed["reason"].as_str(),
                failed["candidate_count"].as_u64()
            ),
            (Some("delivery_failed"), Some("empty_unjoined_sms"), Some(1))
        );
        let mut not_ghost = ghost.clone();
        not_ghost["has_body"] = json!(true);
        assert_eq!(
            correlate(&text_part("hello"), &route, &[not_ghost])["disposition"],
            "indeterminate"
        );
        let mut joined = ghost;
        joined["capture_chat_guid"] = json!("chat-9");
        assert_eq!(
            correlate(&text_part("hello"), &route, &[joined])["disposition"],
            "indeterminate"
        );
        assert_eq!(
            correlate(&text_part("hello"), &route, &[])["disposition"],
            "indeterminate"
        );
    }

    #[test]
    fn the_aggregate_is_only_as_good_as_its_worst_part() {
        let state = json!({"max_row_id": 1});
        let part = |disposition: &str, key: &str| json!({"disposition": disposition, "native_key": key, "observed": !key.is_empty()});
        let all = |parts: &[Value]| aggregate(parts, "fp", &state);
        assert_eq!(
            all(&[part("delivered", "a"), part("delivered", "b")])["disposition"],
            "delivered"
        );
        assert_eq!(
            all(&[part("delivered", "a"), part("locally_observed", "b")])["disposition"],
            "locally_observed"
        );
        assert_eq!(
            all(&[part("delivered", "a"), part("delivery_failed", "b")])["disposition"],
            "delivery_failed"
        );
        assert_eq!(
            all(&[part("delivery_failed", "a"), part("indeterminate", "")])["disposition"],
            "indeterminate"
        );
        assert_eq!(
            all(&[
                part("delivered", "a"),
                json!({"disposition": "not_dispatched"})
            ])["disposition"],
            "indeterminate"
        );
        let mixed = all(&[part("indeterminate", ""), part("delivered", "b")]);
        assert_eq!(
            (mixed["native_key"].as_str(), mixed["observed"].as_bool()),
            (Some("b"), Some(true))
        );
        let none = all(&[part("indeterminate", "")]);
        assert!(none["native_key"].is_null());
        assert_eq!(none["observed"], false);
        assert_eq!(
            (none["fingerprint"].as_str(), none["baseline"].clone()),
            (Some("fp"), state)
        );
    }

    fn chat_probe() -> Fake {
        let mut probe = Fake::new();
        probe.direct = vec![json!({"guid": "chat-1", "id": "+15550100100"})];
        probe
    }

    fn prepared(probe: &Fake, cmd: &Value) -> Value {
        prepare(probe, cmd).unwrap()
    }

    fn sent(probe: &Fake) -> usize {
        probe
            .calls
            .borrow()
            .iter()
            .filter(|call| *call == "messages.send_part")
            .count()
    }

    #[test]
    fn a_send_is_verified_staged_sent_and_observed_in_order() {
        let probe = chat_probe();
        probe
            .rows
            .borrow_mut()
            .push(vec![row("g1", "chat-1", "hello")]);
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(
            (
                written["disposition"].as_str(),
                written["native_key"].as_str()
            ),
            (Some("locally_observed"), Some("g1"))
        );
        assert_eq!(written["parts"][0]["apple_event_error"], 0);
        assert_eq!(written["parts"][0]["payload_fingerprint"], "fp");
        let calls = probe.calls.borrow();
        let at = |name: &str| calls.iter().rposition(|call| call == name).unwrap();
        assert!(at("messages.stage") < at("messages.send_part"));
        assert!(at("messages.send_part") < at("messages.rows_after"));
        assert!(at("messages.rows_after") < at("messages.reconcile"));
        assert!(calls.iter().any(|call| call == "messages.record_proof"));
        assert_eq!(
            settle(SEND, &written).unwrap().disposition,
            "locally_observed"
        );
    }

    #[test]
    fn a_route_that_changed_since_prepare_sends_nothing() {
        let mut probe = chat_probe();
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        probe.direct = vec![json!({"guid": "chat-2", "id": "+15550100100"})];
        assert_eq!(
            write(&probe, "c1", &cmd["action"], &state).unwrap_err(),
            "conflict"
        );
        assert_eq!(sent(&probe), 0);
        let mut gated = Fake::new();
        let tagged = command(
            "",
            "device:abc",
            &json!({"text": "[ab-test x] hi", "handles": ["+15550100100"], "service": "iMessage", "files": []}),
        );
        let state = prepared(&gated, &tagged);
        gated.approved = vec!["+15557770000".into()];
        assert!(write(&gated, "c1", &tagged["action"], &state).is_err());
        assert_eq!(sent(&gated), 0);
    }

    #[test]
    fn a_resend_and_the_agent_policy_are_checked_again_before_anything_is_sent() {
        let mut probe = chat_probe();
        let failed =
            json!({"exists": true, "is_from_me": true, "error": 22, "capture_chat_guid": "chat-1"});
        probe.messages.insert("g0".into(), failed.clone());
        let mut cmd = direct("hello");
        cmd["action"]["messages"]["resend_of"] = json!("g0");
        let state = prepared(&probe, &cmd);
        let mut healed = failed;
        healed["error"] = json!(0);
        probe.messages.insert("g0".into(), healed);
        assert_eq!(
            write(&probe, "c1", &cmd["action"], &state).unwrap_err(),
            "conflict"
        );
        assert_eq!(sent(&probe), 0);
        let mut pinned = chat_probe();
        pinned.agent = json!({"configured": true, "policy": {"chat_guid": "chat-1", "text": "hello"}, "used": false});
        let cmd = direct("hello");
        let state = prepared(&pinned, &cmd);
        pinned.agent["used"] = json!(true);
        assert_eq!(
            write(&pinned, "c1", &cmd["action"], &state).unwrap_err(),
            "invalid"
        );
        assert_eq!(sent(&pinned), 0);
    }

    #[test]
    fn the_agent_marker_is_written_before_the_send_and_a_failed_marker_stops_it() {
        let mut probe = chat_probe();
        probe.agent = json!({"configured": true, "policy": {"chat_guid": "chat-1", "text": "hello"}, "used": false});
        probe
            .rows
            .borrow_mut()
            .push(vec![row("g1", "chat-1", "hello")]);
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        write(&probe, "c1", &cmd["action"], &state).unwrap();
        let calls = probe.calls.borrow();
        let marker = calls
            .iter()
            .position(|call| call == "messages.mark_policy_used")
            .unwrap();
        let send = calls
            .iter()
            .position(|call| call == "messages.send_part")
            .unwrap();
        assert!(marker < send);
        drop(calls);
        let mut again = chat_probe();
        again.agent = probe.agent.clone();
        again.policy_used_fails = true;
        let state = prepared(&again, &cmd);
        assert!(write(&again, "c1", &cmd["action"], &state).is_err());
        assert_eq!(sent(&again), 0);
    }

    #[test]
    fn a_staging_failure_is_a_failed_no_effect_and_nothing_is_sent() {
        let mut probe = chat_probe();
        probe.stage = json!({"failed": {"domain": "NSCocoaErrorDomain", "code": 4}});
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(
            (
                written["disposition"].as_str(),
                written["reason"].as_str(),
                written["observed"].as_bool()
            ),
            (
                Some("failed_no_effect"),
                Some("staging_failed"),
                Some(false)
            )
        );
        assert_eq!(sent(&probe), 0);
        assert_eq!(
            settle(SEND, &written).unwrap().disposition,
            "failed_no_effect"
        );
    }

    #[test]
    fn a_part_that_never_appears_leaves_the_rest_unsent_and_the_command_indeterminate() {
        let mut probe = chat_probe();
        probe.stage = json!({"parts": [{"kind": "text", "text": "hello"}, {"kind": "attachment", "name": "0-a.txt"}]});
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(written["disposition"], "indeterminate");
        assert_eq!(written["parts"][1]["disposition"], "not_dispatched");
        assert_eq!(written["parts"][1]["reason"], "prior_part_indeterminate");
        assert_eq!(sent(&probe), 1);
        assert_eq!(written["observed"], false);
        assert!(
            !probe
                .calls
                .borrow()
                .iter()
                .any(|call| call == "messages.reconcile")
        );
    }

    #[test]
    fn two_matching_rows_stop_the_command_as_ambiguous() {
        let probe = chat_probe();
        probe.rows.borrow_mut().push(vec![
            row("g1", "chat-1", "hello"),
            row("g2", "chat-1", "hello"),
        ]);
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(written["disposition"], "indeterminate");
        assert_eq!(written["parts"][0]["ambiguous"], true);
        assert_eq!(settle(SEND, &written).unwrap().disposition, "indeterminate");
    }

    #[test]
    fn a_text_and_a_file_are_sent_one_after_the_other_and_aggregated() {
        let mut probe = chat_probe();
        probe.stage = json!({"parts": [{"kind": "text", "text": "hello"}, {"kind": "attachment", "name": "0-a.txt"}]});
        let mut with_file = row("g2", "chat-1", "");
        with_file["attachment_names"] = json!(["0-a.txt"]);
        let mut delivered = row("g1", "chat-1", "hello");
        delivered["is_delivered"] = json!(true);
        probe.rows.borrow_mut().push(vec![delivered, with_file]);
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(written["disposition"], "locally_observed");
        assert_eq!(sent(&probe), 2);
        assert_eq!(written["parts"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn outside_the_lab_no_proof_is_recorded_and_other_operations_are_refused() {
        let mut probe = chat_probe();
        probe.lab = false;
        probe.proofs = vec![route_proof(
            &json!({"kind": "chat_id_raw", "chat": "chat-1"}),
            "text",
        )];
        probe
            .rows
            .borrow_mut()
            .push(vec![row("g1", "chat-1", "hello")]);
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert!(
            !probe
                .calls
                .borrow()
                .iter()
                .any(|call| call == "messages.record_proof")
        );
        let mut chat_op = direct("hi");
        chat_op["action"]["op"] = json!("messages.explode");
        assert_eq!(
            write(&probe, "c1", &chat_op["action"], &state).unwrap_err(),
            "unsupported"
        );
        assert!(settle("messages.explode", &json!({"disposition": "delivered"})).is_none());
    }

    #[test]
    fn resolve_observes_only_parts_the_store_shows_and_keeps_unsent_parts_unsent() {
        let probe = chat_probe();
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        let saved = json!({"action": cmd["action"], "native_state": state, "request_hash": "h1"});
        let evidence = json!({"prepared": saved, "parts": [], "observed": false});
        assert!(resolved(&probe, &evidence).is_none());
        probe
            .rows
            .borrow_mut()
            .push(vec![row("g1", "chat-1", "hello")]);
        let found = resolved(&probe, &evidence).unwrap();
        assert_eq!(
            (
                found["native_key"].as_str(),
                found["disposition"].as_str(),
                found["fingerprint"].as_str()
            ),
            (Some("g1"), Some("locally_observed"), Some("h1"))
        );
        assert_eq!(found["prepared"], saved);
        let other = Fake::new();
        other
            .rows
            .borrow_mut()
            .push(vec![row("g9", "chat-9", "hello")]);
        assert!(resolved(&other, &evidence).is_none());
        let mut edit = evidence.clone();
        edit["prepared"]["action"]["op"] = json!("messages.edit");
        assert!(resolved(&probe, &edit).is_none());
        let mut two = cmd.clone();
        two["action"]["messages"]["files"] = json!([{"name": "a.txt", "data": ""}]);
        let saved = json!({"action": two["action"], "native_state": prepared(&probe, &two), "request_hash": "h2"});
        let withheld = json!({"prepared": saved, "parts": [{"index": 1, "kind": "attachment", "disposition": "not_dispatched"}]});
        let found = resolved(&probe, &withheld).unwrap();
        assert_eq!(found["parts"][1]["disposition"], "not_dispatched");
        assert_eq!(found["disposition"], "indeterminate");
    }

    #[test]
    fn parts_past_the_dispatch_deadline_are_never_sent() {
        let mut probe = chat_probe();
        probe.stage = json!({"parts": [{"kind": "text", "text": "hello"}, {"kind": "text", "text": "again"}]});
        probe.send_delay = Duration::from_millis(400);
        probe
            .rows
            .borrow_mut()
            .push(vec![row("g1", "chat-1", "hello")]);
        let cmd = direct("hello");
        let state = prepared(&probe, &cmd);
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(sent(&probe), 1);
        assert_eq!(written["parts"][1]["disposition"], "not_dispatched");
        assert!(written["parts"][1]["reason"].is_null());
        assert_eq!(written["disposition"], "indeterminate");
    }

    #[test]
    fn an_unobserved_aggregate_surfaces_the_reason_of_its_parts() {
        let ghost =
            json!({"disposition": "delivery_failed", "reason": "empty_unjoined_sms", "index": 0});
        let result = aggregate(&[ghost], "fp", &Value::Null);
        assert_eq!(
            (result["reason"].as_str(), result["observed"].as_bool()),
            (Some("empty_unjoined_sms"), Some(false))
        );
        assert!(result["native_key"].is_null());
        let seen = json!({"disposition": "locally_observed", "observed": true, "native_key": "g1", "reason": "x"});
        assert!(aggregate(&[seen], "fp", &Value::Null)["reason"].is_null());
    }

    fn message_target(text_body: &str, sent_ago: f64) -> Value {
        let sent = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_secs_f64(sent_ago))
            .unwrap();
        json!({"exists": true,
            "target": {"text": text_body, "is_from_me": true, "edit_history": [], "retracted_parts": [], "retracted_at": null,
                "deleted_at": null, "sent_at": sent.to_string()},
            "row": {"guid": "m1", "capture_chat_guid": "chat-1", "service": "iMessage", "is_from_me": true, "item_type": 0}})
    }

    fn targeted(operation: &str, extra: &Value) -> Value {
        let mut action =
            json!({"op": operation, "native_key": "m1", "messages": {"text": "new words"}});
        for (key, value) in extra.as_object().unwrap() {
            action["messages"][key] = value.clone();
        }
        json!({"command_id": "c1", "principal": "device:abc", "origin": "interactive", "expires_at": "2099-01-01T00:00:00Z",
            "request_hash": "h", "action": action})
    }

    fn ui_probe(text_body: &str, sent_ago: f64) -> Fake {
        let mut probe = chat_probe();
        probe
            .targets
            .borrow_mut()
            .insert("m1".into(), message_target(text_body, sent_ago));
        probe
            .chats
            .insert("chat-1".into(), vec![json!("+15550100100")]);
        probe
    }

    #[test]
    fn an_edit_is_prepared_inside_its_window_and_refused_outside_it() {
        let cmd = targeted("messages.edit", &json!({}));
        let state = prepare(&ui_probe("old", 60.0), &cmd).unwrap();
        assert_eq!(state["route"], "accessibility");
        assert_eq!(state["candidate_ids"], json!(["m1"]));
        assert_eq!(state["self_chat"], true);
        assert!(prepare(&ui_probe("old", 899.0), &cmd).is_ok());
        assert_eq!(
            prepare(&ui_probe("old", 901.0), &cmd).unwrap_err().code,
            "unsupported"
        );
        let unsend = targeted("messages.unsend", &json!({}));
        assert!(prepare(&ui_probe("old", 100.0), &unsend).is_ok());
        assert_eq!(
            prepare(&ui_probe("old", 121.0), &unsend).unwrap_err().code,
            "unsupported"
        );
        let capped = ui_probe("old", 60.0);
        let mut target = message_target("old", 60.0);
        target["target"]["edit_history"] = json!([1, 2, 3, 4, 5]);
        capped.targets.borrow_mut().insert("m1".into(), target);
        assert_eq!(prepare(&capped, &cmd).unwrap_err().code, "unsupported");
        let mut sms = message_target("old", 60.0);
        sms["row"]["service"] = json!("SMS");
        let texted = ui_probe("old", 60.0);
        texted.targets.borrow_mut().insert("m1".into(), sms);
        assert_eq!(prepare(&texted, &cmd).unwrap_err().code, "unsupported");
        let mut theirs = message_target("old", 60.0);
        theirs["row"]["is_from_me"] = json!(false);
        let incoming = ui_probe("old", 60.0);
        incoming.targets.borrow_mut().insert("m1".into(), theirs);
        assert_eq!(prepare(&incoming, &cmd).unwrap_err().code, "unsupported");
    }

    #[test]
    fn a_gated_ui_operation_needs_a_tagged_target_and_a_tagged_text() {
        let mut gated = targeted("messages.edit", &json!({"text": "[ab-test x] new"}));
        gated["origin"] = json!("");
        let probe = ui_probe("[ab-test x] old", 60.0);
        assert!(prepare(&probe, &gated).is_ok());
        let untagged_target = ui_probe("old", 60.0);
        assert_eq!(
            prepare(&untagged_target, &gated).unwrap_err().code,
            "invalid"
        );
        let mut untagged_text = targeted("messages.edit", &json!({"text": "new"}));
        untagged_text["origin"] = json!("");
        assert_eq!(prepare(&probe, &untagged_text).unwrap_err().code, "invalid");
    }

    #[test]
    fn deleted_missing_stale_and_unproven_targets_are_refused() {
        let cmd = targeted("messages.edit", &json!({}));
        assert_eq!(
            prepare(&chat_probe(), &cmd).unwrap_err().code,
            "capability_unproven"
        );
        let deleted = ui_probe("old", 60.0);
        let mut target = message_target("old", 60.0);
        target["target"]["deleted_at"] = json!("2026-01-01T00:00:00Z");
        deleted.targets.borrow_mut().insert("m1".into(), target);
        assert_eq!(prepare(&deleted, &cmd).unwrap_err().code, "unsupported");
        let mut stale = cmd.clone();
        stale["expected_state"] = json!({"text": "something else"});
        assert_eq!(
            prepare(&ui_probe("old", 60.0), &stale).unwrap_err().code,
            "conflict"
        );
        let mut agreeing = cmd.clone();
        agreeing["expected_state"] = json!({"text": "old"});
        assert!(prepare(&ui_probe("old", 60.0), &agreeing).is_ok());
        let mut unproven = ui_probe("old", 60.0);
        unproven.lab = false;
        assert_eq!(
            prepare(&unproven, &cmd).unwrap_err().code,
            "capability_unproven"
        );
        unproven.proofs = vec![route_proof(
            &json!({"kind": "imessage_buddy", "handle": "x", "service": "iMessage"}),
            "messages.edit",
        )];
        assert_eq!(
            prepare(&unproven, &cmd).unwrap_err().code,
            "capability_unproven"
        );
        let mut blind = ui_probe("old", 60.0);
        blind.ui_available = false;
        assert_eq!(
            prepare(&blind, &cmd).unwrap_err().code,
            "capability_unproven"
        );
        let mut refused = ui_probe("old", 60.0);
        refused.ui_prepare_fails = true;
        assert_eq!(prepare(&refused, &cmd).unwrap_err().code, "unavailable");
        let mut anonymous = ui_probe("old", 60.0);
        anonymous.own = None;
        assert_eq!(prepare(&anonymous, &cmd).unwrap_err().code, "unavailable");
    }

    #[test]
    fn an_edit_is_observed_only_when_the_store_shows_the_new_text_and_a_longer_history() {
        let cmd = targeted("messages.edit", &json!({}));
        let probe = ui_probe("old", 60.0);
        let state = prepare(&probe, &cmd).unwrap();
        let mut edited = message_target("new words", 60.0);
        edited["target"]["edit_history"] = json!([{"text": "old"}]);
        probe.effects.borrow_mut().push(("m1".into(), edited));
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(
            (
                written["disposition"].as_str(),
                written["native_key"].as_str(),
                written["observed"].as_bool()
            ),
            (Some("locally_observed"), Some("m1"), Some(true))
        );
        assert_eq!(written["ui"], Value::Null);
        assert_eq!(written["target_verified"], true);
        let calls = probe.calls.borrow();
        assert!(calls.iter().any(|call| call == "messages.record_proof"));
        assert!(calls.iter().any(|call| call == "messages.reconcile"));
        drop(calls);
        let same_text = ui_probe("old", 60.0);
        let state = prepare(&same_text, &cmd).unwrap();
        let mut unchanged = message_target("new words", 60.0);
        unchanged["target"]["edit_history"] = json!([]);
        same_text
            .effects
            .borrow_mut()
            .push(("m1".into(), unchanged));
        let written = write(&same_text, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(written["disposition"], "indeterminate");
        assert!(
            !same_text
                .calls
                .borrow()
                .iter()
                .any(|call| call == "messages.reconcile")
        );
        let wrong_text = ui_probe("old", 60.0);
        let state = prepare(&wrong_text, &cmd).unwrap();
        let mut other = message_target("different", 60.0);
        other["target"]["edit_history"] = json!([{"text": "old"}]);
        wrong_text.effects.borrow_mut().push(("m1".into(), other));
        assert_eq!(
            write(&wrong_text, "c1", &cmd["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
    }

    #[test]
    fn a_target_changed_since_prepare_is_a_conflict_before_the_ui_acts() {
        let cmd = targeted("messages.edit", &json!({}));
        let probe = ui_probe("old", 60.0);
        let state = prepare(&probe, &cmd).unwrap();
        for (field, value) in [
            ("text", json!("edited by someone")),
            ("edit_history", json!([1])),
            ("retracted_parts", json!([0])),
            ("deleted_at", json!("2026-01-01T00:00:00Z")),
        ] {
            let mut moved = message_target("old", 60.0);
            moved["target"][field] = value;
            probe.targets.borrow_mut().insert("m1".into(), moved);
            assert_eq!(
                write(&probe, "c1", &cmd["action"], &state).unwrap_err(),
                "conflict",
                "{field}"
            );
        }
        probe.targets.borrow_mut().remove("m1");
        assert_eq!(
            write(&probe, "c1", &cmd["action"], &state).unwrap_err(),
            "conflict"
        );
        assert!(
            !probe
                .calls
                .borrow()
                .iter()
                .any(|call| call == "messages.ui_perform")
        );
    }

    #[test]
    fn unsend_and_delete_are_observed_through_the_store() {
        let unsend = targeted("messages.unsend", &json!({}));
        let probe = ui_probe("old", 60.0);
        let state = prepare(&probe, &unsend).unwrap();
        let mut retracted = message_target("old", 60.0);
        retracted["target"]["retracted_at"] = json!("2026-01-01T00:00:00Z");
        probe.effects.borrow_mut().push(("m1".into(), retracted));
        assert_eq!(
            write(&probe, "c1", &unsend["action"], &state).unwrap()["disposition"],
            "locally_observed"
        );
        let idle = ui_probe("old", 60.0);
        let state = prepare(&idle, &unsend).unwrap();
        assert_eq!(
            write(&idle, "c1", &unsend["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
        let delete = targeted("messages.delete_message", &json!({}));
        let gone = ui_probe("old", 60.0);
        let state = prepare(&gone, &delete).unwrap();
        gone.effects
            .borrow_mut()
            .push(("m1".into(), json!({"exists": false})));
        let written = write(&gone, "c1", &delete["action"], &state).unwrap();
        assert_eq!(
            (written["disposition"].as_str(), written["absent"].as_bool()),
            (Some("locally_observed"), Some(true))
        );
        let marked = ui_probe("old", 60.0);
        let state = prepare(&marked, &delete).unwrap();
        let mut hidden = message_target("old", 60.0);
        hidden["target"]["deleted_at"] = json!("2026-01-01T00:00:00Z");
        marked.effects.borrow_mut().push(("m1".into(), hidden));
        assert_eq!(
            write(&marked, "c1", &delete["action"], &state).unwrap()["absent"],
            false
        );
    }

    #[test]
    fn a_reply_and_a_reaction_need_exactly_one_new_row() {
        let reply = targeted("messages.reply", &json!({}));
        let probe = ui_probe("old", 60.0);
        let state = prepare(&probe, &reply).unwrap();
        assert_eq!(
            write(&probe, "c1", &reply["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
        probe.reply_guids.borrow_mut().push("r1".into());
        let written = write(&probe, "c1", &reply["action"], &state).unwrap();
        assert_eq!(
            (
                written["native_key"].as_str(),
                written["collection"].as_str()
            ),
            (Some("r1"), Some("messages.message"))
        );
        probe.reply_guids.borrow_mut().push("r2".into());
        assert_eq!(
            write(&probe, "c1", &reply["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
        let react = targeted("messages.react", &json!({"reaction": "love"}));
        let mut tapback = ui_probe("old", 60.0);
        tapback.reaction_state = vec![json!({"guid": "old-r", "type": "2000", "emoji": ""})];
        let state = prepare(&tapback, &react).unwrap();
        tapback
            .reaction_hits
            .borrow_mut()
            .push(json!({"guid": "old-r", "type": "2000", "emoji": ""}));
        assert_eq!(
            write(&tapback, "c1", &react["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
        tapback
            .reaction_hits
            .borrow_mut()
            .push(json!({"guid": "new-r", "type": "2000", "emoji": ""}));
        let written = write(&tapback, "c1", &react["action"], &state).unwrap();
        assert_eq!(
            (
                written["native_key"].as_str(),
                written["collection"].as_str()
            ),
            (Some("new-r"), Some("messages.reaction"))
        );
        let bad = targeted("messages.react", &json!({"reaction": "sparkles"}));
        let state = prepare(&tapback, &bad).unwrap();
        assert_eq!(
            write(&tapback, "c1", &bad["action"], &state).unwrap_err(),
            "invalid"
        );
        let emoji = targeted(
            "messages.react",
            &json!({"reaction": "emoji", "emoji": "x"}),
        );
        let picky = ui_probe("old", 60.0);
        let state = prepare(&picky, &emoji).unwrap();
        picky
            .reaction_hits
            .borrow_mut()
            .push(json!({"guid": "n", "type": "2006", "emoji": "y"}));
        assert_eq!(
            write(&picky, "c1", &emoji["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
    }

    #[test]
    fn the_ui_failing_does_not_hide_an_effect_the_store_shows() {
        let cmd = targeted("messages.edit", &json!({}));
        let mut probe = ui_probe("old", 60.0);
        probe.ui_perform_fails = true;
        let state = prepare(&probe, &cmd).unwrap();
        let mut edited = message_target("new words", 60.0);
        edited["target"]["edit_history"] = json!([{"text": "old"}]);
        probe
            .targets
            .borrow_mut()
            .insert("m1".into(), message_target("old", 60.0));
        let written = write(&probe, "c1", &cmd["action"], &state).unwrap();
        assert_eq!(written["target_verified"], false);
        assert_eq!(written["disposition"], "indeterminate");
        assert!(validate_send(&probe, &cmd["action"], &state).is_ok());
    }

    #[test]
    fn resolve_keeps_only_an_observed_ui_outcome() {
        let probe = Fake::new();
        let saved =
            json!({"action": {"op": "messages.edit"}, "request_hash": "h", "native_state": {}});
        let seen = json!({"prepared": saved, "observed": true, "native_key": "m1", "disposition": "locally_observed"});
        let found = resolved(&probe, &seen).unwrap();
        assert_eq!(
            (found["fingerprint"].as_str(), found["native_key"].as_str()),
            (Some("h"), Some("m1"))
        );
        let unseen = json!({"prepared": saved, "observed": false, "disposition": "indeterminate"});
        assert!(resolved(&probe, &unseen).is_none());
    }

    #[test]
    fn a_gated_ui_operation_needs_a_route_inside_the_approved_handles_or_proofs() {
        let mut gated = targeted("messages.edit", &json!({"text": "[ab-test x] new"}));
        gated["origin"] = json!("");
        let mut foreign = ui_probe("[ab-test x] old", 60.0);
        foreign
            .chats
            .insert("chat-1".into(), vec![json!("+15559990000")]);
        assert_eq!(prepare(&foreign, &gated).unwrap_err().code, "invalid");
        foreign.proofs = vec![route_proof(
            &json!({"kind": "chat_id_raw", "chat": "chat-1"}),
            "messages.edit",
        )];
        let state = prepare(&foreign, &gated).unwrap();
        assert_eq!(
            state["destination"],
            json!({"kind": "chat_id_raw", "chat": "chat-1"})
        );
        foreign.proofs = vec![route_proof(
            &json!({"kind": "chat_id_raw", "chat": "chat-1"}),
            "messages.react",
        )];
        assert_eq!(prepare(&foreign, &gated).unwrap_err().code, "invalid");
        let interactive = targeted("messages.edit", &json!({}));
        let mut anyone = ui_probe("old", 60.0);
        anyone
            .chats
            .insert("chat-1".into(), vec![json!("+15559990000")]);
        assert_eq!(
            prepare(&anyone, &interactive).unwrap()["destination"]["chat"],
            "chat-1"
        );
    }

    fn conversation_probe(stamp: &str) -> Fake {
        let mut probe = chat_probe();
        *probe.conversation.borrow_mut() =
            json!({"exists": true, "row_position": 50, "last_read_message_timestamp": stamp});
        probe.conversation_rows = vec![
            json!({"guid": "m1", "date": "100"}),
            json!({"guid": "m0", "date": "90"}),
        ];
        probe
            .chats
            .insert("chat-1".into(), vec![json!("+15550100100")]);
        let mut newest = message_target("[ab-test r1] hello", 60.0);
        newest["row"]["row_position"] = json!(20);
        probe.targets.borrow_mut().insert("m1".into(), newest);
        probe
            .targets
            .borrow_mut()
            .insert("m0".into(), message_target("[ab-test r1] earlier", 90.0));
        probe.ledger = Some(
            json!({"run_id": "r1", "scenario": "MSG-SELFTEST", "baseline_max_row_id": 10,
            "created_chat_key": "chat-1", "created_by_run": true, "baseline_max_chat_row_id": 40}),
        );
        probe
    }

    fn conversation_command(operation: &str, origin: &str) -> Value {
        json!({"command_id": "c1", "principal": "device:abc", "origin": origin, "expires_at": "2099-01-01T00:00:00Z",
            "request_hash": "h", "action": {"op": operation, "native_key": "chat-1", "origin": if origin.is_empty() { Value::Null } else { json!(origin) }}})
    }

    #[test]
    fn an_interactive_conversation_operation_prepares_with_the_conversation_baseline() {
        let probe = conversation_probe("50");
        let state = prepare(
            &probe,
            &conversation_command("messages.mark_read", "interactive"),
        )
        .unwrap();
        assert_eq!(state["conversation_members"], json!(["m1", "m0"]));
        assert_eq!(state["message_guid"], "m1");
        assert_eq!(state["last_read_timestamp"], "50");
        assert_eq!(state["latest_message_date"], "100");
        assert_eq!(state["candidate_ids"], json!(["chat-1"]));
        assert_eq!(state["route"], "accessibility");
        assert!(
            prepare(
                &probe,
                &conversation_command("messages.delete_chat", "interactive")
            )
            .is_ok()
        );
        assert!(
            probe
                .calls
                .borrow()
                .iter()
                .any(|call| call == "messages.ui_activate_validate")
        );
        let reads = conversation_probe("50");
        prepare(
            &reads,
            &conversation_command("messages.mark_unread", "interactive"),
        )
        .unwrap();
        assert!(
            !reads
                .calls
                .borrow()
                .iter()
                .any(|call| call == "messages.ui_activate_validate")
        );
        let gone = chat_probe();
        assert_eq!(
            prepare(
                &gone,
                &conversation_command("messages.mark_read", "interactive")
            )
            .unwrap_err()
            .code,
            "capability_unproven"
        );
        let mut empty = conversation_probe("50");
        empty.conversation_rows = Vec::new();
        assert_eq!(
            prepare(
                &empty,
                &conversation_command("messages.mark_read", "interactive")
            )
            .unwrap_err()
            .code,
            "capability_unproven"
        );
        let mut blind = conversation_probe("50");
        blind.ui_available = false;
        assert_eq!(
            prepare(
                &blind,
                &conversation_command("messages.mark_read", "interactive")
            )
            .unwrap_err()
            .code,
            "capability_unproven"
        );
        let mut unproven = conversation_probe("50");
        unproven.lab = false;
        assert_eq!(
            prepare(
                &unproven,
                &conversation_command("messages.mark_read", "interactive")
            )
            .unwrap_err()
            .code,
            "capability_unproven"
        );
        let mut refused = conversation_probe("50");
        refused.activate_fails = true;
        assert_eq!(
            prepare(
                &refused,
                &conversation_command("messages.delete_chat", "interactive")
            )
            .unwrap_err()
            .code,
            "unavailable"
        );
    }

    #[test]
    fn a_gated_read_state_change_needs_a_tagged_target_and_an_authorising_ledger() {
        let gated = conversation_command("messages.mark_read", "");
        assert!(prepare(&conversation_probe("50"), &gated).is_ok());
        let untagged = conversation_probe("50");
        untagged
            .targets
            .borrow_mut()
            .insert("m1".into(), message_target("hello", 60.0));
        assert_eq!(prepare(&untagged, &gated).unwrap_err().code, "invalid");
        let mut missing = conversation_probe("50");
        missing.ledger = None;
        assert_eq!(prepare(&missing, &gated).unwrap_err().code, "unavailable");
        for (field, value) in [
            ("scenario", json!("MSG-OTHER")),
            ("baseline_max_row_id", json!(25)),
            ("run_id", json!("r2")),
            ("created_chat_key", json!("chat-9")),
        ] {
            let mut probe = conversation_probe("50");
            probe.ledger.as_mut().unwrap()[field] = value;
            assert_eq!(
                prepare(&probe, &gated).unwrap_err().code,
                "invalid",
                "{field}"
            );
        }
        let mut stranger = conversation_probe("50");
        stranger.self_chat = false;
        assert_eq!(prepare(&stranger, &gated).unwrap_err().code, "invalid");
        for flag in ["foreign_unread", "foreign_new"] {
            let mut busy = conversation_probe("50");
            busy.foreign_activity[flag] = json!(true);
            assert_eq!(
                prepare(&busy, &gated).unwrap_err().code,
                "conflict",
                "{flag}"
            );
        }
        let mut foreign_chat = conversation_probe("50");
        foreign_chat
            .chats
            .insert("chat-1".into(), vec![json!("+15559990000")]);
        assert_eq!(prepare(&foreign_chat, &gated).unwrap_err().code, "invalid");
    }

    #[test]
    fn a_gated_chat_delete_needs_every_message_tagged_and_a_ledger_for_this_chat() {
        let gated = conversation_command("messages.delete_chat", "");
        assert!(prepare(&conversation_probe("50"), &gated).is_ok());
        let untagged_newest = conversation_probe("50");
        untagged_newest
            .targets
            .borrow_mut()
            .insert("m1".into(), message_target("hello", 60.0));
        assert_eq!(
            prepare(&untagged_newest, &gated).unwrap_err().code,
            "invalid"
        );
        let untagged_older = conversation_probe("50");
        untagged_older
            .targets
            .borrow_mut()
            .insert("m0".into(), message_target("hello", 90.0));
        assert_eq!(
            prepare(&untagged_older, &gated).unwrap_err().code,
            "unsupported"
        );
        let other_run = conversation_probe("50");
        other_run
            .targets
            .borrow_mut()
            .insert("m0".into(), message_target("[ab-test r2] earlier", 90.0));
        assert_eq!(prepare(&other_run, &gated).unwrap_err().code, "invalid");
        for (field, value) in [
            ("created_by_run", json!(false)),
            ("created_chat_key", json!("chat-9")),
            ("baseline_max_chat_row_id", json!(60)),
            ("run_id", json!("r2")),
        ] {
            let mut probe = conversation_probe("50");
            probe.ledger.as_mut().unwrap()[field] = value;
            assert_eq!(
                prepare(&probe, &gated).unwrap_err().code,
                "unsupported",
                "{field}"
            );
        }
        let mut missing = conversation_probe("50");
        missing.ledger = None;
        assert_eq!(prepare(&missing, &gated).unwrap_err().code, "unavailable");
    }

    #[test]
    fn a_conversation_that_changed_since_prepare_is_a_conflict_before_the_ui_acts() {
        let cmd = conversation_command("messages.mark_read", "interactive");
        let probe = conversation_probe("50");
        let state = prepare(&probe, &cmd).unwrap();
        *probe.conversation.borrow_mut() =
            json!({"exists": true, "row_position": 50, "last_read_message_timestamp": "60"});
        assert_eq!(
            write(&probe, "c1", &cmd["action"], &state).unwrap_err(),
            "conflict"
        );
        *probe.conversation.borrow_mut() =
            json!({"exists": true, "row_position": 50, "last_read_message_timestamp": "50"});
        let mut more = conversation_probe("50");
        more.conversation_rows
            .insert(0, json!({"guid": "m2", "date": "110"}));
        assert_eq!(
            write(&more, "c1", &cmd["action"], &state).unwrap_err(),
            "conflict"
        );
        let mut later = conversation_probe("50");
        later.conversation_rows[0]["date"] = json!("101");
        assert_eq!(
            write(&later, "c1", &cmd["action"], &state).unwrap_err(),
            "conflict"
        );
        let gated = conversation_command("messages.mark_read", "");
        let mut moved = conversation_probe("50");
        let gated_state = prepare(&moved, &gated).unwrap();
        moved
            .chats
            .insert("chat-1".into(), vec![json!("+15559990000")]);
        assert_eq!(
            write(&moved, "c1", &gated["action"], &gated_state).unwrap_err(),
            "invalid"
        );
        assert!(![&probe, &more, &later, &moved].iter().any(|p| {
            p.calls
                .borrow()
                .iter()
                .any(|call| call == "messages.ui_perform")
        }));
    }

    #[test]
    fn a_read_state_change_is_observed_only_when_the_timestamp_catches_up() {
        let read = conversation_command("messages.mark_read", "interactive");
        let probe = conversation_probe("50");
        let state = prepare(&probe, &read).unwrap();
        *probe.conversation_effect.borrow_mut() =
            Some(json!({"exists": true, "row_position": 50, "last_read_message_timestamp": "100"}));
        let written = write(&probe, "c1", &read["action"], &state).unwrap();
        assert_eq!(
            (
                written["disposition"].as_str(),
                written["native_key"].as_str(),
                written["collection"].as_str()
            ),
            (
                Some("locally_observed"),
                Some("chat-1"),
                Some("messages.chat")
            )
        );
        let stuck = conversation_probe("50");
        let state = prepare(&stuck, &read).unwrap();
        let written = write(&stuck, "c1", &read["action"], &state).unwrap();
        assert_eq!(
            (written["disposition"].as_str(), written["reason"].as_str()),
            (Some("indeterminate"), Some("conversation_effect_unproven"))
        );
        assert!(
            !stuck
                .calls
                .borrow()
                .iter()
                .any(|call| call == "messages.reconcile")
        );
        let unread = conversation_command("messages.mark_unread", "interactive");
        let reading = conversation_probe("100");
        let state = prepare(&reading, &unread).unwrap();
        assert_eq!(
            write(&reading, "c1", &unread["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
        *reading.conversation_effect.borrow_mut() =
            Some(json!({"exists": true, "row_position": 50, "last_read_message_timestamp": "10"}));
        assert_eq!(
            write(&reading, "c1", &unread["action"], &state).unwrap()["disposition"],
            "locally_observed"
        );
    }

    #[test]
    fn a_chat_delete_is_observed_when_the_chat_is_gone_or_recoverable() {
        let delete = conversation_command("messages.delete_chat", "interactive");
        let gone = conversation_probe("50");
        let state = prepare(&gone, &delete).unwrap();
        *gone.conversation_effect.borrow_mut() = Some(json!({"exists": false}));
        let written = write(&gone, "c1", &delete["action"], &state).unwrap();
        assert_eq!(
            (written["disposition"].as_str(), written["absent"].as_bool()),
            (Some("locally_observed"), Some(true))
        );
        let recoverable = conversation_probe("50");
        let state = prepare(&recoverable, &delete).unwrap();
        *recoverable.recoverable.borrow_mut() = true;
        assert_eq!(
            write(&recoverable, "c1", &delete["action"], &state).unwrap()["absent"],
            false
        );
        let kept = conversation_probe("50");
        let state = prepare(&kept, &delete).unwrap();
        assert_eq!(
            write(&kept, "c1", &delete["action"], &state).unwrap()["disposition"],
            "indeterminate"
        );
    }
}
