use super::{
    Connection, ErrorCode, Record, Result, Sql, Value, Visibility, b, data, edge, handle, json,
    kind, limit, list, normalize, one, params, pin_text, s, text, token, visible,
};

#[allow(clippy::too_many_lines)]
pub(super) fn chats(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
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
            .map_or(Value::Null, |timestamp| {
                json!(timestamp > crate::replay::now())
            })
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
                .is_some_and(|attachments| !attachments.is_empty())
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
                for attachment in &media {
                    let k = kind(&data(attachment)?);
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
