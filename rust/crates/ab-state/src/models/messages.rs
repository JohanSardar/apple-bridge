use super::*;

pub(super) fn thread(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
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
