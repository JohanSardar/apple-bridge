use super::{Request, Result, Shared, Value, enqueue, invalid, json, lock, mpsc, now, string};

pub(super) const REFUSED: &str = "container-removal-refused";

fn collection(kind: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match kind {
        "calendar" => Ok(("calendar.calendar", "calendar.event", "calendar_id")),
        "reminder_list" => Ok(("reminders.list", "reminders.reminder", "list_id")),
        "notes_folder" => Ok(("notes.folder", "notes.note", "folder_id")),
        _ => Err(invalid("container kind")),
    }
}

fn tag_of(input: &Value) -> Result<String> {
    let tag = string(input, "tag");
    if tag.starts_with("[ab-test ") && tag.ends_with(']') && tag.len() > 11 && !tag.contains('\n') {
        Ok(tag)
    } else {
        Err(invalid("run tag"))
    }
}

fn live(record: &Value) -> bool {
    !record.is_null() && record["kind"] != "absent"
}

fn title(record: &Value) -> &str {
    record["data"]["title"].as_str().unwrap_or_default()
}

fn owned(title: &str, tag: &str) -> bool {
    let base = format!("Bridge Test {tag}");
    title == base || title.starts_with(&format!("{base} "))
}

pub(super) fn control(state: &Shared, tx: &mpsc::Sender<Request>, input: &Value) -> Result<Value> {
    match input["action"].as_str() {
        Some("remove_created") => remove(state, tx, input),
        Some("remaining") => remaining(state, input),
        _ => Err(invalid("containers action")),
    }
}

fn remove(state: &Shared, tx: &mpsc::Sender<Request>, input: &Value) -> Result<Value> {
    let (container, item, field) = collection(&string(input, "kind"))?;
    let tag = tag_of(input)?;
    let key = string(input, "id");
    if key.is_empty() {
        return Err(invalid("container id"));
    }
    let mut s = lock(state)?;
    if s.fault == REFUSED {
        return Err(invalid("container removal refused"));
    }
    let record = s.data["records"][&key].clone();
    if !live(&record) {
        return Ok(json!({"removed": false, "absent": true}));
    }
    if record["collection"] != container || !owned(title(&record), &tag) {
        return Err(invalid("container not owned by this run"));
    }
    let contents: Vec<Value> = s
        .records()
        .into_iter()
        .filter(|r| {
            r["collection"] == item
                && live(r)
                && r["data"][field]["$ref"]["native_key"] == key.as_str()
        })
        .collect();
    if contents.iter().any(|r| !title(r).contains(&tag)) {
        return Err(invalid("container holds untagged content"));
    }
    for mut gone in contents.into_iter().chain(std::iter::once(record)) {
        gone["kind"] = json!("absent");
        gone["revalidated"] = json!(true);
        gone["observed_at"] = json!(now());
        s.put(gone);
    }
    s.save()?;
    let source = container.split('.').next().unwrap_or_default();
    let frame = s.scan(source)?;
    drop(s);
    enqueue(tx, frame, false)?;
    Ok(json!({"removed": true, "absent": true}))
}

fn remaining(state: &Shared, input: &Value) -> Result<Value> {
    let tags: Vec<String> = input["tags"]
        .as_array()
        .ok_or_else(|| invalid("tags"))?
        .iter()
        .map(|tag| tag_of(&json!({"tag": tag})))
        .collect::<Result<_>>()?;
    let created = input["created"]
        .as_array()
        .ok_or_else(|| invalid("created"))?;
    let s = lock(state)?;
    let records = s.records();
    let mut counts = json!({});
    for kind in input["kinds"].as_array().ok_or_else(|| invalid("kinds"))? {
        let kind = kind.as_str().unwrap_or_default();
        let (container, _, _) = collection(kind)?;
        let ids: Vec<&str> = created
            .iter()
            .filter(|entry| entry["kind"] == kind)
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        let count = records
            .iter()
            .filter(|r| r["collection"] == container && live(r))
            .filter(|r| {
                ids.contains(&string(r, "native_key").as_str())
                    || tags.iter().any(|tag| title(r).contains(tag.as_str()))
            })
            .count();
        counts[kind] = json!(count);
    }
    Ok(counts)
}
