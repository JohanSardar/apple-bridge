use crate::projection::{b, s, valid_digest};
use crate::{ErrorCode, Record, Result, Visibility, records};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
fn data(record: &Record) -> Result<Value> {
    Ok(serde_json::from_str(record.data.get())?)
}
fn preview(raw: &str, title: &str) -> String {
    let raw = raw.replace("\r\n", "\n");
    let mut lines = raw.split('\n').peekable();
    if lines.peek().is_some_and(|line| line.trim() == title.trim()) {
        lines.next();
    }
    for line in lines {
        let line = line
            .replace('\u{fffc}', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !line.is_empty() {
            return line.chars().take(160).collect();
        }
    }
    String::new()
}
fn thumbnail(fields: &Value) -> Value {
    let thumb = &fields["thumbnail"];
    if valid_digest(s(thumb, "digest")) && s(thumb, "state") != "unavailable" {
        return json!({"digest":thumb["digest"],"variant":"thumb"});
    }
    for attachment in fields["attachments"].as_array().into_iter().flatten() {
        let thumb = &attachment["derivatives"]["thumb"];
        if valid_digest(s(thumb, "digest")) && s(thumb, "state") != "unavailable" {
            return json!({"digest":thumb["digest"],"variant":"thumb"});
        }
    }
    Value::Null
}
#[allow(clippy::too_many_lines)]
pub(crate) fn list(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let sort = if s(query, "sort").is_empty() {
        "modified"
    } else {
        s(query, "sort")
    };
    let zone_name = if s(query, "time_zone").is_empty() {
        "UTC"
    } else {
        s(query, "time_zone")
    };
    let zone = jiff::tz::TimeZone::get(zone_name).map_err(|_| ErrorCode::Invalid)?;
    if !["modified", "created", "title"].contains(&sort) {
        return Err(ErrorCode::Invalid.into());
    }
    let mut out = json!({"accounts":[],"folders":[],"sections":[],"count":0,"sort":sort,"time_zone":zone_name});
    if visibility.get("notes") != Some(&true) {
        return Ok(out);
    }
    let stored = records(
        db,
        "SELECT payload FROM records WHERE collection IN ('notes.folder','notes.note') AND deleted=0 ORDER BY collection,id",
        &[],
    )?;
    let mut folders = BTreeMap::<String, Value>::new();
    for record in &stored {
        if record.collection != ab_domain::Collection::NotesFolder {
            continue;
        }
        let fields = data(record)?;
        let parent = s(&fields, "parent_id");
        let recent = b(&fields, "recently_deleted");
        folders.insert(record.id.as_str().into(),json!({"folder":record,"count":0,"account_id":s(&fields,"account"),"parent_id":if parent.is_empty()||parent==record.id.as_str(){Value::Null}else{json!(parent)},"read_only":recent||b(&fields,"read_only"),"recently_deleted":recent}));
    }
    if !s(query, "folder").is_empty() && !folders.contains_key(s(query, "folder")) {
        return Err(ErrorCode::NotFound.into());
    }
    let mut accounts = BTreeMap::<String, Value>::new();
    for id in folders.keys().cloned().collect::<Vec<_>>() {
        let mut folder = folders[&id].clone();
        let mut seen = HashSet::from([id.clone()]);
        let mut parent = s(&folder, "parent_id").to_owned();
        while !parent.is_empty() {
            if !seen.insert(parent.clone()) {
                return Err(ErrorCode::Corrupt.into());
            }
            let Some(ancestor) = folders
                .get(&parent)
                .filter(|ancestor| ancestor["account_id"] == folder["account_id"])
            else {
                folder["parent_id"] = Value::Null;
                break;
            };
            folder["read_only"] = json!(b(&folder, "read_only") || b(ancestor, "read_only"));
            folder["recently_deleted"] =
                json!(b(&folder, "recently_deleted") || b(ancestor, "recently_deleted"));
            parent = s(ancestor, "parent_id").into();
        }
        let account = s(&folder, "account_id").to_owned();
        let fields = &folder["folder"]["data"];
        let title = s(fields, "account_name");
        let entry = accounts
            .entry(account.clone())
            .or_insert_with(|| json!({"id":account,"title":"Notes","count":0,"folder_ids":[]}));
        if !title.is_empty() {
            entry["title"] = title.into();
        }
        if let Some(ids) = entry["folder_ids"].as_array_mut() {
            ids.push(id.clone().into());
        }
        folders.insert(id, folder);
    }
    let mut items = Vec::new();
    for record in &stored {
        if record.collection != ab_domain::Collection::NotesNote {
            continue;
        }
        let fields = data(record)?;
        let folder_id = s(&fields, "folder_id");
        let folder = folders.get(folder_id).cloned().unwrap_or(Value::Null);
        let recent = b(&folder, "recently_deleted") || b(&fields, "recently_deleted");
        if let Some(folder) = folders.get_mut(folder_id) {
            folder["count"] = json!(folder["count"].as_i64().unwrap_or(0) + 1);
            if !recent && let Some(account) = accounts.get_mut(s(folder, "account_id")) {
                account["count"] = json!(account["count"].as_i64().unwrap_or(0) + 1);
            }
        }
        if s(query, "folder").is_empty() && recent
            || !s(query, "folder").is_empty() && s(query, "folder") != folder_id
        {
            continue;
        }
        let mut item = json!({"note":record,"preview":"","thumbnail":null,"pinned":fields["pinned"],"read_only":recent||b(&folder,"read_only")||s(&fields,"editable")!="plain"});
        if b(&fields, "locked") || s(&fields, "content_status") == "locked" {
            item["read_only"] = true.into();
            item["note"]["data"] = json!({"title":s(&fields,"title"),"folder_id":folder_id,"locked":true,"content_status":"locked","editable":"none","created_at":s(&fields,"created_at"),"modified_at":s(&fields,"modified_at")});
        } else {
            let mut text = preview(s(&fields, "preview"), "");
            if text.is_empty() {
                text = preview(s(&fields, "plain_text"), s(&fields, "title"));
            }
            item["preview"] = text.into();
            item["thumbnail"] = thumbnail(&fields);
        }
        let date = s(
            &fields,
            if sort == "created" {
                "created_at"
            } else {
                "modified_at"
            },
        )
        .parse::<jiff::Timestamp>()
        .ok();
        items.push((item, s(&fields, "title").to_lowercase(), date));
    }
    items.sort_by(|a, right| {
        b(&right.0, "pinned")
            .cmp(&b(&a.0, "pinned"))
            .then(if sort == "title" {
                a.1.cmp(&right.1)
            } else {
                right.2.cmp(&a.2)
            })
            .then(s(&a.0["note"], "id").cmp(s(&right.0["note"], "id")))
    });
    out["count"] = items.len().into();
    let today = crate::replay::now().to_zoned(zone.clone()).date();
    let mut sections = Vec::<Value>::new();
    for (item, _, date) in items {
        let mut key = "older";
        let mut title = "Earlier";
        if b(&item, "pinned") {
            key = "pinned";
            title = "Pinned";
        } else if sort == "title" {
            key = "notes";
            title = "Notes";
        } else if let Some(date) = date {
            let day = date.to_zoned(zone.clone()).date();
            for (days, k, t) in [
                (0, "today", "Today"),
                (1, "yesterday", "Yesterday"),
                (7, "previous_7_days", "Previous 7 Days"),
                (30, "previous_30_days", "Previous 30 Days"),
            ] {
                if day
                    >= today
                        .checked_sub(jiff::Span::new().days(days))
                        .map_err(|_| ErrorCode::Invalid)?
                {
                    key = k;
                    title = t;
                    break;
                }
            }
        }
        if sections.last().is_none_or(|sec| s(sec, "key") != key) {
            sections.push(json!({"key":key,"title":title,"notes":[]}));
        }
        if let Some(sec) = sections.last_mut()
            && let Some(notes) = sec["notes"].as_array_mut()
        {
            notes.push(item);
        }
    }
    let mut folders = folders.into_values().collect::<Vec<_>>();
    folders.sort_by(|a, b| {
        s(a, "account_id")
            .cmp(s(b, "account_id"))
            .then(
                s(&a["folder"]["data"], "title")
                    .to_lowercase()
                    .cmp(&s(&b["folder"]["data"], "title").to_lowercase()),
            )
            .then(s(&a["folder"], "id").cmp(s(&b["folder"], "id")))
    });
    let mut accounts = accounts.into_values().collect::<Vec<_>>();
    for account in &mut accounts {
        if let Some(ids) = account["folder_ids"].as_array_mut() {
            ids.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
    }
    accounts.sort_by(|a, b| {
        s(a, "title")
            .cmp(s(b, "title"))
            .then(s(a, "id").cmp(s(b, "id")))
    });
    out["folders"] = folders.into();
    out["accounts"] = accounts.into();
    out["sections"] = sections.into();
    Ok(out)
}
