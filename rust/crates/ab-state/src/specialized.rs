use crate::projection::{b, normalize, s};
use crate::{ErrorCode, Record, Result, Visibility, allows, project, records};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
fn data(record: &Record) -> Result<Value> {
    Ok(serde_json::from_str(record.data.get())?)
}
fn all(db: &Connection, collection: &str) -> Result<Vec<Record>> {
    records(
        db,
        "SELECT payload FROM records WHERE collection=? AND deleted=0 ORDER BY id",
        &[&collection],
    )
}
fn show(visibility: &Visibility, source: &str) -> bool {
    visibility.get(source) == Some(&true) || visibility.get("admin") == Some(&true)
}
fn handles(fields: &Value) -> Vec<String> {
    ["phones", "emails"]
        .iter()
        .flat_map(|key| fields[key].as_array().into_iter().flatten())
        .map(|entry| normalize(s(entry, "value")))
        .filter(|s| !s.is_empty())
        .collect()
}
pub(crate) fn lookup(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let raw = s(query, "q");
    let needle = raw.trim();
    let mut count = query["limit"].as_i64().unwrap_or(0);
    if count < 0
        || needle.chars().count() > 64
        || !raw.is_empty() && needle.is_empty()
        || query.get("q").is_some() && raw.is_empty()
    {
        return Err(ErrorCode::Invalid.into());
    }
    if !needle.is_empty() && count == 0 || count > 25 {
        count = 25;
    }
    let phone = if needle
        .chars()
        .all(|c| c.is_ascii_digit() || c.is_whitespace() || "+().-".contains(c))
    {
        normalize(needle)
    } else {
        String::new()
    };
    let mut selected = Vec::new();
    if show(visibility, "contacts") {
        for record in all(db, "contacts.contact")? {
            let fields = data(&record)?;
            let name = [
                "given_name",
                "middle_name",
                "family_name",
                "nickname",
                "organization",
            ]
            .iter()
            .map(|key| s(&fields, key))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
            let mut matches = s(query, "name").is_empty() && s(query, "handle").is_empty();
            if !s(query, "name").is_empty() {
                matches = name.contains(&s(query, "name").to_lowercase());
            }
            if !s(query, "handle").is_empty() {
                matches = handles(&fields).contains(&normalize(s(query, "handle")));
            }
            if !needle.is_empty() {
                let mut found = name.trim().contains(&needle.to_lowercase());
                for key in ["phones", "emails"] {
                    for entry in fields[key].as_array().into_iter().flatten() {
                        let value = s(entry, "value");
                        found |= if key == "emails" {
                            value.to_lowercase().contains(&needle.to_lowercase())
                        } else {
                            !phone.is_empty() && normalize(value).contains(&phone)
                        };
                    }
                }
                matches &= found;
            }
            if matches {
                selected.push((name.trim().to_owned(), record));
            }
        }
    }
    if !needle.is_empty() || count > 0 {
        selected.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.id.as_str().cmp(b.1.id.as_str())));
        if count > 0 {
            selected.truncate(usize::try_from(count).map_err(|_| ErrorCode::Invalid)?);
        }
    }
    Ok(json!({"records":selected.into_iter().map(|(_,record)|record).collect::<Vec<_>>()}))
}
fn name(fields: &Value, order: &str) -> String {
    if s(fields, "contact_type") == "organization" && !s(fields, "organization").trim().is_empty() {
        return s(fields, "organization").trim().into();
    }
    let keys = if order == "family" {
        ["family_name", "given_name", "middle_name"]
    } else {
        ["given_name", "middle_name", "family_name"]
    };
    let parts = keys
        .iter()
        .map(|key| s(fields, key).trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if !parts.is_empty() {
        return parts.join(" ");
    }
    for key in ["nickname", "organization"] {
        if !s(fields, key).trim().is_empty() {
            return s(fields, key).trim().into();
        }
    }
    String::new()
}
fn fold(raw: &str) -> String {
    raw.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}
fn section(raw: &str) -> String {
    raw.chars()
        .next()
        .filter(char::is_ascii_lowercase)
        .map_or_else(|| "#".into(), |c| c.to_ascii_uppercase().into())
}
#[allow(clippy::too_many_lines)]
pub(crate) fn contacts(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let sort = if s(query, "sort").is_empty() {
        "family"
    } else {
        s(query, "sort")
    };
    let display = if s(query, "display_order").is_empty() {
        "given"
    } else {
        s(query, "display_order")
    };
    if !["given", "family"].contains(&sort) || !["given", "family"].contains(&display) {
        return Err(ErrorCode::Invalid.into());
    }
    let mut out = json!({"sections":[],"groups":[],"containers":[],"count":0,"sort":sort,"display_order":display});
    if !show(visibility, "contacts") {
        return Ok(out);
    }
    let contacts = all(db, "contacts.contact")?;
    let mut groups = Vec::new();
    let mut members = None;
    for record in all(db, "contacts.group")? {
        let fields = data(&record)?;
        let ids: HashSet<_> = fields["member_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| contacts.iter().any(|record| record.id.as_str() == *id))
            .map(str::to_owned)
            .collect();
        if record.id.as_str() == s(query, "group") {
            members = Some(ids.clone());
        }
        groups.push((
            fold(s(&fields, "name")),
            json!({"group":record,"count":ids.len()}),
        ));
    }
    if !s(query, "group").is_empty() && members.is_none() {
        return Err(ErrorCode::NotFound.into());
    }
    groups.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(s(&a.1["group"], "id").cmp(s(&b.1["group"], "id")))
    });
    out["groups"] = groups
        .into_iter()
        .map(|(_, g)| g)
        .collect::<Vec<_>>()
        .into();
    out["containers"] = serde_json::to_value(all(db, "contacts.container")?)?;
    let mut conversations: HashMap<String, Vec<String>> = HashMap::new();
    if show(visibility, "messages") {
        let mut stmt=db.prepare_cached("SELECT DISTINCT json_extract(h.payload,'$.data.contact_id'),p.chat_id FROM records h JOIN chat_participant p ON p.handle_id=h.id JOIN records c ON c.collection='messages.chat' AND c.id=p.chat_id AND c.deleted=0 WHERE h.collection='messages.handle' AND h.deleted=0 AND json_extract(h.payload,'$.data.contact_id') IS NOT NULL ORDER BY p.chat_id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            conversations
                .entry(row.get(0)?)
                .or_default()
                .push(row.get(1)?);
        }
    }
    let mut items = Vec::new();
    for record in contacts {
        if members
            .as_ref()
            .is_some_and(|m| !m.contains(record.id.as_str()))
        {
            continue;
        }
        let fields = data(&record)?;
        let display_name = name(&fields, display);
        let sort_key = fold(&name(&fields, sort));
        let display_name = if display_name.is_empty() {
            "No Name".into()
        } else {
            display_name
        };
        let initials = display_name
            .split_whitespace()
            .take(2)
            .filter_map(|w| w.chars().next())
            .flat_map(char::to_uppercase)
            .collect::<String>();
        let mut avatar = Value::Null;
        for key in ["thumbnail_image", "image"] {
            if crate::projection::valid_digest(s(&fields[key], "digest"))
                && s(&fields[key], "state") != "unavailable"
            {
                avatar = json!({"digest":fields[key]["digest"],"variant":"original"});
                break;
            }
        }
        items.push(json!({"contact":record,"display_name":display_name,"sort_key":sort_key,"initials":initials,"avatar":avatar,"is_me":fields["is_me"],"conversation_ids":conversations.get(record.id.as_str()).cloned().unwrap_or_default()}));
    }
    items.sort_by(|a, b| {
        section(s(a, "sort_key"))
            .cmp(&section(s(b, "sort_key")))
            .then(s(a, "sort_key").cmp(s(b, "sort_key")))
            .then(s(&a["contact"], "id").cmp(s(&b["contact"], "id")))
    });
    out["count"] = items.len().into();
    let mut sections = Vec::<Value>::new();
    for item in items {
        let key = section(s(&item, "sort_key"));
        if sections.last().is_none_or(|sec| s(sec, "key") != key) {
            sections.push(json!({"key":key,"contacts":[]}));
        }
        if let Some(sec) = sections.last_mut()
            && let Some(items) = sec["contacts"].as_array_mut()
        {
            items.push(item);
        }
    }
    out["sections"] = sections.into();
    Ok(out)
}
pub(crate) fn sources(db: &Connection, visibility: &Visibility) -> Result<Value> {
    let mut out = Vec::new();
    for record in all(db, "system.source")? {
        if !allows(visibility, &record)? {
            continue;
        }
        let mut record = project(record, visibility)?;
        let mut fields = data(&record)?;
        let source = s(&fields, "source").to_owned();
        let mut grants = vec![format!("{source}:read"), format!("{source}:write")];
        if source == "system" {
            grants.push("admin".into());
        }
        fields["grants"] = json!(grants);
        record.data = serde_json::value::to_raw_value(&fields)?;
        out.push(record);
    }
    out.truncate(1000);
    Ok(json!({"records":out}))
}
fn zone(query: &Value) -> Result<jiff::tz::TimeZone> {
    jiff::tz::TimeZone::get(if s(query, "time_zone").is_empty() {
        "UTC"
    } else {
        s(query, "time_zone")
    })
    .map_err(|_| ErrorCode::Invalid.into())
}
fn instant(raw: &str, zone: &jiff::tz::TimeZone) -> Option<jiff::Timestamp> {
    raw.parse().ok().or_else(|| {
        raw.parse::<jiff::civil::Date>()
            .ok()?
            .at(0, 0, 0, 0)
            .to_zoned(zone.clone())
            .ok()
            .map(|t| t.timestamp())
    })
}
fn float(value: f64) -> Value {
    let raw = value.to_string();
    serde_json::from_str(&raw).unwrap_or(Value::Null)
}
pub(crate) fn health(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    if !show(visibility, "health") {
        return Err(ErrorCode::Unauthorized.into());
    }
    let period = s(query, "period");
    if !["day", "week"].contains(&period) {
        return Err(ErrorCode::Invalid.into());
    }
    let zone = zone(query)?;
    let mut date = s(query, "date")
        .parse::<jiff::civil::Date>()
        .map_err(|_| ErrorCode::Invalid)?;
    if period == "week" {
        date = date
            .checked_sub(jiff::Span::new().days(i64::from(date.weekday().to_monday_zero_offset())))
            .map_err(|_| ErrorCode::Invalid)?;
    }
    let start = date
        .at(0, 0, 0, 0)
        .to_zoned(zone.clone())
        .map_err(|_| ErrorCode::Invalid)?;
    let end = start
        .checked_add(jiff::Span::new().days(if period == "week" { 7 } else { 1 }))
        .map_err(|_| ErrorCode::Invalid)?;
    let mut metrics = BTreeMap::<String, Value>::new();
    for record in all(db, "health.sample")? {
        let fields = data(&record)?;
        if !s(query, "device_id").is_empty() && s(query, "device_id") != s(&fields, "device_id") {
            continue;
        }
        let first = instant(s(&fields, "start"), &zone).ok_or(ErrorCode::Corrupt)?;
        let last = instant(s(&fields, "end"), &zone).ok_or(ErrorCode::Corrupt)?;
        if first >= end.timestamp()
            || last < start.timestamp()
            || last == start.timestamp() && first < start.timestamp()
        {
            continue;
        }
        let mut key = format!("{}/{}", s(&fields, "type"), s(&fields, "unit"));
        if let Some(category) = fields["category"].as_i64() {
            let _ = write!(key, "/{category}");
        }
        key = key.trim_end_matches('/').into();
        let mut metric = metrics
            .get(&key)
            .cloned()
            .unwrap_or_else(|| json!({"count":0,"sum":0,"average":0,"min":0,"max":0,"seconds":0}));
        let count = metric["count"].as_i64().unwrap_or(0) + 1;
        metric["count"] = count.into();
        if let Some(value) = fields["value"].as_f64() {
            if first < start.timestamp() {
                continue;
            }
            let sum = metric["sum"].as_f64().unwrap_or(0.0) + value;
            metric["sum"] = float(sum);
            if count == 1 || value < metric["min"].as_f64().unwrap_or(0.0) {
                metric["min"] = float(value);
            }
            if count == 1 || value > metric["max"].as_f64().unwrap_or(0.0) {
                metric["max"] = float(value);
            }
            metric["average"] = float(sum / metric["count"].as_f64().unwrap_or(1.0));
        }
        let seconds = (last.min(end.timestamp()).as_nanosecond()
            - first.max(start.timestamp()).as_nanosecond())
        .to_string()
        .parse::<f64>()
        .map_err(|_| ErrorCode::Corrupt)?
            / 1e9;
        metric["seconds"] = float(metric["seconds"].as_f64().unwrap_or(0.0) + seconds);
        metrics.insert(key, metric);
    }
    Ok(
        json!({"start":start.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string().replace("+00:00","Z"),"end":end.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string().replace("+00:00","Z"),"metrics":metrics}),
    )
}
#[allow(clippy::too_many_lines)]
pub(crate) fn calendar(db: &Connection, visibility: &Visibility, query: &Value) -> Result<Value> {
    let start = s(query, "start")
        .parse::<jiff::Timestamp>()
        .map_err(|_| ErrorCode::Invalid)?;
    let end = s(query, "end")
        .parse::<jiff::Timestamp>()
        .map_err(|_| ErrorCode::Invalid)?;
    let zone = zone(query)?;
    if end <= start
        || end.as_second() - start.as_second() > 1464 * 24 * 3600
        || query["calendars"].as_array().is_some_and(|a| a.len() > 256)
        || s(query, "text").len() > 256
    {
        return Err(ErrorCode::Invalid.into());
    }
    let mut out = json!({"events":[],"calendars":[],"coverage_notes":[]});
    if !show(visibility, "calendar") {
        return Ok(out);
    }
    out["calendars"] = serde_json::to_value(all(db, "calendar.calendar")?)?;
    let selected = query["calendars"].as_array().cloned().unwrap_or_default();
    let text = s(query, "text").to_lowercase();
    let mut nodes = Vec::new();
    let mut coverage_notes = Vec::new();
    let civil_start = start.to_zoned(zone.clone()).date().to_string();
    let local_end = end.to_zoned(zone.clone());
    let mut civil_end = local_end.date();
    if local_end.time() != jiff::civil::Time::midnight() {
        civil_end = civil_end
            .checked_add(jiff::Span::new().days(1))
            .map_err(|_| ErrorCode::Invalid)?;
    }
    let civil_end = civil_end.to_string();
    let end_raw = end.to_string();
    let start_raw = start.to_string();
    let candidates = records(
        db,
        "SELECT payload FROM records WHERE collection='calendar.event' AND deleted=0 AND coalesce(json_extract(payload,'$.data.all_day'),0)=0 AND julianday(json_extract(payload,'$.data.start'))<=julianday(?) AND julianday(json_extract(payload,'$.data.end'))>=julianday(?) UNION ALL SELECT payload FROM records WHERE collection='calendar.event' AND deleted=0 AND json_extract(payload,'$.data.all_day')=1 AND json_extract(payload,'$.data.start')<? AND json_extract(payload,'$.data.end')>? LIMIT 10001",
        &[&end_raw, &start_raw, &civil_end, &civil_start],
    )?;
    if candidates.len() > 10000 {
        return Err(ErrorCode::Capacity.into());
    }
    for record in candidates {
        let fields = data(&record)?;
        if !selected.is_empty() && !selected.iter().any(|id| id == s(&fields, "calendar_id")) {
            continue;
        }
        if !text.is_empty() {
            let mut content = [
                s(&fields, "title"),
                s(&fields, "location"),
                s(&fields, "notes"),
            ]
            .join(" ");
            for attendee in fields["attendees"].as_array().into_iter().flatten() {
                content.push(' ');
                content.push_str(s(attendee, "name"));
                content.push(' ');
                content.push_str(s(attendee, "url"));
            }
            if !content.to_lowercase().contains(&text) {
                continue;
            }
        }
        let (Some(first), Some(last)) = (
            instant(s(&fields, "start"), &zone),
            instant(s(&fields, "end"), &zone),
        ) else {
            coverage_notes.push("An event has an unsupported date range");
            continue;
        };
        if last <= first {
            coverage_notes.push("An event has an unsupported date range");
            continue;
        }
        if first >= end || last <= start {
            continue;
        }
        nodes.push((record, first, last, b(&fields, "all_day"), 0usize));
    }
    if nodes.len() > 10000 {
        return Err(ErrorCode::Capacity.into());
    }
    nodes.sort_by(|a, b| {
        b.3.cmp(&a.3)
            .then(a.1.cmp(&b.1))
            .then(b.2.cmp(&a.2))
            .then(a.0.id.as_str().cmp(b.0.id.as_str()))
    });
    let mut events = Vec::new();
    let mut group = 0usize;
    let mut group_end = start;
    let mut columns = Vec::<jiff::Timestamp>::new();
    for index in 0..nodes.len() {
        let node = &nodes[index];
        if index == 0 || node.3 != nodes[index - 1].3 || node.1 >= group_end {
            finish_group(&nodes, group, index, &columns, &mut events);
            group = index;
            group_end = node.2;
            columns.clear();
        }
        let col = columns
            .iter()
            .position(|end| *end <= node.1)
            .unwrap_or(columns.len());
        if col == columns.len() {
            columns.push(node.2);
        } else {
            columns[col] = node.2;
        }
        group_end = group_end.max(node.2);
        nodes[index].4 = col;
    }
    finish_group(&nodes, group, nodes.len(), &columns, &mut events);
    out["events"] = events.into();
    out["coverage_notes"] = json!(coverage_notes);
    Ok(out)
}
type CalendarNode = (Record, jiff::Timestamp, jiff::Timestamp, bool, usize);
fn finish_group(
    nodes: &[CalendarNode],
    first: usize,
    end: usize,
    columns: &[jiff::Timestamp],
    out: &mut Vec<Value>,
) {
    if first == end {
        return;
    }
    for node in &nodes[first..end] {
        out.push(json!({"event":node.0,"overlap_group":nodes[first].0.id,"column":node.4,"columns":columns.len()}));
    }
}
