use super::media;
use super::{
    Duration, Instant, Request, Result, Session, Shared, State, Ticket, Value, enqueue, id,
    invalid, json, lock, mpsc, now, observation, send, string,
};
fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref":{"collection":format!("messages.{collection}"),"native_key":key}})
}
fn put(s: &mut State, collection: &str, key: &str, data: Value) {
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
pub(super) fn control_messages(
    state: &Shared,
    tx: &mpsc::Sender<Request>,
    input: &Value,
) -> Result<Value> {
    let mut s = lock(state)?;
    let action = string(input, "action");
    let key = string(input, "key");
    if !input["count"].is_null() && input["count"].as_u64().is_none_or(|n| n > 20000) {
        return Err(invalid("message count"));
    }
    match action.as_str() {
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
    let capture = s.scan("messages")?;
    drop(s);
    enqueue(tx, capture, true)?;
    Ok(json!({"effect_done":true}))
}
impl Session {
    pub(super) fn message_command(&mut self, state: &Shared, frame: &Value) -> Result<bool> {
        let command = string(frame, "command_id");
        if frame["t"] == "Prepare" && string(&frame["action"], "op").starts_with("messages.") {
            let mut s = lock(state)?;
            let action = &frame["action"];
            let op = string(action, "op");
            if !["messages.send", "messages.reply"].contains(&op.as_str()) {
                drop(s);
                self.reject(frame, "unsupported")?;
                return Ok(true);
            }
            if s.denied.get("messages") == Some(&true) {
                drop(s);
                self.reject(frame, "capability_unproven")?;
                return Ok(true);
            }
            if s.fault == "ui-interference-before" {
                s.fault.clear();
                drop(s);
                self.reject(frame, "conflict")?;
                return Ok(true);
            }
            let key = string(action, "native_key");
            let before = s.data["records"][&key]["data"].clone();
            let ticket = id("ticket")?;
            self.tickets.insert(
                ticket.clone(),
                Ticket {
                    prepare: frame.clone(),
                    before: before.clone(),
                    deadline: Instant::now() + Duration::from_secs(30),
                },
            );
            drop(s);
            send(
                &mut self.stream,
                &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":before,"route":"sim-messages","fingerprint":frame["request_hash"],"candidate_ids":["0",action["list_key"]]}),
            )?;
            return Ok(true);
        }
        if frame["t"] == "Dispatch"
            && self
                .tickets
                .get(&string(frame, "ticket"))
                .is_some_and(|t| string(&t.prepare["action"], "op").starts_with("messages."))
        {
            let ticket = self
                .tickets
                .remove(&string(frame, "ticket"))
                .ok_or_else(|| invalid("ticket"))?;
            if ticket.prepare["command_id"] != command
                || frame["owner_fence"].as_u64() != Some(self.fence)
                || Instant::now() >= ticket.deadline
            {
                self.reject(frame, "expired")?;
                return Ok(true);
            }
            let mut s = lock(state)?;
            s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
            let fault = std::mem::take(&mut s.fault);
            if fault == "ui-interference" {
                drop(s);
                send(
                    &mut self.stream,
                    &json!({"t":"Outcome","command_id":command,"disposition":"indeterminate"}),
                )?;
                return Ok(true);
            }
            let action = &ticket.prepare["action"];
            let text = string(&action["messages"], "text");
            let mut chat = string(action, "list_key");
            if chat.is_empty() {
                chat = string(&ticket.before["chat_id"]["$ref"], "native_key");
            }
            let key = id("message")?;
            let files = action["messages"]["files"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !text.is_empty() || files.is_empty() {
                message(&mut s, &key, &text, &chat);
            }
            let mut parts = Vec::new();
            for file in &files {
                let Ok(bytes) = std::fs::read(string(file, "path")) else {
                    drop(s);
                    self.reject(frame, "invalid")?;
                    return Ok(true);
                };
                let digest = media::digest(&bytes);
                if digest != string(file, "digest") {
                    drop(s);
                    self.reject(frame, "invalid")?;
                    return Ok(true);
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
                    "message",
                    &owner,
                    json!({"chat_id":reference("chat",&chat),"text":"","parts":[{"index":0,"kind":"attachment","attachment_id":reference("attachment",&attachment)}],"is_from_me":true,"sent_at":now(),"decode_status":"decoded","kind":"message","error_code":0}),
                );
                put(
                    &mut s,
                    "attachment",
                    &attachment,
                    json!({"message_id":reference("message",&owner),"filename":name,"size":bytes.len(),"mime_type":media::mime(&name,&bytes),"blob":{"digest":digest,"state":"present"},"derivatives":derived.derivatives,"media":derived.media}),
                );
                parts.push(json!({"index":parts.len(),"kind":"attachment","native_key":owner,"disposition":"locally_observed","observed":true}));
            }
            if fault == "identical-manual-send" {
                message(&mut s, &format!("{key}-manual"), &text, &chat);
            }
            let n = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
            s.data["effects"][&command] = json!(n);
            let disposition = if fault == "identical-manual-send" || fault == "ambiguous-outcome" {
                "indeterminate"
            } else {
                "locally_observed"
            };
            let mut outcome = json!({"t":"Outcome","command_id":command,"disposition":disposition,"evidence":{"source":"messages","collection":"messages.message","native_key":key,"fingerprint":ticket.prepare["request_hash"],"observed":disposition=="locally_observed","candidate_count":if fault=="identical-manual-send"{2}else{1},"parts":parts}});
            s.data["evidence"][&command] = outcome.clone();
            s.save()?;
            if fault == "crash-after-effect" {
                std::process::exit(78);
            }
            let capture = s.scan("messages")?;
            drop(s);
            self.emit(state, capture)?;
            outcome["evidence"]["capture_checkpoint"] = json!(self.checkpoints.get("messages"));
            let mut s = lock(state)?;
            s.data["evidence"][&command] = outcome.clone();
            s.save()?;
            s.guard(&json!({"command_id":command,"outcome":outcome}))?;
            drop(s);
            send(&mut self.stream, &outcome)?;
            return Ok(true);
        }
        if frame["t"] == "Resolve" {
            let s = lock(state)?;
            let outcome = s.data["evidence"][&command].clone();
            if outcome["evidence"]["source"] == "messages" {
                drop(s);
                send(&mut self.stream, &outcome)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
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
