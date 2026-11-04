use super::owned::{Probe, Refusal, native_failure, refuse, text};
use crate::messages_policy::{Failure, Origin, Refusal as Policy, check_with, normalize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const SEND: &str = "messages.send";
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
    json!({
        "source": "messages",
        "collection": "messages.message",
        "native_key": native_key,
        "observed": parts.iter().any(|part| part["observed"] == true),
        "disposition": disposition,
        "parts": parts,
        "fingerprint": fingerprint,
        "baseline": baseline,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};

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
        let mut edit = direct("hi");
        edit["action"]["op"] = json!("messages.edit");
        assert_eq!(
            prepare(&Fake::new(), &edit).unwrap_err().code,
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
}
