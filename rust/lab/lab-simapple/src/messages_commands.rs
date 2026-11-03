use super::media;
use super::{
    Duration, Instant, Result, Session, Shared, State, Ticket, Value, id, invalid, json, lock, now,
    send, string,
};
use std::os::unix::fs::MetadataExt;

const EDIT_WINDOW_SECS: i64 = 15 * 60;
const UNSEND_WINDOW_SECS: i64 = 2 * 60;
const EDIT_LIMIT: usize = 5;
const FAULTS: [&str; 13] = [
    "delivery-failed",
    "ghost-send",
    "receipt-before-outcome",
    "outcome-before-capture",
    "outcome-without-capture",
    "part-no-effect",
    "part-delivery-failed",
    "part-progress",
    "part-progress-failure",
    "part-progress-stale",
    "part-receipt-failed",
    "part-receipt-failed-first",
    "part-unbound-failure",
];

pub(super) fn fault_known(name: &str) -> bool {
    FAULTS.contains(&name)
}

fn parse_time(value: &Value) -> Option<jiff::Timestamp> {
    value.as_str()?.parse().ok()
}
fn plus(at: jiff::Timestamp, seconds: i64) -> Value {
    at.checked_add(jiff::SignedDuration::from_secs(seconds))
        .map_or(Value::Null, |t| json!(t.to_string()))
}
fn fill(data: &mut Value, defaults: &Value) {
    let (Some(target), Some(source)) = (data.as_object_mut(), defaults.as_object()) else {
        return;
    };
    for (field, value) in source {
        target.entry(field.clone()).or_insert_with(|| value.clone());
    }
}
fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref":{"collection":format!("messages.{collection}"),"native_key":key}})
}
pub(super) fn facts(records: &Value, collection: &str, data: &mut Value) {
    if !data.is_object() {
        return;
    }
    match collection.trim_start_matches("messages.") {
        "message" => message_facts(records, data),
        "chat" => fill(
            data,
            &json!({"display_name":"","capabilities":[],"filter":"known","is_archived":false,"is_spam":false,"read_receipts":"default"}),
        ),
        "attachment" => fill(
            data,
            &json!({"genmoji":false,"hidden":false,"is_sticker":false,"uti":"public.data"}),
        ),
        "reaction" => fill(data, &json!({"sent_at":now()})),
        _ => {}
    }
}
fn message_facts(records: &Value, data: &mut Value) {
    if !data["service"].is_string() {
        let chat = string(&data["chat_id"]["$ref"], "native_key");
        let service = records[&chat]["data"]["service"]
            .as_str()
            .filter(|service| !service.is_empty())
            .unwrap_or("iMessage");
        data["service"] = json!(service);
    }
    fill(
        data,
        &json!({"kind":"message","decode_status":"decoded","error_code":0,"edit_history":[],"retracted_parts":[],"formatting":[],
            "audio":{"is_audio_message":false,"expirable":false},"delivery":{"quietly":false,"notified_anyway":false},
            "is_spam":false,"has_unseen_mention":false,"via_satellite":false,"is_sos":false}),
    );
    if data.get("parts").is_none() {
        data["parts"] = json!([]);
        if let Some(text) = data["text"].as_str().filter(|text| !text.is_empty()) {
            data["parts"] = json!([{"index":0,"kind":"text","text":text}]);
        }
    }
    if let Some(object) = data.as_object_mut() {
        object.remove("edit_until");
        object.remove("unsend_until");
    }
    let Some(sent) = parse_time(&data["sent_at"]) else {
        return;
    };
    if data["service"] != "iMessage" || data["is_from_me"] != true || data["kind"] == "group_event"
    {
        return;
    }
    if data["edit_history"].as_array().map_or(0, Vec::len) < EDIT_LIMIT {
        data["edit_until"] = plus(sent, EDIT_WINDOW_SECS);
    }
    data["unsend_until"] = plus(sent, UNSEND_WINDOW_SECS);
}
fn window_open(data: &Value, edit: bool) -> bool {
    let field = if edit { "edit_until" } else { "unsend_until" };
    let (Some(sent), Some(until)) = (parse_time(&data["sent_at"]), parse_time(&data[field])) else {
        return false;
    };
    let current = jiff::Timestamp::now();
    current >= sent && current < until
}
fn own_handle(handle: &str) -> bool {
    handle == "self@example.com" || handle == "second-self@example.com"
}
fn own_key(key: &str) -> bool {
    key == "message-self" || key == "message-self-email"
}
fn handle_key(handle: &str) -> String {
    if handle == "self@example.com" {
        "message-self".into()
    } else if handle == "second-self@example.com" {
        "message-self-email".into()
    } else {
        format!("synthetic-{}", handle.split('@').next().unwrap_or_default())
    }
}
fn own_recipients(s: &State, action: &Value) -> bool {
    let list = string(action, "list_key");
    if list.is_empty() {
        return action["messages"]["handles"]
            .as_array()
            .is_some_and(|h| h.len() == 1 && h[0].as_str().is_some_and(own_handle));
    }
    let peers = s.data["records"][&list]["data"]["participants"].as_array();
    peers.is_some_and(|peers| {
        !peers.is_empty()
            && peers
                .iter()
                .all(|peer| own_key(&string(&peer["$ref"], "native_key")))
    })
}
fn upload_boundary(s: &State, path: &str) -> bool {
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return false;
    };
    let Some(parent) = resolved.parent() else {
        return false;
    };
    let (Ok(parent_meta), Ok(file_meta), Ok(owner)) = (
        std::fs::metadata(parent),
        std::fs::metadata(&resolved),
        std::fs::metadata(&s.path),
    ) else {
        return false;
    };
    parent.file_name().is_some_and(|n| n == "message-uploads")
        && parent_meta.is_dir()
        && parent_meta.mode() & 0o7777 == 0o700
        && file_meta.is_file()
        && parent_meta.uid() == owner.uid()
        && file_meta.uid() == owner.uid()
}
type Problem = (&'static str, &'static str);
fn problem(code: &'static str) -> Problem {
    (code, code)
}
fn validate(s: &State, action: &Value) -> Option<Problem> {
    let m = &action["messages"];
    if !m.is_object() {
        return Some(problem("invalid"));
    }
    let op = string(action, "op");
    let origin = string(action, "origin");
    let resend = string(m, "resend_of");
    if !resend.is_empty() {
        let previous = &s.data["records"][&resend]["data"];
        if previous["is_from_me"] != true || previous["error_code"].as_i64().unwrap_or(0) == 0 {
            return Some(problem("conflict"));
        }
    }
    if s.denied.get("messages") == Some(&true) || s.data["message_capability_unproven"] == true {
        return Some(problem("capability_unproven"));
    }
    for file in m["files"].as_array().into_iter().flatten() {
        let path = string(file, "path");
        if path.is_empty() || string(file, "digest").is_empty() || !upload_boundary(s, &path) {
            return Some(problem("invalid"));
        }
    }
    let text = string(m, "text");
    if origin != "interactive"
        && !text.is_empty()
        && (!text.contains("[ab-test ") || !text.contains(']'))
        && (origin != "owner_self" || op != "messages.send" || !own_recipients(s, action))
    {
        return Some((
            "invalid",
            "untagged text requires a local owner token and verified own recipients; other sends require [ab-test <id>]",
        ));
    }
    if origin != "interactive"
        && op == "messages.send"
        && m["handles"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|h| !h.as_str().is_some_and(own_handle))
    {
        return Some(problem("unauthorized"));
    }
    let target = string(action, "native_key");
    let mut data = Value::Null;
    let mut chat = string(action, "list_key");
    if !target.is_empty() {
        let record = &s.data["records"][&target];
        if record.is_null() || record["kind"] == "absent" {
            return Some(problem("not_found"));
        }
        data = record["data"].clone();
        if data.is_object() {
            chat = if record["collection"] == "messages.chat" {
                target.clone()
            } else {
                string(&data["chat_id"]["$ref"], "native_key")
            };
        }
    }
    if !chat.is_empty() {
        let record = &s.data["records"][&chat];
        if record.is_null() || record["kind"] == "absent" {
            return Some(problem("not_found"));
        }
        if origin != "interactive" && (op == "messages.send" || op == "messages.reply") {
            for peer in record["data"]["participants"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if !own_key(&string(&peer["$ref"], "native_key")) {
                    return Some(problem("unauthorized"));
                }
            }
        }
    }
    if matches!(op.as_str(), "messages.edit" | "messages.unsend")
        && !window_open(&data, op == "messages.edit")
    {
        return Some(problem("unsupported"));
    }
    None
}
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(l, r)| same(l, r))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|other| same(v, other)))
        }
        _ => a == b,
    }
}
fn put(
    s: &mut State,
    seen: &mut Vec<Value>,
    collection: &str,
    key: &str,
    mut data: Value,
    absent: bool,
) {
    if !absent {
        facts(&s.data["records"], collection, &mut data);
    }
    let name = format!("messages.{collection}");
    let record = json!({"kind":if absent{"absent"}else{"upsert"},"collection":name,"native_key":key,"scoped_alias":format!("{name}/{key}"),"data":data,"observed_at":now(),"revalidated":true});
    s.put(record.clone());
    seen.push(record);
}
fn new_message(
    s: &mut State,
    seen: &mut Vec<Value>,
    key: &str,
    text: &str,
    chat: &str,
    mine: bool,
) {
    put(
        s,
        seen,
        "message",
        key,
        json!({"chat_id":reference("chat",chat),"text":text,"parts":[{"index":0,"kind":"text","text":text}],"is_from_me":mine,"sent_at":now(),"decode_status":"decoded","edit_history":[],"retracted_parts":[],"kind":"message","error_code":0}),
        false,
    );
}
fn record_data(s: &State, key: &str) -> Value {
    let data = &s.data["records"][key]["data"];
    if data.is_object() {
        data.clone()
    } else {
        json!({})
    }
}
fn set_error(s: &mut State, seen: &mut Vec<Value>, key: &str) {
    let mut data = record_data(s, key);
    data["error_code"] = json!(100);
    put(s, seen, "message", key, data, false);
}

