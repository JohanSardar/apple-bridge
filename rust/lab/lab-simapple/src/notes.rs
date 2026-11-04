use super::{
    Duration, Instant, Result, Session, Shared, Ticket, Value, id, invalid, json, lock, now,
    observation, send, string,
};

fn tokens(text: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = if rest.starts_with('<') {
            rest.find('>')? + 1
        } else {
            rest.find('<').unwrap_or(rest.len())
        };
        result.push(rest[..end].to_owned());
        rest = &rest[end..];
    }
    Some(result)
}

fn structure(text: &str) -> Option<Vec<String>> {
    Some(
        tokens(text)?
            .into_iter()
            .map(|token| {
                if token.starts_with('<') {
                    token
                } else {
                    "text".to_owned()
                }
            })
            .collect(),
    )
}

fn compatible(before: &Value, html: &str) -> bool {
    let original = string(before, "html");
    if original.is_empty()
        || string(before, "content_status") != "complete"
        || string(before, "block_status") != "complete"
        || string(before, "round_trip_status") != "unproven"
        || before["attachments"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        || html.len() > 1_000_000
        || html.contains('\0')
    {
        return false;
    }
    if [
        "data-notes-read-only",
        "<table",
        "<ul",
        "<ol",
        "<pre",
        "<blockquote",
    ]
    .iter()
    .any(|marker| original.contains(marker))
    {
        return false;
    }
    let left = structure(&original);
    left.is_some() && left == structure(html)
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| {
                    number.strip_prefix(['x', 'X']).map_or_else(
                        || number.parse::<u32>().ok(),
                        |hex| u32::from_str_radix(hex, 16).ok(),
                    )
                })
                .and_then(char::from_u32),
        };
        if let Some(character) = replacement {
            out.push(character);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn plain(html: &str) -> Result<String> {
    let mut text = String::new();
    for token in tokens(html).ok_or_else(|| invalid("html"))? {
        if token.starts_with('<') {
            let name = token
                .trim_start_matches('<')
                .trim_end_matches('>')
                .trim_end_matches('/');
            if name.trim() == "/div" {
                text.push('\n');
            }
        } else {
            text.push_str(&unescape(&token));
        }
    }
    Ok(text)
}

fn folder(key: &str) -> Value {
    json!({"$ref":{"collection":"notes.folder","native_key":key}})
}

fn notes_session_source() -> bool {
    std::env::var_os("AB_LAB_SESSION").is_some_and(|binary| !binary.is_empty())
        && std::env::var("AB_LAB_SESSION_SOURCES")
            .unwrap_or_default()
            .split(',')
            .any(|source| source.trim() == "notes")
}

fn notes_data(action: &Value, op: &str, mut data: Value) -> Result<(&'static str, Value)> {
    let args = &action["note_args"];
    let mut collection = "notes.note";
    let title = string(args, "title");
    match op {
        "notes.rename_folder" => {
            collection = "notes.folder";
            data["title"] = json!(title);
        }
        "notes.create_folder" => {
            collection = "notes.folder";
            data = json!({"title":title,"account":"sim"});
        }
        "notes.create" => {
            let text = string(args, "plain_text");
            data = json!({"folder_id":folder(&string(action,"folder_key")),"title":title,"plain_text":format!("{title}\n{text}"),"locked":false,"editable":"plain","content_status":"complete","attachments":[],"created_at":now(),"modified_at":now()});
        }
        "notes.move" => {
            data["folder_id"] = folder(&string(action, "folder_key"));
        }
        "notes.update" => {
            let mut text;
            if let Some(html) = args["html"].as_str() {
                data["html"] = json!(html);
                text = plain(html)?;
            } else {
                text = string(args, "plain_text");
            }
            if string(args, "mode") == "append" {
                text = format!("{}{text}", string(&data, "plain_text"));
            }
            data["title"] = json!(text.split('\n').next().unwrap_or_default());
            data["plain_text"] = json!(text);
            data["modified_at"] = json!(now());
        }
        _ => {}
    }
    Ok((collection, data))
}

impl Session<'_> {
    pub(super) fn note_prepare(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let action = &frame["action"];
        if action["note_args"].is_null() {
            return self.reject(frame, "invalid");
        }
        let s = lock(state)?;
        let key = string(action, "native_key");
        let mut before = Value::Null;
        if !key.is_empty() {
            let record = &s.data["records"][&key];
            if record.is_null() || record["kind"] == "absent" {
                drop(s);
                return self.reject(frame, "conflict");
            }
            before = record["data"].clone();
            let mut expected = frame["expected_state"].clone();
            let mut native = before.clone();
            for value in [&mut expected, &mut native] {
                if value.is_null() {
                    *value = json!({});
                }
            }
            for field in ["content_version", "folder_id"] {
                if let Some(object) = expected.as_object_mut() {
                    object.remove(field);
                }
            }
            if let Some(object) = native.as_object_mut() {
                object.remove("folder_id");
            }
            if !notes_session_source()
                && expected != native
                && !(expected.is_null()
                    && native.as_object().is_some_and(serde_json::Map::is_empty))
            {
                drop(s);
                return self.reject(frame, "conflict");
            }
            let args = &action["note_args"];
            if string(action, "op") == "notes.update"
                && (args["html"].is_null() && before["editable"] != "plain"
                    || args["html"]
                        .as_str()
                        .is_some_and(|html| !compatible(&before, html)))
            {
                drop(s);
                return self.reject(frame, "unsupported");
            }
        }
        drop(s);
        let ticket = id("ticket")?;
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: before.clone(),
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":before,"route":"applescript","fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    pub(super) fn note_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let Some(ticket) = self.tickets.remove(&string(frame, "ticket")) else {
            return self.reject(frame, "expired");
        };
        if ticket.prepare["command_id"] != command || Instant::now() >= ticket.deadline {
            return self.reject(frame, "expired");
        }
        let action = &ticket.prepare["action"];
        let op = string(action, "op");
        let mut key = string(action, "native_key");
        let mut s = lock(state)?;
        if !key.is_empty() && s.data["records"][&key]["data"] != ticket.before {
            drop(s);
            return self.reject(frame, "conflict");
        }
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        let data = if key.is_empty() {
            key = id("note")?;
            json!({})
        } else {
            ticket.before.clone()
        };
        let (collection, data) = notes_data(action, &op, data)?;
        let mut record = observation(collection, &key, data);
        let absent = op == "notes.delete";
        if absent {
            record["kind"] = json!("absent");
            record["revalidated"] = json!(true);
        }
        s.put(record.clone());
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        s.save()?;
        drop(s);
        self.emit(
            state,
            json!({"t":"Capture","source":"notes","observations":[record],"scan_id":id("scan")?,"scope":[collection],"scan_started_at":now(),"completion_evidence":"complete"}),
        )?;
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"source":"notes","native_key":key,"observed":true,"absent":absent,"fingerprint":ticket.prepare["request_hash"],"capture_checkpoint":self.checkpoints.get("notes")}});
        let mut s = lock(state)?;
        s.data["evidence"][&command] = outcome.clone();
        s.save()?;
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}

