use super::{
    Duration, Instant, Result, Session, Shared, Ticket, Value, id, json, lock, now, observation,
    send, string,
};
use base64::Engine;

const ROOT: &str = "/Bridge Test";
const QUERY_KEYS: [&str; 4] = ["root", "path", "query", "field"];

fn join(parts: &[&str]) -> String {
    let segments: Vec<&str> = parts
        .iter()
        .flat_map(|part| part.split('/'))
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    format!("/{}", segments.join("/"))
}

fn reply(id: &Value, result: &Value, blobs: &Value) -> Value {
    json!({"t":"QueryResult","id":id,"result_json":result,"blobs":blobs})
}

fn failure(id: &Value, code: &str) -> Value {
    json!({"t":"QueryError","id":id,"code":code})
}

pub(super) fn query(state: &Shared, frame: &Value) -> Result<Option<Value>> {
    let op = string(frame, "op");
    if op != "files.search" && op != "files.read" {
        return Ok(None);
    }
    let id = &frame["id"];
    let Some(args) = frame
        .get("args")
        .and_then(Value::as_object)
        .filter(|map| map.keys().all(|key| QUERY_KEYS.contains(&key.as_str())))
    else {
        return Ok(Some(failure(id, "invalid")));
    };
    let field = |name: &str| args.get(name).and_then(Value::as_str).unwrap_or_default();
    let s = lock(state)?;
    let rows = s.records().into_iter().filter(|row| {
        row["collection"] == "files.item" && row["kind"] != "absent" && row["data"].is_object()
    });
    if op == "files.search" {
        let needle = field("query").to_lowercase();
        let items: Vec<Value> = rows
            .map(|row| row["data"].clone())
            .filter(|data| string(data, "name").to_lowercase().contains(&needle))
            .collect();
        return Ok(Some(reply(id, &json!({"items":items}), &json!([]))));
    }
    for row in rows {
        let data = &row["data"];
        if string(data, "root") != field("root") || string(data, "path") != field("path") {
            continue;
        }
        let digest = string(data, "digest");
        let bytes = s.data["blobs"][&digest]
            .as_str()
            .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok());
        let Some(bytes) = bytes else { break };
        let size = bytes.len();
        return Ok(Some(reply(
            id,
            &json!({"digest":digest,"size":size}),
            &json!([{"digest":digest,"size":size,"handle":digest}]),
        )));
    }
    Ok(Some(failure(id, "unavailable")))
}

impl Session<'_> {
    pub(super) fn file_prepare(&mut self, frame: &Value) -> Result<()> {
        let action = &frame["action"];
        let args = &action["args"];
        if !args.is_object() || string(args, "root") != ROOT {
            return self.reject(frame, "invalid");
        }
        let ticket = id("ticket")?;
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: Value::Null,
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":frame["command_id"],"ticket":ticket,"fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    fn file_conflict(&mut self, command: &str) -> Result<()> {
        send(
            &mut self.stream,
            &json!({"t":"Outcome","command_id":command,"disposition":"failed_no_effect","error":{"code":"conflict","message":"conflict"}}),
        )?;
        Ok(())
    }

    pub(super) fn file_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
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
        let action = &ticket.prepare["action"];
        let args = &action["args"];
        let op = string(action, "op");
        let (root, name) = (string(args, "root"), string(args, "name"));
        let mut key = join(&[&root, &string(args, "path")]);
        let target = join(&[&root, &string(args, "parent"), &name]);
        let writes = op == "files.create" || op == "files.write";
        if writes {
            key.clone_from(&target);
        }
        let mut s = lock(state)?;
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        let existing = s.data["records"][&key].clone();
        let exists = existing.is_object() && existing["kind"] != "absent";
        if (op == "files.create" && exists) || (op != "files.create" && !exists) {
            drop(s);
            return self.file_conflict(&command);
        }
        let mut data = if exists {
            existing["data"].clone()
        } else {
            json!({})
        };
        let mut observations = Vec::new();
        let mut evidence = json!({"source":"files","observed":true,"fingerprint":ticket.prepare["request_hash"],"path":key});
        if writes {
            let content = string(args, "content_base64");
            let size = base64::engine::general_purpose::STANDARD
                .decode(&content)
                .map(|bytes| bytes.len())
                .unwrap_or_default();
            let digest = string(args, "digest");
            s.data["blobs"][&digest] = json!(content);
            data = json!({"root":root,"path":key.trim_start_matches(&format!("{root}/")),"name":name,
                "kind":"txt","size":size,"created":null,"modified":null,"tags":[],
                "downloaded":true,"icloud_status":"local","digest":digest});
            evidence["digest"] = json!(digest);
        }
        if matches!(op.as_str(), "files.move" | "files.rename" | "files.trash") {
            let other = &s.data["records"][&target];
            if op != "files.trash" && other.is_object() && other["kind"] != "absent" {
                drop(s);
                return self.file_conflict(&command);
            }
            let mut absent = existing.clone();
            absent["kind"] = json!("absent");
            absent["revalidated"] = json!(true);
            absent["observed_at"] = json!(now());
            s.put(absent.clone());
            observations.push(absent);
            if op == "files.trash" {
                evidence["trashed"] = json!(true);
                evidence["trash_path"] = json!(format!("/.Trash/{command}"));
            } else {
                key.clone_from(&target);
                data["path"] = json!(key.trim_start_matches(&format!("{root}/")));
                data["name"] = json!(name);
                evidence["path"] = json!(key);
            }
        }
        if op == "files.set_tags" {
            let tags = if args["tags"].is_array() {
                args["tags"].clone()
            } else {
                json!([])
            };
            data["tags"] = tags.clone();
            evidence["tags"] = tags;
        }
        if op == "files.download" || op == "files.evict" {
            data["downloaded"] = json!(op == "files.download");
            evidence["downloaded"] = json!(op == "files.download");
        }
        if op != "files.trash" {
            let record = observation("files.item", &key, data);
            s.put(record.clone());
            observations.push(record);
        }
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        s.save()?;
        drop(s);
        self.emit(
            state,
            json!({"t":"Capture","source":"files","observations":observations,"scan_id":id("scan")?,"scope":["files.item"],"scan_started_at":now(),"completion_evidence":"complete"}),
        )?;
        evidence["native_key"] = json!(key);
        evidence["capture_checkpoint"] = json!(self.checkpoints.get("files"));
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":evidence});
        let mut s = lock(state)?;
        s.data["evidence"][&command] = outcome.clone();
        s.save()?;
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::join;

    #[test]
    fn join_cleans_segments() {
        assert_eq!(
            join(&["/Bridge Test", "folder", "c.txt"]),
            "/Bridge Test/folder/c.txt"
        );
        assert_eq!(join(&["/Bridge Test", "", "a.txt"]), "/Bridge Test/a.txt");
    }
}
