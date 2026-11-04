use super::{
    Duration, Instant, Result, Session, Shared, Ticket, Value, id, invalid, json, lock, now,
    observation, send, string,
};
use std::thread;

const PROJECTION: [&str; 28] = [
    "store_identifier",
    "flagged",
    "parent_id",
    "section_id",
    "tags",
    "manual_order",
    "created_at",
    "early_reminders",
    "assignment",
    "store_alarms",
    "store_recurrence",
    "attachment_ids",
    "badge_emblem",
    "is_group",
    "display_order",
    "pinned",
    "pinned_at",
    "sorting_style",
    "categorize_groceries",
    "grocery_locale",
    "sharing_status",
    "shared_owner_name",
    "smart_list_type",
    "smart_filter",
    "sections",
    "store_color",
    "store_status",
    "store_coverage",
];

fn native_fields(data: &Value) -> Value {
    let mut data = data.clone();
    if let Some(fields) = data.as_object_mut() {
        fields.retain(|key, _| !PROJECTION.contains(&key.as_str()));
    }
    data
}

fn live_record<'a>(state: &'a super::State, key: &str, collection: &str) -> Option<&'a Value> {
    let record = &state.data["records"][key];
    (!key.is_empty() && record["collection"] == collection && record["kind"] != "absent")
        .then_some(record)
}

const LIST_OPS: [&str; 3] = [
    "reminders.create_list",
    "reminders.rename_list",
    "reminders.delete_list",
];
const OPS: [&str; 8] = [
    "reminders.create",
    "reminders.create_list",
    "reminders.rename_list",
    "reminders.delete_list",
    "reminders.update",
    "reminders.complete",
    "reminders.uncomplete",
    "reminders.delete",
];
const COVERED: [&str; 6] = [
    "flagged",
    "parent_id",
    "section_id",
    "tags",
    "assignment",
    "created_at",
];

fn reference(key: &str) -> Value {
    json!({"$ref":{"collection":"reminders.list","native_key":key}})
}
fn membership(data: &Value) -> String {
    string(&data["list_id"]["$ref"], "native_key")
}
fn writable(state: &super::State, key: &str) -> bool {
    !key.is_empty() && state.data["records"][key]["data"]["writable"] == true
}
fn destinations_writable(state: &super::State, action: &Value) -> bool {
    let list = string(action, "list_key");
    let source = string(action, "expected_list_key");
    writable(state, &list) && (source.is_empty() || writable(state, &source))
}

