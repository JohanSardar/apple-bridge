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
pub(super) fn control_messages(
    state: &Shared,
    tx: &mpsc::Sender<Request>,
    input: &Value,
) -> Result<Value> {
    let mut s = lock(state)?;
    let action = string(input, "action");
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
                message(
                    &mut s,
                    &format!("message-{i:06}"),
                    "[ab-test] synthetic message",
                    "message-chat",
                );
            }
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
            return Ok(json!({"effect_done":true}));
        }
        "db-replacement" | "schema-change" => {
            s.data["generation"] = json!(id("sim")?);
        }
        _ => return Err(invalid("unsupported message control")),
    }
    s.save()?;
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
            message(&mut s, &key, &text, &chat);
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
            let mut outcome = json!({"t":"Outcome","command_id":command,"disposition":disposition,"evidence":{"source":"messages","collection":"messages.message","native_key":key,"fingerprint":ticket.prepare["request_hash"],"observed":disposition=="locally_observed","candidate_count":if fault=="identical-manual-send"{2}else{1},"parts":[]}});
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
