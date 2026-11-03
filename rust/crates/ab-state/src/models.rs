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
                if let Some(status) = self.project_replica_status(visibility)? {
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
fn thread(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let count = limit(query)?;
    let chat = s(query, "chat_id");
    let before = s(query, "before");
    let after = s(query, "after");
    let anchor = s(query, "anchor");
    let root = s(query, "thread_originator_id");
    if chat.is_empty()
        || !before.is_empty() && !after.is_empty()
        || !anchor.is_empty() && (!before.is_empty() || !after.is_empty())
    {
        return Err(ErrorCode::Invalid.into());
    }
    let scope = if root.is_empty() {
        chat.to_owned()
    } else {
        String::from_utf8_lossy(&crate::encoded(&[chat, root])?).into_owned()
    };
    let (at, id) = edge(
        if before.is_empty() { after } else { before },
        "thread",
        &scope,
    )?;
    let mut out = json!({"items":[]});
    if !visible(visibility, "messages") {
        return Ok(out);
    }
    one(db, "messages.chat", chat)?;
    let mut args = vec![text(chat)];
    let mut selection = "msg_by_chat WHERE chat_id=?".to_owned();
    if !root.is_empty() {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM msg_by_chat WHERE chat_id=? AND id=?)",
            params![chat, root],
            |record| record.get(0),
        )?;
        if !exists {
            return Err(ErrorCode::NotFound.into());
        }
        selection.push_str(" AND (id=? OR originator=?)");
        args.extend([text(root), text(root)]);
    }
    let query = |condition: &str,
                 order: &str,
                 count: usize,
                 extra: Vec<Sql>|
     -> Result<Vec<Record>> {
        let mut a = args.clone();
        a.extend(extra);
        a.push(Sql::Integer(
            i64::try_from(count).map_err(|_| ErrorCode::Invalid)?,
        ));
        list(
            db,
            &format!(
                "SELECT payload FROM {selection} {condition} ORDER BY sent_at {order},id {order} LIMIT ?"
            ),
            &a,
        )
    };
    let mut rs;
    if !before.is_empty() {
        rs = query(
            "AND (sent_at,id)<(?,?)",
            "DESC",
            count,
            vec![text(&at), text(&id)],
        )?;
        rs.reverse();
    } else if !after.is_empty() {
        rs = query(
            "AND (sent_at,id)>(?,?)",
            "ASC",
            count,
            vec![text(&at), text(&id)],
        )?;
    } else if anchor == "oldest" {
        rs = query("", "ASC", count, vec![])?;
    } else if !anchor.is_empty() && anchor != "latest" {
        let mut a = args.clone();
        if anchor != "unread" {
            a.push(text(anchor));
        }
        let found = list(
            db,
            &format!(
                "SELECT payload FROM {selection} AND {} ORDER BY sent_at,id LIMIT 1",
                if anchor == "unread" {
                    "unread=1"
                } else {
                    "id=?"
                }
            ),
            &a,
        )?;
        if let Some(record) = found.first() {
            let fields = data(record)?;
            let at = time(s(&fields, "sent_at"));
            if anchor == "unread" {
                out["unread_anchor"] = record.id.as_str().into();
            }
            rs = query(
                "AND (sent_at,id)<(?,?)",
                "DESC",
                count / 2,
                vec![text(&at), text(record.id.as_str())],
            )?;
            rs.reverse();
            rs.extend(query(
                "AND (sent_at,id)>=(?,?)",
                "ASC",
                count - rs.len(),
                vec![text(&at), text(record.id.as_str())],
            )?);
        } else if anchor == "unread" {
            rs = query("", "DESC", count, vec![])?;
            rs.reverse();
        } else {
            return Err(ErrorCode::NotFound.into());
        }
    } else {
        rs = query("", "DESC", count, vec![])?;
        rs.reverse();
    }
    if let (Some(first), Some(last)) = (rs.first(), rs.last()) {
        for (record, op, key) in [(first, "<", "before"), (last, ">", "after")] {
            let fields = data(record)?;
            let at = time(s(&fields, "sent_at"));
            if !query(
                &format!("AND (sent_at,id){op}(?,?)"),
                "ASC",
                1,
                vec![text(&at), text(record.id.as_str())],
            )?
            .is_empty()
            {
                out[key] = token("thread", &scope, &at, record.id.as_str())?.into();
            }
        }
    }
    out["items"] = thread_items(db, rs, false)?.into();
    Ok(out)
}
fn collection(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let count = limit(query)?;
    let chat = s(query, "chat_id");
    let k = s(query, "kind");
    if chat.is_empty()
        || ![
            "photos",
            "videos",
            "media",
            "audio",
            "documents",
            "links",
            "locations",
        ]
        .contains(&k)
    {
        return Err(ErrorCode::Invalid.into());
    }
    let scope = format!("{chat}:{k}");
    let (at, id) = edge(s(query, "after"), "collection", &scope)?;
    let mut out = json!({"items":[],"coverage_notes":[]});
    if !visible(visibility, "messages") {
        return Ok(out);
    }
    one(db, "messages.chat", chat)?;
    let mut args = vec![text(chat)];
    let (select, from, filter, ident) = if ["links", "locations"].contains(&k) {
        args.push(text(if k == "links" { "link" } else { "find_my" }));
        (
            "m.payload",
            "msg_by_chat m",
            "json_extract(m.payload,'$.data.balloon.kind')=?",
            "m.id",
        )
    } else {
        let select = if k == "media" {
            "json_set(a.payload,'$.data.message_id',m.id,'$.data.sent_at',m.sent_at)"
        } else {
            "a.payload"
        };
        let filter = if k == "media" {
            "a.kind IN ('photos','videos')"
        } else {
            args.push(text(k));
            "a.kind=?"
        };
        (
            select,
            "msg_by_chat m JOIN attachment_by_message a ON a.message_id=m.id",
            filter,
            "a.id",
        )
    };
    let mut sql = format!(
        "SELECT {select} FROM {from} WHERE m.chat_id=? AND {filter} AND json_extract(m.payload,'$.data.deleted_at') IS NULL"
    );
    if ident == "a.id" {
        sql.push_str(" AND coalesce(json_extract(a.payload,'$.data.hidden'),0)=0");
    }
    if !s(query, "after").is_empty() {
        let _ = write!(sql, " AND (m.sent_at<? OR (m.sent_at=? AND {ident}>?))");
        args.extend([text(&at), text(&at), text(&id)]);
    }
    let _ = write!(sql, " ORDER BY m.sent_at DESC,{ident} LIMIT ?");
    args.push(Sql::Integer(
        i64::try_from(count + 1).map_err(|_| ErrorCode::Invalid)?,
    ));
    let mut rs = list(db, &sql, &args)?;
    if rs.len() > count {
        rs.truncate(count);
        if let Some(last) = rs.last() {
            let fields = data(last)?;
            let at = if ident == "m.id" {
                time(s(&fields, "sent_at"))
            } else {
                db.query_row(
                    "SELECT sent_at FROM msg_by_chat WHERE id=?",
                    [s(&fields, "message_id")],
                    |record| record.get(0),
                )?
            };
            out["next"] = token("collection", &scope, &at, last.id.as_str())?.into();
        }
    }
    out["items"] = serde_json::to_value(rs)?;
    Ok(out)
}
fn search(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let count = limit(query)?;
    let txt = s(query, "text").trim();
    let k = s(query, "kind");
    let h = s(query, "from_handle_id");
    let c = s(query, "from_contact_id");
    if txt.chars().count() > 512
        || h.len() > 128
        || c.len() > 128
        || !h.is_empty() && !c.is_empty()
        || !["", "photos", "links", "documents", "locations"].contains(&k)
        || txt.is_empty() && k.is_empty() && h.is_empty() && c.is_empty() && !b(query, "unread")
    {
        return Err(ErrorCode::Invalid.into());
    }
    if !c.is_empty() && !visible(visibility, "contacts") {
        return Err(ErrorCode::Unauthorized.into());
    }
    let scope = format!(
        "{:x}",
        Sha256::digest(crate::encoded(&Identity {
            text: txt,
            kind: k,
            from_handle_id: h,
            from_contact_id: c,
            unread: b(query, "unread")
        })?)
    );
    let (at, id) = edge(s(query, "after"), "message-search", &scope)?;
    let mut out = json!({"items":[],"coverage_notes":[]});
    if !visible(visibility, "messages") {
        return Ok(out);
    }
    let mut sql="SELECT m.payload FROM msg_by_chat m JOIN records c ON c.collection='messages.chat' AND c.id=m.chat_id WHERE c.deleted=0 AND coalesce(json_extract(c.payload,'$.data.filter'),'')!='spam' AND coalesce(json_extract(c.payload,'$.data.is_spam'),0)=0 AND coalesce(json_extract(c.payload,'$.data.is_archived'),0)=0 AND json_extract(c.payload,'$.data.deleted_at') IS NULL AND json_extract(m.payload,'$.data.deleted_at') IS NULL AND json_extract(m.payload,'$.data.retracted_at') IS NULL AND coalesce(json_extract(m.payload,'$.data.is_spam'),0)=0".to_owned();
    let mut args = Vec::new();
    if !txt.is_empty() {
        let terms = txt
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        if terms.is_empty() {
            return Ok(out);
        }
        sql.push_str(" AND (m.id IN (SELECT id FROM records_fts WHERE collection='messages.message' AND records_fts MATCH ?) OR m.id IN (SELECT a.message_id FROM attachment_by_message a JOIN records_fts ON records_fts.collection='messages.attachment' AND records_fts.id=a.id WHERE records_fts MATCH ? AND coalesce(json_extract(a.payload,'$.data.hidden'),0)=0))");
        args.extend([text(&terms), text(&terms)]);
    }
    if !h.is_empty() {
        sql.push_str(" AND json_extract(m.payload,'$.data.sender_handle_id')=?");
        args.push(text(h));
    }
    if !c.is_empty() {
        sql.push_str(" AND json_extract(m.payload,'$.data.sender_handle_id') IN (SELECT id FROM records WHERE collection='messages.handle' AND deleted=0 AND json_extract(payload,'$.data.contact_id')=?)");
        args.push(text(c));
    }
    if b(query, "unread") {
        sql.push_str(" AND m.unread=1");
    }
    match k {
        "photos" | "documents" => {
            let _ = write!(
                sql,
                " AND EXISTS(SELECT 1 FROM attachment_by_message a WHERE a.message_id=m.id AND {} AND coalesce(json_extract(a.payload,'$.data.hidden'),0)=0)",
                if k == "photos" {
                    "a.kind IN ('photos','videos')"
                } else {
                    "a.kind='documents'"
                }
            );
        }
        "links" | "locations" => {
            sql.push_str(" AND json_extract(m.payload,'$.data.balloon.kind')=?");
            args.push(text(if k == "links" { "link" } else { "find_my" }));
        }
        _ => {}
    }
    if !s(query, "after").is_empty() {
        sql.push_str(" AND (m.sent_at<? OR (m.sent_at=? AND m.id>?))");
        args.extend([text(&at), text(&at), text(&id)]);
    }
    sql.push_str(" ORDER BY m.sent_at DESC,m.id LIMIT ?");
    args.push(Sql::Integer(
        i64::try_from(count + 1).map_err(|_| ErrorCode::Invalid)?,
    ));
    let mut rs = list(db, &sql, &args)?;
    if rs.len() > count {
        rs.truncate(count);
        if let Some(last) = rs.last() {
            out["next"] = token(
                "message-search",
                &scope,
                &time(s(&data(last)?, "sent_at")),
                last.id.as_str(),
            )?
            .into();
        }
    }
    out["items"] = thread_items(db, rs, true)?.into();
    Ok(out)
}
#[allow(clippy::too_many_lines)]
fn chats(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let count = limit(query)?;
    let filter = s(query, "filter");
    if ![
        "", "all", "known", "unknown", "unread", "deleted", "spam", "archived",
    ]
    .contains(&filter)
    {
        return Err(ErrorCode::Invalid.into());
    }
    let mut handles: Option<Vec<String>> = None;
    if let Some(ps) = query["participants"].as_array() {
        if ps.is_empty() || ps.len() > 32 {
            return Err(ErrorCode::Invalid.into());
        }
        let mut hs = Vec::new();
        for p in ps {
            let p = p.as_str().ok_or(ErrorCode::Invalid)?;
            let normalized = normalize(p);
            if normalized.is_empty() || p.chars().count() > 254 {
                return Err(ErrorCode::Invalid.into());
            }
            hs.push(normalized);
        }
        hs.sort();
        hs.dedup();
        handles = Some(hs);
    }
    let scope = if let Some(hs) = &handles {
        format!(
            "{filter}:participants:{}",
            String::from_utf8_lossy(&crate::encoded(hs)?)
        )
    } else {
        filter.to_owned()
    };
    let (at, id) = edge(s(query, "after"), "chats", &scope)?;
    let mut out =
        json!({"chats":[],"coverage_notes":["reactions without sent_at use observed_at"]});
    if !visible(visibility, "messages") {
        return Ok(out);
    }
    let mut sql="SELECT r.payload FROM chat_summary s INDEXED BY chat_activity CROSS JOIN records r ON r.collection='messages.chat' AND r.id=s.id WHERE r.deleted=0".to_owned();
    let mut args = Vec::new();
    if let Some(hs) = handles {
        let raw = serde_json::to_string(&hs)?;
        sql.push_str(" AND r.id IN (SELECT p.chat_id FROM handle_address h JOIN chat_participant p ON p.handle_id=h.id WHERE h.address IN (SELECT value FROM json_each(?)) GROUP BY p.chat_id HAVING COUNT(DISTINCT h.address)=?) AND NOT EXISTS(SELECT 1 FROM chat_participant extra LEFT JOIN handle_address address ON address.id=extra.handle_id WHERE extra.chat_id=r.id AND (address.address IS NULL OR address.address NOT IN (SELECT value FROM json_each(?))))");
        args.extend([
            text(&raw),
            Sql::Integer(i64::try_from(hs.len()).map_err(|_| ErrorCode::Invalid)?),
            text(&raw),
        ]);
    }
    if filter == "archived" {
        sql.push_str(" AND coalesce(json_extract(r.payload,'$.data.is_archived'),0)=1");
    } else if filter != "deleted" {
        sql.push_str(" AND coalesce(json_extract(r.payload,'$.data.is_archived'),0)=0");
    }
    if filter == "deleted" {
        sql.push_str(" AND (json_extract(r.payload,'$.data.deleted_at') IS NOT NULL OR EXISTS(SELECT 1 FROM msg_by_chat m WHERE m.chat_id=s.id AND json_extract(m.payload,'$.data.deleted_at') IS NOT NULL))");
    } else {
        sql.push_str(" AND json_extract(r.payload,'$.data.deleted_at') IS NULL");
    }
    if filter == "unread" {
        sql.push_str(" AND (s.unread_count>0 OR json_extract(r.payload,'$.data.is_unread')=1)");
    }
    if filter == "spam" {
        sql.push_str(" AND (coalesce(json_extract(r.payload,'$.data.is_spam'),0)=1 OR EXISTS(SELECT 1 FROM msg_by_chat m WHERE m.chat_id=s.id AND json_extract(m.payload,'$.data.is_spam')=1))");
    }
    if ["known", "unknown"].contains(&filter) {
        sql.push_str(if filter == "unknown" {
            " AND NOT "
        } else {
            " AND "
        });
        sql.push_str("(coalesce(json_extract(r.payload,'$.data.filter'),'known')='known' OR EXISTS(SELECT 1 FROM json_each(r.payload,'$.data.participants') p CROSS JOIN records h ON h.collection='messages.handle' AND h.id=p.value AND h.deleted=0 WHERE json_extract(h.payload,'$.data.contact_id') IS NOT NULL))");
    }
    if !s(query, "after").is_empty() {
        sql.push_str(" AND (s.last_activity_at<? OR (s.last_activity_at=? AND s.id>?))");
        args.extend([text(&at), text(&at), text(&id)]);
    }
    sql.push_str(" ORDER BY s.last_activity_at DESC,s.id LIMIT ?");
    args.push(Sql::Integer(
        i64::try_from(count + 1).map_err(|_| ErrorCode::Invalid)?,
    ));
    let mut rs = list(db, &sql, &args)?;
    if rs.len() > count {
        rs.truncate(count);
        if let Some(last) = rs.last() {
            let at: String = db.query_row(
                "SELECT last_activity_at FROM chat_summary WHERE id=?",
                [last.id.as_str()],
                |record| record.get(0),
            )?;
            out["next"] = token("chats", &scope, &at, last.id.as_str())?.into();
        }
    }
    let mut summaries = Vec::new();
    for record in rs {
        summaries.push(summary(db, visibility, &record)?);
    }
    out["chats"] = summaries.into();
    Ok(out)
}
#[allow(clippy::too_many_lines)]
fn summary(db: &Connection, visibility: &Visibility, record: &Record) -> Result<Value> {
    let fields = data(record)?;
    let id = record.id.as_str();
    let (at, unread): (String, i64) = db.query_row(
        "SELECT last_activity_at,max(unread_count,?) FROM chat_summary WHERE id=?",
        params![i64::from(b(&fields, "is_unread")), id],
        |record| Ok((record.get(0)?, record.get(1)?)),
    )?;
    let mention:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM msg_by_chat WHERE chat_id=? AND json_extract(payload,'$.data.has_unseen_mention')=1 AND json_extract(payload,'$.data.deleted_at') IS NULL)",[id],|record|record.get(0))?;
    let muted = if fields["muted"].is_boolean() {
        fields["muted"].clone()
    } else {
        s(&fields, "muted_until")
            .parse::<jiff::Timestamp>()
            .ok()
            .map_or(Value::Null, |t| json!(t > jiff::Timestamp::now()))
    };
    let mut known = fields["filter"].is_null() || s(&fields, "filter") == "known";
    let mut avatars = Vec::new();
    let mut names = Vec::new();
    if let Some(ps) = fields["participants"].as_array() {
        for p in ps {
            if let Some(id) = p.as_str() {
                match one(db, "messages.handle", id) {
                    Ok(h) => {
                        let hd = data(&h)?;
                        known = known || !hd["contact_id"].is_null();
                        let h = handle(db, visibility, &h)?;
                        if avatars.len() < 3 {
                            avatars.push(json!({"id":id,"contact_id":h["contact_id"]}));
                        }
                        if !s(&h, "display_name").is_empty() {
                            names.push(s(&h, "display_name").to_owned());
                        }
                    }
                    Err(crate::Error::Contract(ErrorCode::NotFound)) => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }
    let title = if s(&fields, "display_name").is_empty() {
        match names.len() {
            0 => String::new(),
            1 => names[0].clone(),
            2 => names.join(" & "),
            3 => format!("{}, {} & {}", names[0], names[1], names[2]),
            count => format!("{}, {} & {} others", names[0], names[1], count - 2),
        }
    } else {
        s(&fields, "display_name").to_owned()
    };
    let mut last = Value::Null;
    let activity = list(
        db,
        "SELECT m.payload FROM activity_by_chat a JOIN msg_by_chat m ON m.id=a.message_id WHERE a.chat_id=? ORDER BY a.at DESC,a.id DESC LIMIT 1",
        &[text(id)],
    )?;
    if let Some(m) = activity.first() {
        let md = data(m)?;
        let mut preview = json!({"kind":"text"});
        if !s(&md, "text").is_empty() {
            preview["text"] = md["text"].clone();
        }
        if !md["retracted_at"].is_null()
            || md["retracted_parts"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        {
            preview["kind"] = "unsent".into();
        } else if s(&md, "kind") == "group_event" {
            preview["kind"] = "group_event".into();
            if md["group_event"].is_object() {
                preview["group_event"] = md["group_event"].clone();
            }
        } else if !md["edited_at"].is_null() {
            preview["kind"] = "edited".into();
        } else {
            let media = list(
                db,
                "SELECT payload FROM attachment_by_message WHERE message_id=? AND coalesce(json_extract(payload,'$.data.hidden'),0)=0 ORDER BY id",
                &[text(m.id.as_str())],
            )?;
            if !media.is_empty() {
                let mut kinds = Vec::new();
                for a in &media {
                    let k = kind(&data(a)?);
                    if !kinds.contains(&k) {
                        kinds.push(k);
                    }
                }
                preview["kind"] = "attachments".into();
                preview["count"] = media.len().into();
                preview["attachment_kinds"] = json!(kinds);
            }
        }
        let aid:String=db.query_row("SELECT id FROM activity_by_chat WHERE chat_id=? AND message_id=? ORDER BY at DESC,id DESC LIMIT 1",params![id,m.id.as_str()],|record|record.get(0))?;
        if aid != m.id.as_str() {
            let rd = data(&one(db, "messages.reaction", &aid)?)?;
            let k = s(&rd, "kind");
            let verb = if b(&rd, "removed") {
                String::from("Removed reaction from")
            } else {
                match k {
                    "love" => "Loved".into(),
                    "like" => "Liked".into(),
                    "dislike" => "Disliked".into(),
                    "laugh" => "Laughed at".into(),
                    "emphasize" => "Emphasized".into(),
                    "question" => "Questioned".into(),
                    "emoji" => format!("Reacted {} to", s(&rd, "emoji")),
                    "sticker" => "Reacted with a sticker to".into(),
                    _ => String::new(),
                }
            };
            let mut reaction = json!({"verb":verb,"target_excerpt":s(&md,"text")});
            if !k.is_empty() {
                reaction["kind"] = k.into();
            }
            if b(&rd, "removed") {
                reaction["removed"] = true.into();
            }
            preview = json!({"kind":"reaction","reaction":reaction});
        }
        last = json!({"id":m.id,"sent_at":s(&md,"sent_at"),"is_from_me":b(&md,"is_from_me"),"preview":preview});
        for k in ["service", "sender_handle_id"] {
            if !s(&md, k).is_empty() {
                last[k] = md[k].clone();
            }
        }
    }
    let mut pin = Value::Null;
    if unread > 0 {
        let unread = list(
            db,
            "SELECT payload FROM msg_by_chat WHERE chat_id=? AND unread=1 ORDER BY sent_at DESC,id DESC LIMIT 1",
            &[text(id)],
        )?;
        if let Some(m) = unread.first() {
            let md = data(m)?;
            let fallback = pin_text(db, m, &md)?;
            let txt = fallback.as_str();
            let txt = if txt.chars().count() > 200 {
                format!("{}…", txt.chars().take(200).collect::<String>())
            } else {
                txt.into()
            };
            pin = json!({"message_id":m.id,"text_preview":txt,"at":s(&md,"sent_at")});
        }
    }
    Ok(
        json!({"chat":record,"title":title,"avatar_handles":avatars,"last_message":last,"last_activity_at":at,"unread_count":unread,"has_unseen_mention":mention,"muted":muted,"pinned_index":fields["pinned_index"],"known_sender":known,"pin_preview":pin}),
    )
}
fn agenda(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let start = s(query, "start")
        .parse::<jiff::Timestamp>()
        .map_err(|_| ErrorCode::Invalid)?;
    let end = s(query, "end")
        .parse::<jiff::Timestamp>()
        .map_err(|_| ErrorCode::Invalid)?;
    if end <= start {
        return Err(ErrorCode::Invalid.into());
    }
    let mut out = Vec::new();
    if visible(visibility, "calendar") {
        for record in list(
            db,
            "SELECT payload FROM records WHERE collection='calendar.event' AND deleted=0",
            &[],
        )? {
            let fields = data(&record)?;
            if !s(query, "calendar_id").is_empty()
                && s(&fields, "calendar_id") != s(query, "calendar_id")
            {
                continue;
            }
            let overlaps = if b(&fields, "all_day") {
                let start_day = &s(query, "start")[..10];
                let raw_end = s(query, "end");
                let mut end_day = raw_end[..10]
                    .parse::<jiff::civil::Date>()
                    .map_err(|_| ErrorCode::Invalid)?;
                let local_time = raw_end[11..]
                    .split(['Z', '+', '-'])
                    .next()
                    .ok_or(ErrorCode::Invalid)?
                    .parse::<jiff::civil::Time>()
                    .map_err(|_| ErrorCode::Invalid)?;
                if local_time != jiff::civil::Time::midnight() {
                    end_day = end_day
                        .checked_add(jiff::Span::new().days(1))
                        .map_err(|_| ErrorCode::Invalid)?;
                }
                s(&fields, "start") < end_day.to_string().as_str() && s(&fields, "end") > start_day
            } else {
                match (
                    s(&fields, "start").parse::<jiff::Timestamp>(),
                    s(&fields, "end").parse::<jiff::Timestamp>(),
                ) {
                    (Ok(a), Ok(b)) => a < end && b > start,
                    _ => false,
                }
            };
            if overlaps {
                out.push((s(&fields, "start").to_owned(), record));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.as_str().cmp(b.1.id.as_str())));
    Ok(json!({"records":out.into_iter().map(|(_,record)|record).collect::<Vec<_>>()}))
}

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

fn validate_query(operation: &str, query: &Value) -> Result<()> {
    let allowed = match operation {
        "messages.chats" => "participants filter limit after",
        "messages.thread" => "thread_originator_id chat_id anchor before after limit",
        "messages.collection" => "chat_id kind after limit",
        "messages.search" => "text kind from_handle_id from_contact_id unread after limit",
        "messages.handles" => "ids",
        "reminders.lists" => "time_zone",
        "reminders.items" => "time_zone list smart tag show_completed sort",
        "calendar.agenda" => "start end calendar_id",
        "calendar.range" => "start end calendars time_zone text",
        "contacts.lookup" => "q limit name handle",
        "contacts.list" => "group sort display_order",
        "notes.list" => "folder sort time_zone",
        "health.summary" => "period date time_zone device_id",
        "replica.status" | "sources.status" => "",
        _ => return Err(ErrorCode::Unsupported.into()),
    };
    let fields = query.as_object().ok_or(ErrorCode::Invalid)?;
    for (key, value) in fields {
        if !allowed.split_whitespace().any(|a| a == key) {
            return Err(ErrorCode::Invalid.into());
        }
        if value.is_null() {
            continue;
        }
        let valid = match key.as_str() {
            "limit" => value.as_i64().is_some(),
            "unread" | "show_completed" => value.is_boolean(),
            "participants" | "ids" | "calendars" => value
                .as_array()
                .is_some_and(|items| items.iter().all(|v| v.is_string() || v.is_null())),
            _ => value.is_string(),
        };
        if !valid {
            return Err(ErrorCode::Invalid.into());
        }
    }
    Ok(())
}
