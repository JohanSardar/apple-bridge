use super::messages_commands::facts;
use super::{
    Request, Result, Shared, State, Value, enqueue, id, invalid, json, lock, mpsc, now,
    observation, string,
};
fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref":{"collection":format!("messages.{collection}"),"native_key":key}})
}
fn put(s: &mut State, collection: &str, key: &str, mut data: Value) {
    facts(&s.data["records"], collection, &mut data);
    let mut record = observation(&format!("messages.{collection}"), key, data);
    record["scoped_alias"] = json!(format!("messages.{collection}/{key}"));
    s.put(record);
}
fn message(s: &mut State, key: &str, text: &str, chat: &str) {
    put(
        s,
        "message",
        key,
        json!({"title":text,"text":text,"chat_id":reference("chat",chat),"sender_handle_id":reference("handle","message-self"),"is_from_me":true,"sent_at":now(),"parts":[{"index":0,"kind":"text","text":text}],"decode_status":"decoded","edit_history":[],"retracted_parts":[],"error_code":0,"kind":"message"}),
    );
}
fn incoming(s: &mut State, key: &str, from_me: bool) {
    let text = "[ab-test] Unicode 🧪 שלום e\u{0301}";
    message(s, key, text, "message-chat");
    s.data["records"][key]["data"]["title"] = json!(format!("[ab-test] {key}"));
    s.data["records"][key]["data"]["is_from_me"] = json!(from_me);
}
const OPERATIONS: [&str; 9] = [
    "messages.send",
    "messages.reply",
    "messages.react",
    "messages.edit",
    "messages.unsend",
    "messages.mark_read",
    "messages.mark_unread",
    "messages.delete_message",
    "messages.delete_chat",
];
pub(super) fn operations() -> Value {
    json!(OPERATIONS)
}
pub(super) fn coverage(proven: bool, reason: bool) -> Value {
    let operations: Vec<Value> = OPERATIONS
        .iter()
        .map(|op| {
            json!({"operation":op,"proven":proven,"reason":if !proven{"capability_unproven"}else if reason{"simulated_native_route"}else{""}})
        })
        .collect();
    json!({"fields":"complete","missing":[],"operations":operations})
}
pub(super) fn revalidate(s: &State, frame: &mut Value) {
    let Some(observations) = frame["observations"].as_array_mut() else {
        return;
    };
    for record in observations {
        let current = &s.data["records"][string(record, "native_key")];
        if !current.is_null() && record["kind"] != "absent" {
            *record = current.clone();
        }
    }
}
pub(super) fn control_messages(
    state: &Shared,
    tx: &mpsc::Sender<Request>,
    input: &Value,
) -> Result<Value> {
    let mut s = lock(state)?;
    let action = string(input, "action");
    let key = string(input, "key");
    let mut changed: Vec<String> = vec![];
    if !input["count"].is_null() && input["count"].as_u64().is_none_or(|n| n > 20000) {
        return Err(invalid("message count"));
    }
    match action.as_str() {
        "bulk-pause" => {
            s.bulk.target = key;
            s.bulk.paused = true;
            return Ok(json!({"effect_done":true}));
        }
        "bulk-release" => {
            s.bulk.paused = false;
            return Ok(json!({"effect_done":true}));
        }
        "seed" => {
            put(
                &mut s,
                "handle",
                "message-self",
                json!({"title":"[ab-test] self","address":"self@example.com","service":"iMessage"}),
            );
            put(
                &mut s,
                "chat",
                "message-chat",
                json!({"title":"[ab-test] chat","service":"iMessage","style":"direct","display_name":"Bridge Test","participants":[reference("handle","message-self")],"capabilities":[]}),
            );
            for i in 0..input["count"].as_u64().unwrap_or_default() {
                incoming(&mut s, &format!("message-{i:06}"), true);
            }
        }
        "incoming" => {
            if key.is_empty() {
                return Err(invalid("message key"));
            }
            incoming(&mut s, &format!("{key}-outgoing"), true);
            incoming(&mut s, &key, false);
            changed = vec![format!("{key}-outgoing"), key.clone()];
        }
        "receipts" | "malformed" | "delete" => {
            let mut record = s.data["records"][&key].clone();
            if record.is_null() {
                return Err(invalid("message not found"));
            }
            record["observed_at"] = json!(now());
            match action.as_str() {
                "delete" => {
                    record["kind"] = json!("absent");
                    record["revalidated"] = json!(true);
                }
                "receipts" => {
                    record["data"]["delivered_at"] = json!(now());
                    record["data"]["read_at"] = json!(now());
                }
                _ => {
                    record["data"]
                        .as_object_mut()
                        .ok_or_else(|| invalid("message data"))?
                        .remove("text");
                    record["data"]["parts"] = json!([]);
                    record["data"]["decode_status"] = json!("undecoded");
                    record["data"]["raw_body_digest"] =
                        json!("a50c70b58b846b7ac9a6f6bbf92c4b58b6a5346202994187ad0d83346607636c");
                }
            }
            s.put(record);
            changed = vec![key.clone()];
        }
        "release" => {
            let held: Vec<String> = s.data["records"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, record)| record["deferred"] == true || record["held"] == true)
                .map(|(key, _)| key.clone())
                .collect();
            for held_key in &held {
                if let Some(record) = s.data["records"][held_key].as_object_mut() {
                    record.remove("deferred");
                    record.remove("held");
                }
            }
            changed = vec![];
        }
        "nonself-chat" => {
            put(
                &mut s,
                "handle",
                "message-other",
                json!({"address":"other@example.com","service":"iMessage"}),
            );
            put(
                &mut s,
                "chat",
                "message-other-chat",
                json!({"service":"iMessage","style":"direct","display_name":"Synthetic other","participants":[reference("handle","message-other")],"capabilities":[]}),
            );
            changed = vec!["message-other".into(), "message-other-chat".into()];
        }
        "capability-unproven" | "capability-proven" | "capability-proven-empty-reason" => {
            let unproven = action == "capability-unproven";
            s.data["message_capability_unproven"] = json!(unproven);
            s.save()?;
            let mut frame = json!({"t":"SourceState","source":"messages","state":"ready","coverage":{"messages":coverage(!unproven, action != "capability-proven-empty-reason")},"capabilities":{"messages":if unproven{json!([])}else{operations()}}});
            if unproven {
                frame["reason"] = json!("capability_unproven");
            }
            drop(s);
            enqueue(tx, frame, true)?;
            return Ok(json!({"effect_done":true}));
        }
        "watcher-rearm" => {
            s.data["message_watcher_armed"] = json!(false);
            s.data["message_poll_pending"] = json!(true);
            if !key.is_empty() {
                incoming(&mut s, &key, false);
            }
            s.save()?;
            return Ok(json!({"effect_done":true}));
        }
        "watcher-poll" => {
            if s.data["message_poll_pending"] != true {
                return Ok(json!({"effect_done":true}));
            }
            s.data["message_watcher_armed"] = json!(true);
            s.data["message_poll_pending"] = json!(false);
        }
        "permission-loss" | "permission-restore" => {
            let missing = action == "permission-loss";
            s.denied.insert("messages".into(), missing);
            drop(s);
            enqueue(
                tx,
                json!({"t":"SourceState","source":"messages","state":if missing{"permission_missing"}else{"ready"},"coverage":{"messages":{"fields":if missing{"partial"}else{"complete"},"missing":if missing{vec!["permission_missing"]}else{vec![]}}}}),
                false,
            )?;
            if !missing {
                let capture = lock(state)?.scan("messages")?;
                enqueue(tx, capture, true)?;
            }
            return Ok(json!({"effect_done":true}));
        }
        "db-replacement" | "schema-change" => {
            s.data["message_generation"] = json!(id("messages")?);
        }
        _ => return Err(invalid("unsupported message control")),
    }
    s.save()?;
    if s.denied.get("messages") == Some(&true) {
        return Ok(json!({"effect_done":true}));
    }
    if s.bulk.paused && !changed.is_empty() {
        let observations: Vec<Value> = changed
            .iter()
            .map(|key| s.data["records"][key].clone())
            .collect();
        drop(s);
        enqueue(
            tx,
            json!({"t":"Capture","source":"messages","observations":observations}),
            true,
        )?;
        return Ok(json!({"effect_done":true}));
    }
    let wait = !s.bulk.paused && input["wait"] != json!(false);
    let capture = s.scan("messages")?;
    drop(s);
    enqueue(tx, capture, wait)?;
    Ok(json!({"effect_done":true}))
}
pub(super) fn aliases(frame: &mut Value) {
    fn collect(value: &Value, found: &mut std::collections::BTreeMap<String, Value>) {
        if let Some(reference) = value.get("$ref") {
            let collection = string(reference, "collection");
            let key = string(reference, "native_key");
            let alias = format!("{collection}/{key}");
            found.insert(alias.clone(), json!({"kind":"alias","collection":collection,"native_key":key,"scoped_alias":alias,"observed_at":now()}));
        } else if let Some(object) = value.as_object() {
            for child in object.values() {
                collect(child, found);
            }
        } else if let Some(array) = value.as_array() {
            for child in array {
                collect(child, found);
            }
        }
    }
    let mut found = std::collections::BTreeMap::new();
    collect(&frame["observations"], &mut found);
    let mut records: Vec<_> = found.into_values().collect();
    if let Some(observations) = frame["observations"].as_array() {
        records.extend_from_slice(observations);
    }
    frame["observations"] = json!(records);
}
