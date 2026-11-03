use super::{
    Duration, Instant, Request, Result, Session, Shared, Ticket, Value, enqueue, id, invalid, json,
    lock, mpsc, now, observation, send, string,
};
use base64::Engine;
use sha2::{Digest, Sha256};

const GROUP_OPS: [&str; 5] = [
    "contacts.create_group",
    "contacts.rename_group",
    "contacts.delete_group",
    "contacts.add_member",
    "contacts.remove_member",
];

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn reference(key: &str) -> Value {
    json!({"$ref":{"collection":"contacts.contact","native_key":key}})
}

fn object(value: &Value) -> Value {
    if value.is_null() {
        json!({})
    } else {
        value.clone()
    }
}

fn store_image(s: &mut super::State, encoded: &str) -> Option<Value> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let sum = digest(&bytes);
    s.data["blobs"][&sum] = json!(encoded);
    Some(json!({"digest":sum,"state":"present"}))
}

fn alias(record: &mut Value, collection: &str, key: &str) {
    record["scoped_alias"] = json!(format!("contacts/{collection}/{key}"));
}

pub(super) fn snapshot(s: &super::State, drop_absent: bool) -> Result<Value> {
    let mut frame = s.scan("contacts")?;
    if drop_absent && let Some(items) = frame["observations"].as_array_mut() {
        items.retain(|record| record["kind"] != "absent");
    }
    let handles: serde_json::Map<String, Value> = s.data["blobs"]
        .as_object()
        .map(|blobs| blobs.keys().map(|d| (d.clone(), json!(d))).collect())
        .unwrap_or_default();
    frame["blob_handles"] = Value::Object(handles);
    Ok(frame)
}