impl Session<'_> {
    pub(super) fn notes_primitive(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let id = frame["id"].clone();
        let answer = self.notes_answer(state, &frame["args"], &string(frame, "op"));
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

    fn notes_answer(
        &mut self,
        state: &Shared,
        args: &Value,
        op: &str,
    ) -> std::result::Result<Value, &'static str> {
        let fail = |_| "unavailable";
        let key = string(args, "native_key");
        let mut s = lock(state).map_err(fail)?;
        match op {
            "notes.read" => {
                let record = &s.data["records"][&key];
                if key.is_empty()
                    || record["collection"] != string(args, "collection").as_str()
                    || record["kind"] == "absent"
                {
                    return Ok(json!({"exists":false}));
                }
                let digest = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                    record["data"].to_string(),
                ));
                Ok(json!({"exists":true,"data":record["data"],"folder_fingerprint":digest}))
            }
            "notes.trash_folder" => Ok(json!({"scoped_alias": null})),
            "notes.observe" => {
                let target = string(args, "script_id");
                let record = &s.data["records"][&target];
                let live = !record.is_null() && record["kind"] != "absent";
                let mut result =
                    json!({"observed":live,"deletion_state":if live { "active" } else { "gone" }});
                if live {
                    result["native_key"] = json!(target);
                    result["data"] = record["data"].clone();
                }
                Ok(result)
            }
            "notes.write"
                if std::env::var("AB_LAB_PRIMITIVE_CONTROL").as_deref() == Ok("refuse") =>
            {
                Err("unavailable")
            }
            "notes.write" => {
                let action = json!({"op":args["op"],"native_key":args["native_key"],"folder_key":args["folder_key"],"note_args":args["note_args"]});
                let op = string(args, "op");
                let command = string(args, "command_id");
                let mut target = key.clone();
                let data = if target.is_empty() {
                    target = id("note").map_err(fail)?;
                    json!({})
                } else {
                    s.data["records"][&target]["data"].clone()
                };
                let (collection, data) = notes_data(&action, &op, data).map_err(fail)?;
                let mut record = observation(collection, &target, data);
                if op == "notes.delete" {
                    record["kind"] = json!("absent");
                    record["revalidated"] = json!(true);
                }
                s.put(record);
                if !command.is_empty() {
                    let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
                    s.data["effects"][&command] = json!(count);
                }
                s.save().map_err(fail)?;
                Ok(json!({"script_id":target,"failure":null}))
            }
            "notes.reconcile" => {
                let capture = s.scan("notes").map_err(fail)?;
                drop(s);
                self.emit(state, capture).map_err(fail)?;
                Ok(json!({}))
            }
            _ => Err("unsupported"),
        }
    }
}
