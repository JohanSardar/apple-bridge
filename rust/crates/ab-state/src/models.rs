use std::fmt::Write;

#[derive(serde::Serialize)]
struct Identity<'a> {
    #[serde(skip_serializing_if = "str::is_empty")]
    text: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    kind: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    from_handle_id: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    from_contact_id: &'a str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    unread: bool,
}

use crate::projection::{b, kind, normalize, s, time};
use crate::{ErrorCode, Record, Result, State, Visibility, head, records};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, params, types::Value as Sql};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
fn data(record: &Record) -> Result<Value> {
    Ok(serde_json::from_str(record.data.get())?)
}
fn list(db: &Connection, sql: &str, args: &[Sql]) -> Result<Vec<Record>> {
    let refs: Vec<&dyn rusqlite::ToSql> = args
        .iter()
        .map(|visibility| visibility as &dyn rusqlite::ToSql)
        .collect();
    records(db, sql, &refs)
}
fn text(s: &str) -> Sql {
    Sql::Text(s.into())
}
fn one(db: &Connection, c: &str, id: &str) -> Result<Record> {
    list(
        db,
        "SELECT payload FROM records WHERE collection=? AND id=? AND deleted=0",
        &[text(c), text(id)],
    )?
    .pop()
    .ok_or_else(|| ErrorCode::NotFound.into())
}
fn token(view: &str, scope: &str, at: &str, id: &str) -> Result<String> {
    #[derive(serde::Serialize)]
    struct View<'a> {
        #[serde(rename = "v")]
        visibility: &'a str,
        s: &'a str,
        t: &'a str,
        i: &'a str,
    }
    Ok(URL_SAFE_NO_PAD.encode(crate::encoded(&View {
        visibility: view,
        s: scope,
        t: at,
        i: id,
    })?))
}
fn edge(raw: &str, view: &str, scope: &str) -> Result<(String, String)> {
    if raw.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let raw = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| ErrorCode::Invalid)?;
    let visibility: Value = serde_json::from_slice(&raw).map_err(|_| ErrorCode::Invalid)?;
    if s(&visibility, "v") != view || s(&visibility, "s") != scope || s(&visibility, "i").is_empty()
    {
        return Err(ErrorCode::Invalid.into());
    }
    Ok((s(&visibility, "t").into(), s(&visibility, "i").into()))
}
fn limit(query: &Value) -> Result<usize> {
    let count = query["limit"].as_i64().unwrap_or(0);
    if !(0..=200).contains(&count) {
        return Err(ErrorCode::Invalid.into());
    }
    Ok(if count == 0 {
        50
    } else {
        usize::try_from(count).map_err(|_| ErrorCode::Invalid)?
    })
}
fn visible(visibility: &Visibility, source: &str) -> bool {
    visibility.get(source) == Some(&true)
}
fn handle(db: &Connection, visibility: &Visibility, record: &Record) -> Result<Value> {
    let fields = data(record)?;
    let address = s(&fields, "address");
    let mut out = json!({"handle_id":record.id,"address":address,"service":s(&fields,"service"),"display_name":address,"initials":""});
    if visible(visibility, "contacts") && !s(&fields, "contact_id").is_empty() {
        match one(db, "contacts.contact", s(&fields, "contact_id")) {
            Ok(c) => {
                let c = data(&c)?;
                out["contact_id"] = fields["contact_id"].clone();
                for k in ["given_name", "family_name"] {
                    if !s(&c, k).is_empty() {
                        out[k] = c[k].clone();
                    }
                }
                let names: Vec<_> = ["given_name", "middle_name", "family_name"]
                    .iter()
                    .map(|k| s(&c, k))
                    .filter(|count| !count.is_empty())
                    .collect();
                let name = if names.is_empty() {
                    if s(&c, "nickname").is_empty() {
                        s(&c, "organization").to_owned()
                    } else {
                        s(&c, "nickname").to_owned()
                    }
                } else {
                    names.join(" ")
                };
                if !name.is_empty() {
                    out["display_name"] = name.into();
                }
                for k in ["thumbnail_image", "image"] {
                    if !s(&c[k], "digest").is_empty() {
                        out["avatar"] = json!({"digest":c[k]["digest"],"variant":"original"});
                        break;
                    }
                }
            }
            Err(crate::Error::Contract(ErrorCode::NotFound)) => {}
            Err(e) => return Err(e),
        }
    }
    let name = s(&out, "display_name");
    let words: Vec<_> = name
        .split(|c: char| c.is_whitespace() || c == '@')
        .filter(|x| !x.is_empty())
        .collect();
    let mut initials = words
        .first()
        .and_then(|s| s.chars().next())
        .map_or_else(String::new, |c| c.to_uppercase().collect());
    if words.len() > 1
        && !name.contains('@')
        && let Some(c) = words.last().and_then(|s| s.chars().next())
    {
        initials.extend(c.to_uppercase());
    }
    out["initials"] = initials.into();
    Ok(out)
}
fn fold(rs: Vec<Record>) -> Result<Vec<Record>> {
    let mut current = BTreeMap::new();
    for record in rs {
        let fields = data(&record)?;
        let k = s(&fields, "kind");
        let family = match k {
            "emoji" => format!("emoji:{}", s(&fields, "emoji")),
            "sticker" => format!(
                "sticker:{}",
                json!([
                    fields["sticker_id"],
                    fields["attachment_id"],
                    fields["emoji"],
                    fields["sticker"],
                    fields["blob"]
                ])
            ),
            _ => "classic".into(),
        };
        let sender = if b(&fields, "is_from_me") {
            "self"
        } else {
            s(&fields, "sender_handle_id")
        };
        let key = format!("{sender}/{}/{family}", fields["part_index"]);
        if b(&fields, "removed") {
            if let Some(prev) = current.get(&key)
                && s(&data(prev)?, "kind") == k
            {
                current.remove(&key);
            }
        } else {
            current.insert(key, record);
        }
    }
    let mut rs: Vec<_> = current.into_values().collect();
    rs.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    Ok(rs)
}
fn thread_items(db: &Connection, rs: Vec<Record>, search: bool) -> Result<Vec<Value>> {
    let ids = serde_json::to_string(
        &rs.iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
    )?;
    let hidden = if search {
        " AND coalesce(json_extract(payload,'$.data.hidden'),0)=0"
    } else {
        ""
    };
    let attachments = list(
        db,
        &format!(
            "SELECT payload FROM attachment_by_message WHERE message_id IN (SELECT value FROM json_each(?)){hidden} ORDER BY id"
        ),
        &[text(&ids)],
    )?;
    let mut media: HashMap<String, Vec<Record>> = HashMap::new();
    for record in attachments {
        media
            .entry(crate::projection::message(&data(&record)?).into())
            .or_default()
            .push(record);
    }
    let mut reactions: HashMap<String, Vec<Record>> = HashMap::new();
    let mut counts: HashMap<String, i64> = HashMap::new();
    if !search {
        for record in list(
            db,
            "SELECT payload FROM reaction_by_message WHERE message_id IN (SELECT value FROM json_each(?)) ORDER BY at,seq,id",
            &[text(&ids)],
        )? {
            reactions
                .entry(s(&data(&record)?, "message_id").into())
                .or_default()
                .push(record);
        }
        let mut stmt=db.prepare_cached("SELECT originator,count(*) FROM msg_by_chat WHERE originator IN (SELECT value FROM json_each(?)) AND originator<>id GROUP BY originator")?;
        let mut rows = stmt.query([ids])?;
        while let Some(row) = rows.next()? {
            counts.insert(row.get(0)?, row.get(1)?);
        }
    }
    let mut out = Vec::new();
    for record in rs {
        let id = record.id.as_str();
        let mut item = serde_json::to_value(&record)?;
        item["attachments"] = serde_json::to_value(media.remove(id).unwrap_or_default())?;
        item["reactions"] = serde_json::to_value(fold(reactions.remove(id).unwrap_or_default())?)?;
        item["reply_count"] = counts.get(id).copied().unwrap_or(0).into();
        out.push(item);
    }
    Ok(out)
}
impl<Role> State<Role> {
    pub fn read_model(
        &self,
        visibility: &Visibility,
        operation: &str,
        query: &Value,
    ) -> Result<Value> {
        validate_query(operation, query)?;
        let tx = self.db.unchecked_transaction()?;
        let mut out = match operation {
            "messages.handles" => {
                let ids = query["ids"].as_array().cloned().unwrap_or_default();
                if ids.len() > 500 {
                    return Err(ErrorCode::Invalid.into());
                }
                let mut handles = Vec::new();
                let mut seen = std::collections::HashSet::new();
                if visible(visibility, "messages") {
                    for id in ids {
                        let id = id.as_str().ok_or(ErrorCode::Invalid)?;
                        if id.is_empty() {
                            return Err(ErrorCode::Invalid.into());
                        }
                        if !seen.insert(id.to_owned()) {
                            continue;
                        }
                        match one(&tx, "messages.handle", id) {
                            Ok(record) => handles.push(handle(&tx, visibility, &record)?),
                            Err(crate::Error::Contract(ErrorCode::NotFound)) => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
                json!({"handles":handles})
            }
            "health.summary" => super::specialized::health(&tx, visibility, query)?,
            "calendar.range" => super::specialized::calendar(&tx, visibility, query)?,
            "notes.list" => super::notes::list(&tx, visibility, query)?,
            "contacts.lookup" => super::specialized::lookup(&tx, visibility, query)?,
            "contacts.list" => super::specialized::contacts(&tx, visibility, query)?,
            "sources.status" => {
                let mut result = super::specialized::sources(&tx, visibility)?;
                for record in result["records"].as_array_mut().into_iter().flatten() {
                    if let Some((count, code, active)) = record["data"]["source"]
                        .as_str()
                        .and_then(|source| self.capture_failures.get(source))
                    {
                        let reason = format!("gateway_capture_rejected:{code}");
                        record["data"]["capture_rejected_count"] = (*count).into();
                        record["data"]["last_capture_rejected_reason"] = reason.clone().into();
                        if *active {
                            record["data"]["state"] = "degraded".into();
                            record["data"]["reason"] = reason.into();
                        }
                    }
                    if record["data"]["source"]
                        .as_str()
                        .is_some_and(|s| self.capacity.get(s) == Some(&true))
                    {
                        record["data"]["state"] = "degraded".into();
                        record["data"]["reason"] = "capacity".into();
                    }
                }
                if crate::scope::sees_replica_status(visibility)
                    && let Some(status) = self.project_replica_status(visibility)?
                {
                    result["replica"] = status;
                }
                result
            }
            "replica.status" => self.replica_status_result(visibility)?,
            "messages.thread" => thread(&tx, visibility, query)?,
            "messages.collection" => collection(&tx, visibility, query)?,
            "messages.search" => search(&tx, visibility, query)?,
            "messages.chats" => chats(&tx, visibility, query)?,
            "reminders.lists" => super::reminders::lists(&tx, visibility, query)?,
            "reminders.items" => super::reminders::items(&tx, visibility, query)?,
            "calendar.agenda" => agenda(&tx, visibility, query)?,
            _ => return Err(ErrorCode::Unsupported.into()),
        };
        out["cursor"] = serde_json::to_value(head(&tx)?)?;
        Ok(out)
    }
}
#[allow(clippy::too_many_lines)]
fn pin_text(db: &Connection, record: &Record, fields: &Value) -> Result<String> {
    if !s(fields, "text").is_empty() {
        return Ok(s(fields, "text").into());
    }
    let media = list(
        db,
        "SELECT payload FROM attachment_by_message WHERE message_id=? AND coalesce(json_extract(payload,'$.data.hidden'),0)=0 ORDER BY id",
        &[text(record.id.as_str())],
    )?;
    if !media.is_empty() {
        let kinds = media
            .iter()
            .map(|r| data(r).map(|d| kind(&d)))
            .collect::<Result<Vec<_>>>()?;
        let count = media.len();
        if kinds.iter().all(|k| *k == "photos") {
            return Ok(format!(
                "Attachment: {count} {}",
                if count == 1 { "Image" } else { "Images" }
            ));
        }
        if count == 1 {
            let d = data(&media[0])?;
            if s(&d, "mime_type").to_lowercase().contains("vcard") {
                return Ok("Contact Card".into());
            }
            let label = match kinds[0] {
                "videos" => "Video",
                "audio" => "Audio Message",
                "stickers" => "Sticker",
                _ => "",
            };
            if !label.is_empty() {
                return Ok(label.into());
            }
        }
        return Ok(format!(
            "Attachment: {count} {}",
            if count == 1 { "File" } else { "Files" }
        ));
    }
    Ok(match s(&fields["balloon"], "kind") {
        "link" => "Link",
        "find_my" => "Location",
        _ => "Message",
    }
    .into())
}

mod agenda;
mod chats;
mod collections;
mod messages;
mod validation;
use agenda::agenda;
use chats::{chats, summary};
use collections::{collection, search};
use messages::thread;
use validation::validate_query;