impl Session<'_> {
    fn reminder_conflict(&mut self, state: &Shared, frame: &Value, key: &str) -> Result<()> {
        let record = lock(state)?.data["records"][key].clone();
        self.emit(
            state,
            json!({"t":"Capture","source":"reminders","observations":[record]}),
        )?;
        send(
            &mut self.stream,
            &json!({"t":"Rejected","command_id":frame["command_id"],"disposition":"rejected_before_dispatch","error":{"code":"conflict","message":"conflict","retryable":false},"evidence":{"native_key":key,"capture_checkpoint":self.checkpoints.get("reminders").cloned().unwrap_or_default()}}),
        )?;
        Ok(())
    }

    pub(super) fn reminder_prepare(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let action = &frame["action"];
        let op = string(action, "op");
        if !OPS.contains(&op.as_str()) {
            return self.reject(frame, "unsupported");
        }
        if string(frame, "principal").is_empty() || string(frame, "request_hash").is_empty() {
            return self.reject(frame, "invalid");
        }
        let key = string(action, "native_key");
        let mut s = lock(state)?;
        if s.fault == "external-move-before-prepare" && !key.is_empty() {
            let mut record = s.data["records"][&key].clone();
            if !record.is_null() {
                record["data"]["list_id"] = reference("other-list");
                record["observed_at"] = json!(now());
                s.put(record.clone());
                s.fault.clear();
                s.save()?;
                drop(s);
                self.emit(
                    state,
                    json!({"t":"Capture","source":"reminders","observations":[record]}),
                )?;
                s = lock(state)?;
            }
        }
        let mut before = Value::Null;
        if !key.is_empty() {
            let record = s.data["records"][&key].clone();
            if record.is_null() || record["kind"] == "absent" {
                drop(s);
                return self.reject(frame, "not_found");
            }
            before = record["data"].clone();
            if (op == "reminders.rename_list" || op == "reminders.delete_list")
                && (record["collection"] != "reminders.list" || before["writable"] != true)
            {
                drop(s);
                return self.reject(frame, "unsupported");
            }
            let expected = &frame["expected_state"];
            if expected.as_object().is_some_and(|m| !m.is_empty()) {
                let mut expected = expected.clone();
                let mut native = before.clone();
                if membership(&native) != string(action, "expected_list_key") {
                    drop(s);
                    return self.reminder_conflict(state, frame, &key);
                }
                for value in [&mut expected, &mut native] {
                    if let Some(object) = value.as_object_mut() {
                        object.remove("list_id");
                    }
                }
                if expected != native {
                    drop(s);
                    return self.reminder_conflict(state, frame, &key);
                }
            }
        }
        if !LIST_OPS.contains(&op.as_str()) && !destinations_writable(&s, action) {
            drop(s);
            return self.reject(frame, "unsupported");
        }
        self.tickets.retain(|_, t| Instant::now() < t.deadline);
        let list_key = string(action, "list_key");
        let busy = self.tickets.values().any(|t| {
            let other = &t.prepare["action"];
            let other_key = string(other, "native_key");
            (!key.is_empty() && key == other_key)
                || (key.is_empty() && other_key.is_empty() && list_key == string(other, "list_key"))
        });
        if busy {
            drop(s);
            return self.reject(frame, "unavailable");
        }
        s.data["origins"][&command] = json!({"Prepare":frame["origin"],"action":action["origin"]});
        let ticket = id("ticket")?;
        let requested = string(frame, "expires_at")
            .parse::<jiff::Timestamp>()
            .map_err(|_| invalid("expiry"))?;
        let millis =
            (requested.as_millisecond() - jiff::Timestamp::now().as_millisecond()).clamp(0, 30_000);
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: before.clone(),
                deadline: Instant::now() + Duration::from_millis(millis.cast_unsigned()),
            },
        );
        let fault = s.fault.clone();
        let pause = s.pause;
        let mut changed = None;
        if fault == "pause" || fault == "external-edit-between-prepare-and-dispatch" {
            s.fault.clear();
        }
        if fault == "external-edit-between-prepare-and-dispatch" && !key.is_empty() {
            let mut record = s.data["records"][&key].clone();
            record["data"]["title"] = json!("[ab-test] external race");
            record["observed_at"] = json!(now());
            s.put(record.clone());
            s.save()?;
            changed = Some(record);
        }
        drop(s);
        if let Some(record) = changed {
            self.emit(
                state,
                json!({"t":"Capture","source":"reminders","observations":[record]}),
            )?;
        }
        if fault == "pause" {
            thread::sleep(Duration::from_millis(pause));
        }
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":before,"route":"sim-reminders","fingerprint":frame["request_hash"],"candidate_ids":[key]}),
        )?;
        Ok(())
    }

    pub(super) fn reminder_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let Some(ticket) = self.tickets.remove(&string(frame, "ticket")) else {
            return self.reject(frame, "expired");
        };
        if ticket.prepare["command_id"] != command
            || frame["owner_fence"].as_u64() != Some(self.fence)
            || Instant::now() >= ticket.deadline
        {
            return self.reject(frame, "expired");
        }
        let action = ticket.prepare["action"].clone();
        let op = string(&action, "op");
        let mut key = string(&action, "native_key");
        let mut s = lock(state)?;
        if !key.is_empty() && s.data["records"][&key]["data"] != ticket.before {
            drop(s);
            return self.reminder_conflict(state, frame, &key);
        }
        if !LIST_OPS.contains(&op.as_str()) && !destinations_writable(&s, &action) {
            drop(s);
            return self.reject(frame, "unsupported");
        }
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        if key.is_empty() {
            key = id("native")?;
        }
        let collection = if LIST_OPS.contains(&op.as_str()) {
            "reminders.list"
        } else {
            "reminders.reminder"
        };
        let mut data = if ticket.before.is_object() {
            ticket.before.clone()
        } else {
            json!({})
        };
        if let Some(args) = action["args"].as_object() {
            for (k, v) in args {
                if k == "id" || k == "list_id" {
                    continue;
                }
                let clears = matches!(k.as_str(), "due" | "start" | "url" | "notes") && v == "";
                data[k] = if clears { Value::Null } else { v.clone() };
            }
        }
        if collection == "reminders.list" {
            data["writable"] = json!(true);
        } else {
            if op == "reminders.create" {
                data["flagged"] = json!(false);
                data["parent_id"] = Value::Null;
                data["section_id"] = Value::Null;
                data["tags"] = json!([]);
                data["assignment"] = Value::Null;
                data["created_at"] = json!(now());
                let mut coverage = serde_json::Map::new();
                for field in COVERED {
                    coverage.insert(field.into(), json!({"status":"complete"}));
                }
                data["store_coverage"] = Value::Object(coverage);
            }
            if data["completed"].is_null() {
                data["completed"] = json!(false);
            }
            data["list_id"] = reference(&string(&action, "list_key"));
        }
        if op == "reminders.complete" {
            data["completed"] = json!(true);
        }
        if op == "reminders.uncomplete" {
            data["completed"] = json!(false);
        }
        let mut record = observation(collection, &key, data);
        if op == "reminders.delete" || op == "reminders.delete_list" {
            record["kind"] = json!("absent");
            record["revalidated"] = json!(true);
        }
        s.put(record.clone());
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        s.data["origins"][&command]["Dispatch"] = frame["origin"].clone();
        let mut outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"native_key":key,"fingerprint":ticket.prepare["request_hash"],"observed":true,"absent":record["kind"]=="absent"}});
        s.data["evidence"][&command] = outcome.clone();
        s.save()?;
        let fault = std::mem::take(&mut s.fault);
        if fault == "crash-after-effect" {
            std::process::exit(78);
        }
        let capture = if record["kind"] == "absent" {
            s.scan("reminders")?
        } else {
            json!({"t":"Capture","source":"reminders","observations":[record]})
        };
        drop(s);
        self.emit(state, capture)?;
        outcome["evidence"]["capture_checkpoint"] = json!(self.checkpoints.get("reminders"));
        let mut s = lock(state)?;
        s.data["evidence"][&command] = outcome.clone();
        s.save()?;
        if fault == "crash-before-outcome" {
            std::process::exit(78);
        }
        if fault == "ambiguous-outcome" {
            outcome["disposition"] = json!("indeterminate");
        }
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }

    pub(super) fn primitive(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let id = frame["id"].clone();
        let args = &frame["args"];
        let key = string(args, "native_key");
        let answer = match string(frame, "op").as_str() {
            "reminders.read_list" => {
                let s = lock(state)?;
                Ok(match live_record(&s, &key, "reminders.list") {
                    Some(record) => {
                        let data = native_fields(&record["data"]);
                        json!({"exists":true,"writable":data["writable"]==true,"data":data})
                    }
                    None => json!({"exists":false}),
                })
            }
            "reminders.read_reminder" => self.read_reminder(state, &key),
            "reminders.can_create_list" => Ok(json!({"allowed":true})),
            "reminders.reconcile" => self.reconcile(state, args),
            "reminders.write"
                if std::env::var("AB_LAB_PRIMITIVE_CONTROL").as_deref() == Ok("refuse") =>
            {
                Err("unavailable")
            }
            "reminders.write" => self.write(state, frame),
            _ => Err("unsupported"),
        };
        match answer {
            Ok(result) => send(
                &mut self.stream,
                &json!({"t":"PrimitiveResult","id":id,"result":result}),
            )?,
            Err(code) => send(
                &mut self.stream,
                &json!({"t":"PrimitiveError","id":id,"code":code}),
            )?,
        };
        Ok(())
    }

    fn read_reminder(
        &mut self,
        state: &Shared,
        key: &str,
    ) -> std::result::Result<Value, &'static str> {
        let fail = |_| "unavailable";
        let mut s = lock(state).map_err(fail)?;
        let fault = s.fault.clone();
        let mut changed = None;
        if fault == "external-move-before-prepare"
            && let Some(mut record) = live_record(&s, key, "reminders.reminder").cloned()
        {
            record["data"]["list_id"] = reference("other-list");
            record["observed_at"] = json!(now());
            s.put(record.clone());
            s.fault.clear();
            s.save().map_err(fail)?;
            changed = Some(record);
        }
        let current = live_record(&s, key, "reminders.reminder").cloned();
        let answer = match &current {
            Some(record) => {
                let list = membership(&record["data"]);
                json!({"exists":true,"writable":writable(&s, &list),"list_key":list,"data":native_fields(&record["data"])})
            }
            None => json!({"exists":false}),
        };
        if fault == "external-edit-between-prepare-and-dispatch"
            && let Some(mut record) = current
        {
            record["data"]["title"] = json!("[ab-test] external race");
            record["observed_at"] = json!(now());
            s.put(record.clone());
            s.fault.clear();
            s.save().map_err(fail)?;
            changed = Some(record);
        }
        drop(s);
        if let Some(record) = changed {
            self.emit(
                state,
                json!({"t":"Capture","source":"reminders","observations":[record]}),
            )
            .map_err(fail)?;
        }
        Ok(answer)
    }

    fn reconcile(
        &mut self,
        state: &Shared,
        args: &Value,
    ) -> std::result::Result<Value, &'static str> {
        let fail = |_| "unavailable";
        let key = string(args, "absent_key");
        let s = lock(state).map_err(fail)?;
        let capture = if key.is_empty() || s.data["records"][&key]["kind"] != "absent" {
            s.scan("reminders").map_err(fail)?
        } else {
            json!({"t":"Capture","source":"reminders","observations":[s.data["records"][&key].clone()]})
        };
        drop(s);
        self.emit(state, capture).map_err(fail)?;
        Ok(json!({}))
    }

    fn write(&mut self, state: &Shared, frame: &Value) -> std::result::Result<Value, &'static str> {
        let fail = |_| "unavailable";
        let args = &frame["args"];
        let op = string(args, "op");
        let mut key = string(args, "native_key");
        let mut s = lock(state).map_err(fail)?;
        let command = string(args, "command_id");
        if key.is_empty() {
            key = id("native").map_err(fail)?;
        }
        let list_op = LIST_OPS.contains(&op.as_str());
        let collection = if list_op {
            "reminders.list"
        } else {
            "reminders.reminder"
        };
        let mut data = s.data["records"][&key]["data"].clone();
        if !data.is_object() {
            data = json!({});
        }
        if let Some(fields) = args["args"].as_object() {
            for (k, v) in fields {
                if k == "id" || k == "list_id" {
                    continue;
                }
                let clears = matches!(k.as_str(), "due" | "start" | "url" | "notes") && v == "";
                data[k] = if clears { Value::Null } else { v.clone() };
            }
        }
        if list_op {
            data["writable"] = json!(true);
        } else {
            if op == "reminders.create" {
                data["flagged"] = json!(false);
                data["parent_id"] = Value::Null;
                data["section_id"] = Value::Null;
                data["tags"] = json!([]);
                data["assignment"] = Value::Null;
                data["created_at"] = json!(now());
                let mut coverage = serde_json::Map::new();
                for field in COVERED {
                    coverage.insert(field.into(), json!({"status":"complete"}));
                }
                data["store_coverage"] = Value::Object(coverage);
            }
            if data["completed"].is_null() {
                data["completed"] = json!(false);
            }
            data["list_id"] = reference(&string(args, "list_key"));
        }
        if op == "reminders.complete" {
            data["completed"] = json!(true);
        }
        if op == "reminders.uncomplete" {
            data["completed"] = json!(false);
        }
        let mut record = observation(collection, &key, data);
        let gone = op == "reminders.delete" || op == "reminders.delete_list";
        if gone {
            record["kind"] = json!("absent");
            record["revalidated"] = json!(true);
        }
        s.put(record.clone());
        if !command.is_empty() {
            let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
            s.data["effects"][&command] = json!(count);
            s.data["evidence"][&command] = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"native_key":key,"observed":true,"absent":gone}});
        }
        s.save().map_err(fail)?;
        let fault = std::mem::take(&mut s.fault);
        if fault == "crash-after-effect" {
            std::process::exit(78);
        }
        let capture = if gone {
            s.scan("reminders").map_err(fail)?
        } else {
            json!({"t":"Capture","source":"reminders","observations":[record.clone()]})
        };
        drop(s);
        self.emit(state, capture).map_err(fail)?;
        if fault == "crash-before-outcome" {
            std::process::exit(78);
        }
        let shown = if gone {
            Value::Null
        } else {
            native_fields(&record["data"])
        };
        let ambiguous = fault == "ambiguous-outcome";
        let observed = if ambiguous {
            Value::Null
        } else {
            shown.clone()
        };
        Ok(
            json!({"native_key":key,"saved":shown,"observed":observed,"exists_after":!gone || ambiguous}),
        )
    }
}