impl Session<'_> {
    fn reject_with(&mut self, frame: &Value, code: &str, message: &str) -> Result<()> {
        send(
            &mut self.stream,
            &json!({"t":"Rejected","command_id":frame["command_id"],"disposition":"rejected_before_dispatch","error":{"code":code,"message":message}}),
        )?;
        Ok(())
    }
    pub(super) fn message_command(&mut self, state: &Shared, frame: &Value) -> Result<bool> {
        let command = string(frame, "command_id");
        if frame["t"] == "Prepare" && string(&frame["action"], "op").starts_with("messages.") {
            self.message_prepare(state, frame, &command)?;
            return Ok(true);
        }
        if frame["t"] == "Dispatch"
            && self
                .tickets
                .get(&string(frame, "ticket"))
                .is_some_and(|t| string(&t.prepare["action"], "op").starts_with("messages."))
        {
            self.message_dispatch(state, frame, &command)?;
            return Ok(true);
        }
        if frame["t"] == "Resolve" {
            let s = lock(state)?;
            let outcome = s.data["evidence"][&command].clone();
            if outcome["evidence"]["source"] == "messages" {
                drop(s);
                self.message_resolve(state, outcome)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn message_prepare(&mut self, state: &Shared, frame: &Value, command: &str) -> Result<()> {
        let mut s = lock(state)?;
        let action = &frame["action"];
        let principal = string(frame, "principal");
        let origin = string(frame, "origin");
        let invalid_envelope = !action.is_object()
            || principal.is_empty()
            || string(frame, "request_hash").is_empty()
            || !(origin.is_empty() || origin == "interactive" || origin == "owner_self")
            || origin == "owner_self" && !principal.starts_with("token:")
            || string(action, "origin") != origin
            || origin == "interactive"
                && !(principal.starts_with("device:")
                    || principal.starts_with("agent:")
                    || principal.contains("/device:"));
        if invalid_envelope {
            drop(s);
            return self.reject(frame, "invalid");
        }
        if let Some((code, message)) = validate(&s, action) {
            drop(s);
            return self.reject_with(frame, code, message);
        }
        if s.fault == "ui-interference-before" {
            s.fault.clear();
            drop(s);
            return self.reject(frame, "conflict");
        }
        let key = string(action, "native_key");
        let before = s.data["records"][&key]["data"].clone();
        if !key.is_empty() && frame["expected_state"].is_object() {
            for field in [
                "text",
                "sent_at",
                "is_from_me",
                "edit_history",
                "retracted_parts",
                "display_name",
                "service",
                "pinned_index",
                "muted",
                "read_receipts",
                "deleted_at",
            ] {
                if let Some(value) = frame["expected_state"].get(field)
                    && !same(value, &before[field])
                {
                    drop(s);
                    return self.reject(frame, "conflict");
                }
            }
        }
        self.tickets.retain(|_, t| Instant::now() < t.deadline);
        if !self.tickets.is_empty() {
            drop(s);
            return self.reject(frame, "unavailable");
        }
        let ticket = id("ticket")?;
        let millis = string(frame, "expires_at")
            .parse::<jiff::Timestamp>()
            .map_or(0, |at| {
                (at.as_millisecond() - jiff::Timestamp::now().as_millisecond()).clamp(0, 30_000)
            });
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: before.clone(),
                deadline: Instant::now() + Duration::from_millis(millis.cast_unsigned()),
            },
        );
        let maximum = s.data["records"]
            .as_object()
            .map_or(0, serde_json::Map::len);
        drop(s);
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":before,"route":"sim-messages","fingerprint":frame["request_hash"],"candidate_ids":[maximum.to_string(),action["list_key"]]}),
        )?;
        Ok(())
    }
    #[allow(clippy::cognitive_complexity)]
    fn message_dispatch(&mut self, state: &Shared, frame: &Value, command: &str) -> Result<()> {
        let ticket = self
            .tickets
            .remove(&string(frame, "ticket"))
            .ok_or_else(|| invalid("ticket"))?;
        if ticket.prepare["command_id"] != command
            || frame["owner_fence"].as_u64() != Some(self.fence)
            || Instant::now() >= ticket.deadline
            || string(frame, "origin") != string(&ticket.prepare["action"], "origin")
        {
            return self.reject(frame, "expired");
        }
        let action = &ticket.prepare["action"];
        let mut s = lock(state)?;
        if let Some((code, message)) = validate(&s, action) {
            drop(s);
            return self.reject_with(frame, code, message);
        }
        let target = string(action, "native_key");
        if ticket.before != s.data["records"][&target]["data"] {
            drop(s);
            return self.reject(frame, "conflict");
        }
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        let fault = std::mem::take(&mut s.fault);
        if fault == "ui-interference" {
            drop(s);
            send(
                &mut self.stream,
                &json!({"t":"Outcome","command_id":command,"disposition":"indeterminate"}),
            )?;
            return Ok(());
        }
        let op = string(action, "op");
        let m = &action["messages"];
        let text = string(m, "text");
        let part_index = m["part_index"].as_u64().unwrap_or_default();
        let mut seen: Vec<Value> = vec![];
        let mut native = target.clone();
        let mut collection = if matches!(
            op.as_str(),
            "messages.mark_read" | "messages.mark_unread" | "messages.delete_chat"
        ) {
            "chat"
        } else {
            "message"
        };
        let mut data = record_data(&s, &target);
        let mut parts: Vec<Value> = vec![];
        let mut disposition = "locally_observed";
        match op.as_str() {
            "messages.send" | "messages.reply" => {
                let files = m["files"].as_array().cloned().unwrap_or_default();
                let mut chat = string(action, "list_key");
                if op == "messages.reply" {
                    chat = string(&data["chat_id"]["$ref"], "native_key");
                }
                if chat.is_empty() {
                    chat = id("chat")?;
                    let mut peers = vec![];
                    for handle in m["handles"].as_array().into_iter().flatten() {
                        let address = handle.as_str().unwrap_or_default();
                        let key = handle_key(address);
                        put(
                            &mut s,
                            &mut seen,
                            "handle",
                            &key,
                            json!({"address":address,"service":m["service"]}),
                            false,
                        );
                        peers.push(reference("handle", &key));
                    }
                    put(
                        &mut s,
                        &mut seen,
                        "chat",
                        &chat,
                        json!({"service":m["service"],"style":"direct","display_name":"","participants":peers}),
                        false,
                    );
                }
                native = id("message")?;
                let has_text = !text.is_empty() || op != "messages.send" || files.is_empty();
                if has_text || fault == "delivery-failed" || fault == "ghost-send" {
                    new_message(&mut s, &mut seen, &native, &text, &chat, true);
                } else {
                    native.clear();
                }
                if op == "messages.reply" {
                    let mut d = record_data(&s, &native);
                    d["thread_originator_id"] = reference("message", &target);
                    put(&mut s, &mut seen, "message", &native, d, false);
                }
                if fault == "delivery-failed" {
                    set_error(&mut s, &mut seen, &native);
                    disposition = "delivery_failed";
                } else if fault == "ghost-send" {
                    let mut d = record_data(&s, &native);
                    if let Some(object) = d.as_object_mut() {
                        object.remove("chat_id");
                    }
                    d["text"] = json!("");
                    d["parts"] = json!([]);
                    d["service"] = json!("SMS");
                    put(&mut s, &mut seen, "message", &native, d, false);
                    disposition = "delivery_failed";
                    parts.push(json!({"index":0,"kind":"text","disposition":"delivery_failed","observed":false,"reason":"empty_unjoined_sms"}));
                } else {
                    if has_text {
                        new_message(
                            &mut s,
                            &mut seen,
                            &format!("{native}-echo"),
                            &text,
                            &chat,
                            false,
                        );
                        parts.push(json!({"index":0,"kind":"text","native_key":native,"disposition":"locally_observed","observed":true}));
                    }
                    for (index, file) in files.iter().enumerate() {
                        if (fault == "part-no-effect" || fault == "part-progress-failure")
                            && index + 1 == files.len()
                        {
                            parts.push(json!({"index":parts.len(),"kind":"attachment","disposition":"failed_no_effect","observed":false}));
                            disposition = "delivery_failed";
                            continue;
                        }
                        let Ok(bytes) = std::fs::read(string(file, "path")) else {
                            drop(s);
                            return self.reject(frame, "invalid");
                        };
                        let digest = media::digest(&bytes);
                        if digest != string(file, "digest") {
                            drop(s);
                            return self.reject(frame, "invalid");
                        }
                        self.blobs.insert(digest.clone(), bytes.clone());
                        let name = string(file, "name");
                        let derived = media::derive(&name, &bytes);
                        for (blob_digest, blob) in &derived.blobs {
                            self.blobs.insert(blob_digest.clone(), blob.clone());
                        }
                        let attachment = id("attachment")?;
                        let owner = id("message")?;
                        put(
                            &mut s,
                            &mut seen,
                            "message",
                            &owner,
                            json!({"chat_id":reference("chat",&chat),"text":"","parts":[{"index":0,"kind":"attachment","attachment_id":reference("attachment",&attachment)}],"is_from_me":true,"sent_at":now(),"decode_status":"decoded","kind":"message","error_code":0}),
                            false,
                        );
                        if native.is_empty() {
                            native.clone_from(&owner);
                        }
                        put(
                            &mut s,
                            &mut seen,
                            "attachment",
                            &attachment,
                            json!({"message_id":reference("message",&owner),"filename":name,"size":bytes.len(),"mime_type":media::mime(&name,&bytes),"blob":{"digest":digest,"state":"present"},"derivatives":derived.derivatives,"media":derived.media}),
                            false,
                        );
                        parts.push(json!({"index":parts.len(),"kind":"attachment","native_key":owner,"disposition":"locally_observed","observed":true}));
                    }
                    if fault == "part-delivery-failed"
                        && let Some(last) = parts.last_mut()
                    {
                        let key = string(last, "native_key");
                        set_error(&mut s, &mut seen, &key);
                        last["disposition"] = json!("delivery_failed");
                        disposition = "delivery_failed";
                    }
                }
                if fault == "part-progress" || fault == "part-progress-failure" {
                    for part in &mut parts {
                        if part["disposition"] == "failed_no_effect" {
                            continue;
                        }
                        part["observed"] = json!(false);
                        part["disposition"] = json!("indeterminate");
                    }
                    if fault == "part-progress" {
                        disposition = "indeterminate";
                    }
                }
                if fault == "identical-manual-send" {
                    new_message(
                        &mut s,
                        &mut seen,
                        &format!("{native}-manual"),
                        &text,
                        &chat,
                        true,
                    );
                    disposition = "indeterminate";
                }
            }
            "messages.edit" => {
                let mut history = data["edit_history"].as_array().cloned().unwrap_or_default();
                history.push(json!({"text":data["text"],"timestamp":now()}));
                data["edit_history"] = json!(history);
                data["text"] = json!(text);
                for part in data["parts"].as_array_mut().into_iter().flatten() {
                    if part["index"].as_u64() == Some(part_index) && part["kind"] == "text" {
                        part["text"] = json!(text);
                    }
                }
                data["edited_at"] = json!(now());
                put(&mut s, &mut seen, collection, &native, data, false);
            }
            "messages.unsend" => {
                data["retracted_at"] = json!(now());
                data["retracted_parts"] = json!([part_index]);
                put(&mut s, &mut seen, collection, &native, data, false);
            }
            "messages.react" => {
                collection = "reaction";
                native = id("reaction")?;
                put(
                    &mut s,
                    &mut seen,
                    collection,
                    &native,
                    json!({"message_id":reference("message",&target),"part_index":part_index,"kind":string(m,"reaction"),"emoji":string(m,"emoji"),"removed":m["remove"]==true,"is_from_me":true}),
                    false,
                );
            }
            "messages.mark_read" | "messages.mark_unread" => {
                data["is_unread"] = json!(op == "messages.mark_unread");
                data["last_read_at"] = json!(now());
                if op == "messages.mark_read" {
                    let keys: Vec<String> = s.data["records"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(_, record)| {
                            record["collection"] == "messages.message"
                                && record["kind"] != "absent"
                                && string(&record["data"]["chat_id"]["$ref"], "native_key")
                                    == native
                                && record["data"]["is_from_me"] != true
                                && record["data"]["read_at"].is_null()
                        })
                        .map(|(key, _)| key.clone())
                        .collect();
                    for key in keys {
                        let mut message = record_data(&s, &key);
                        message["read_at"] = json!(now());
                        put(&mut s, &mut seen, "message", &key, message, false);
                    }
                }
                put(&mut s, &mut seen, collection, &native, data, false);
            }
            "messages.delete_message" | "messages.delete_chat" => {
                put(&mut s, &mut seen, collection, &native, data, true);
            }
            _ => {}
        }
        let effects = s.data["effects"][command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][command] = json!(effects);
        if fault == "ambiguous-outcome" {
            disposition = "indeterminate";
        }
        let mut evidence = json!({"source":"messages","collection":format!("messages.{collection}"),"native_key":native,"fingerprint":ticket.prepare["request_hash"],"observed":disposition!="indeterminate","parts":parts,"candidate_count":1});
        if fault == "outcome-without-capture" {
            evidence["reason"] = json!("capture_omitted");
        }
        if fault == "part-progress" || fault == "part-progress-failure" {
            evidence["reason"] = json!("part_progress");
            evidence["observed"] = json!(false);
        }
        if fault == "ghost-send" {
            evidence["reason"] = json!("empty_unjoined_sms");
            evidence["observed"] = json!(false);
            if let Some(object) = evidence.as_object_mut() {
                object.remove("native_key");
            }
        }
        if fault == "identical-manual-send" {
            evidence["candidate_count"] = json!(2);
        }
        let mut outcome = json!({"t":"Outcome","command_id":command,"disposition":disposition,"evidence":evidence});
        s.data["evidence"][command] = outcome.clone();
        s.save()?;
        if fault == "crash-after-effect" {
            std::process::exit(78);
        }
        let capture = if op.starts_with("messages.delete_") {
            s.scan("messages")?
        } else {
            json!({"t":"Capture","source":"messages","observations":seen})
        };
        drop(s);
        if fault == "outcome-without-capture" {
            send(&mut self.stream, &outcome)?;
            return Ok(());
        }
        if fault == "outcome-before-capture" {
            send(&mut self.stream, &outcome)?;
            std::thread::sleep(Duration::from_millis(300));
        }
        self.emit(state, capture)?;
        outcome["evidence"]["capture_checkpoint"] = json!(self.checkpoints.get("messages"));
        if fault == "receipt-before-outcome" {
            let mut s = lock(state)?;
            let mut receipt = s.data["records"][&native].clone();
            receipt["data"]["delivered_at"] = json!(now());
            receipt["observed_at"] = json!(now());
            s.put(receipt.clone());
            s.save()?;
            drop(s);
            self.emit(
                state,
                json!({"t":"Capture","source":"messages","observations":[receipt]}),
            )?;
        }
        let mut s = lock(state)?;
        s.data["evidence"][command] = outcome.clone();
        s.save()?;
        if fault == "outcome-before-capture" {
            return Ok(());
        }
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
    fn message_resolve(&mut self, state: &Shared, mut outcome: Value) -> Result<()> {
        let fault = {
            let mut s = lock(state)?;
            if s.fault.starts_with("part-")
                && [
                    "part-progress-stale",
                    "part-receipt-failed",
                    "part-receipt-failed-first",
                    "part-unbound-failure",
                ]
                .contains(&s.fault.as_str())
            {
                std::mem::take(&mut s.fault)
            } else {
                String::new()
            }
        };
        let progress_stale = fault == "part-progress-stale";
        let receipt_first = fault == "part-receipt-failed-first";
        let receipt_failed = fault == "part-receipt-failed" || receipt_first;
        let unbound = fault == "part-unbound-failure";
        if receipt_failed {
            let count = outcome["evidence"]["parts"].as_array().map_or(0, Vec::len);
            if count > 0 {
                let index = if receipt_first { 0 } else { count - 1 };
                let key = string(&outcome["evidence"]["parts"][index], "native_key");
                let mut s = lock(state)?;
                let mut record = s.data["records"][&key].clone();
                record["data"]["error_code"] = json!(100);
                record["observed_at"] = json!(now());
                s.put(record.clone());
                s.save()?;
                drop(s);
                self.emit(
                    state,
                    json!({"t":"Capture","source":"messages","observations":[record]}),
                )?;
                outcome["evidence"]["parts"][index]["disposition"] = json!("delivery_failed");
                outcome["evidence"]["capture_checkpoint"] = json!(self.checkpoints.get("messages"));
                outcome["disposition"] = json!("delivery_failed");
            }
        }
        if unbound {
            let count = outcome["evidence"]["parts"].as_array().map_or(0, Vec::len);
            if count > 0
                && let Some(part) = outcome["evidence"]["parts"][count - 1].as_object_mut()
            {
                part.remove("native_key");
                part.insert("observed".into(), json!(false));
                part.insert("disposition".into(), json!("delivery_failed"));
                outcome["disposition"] = json!("delivery_failed");
            }
        }
        if outcome["evidence"]["reason"] == "capture_omitted" {
            outcome["evidence"]["capture_checkpoint"] = json!(self.checkpoints.get("messages"));
        }
        if outcome["evidence"]["reason"] == "part_progress"
            && !progress_stale
            && outcome["evidence"]["parts"]
                .as_array()
                .is_some_and(|parts| !parts.is_empty())
        {
            outcome["evidence"]["parts"][0]["observed"] = json!(true);
            outcome["evidence"]["parts"][0]["disposition"] = json!("locally_observed");
            outcome["evidence"]["observed"] = json!(true);
        }
        if outcome["disposition"] == "locally_observed" {
            outcome["evidence"]["delivery_evidence"] = json!("sim_is_delivered");
            for part in outcome["evidence"]["parts"]
                .as_array_mut()
                .into_iter()
                .flatten()
            {
                if part["disposition"] == "locally_observed" {
                    part["disposition"] = json!("delivered");
                }
            }
            outcome["disposition"] = json!("delivered");
        }
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}
