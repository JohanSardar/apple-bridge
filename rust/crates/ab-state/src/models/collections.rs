use super::{
    Connection, ErrorCode, Identity, Result, Sha256, Sql, Value, Visibility, b, data, edge, json,
    limit, list, one, s, text, thread_items, time, token, visible,
};
use sha2::Digest;
use std::fmt::Write;

pub(super) fn collection(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
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
pub(super) fn search(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
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
            .filter(|term| !term.is_empty())
            .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
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