pub(super) fn control_contacts(
    state: &Shared,
    tx: &mpsc::Sender<Request>,
    input: &Value,
) -> Result<Value> {
    let mut s = lock(state)?;
    let action = string(input, "action");
    let unified = string(input, "unified_key");
    let key = if unified.is_empty() {
        string(input, "key")
    } else {
        unified.clone()
    };
    let mut drop_absent = false;
    match action.as_str() {
        "drop-everything" => {
            s.data["contacts_generation"] = json!(id("contacts")?);
            drop_absent = true;
        }
        "group" => {
            let group = format!("group-{key}");
            let mut record = observation(
                "contacts.group",
                &group,
                json!({"name":"Bridge Test","member_ids":[reference(&key)]}),
            );
            alias(&mut record, "contacts.group", &group);
            s.put(record);
        }
        "create" | "update" | "image" | "delete" | "token-crash" => {
            if key.is_empty() {
                return Err(invalid("invalid"));
            }
            let mut data = object(&s.data["records"][&key]["data"]);
            if let Some(patch) = input["data"].as_object() {
                for (k, v) in patch {
                    if !unified.is_empty() && (k == "phones" || k == "emails") {
                        let mut items = data[k].as_array().cloned().unwrap_or_default();
                        items.extend(v.as_array().cloned().unwrap_or_default());
                        data[k] = json!(items);
                    } else {
                        data[k] = v.clone();
                    }
                }
            }
            let readonly = input["readonly"] == true;
            data["note"] = json!("unavailable");
            data["writable"] = json!(!readonly);
            data["container_id"] = json!(if readonly { "readonly" } else { "default" });
            if action == "image" {
                let encoded = input["data"]["base64"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid"))?;
                if let Some(object) = data.as_object_mut() {
                    object.remove("base64");
                }
                data["image"] = store_image(&mut s, encoded).ok_or_else(|| invalid("invalid"))?;
            }
            let mut record = observation("contacts.contact", &key, data);
            alias(&mut record, "contacts.contact", &key);
            if action == "delete" {
                record["kind"] = json!("absent");
                record["revalidated"] = json!(true);
            }
            s.put(record);
        }
        _ => return Err(invalid("invalid")),
    }
    let sequence = s.data["contact_sequence"].as_u64().unwrap_or_default() + 1;
    s.data["contact_sequence"] = json!(sequence);
    s.save()?;
    let missing = s.denied.get("contacts") == Some(&true);
    let frame = snapshot(&s, drop_absent)?;
    drop(s);
    if !missing {
        if action == "token-crash" {
            std::process::exit(78);
        }
        enqueue(tx, frame, true)?;
    }
    Ok(json!({"effect_done":true}))
}

fn member_available(s: &super::State, action: &Value) -> bool {
    let op = string(action, "op");
    if op != "contacts.add_member" && op != "contacts.remove_member" {
        return true;
    }
    let group = &s.data["records"][string(action, "native_key")];
    let contact = &s.data["records"][string(action, "list_key")];
    !group.is_null()
        && !contact.is_null()
        && group["kind"] != "absent"
        && contact["kind"] != "absent"
        && contact["collection"] == "contacts.contact"
        && !group["data"]["container_id"].is_null()
        && group["data"]["container_id"] == contact["data"]["container_id"]
}

impl Session {
    fn contacts_conflict(&mut self, state: &Shared, frame: &Value, key: &str) -> Result<()> {
        let capture = {
            let s = lock(state)?;
            snapshot(&s, false)?
        };
        self.emit(state, capture)?;
        send(
            &mut self.stream,
            &json!({"t":"Rejected","command_id":frame["command_id"],"disposition":"rejected_before_dispatch","error":{"code":"conflict","message":"conflict"},"evidence":{"native_key":key,"capture_checkpoint":self.checkpoints.get("contacts").cloned().unwrap_or_default()}}),
        )?;
        Ok(())
    }

    pub(super) fn contacts_prepare(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let action = &frame["action"];
        let s = lock(state)?;
        if s.denied.get("contacts") == Some(&true) {
            drop(s);
            return self.reject(frame, "unavailable");
        }
        let key = string(action, "native_key");
        let mut before = Value::Null;
        if !key.is_empty() {
            let record = &s.data["records"][&key];
            if record.is_null() {
                drop(s);
                return self.reject(frame, "not_found");
            }
            if record["kind"] == "absent" {
                drop(s);
                return self.contacts_conflict(state, frame, &key);
            }
            before = record["data"].clone();
            if object(&before) != object(&frame["expected_state"]) {
                drop(s);
                return self.contacts_conflict(state, frame, &key);
            }
            if before["writable"] != true {
                drop(s);
                return self.reject(frame, "unsupported");
            }
        }
        if !member_available(&s, action) {
            drop(s);
            return self.reject(frame, "unsupported");
        }
        drop(s);
        let ticket = id("ticket")?;
        let requested = string(frame, "expires_at")
            .parse::<jiff::Timestamp>()
            .map_err(|_| invalid("expiry"))?;
        let seconds = (requested.as_second() - jiff::Timestamp::now().as_second()).clamp(0, 30);
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: before.clone(),
                deadline: Instant::now() + Duration::from_secs(seconds.cast_unsigned()),
            },
        );
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":before,"route":"sim-contacts","fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn contacts_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
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
        if s.denied.get("contacts") == Some(&true) {
            drop(s);
            return self.reject(frame, "unavailable");
        }
        if !key.is_empty()
            && (s.data["records"][&key]["kind"] == "absent"
                || s.data["records"][&key]["data"] != ticket.before)
        {
            drop(s);
            return self.contacts_conflict(state, frame, &key);
        }
        if !member_available(&s, &action) {
            drop(s);
            return self.reject(frame, "unsupported");
        }
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        if key.is_empty() {
            key = id("contact")?;
        }
        let mut data = json!({"note":"unavailable","container_id":"default","writable":true});
        if let (Some(target), Some(source)) = (data.as_object_mut(), ticket.before.as_object()) {
            for (k, v) in source {
                target.insert(k.clone(), v.clone());
            }
        }
        let mut patch = action["args"].as_object().cloned().unwrap_or_default();
        patch.remove("id");
        patch.remove("contact_id");
        let group = GROUP_OPS.contains(&op.as_str());
        if group {
            if ticket.before.is_null() {
                data = json!({"container_id":"default","writable":true,"member_ids":[]});
            }
            if op == "contacts.add_member" || op == "contacts.remove_member" {
                let member = string(&action, "list_key");
                let contact = s.data["records"][&member].clone();
                if contact.is_null()
                    || contact["kind"] == "absent"
                    || contact["data"]["container_id"] != data["container_id"]
                {
                    drop(s);
                    return self.reject(frame, "unsupported");
                }
                let mut members: Vec<Value> = data["member_ids"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|entry| string(&entry["$ref"], "native_key") != member)
                    .collect();
                if op == "contacts.add_member" {
                    members.push(reference(&member));
                }
                data["member_ids"] = json!(members);
            }
        }
        for (k, v) in &patch {
            data[k] = v.clone();
        }
        if let Some(encoded) = patch
            .get("image")
            .and_then(|image| image["base64"].as_str())
        {
            let Some(image) = store_image(&mut s, encoded) else {
                drop(s);
                return self.reject(frame, "invalid");
            };
            data["image"] = image;
        }
        let collection = if group {
            "contacts.group"
        } else {
            "contacts.contact"
        };
        let observed_digest = digest(&serde_json::to_vec(&data)?);
        let mut record = observation(collection, &key, data);
        alias(&mut record, collection, &key);
        let absent = op == "contacts.delete" || op == "contacts.delete_group";
        if absent {
            record["kind"] = json!("absent");
            record["revalidated"] = json!(true);
        }
        s.put(record);
        let sequence = s.data["contact_sequence"].as_u64().unwrap_or_default() + 1;
        s.data["contact_sequence"] = json!(sequence);
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        let mut outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"source":"contacts","native_key":key,"observed":true,"absent":absent,"fingerprint":ticket.prepare["request_hash"],"observed_digest":observed_digest}});
        s.data["evidence"][&command] = outcome.clone();
        let fault = std::mem::take(&mut s.fault);
        s.save()?;
        if fault == "crash-after-effect" {
            std::process::exit(78);
        }
        let capture = snapshot(&s, false)?;
        drop(s);
        self.emit(state, capture)?;
        outcome["evidence"]["capture_checkpoint"] = json!(self.checkpoints.get("contacts"));
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

    pub(super) fn contacts_resolve(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let s = lock(state)?;
        let mut outcome = s.data["evidence"][&command].clone();
        let key = string(&outcome["evidence"], "native_key");
        let absent = outcome["evidence"]["absent"] == true;
        let record = &s.data["records"][&key];
        let mut matches =
            !outcome.is_null() && !record.is_null() && (record["kind"] == "absent") == absent;
        if !absent {
            matches = matches
                && outcome["evidence"]["observed_digest"]
                    == json!(digest(&serde_json::to_vec(&record["data"])?));
        }
        if matches {
            let capture = snapshot(&s, false)?;
            drop(s);
            self.emit(state, capture)?;
            outcome["evidence"]["capture_checkpoint"] = json!(self.checkpoints.get("contacts"));
            outcome["disposition"] = json!("locally_observed");
            lock(state)?.guard(&json!({"command_id":command,"outcome":outcome}))?;
        } else {
            drop(s);
            outcome = json!({"t":"Outcome","command_id":command,"disposition":"indeterminate"});
        }
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}

pub(super) fn blob(state: &Shared, frame: &Value) -> Result<Value> {
    let handle = string(frame, "handle");
    let digest = handle
        .split_once(':')
        .map_or(handle.as_str(), |(_, digest)| digest)
        .to_owned();
    let encoded = lock(state)?.data["blobs"][&digest]
        .as_str()
        .map(str::to_owned);
    let engine = base64::engine::general_purpose::STANDARD;
    let Some(bytes) = encoded.and_then(|text| engine.decode(text).ok()) else {
        return Ok(json!({"t":"BlobUnavailable","handle":handle,"reason":"missing"}));
    };
    let offset = usize::try_from(frame["offset"].as_u64().unwrap_or_default())
        .map_err(|_| invalid("offset"))?;
    let length = usize::try_from(frame["length"].as_u64().unwrap_or_default())
        .map_err(|_| invalid("length"))?;
    if offset > bytes.len() || length == 0 || length > 128 << 10 {
        return Err(invalid("blob range"));
    }
    let end = bytes.len().min(offset + length);
    Ok(
        json!({"t":"BlobChunk","handle":handle,"offset":offset,"bytes":engine.encode(&bytes[offset..end]),"digest":digest,"eof":end == bytes.len()}),
    )
}
