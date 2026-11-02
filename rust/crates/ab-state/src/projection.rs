use crate::{Result, collection, encoded, number, records, set_meta, source};
use ab_domain::Record;
use rusqlite::{Connection, params};
use serde_json::Value;
pub(crate) fn time(raw: &str) -> String {
    raw.parse::<jiff::Timestamp>()
        .map_or_else(|_| raw.to_owned(), |t| format!("{t:.9}"))
}
pub(crate) fn s<'a>(fields: &'a Value, k: &str) -> &'a str {
    fields[k].as_str().unwrap_or("")
}
pub(crate) fn b(fields: &Value, k: &str) -> bool {
    fields[k].as_bool().unwrap_or(false)
}
pub(crate) fn normalize(raw: &str) -> String {
    let raw = raw.trim().to_lowercase();
    if raw.contains('@') {
        return raw;
    }
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 10 {
        format!("1{digits}")
    } else {
        digits
    }
}
pub(crate) fn kind(fields: &Value) -> &'static str {
    let mime = s(fields, "mime_type").to_lowercase();
    if mime.starts_with("image/") {
        if b(fields, "is_sticker") {
            "stickers"
        } else {
            "photos"
        }
    } else if mime.starts_with("video/") {
        "videos"
    } else if mime.starts_with("audio/") {
        "audio"
    } else {
        "documents"
    }
}
pub(crate) fn message(fields: &Value) -> &str {
    let target = s(&fields["sticker"], "target_message_id");
    if target.is_empty() {
        s(fields, "message_id")
    } else {
        target
    }
}
pub(crate) fn initialize(db: &Connection) -> Result<()> {
    let version = db
        .query_row(
            "SELECT value FROM meta WHERE key='readmodels_version'",
            [],
            |record| record.get::<_, String>(0),
        )
        .ok();
    if version.as_deref() != Some("3") {
        for record in records(
            db,
            "SELECT payload FROM records WHERE collection LIKE 'messages.%' ORDER BY CASE collection WHEN 'messages.message' THEN 0 ELSE 1 END,id",
            &[],
        )? {
            let seq: i64 = db.query_row(
                "SELECT seq FROM records WHERE collection=? AND id=?",
                params![collection(record.collection), record.id.as_str()],
                |record| record.get(0),
            )?;
            model(
                db,
                &record,
                u64::try_from(seq).map_err(|_| crate::ErrorCode::Corrupt)?,
            )?;
        }
        set_meta(db, "readmodels_version", "3")?;
        set_meta(db, "chat_participants_version", "1")?;
    }
    let fts = db
        .query_row(
            "SELECT value FROM meta WHERE key='fts_version'",
            [],
            |record| record.get::<_, String>(0),
        )
        .ok();
    if fts.as_deref() != Some("5") {
        db.execute_batch("DELETE FROM records_fts; DELETE FROM records_fts_map;")?;
        db.execute("UPDATE records SET payload=payload WHERE deleted=0", [])?;
        set_meta(db, "fts_version", "5")?;
    }
    Ok(())
}
pub(crate) fn write(db: &Connection, record: &Record, seq: u64) -> Result<()> {
    let payload = encoded(record)?;
    db.prepare_cached("INSERT INTO records VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(collection,id) DO UPDATE SET version=excluded.version,source=excluded.source,deleted=excluded.deleted,payload=excluded.payload,seq=excluded.seq,observed_at=excluded.observed_at")?.execute(params![collection(record.collection),record.id.as_str(),number(record.version)?,source(record)?,record.deleted,payload,number(seq)?,record.observed_at])?;
    model(db, record, seq)?;
    blob_refs(db, &format!("record:{}", record.id.as_str()), record)
}
pub(crate) fn blob_refs(db: &Connection, owner: &str, record: &Record) -> Result<()> {
    db.execute("DELETE FROM blob_refs WHERE owner=?", [owner])?;
    if !record.deleted {
        let fields: Value = serde_json::from_str(record.data.get())?;
        visit_blobs(db, owner, &fields, 0, false, false)?;
    }
    Ok(())
}
fn valid_digest(fields: &str) -> bool {
    fields.len() == 64
        && fields
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn visit_blobs(
    db: &Connection,
    owner: &str,
    fields: &Value,
    size: i64,
    variant: bool,
    awaiting: bool,
) -> Result<()> {
    match fields {
        Value::Object(map) => {
            let awaiting =
                awaiting || (map.contains_key("media") && !map.contains_key("derivatives"));
            let size = map.get("size").and_then(Value::as_i64).unwrap_or(size);
            for key in ["digest", "raw_body_digest"] {
                if let Some(digest) = map
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|fields| valid_digest(fields))
                {
                    let mut reference = serde_json::json!({"digest":digest,"size":if key=="digest"{size}else{0},"state":if key=="digest"{s(fields,"state")}else{""}});
                    if key == "digest" {
                        if variant {
                            reference["variant"] = true.into();
                        }
                        if awaiting && !variant {
                            reference["awaiting_variants"] = true.into();
                        }
                    }
                    db.execute("INSERT INTO blob_refs VALUES(?,?,?) ON CONFLICT(owner,digest) DO UPDATE SET payload=excluded.payload",params![owner,digest,encoded(&reference)?])?;
                }
            }
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort_by_key(|k| (*k != "derivatives", *k));
            for key in keys {
                visit_blobs(
                    db,
                    owner,
                    &map[key],
                    size,
                    variant || key == "derivatives",
                    awaiting,
                )?;
            }
        }
        Value::Array(items) => {
            for item in items {
                visit_blobs(db, owner, item, size, variant, awaiting)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn model(db: &Connection, record: &Record, seq: u64) -> Result<()> {
    let fields: Value = serde_json::from_str(record.data.get())?;
    let id = record.id.as_str();
    let payload = encoded(record)?;
    match collection(record.collection) {
        "messages.handle" => {
            db.execute("DELETE FROM handle_address WHERE id=?", [id])?;
            let address = normalize(s(&fields, "address"));
            if !record.deleted && !address.is_empty() {
                db.execute(
                    "INSERT INTO handle_address VALUES(?,?)",
                    params![id, address],
                )?;
            }
        }
        "messages.chat" => {
            db.execute("DELETE FROM chat_participant WHERE chat_id=?", [id])?;
            if !record.deleted
                && let Some(ps) = fields["participants"].as_array()
            {
                for p in ps {
                    if let Some(p) = p.as_str()
                        && !p.is_empty()
                    {
                        db.execute(
                            "INSERT OR IGNORE INTO chat_participant VALUES(?,?)",
                            params![id, p],
                        )?;
                    }
                }
            }
            db.execute("INSERT OR IGNORE INTO chat_summary(id) VALUES(?)", [id])?;
        }
        "messages.message" => {
            db.execute("DELETE FROM msg_by_chat WHERE id=?", [id])?;
            db.execute("DELETE FROM activity_by_chat WHERE message_id=?", [id])?;
            if !record.deleted {
                let unread = !b(&fields, "is_from_me")
                    && s(&fields, "kind") == "message"
                    && fields["read_at"].is_null()
                    && fields["deleted_at"].is_null();
                let chat = s(&fields, "chat_id");
                let at = time(s(&fields, "sent_at"));
                db.execute(
                    "INSERT INTO msg_by_chat VALUES(?,?,?,?,?,?)",
                    params![
                        id,
                        chat,
                        at,
                        unread,
                        s(&fields, "thread_originator_id"),
                        payload
                    ],
                )?;
                db.execute(
                    "INSERT INTO activity_by_chat VALUES(?,?,?,?)",
                    params![id, chat, at, id],
                )?;
                db.execute("INSERT INTO activity_by_chat SELECT id,?,at,message_id FROM reaction_by_message WHERE message_id=?",params![chat,id])?;
            }
        }
        "messages.reaction" => {
            db.execute("DELETE FROM reaction_by_message WHERE id=?", [id])?;
            db.execute("DELETE FROM activity_by_chat WHERE id=?", [id])?;
            if !record.deleted {
                let message = s(&fields, "message_id");
                let mut at = time(s(&fields, "sent_at"));
                if at.is_empty() {
                    at = time(&record.observed_at);
                }
                db.execute(
                    "INSERT INTO reaction_by_message VALUES(?,?,?,?,?)",
                    params![id, message, at, number(seq)?, payload],
                )?;
                db.execute("INSERT INTO activity_by_chat SELECT ?,chat_id,?,id FROM msg_by_chat WHERE id=?",params![id,at,message])?;
            }
        }
        "messages.attachment" => {
            db.execute("DELETE FROM attachment_by_message WHERE id=?", [id])?;
            if !record.deleted {
                db.execute(
                    "INSERT INTO attachment_by_message VALUES(?,?,?,?)",
                    params![id, message(&fields), kind(&fields), payload],
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}
