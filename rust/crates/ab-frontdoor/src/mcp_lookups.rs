use crate::{
    mcp_check::{Problem, mac_zone},
    mcp_facade::{Call, Step, local_time_of, message_summary, text},
};
use jiff::{SignedDuration, Timestamp, civil::Time, tz::TimeZone};
use serde_json::{Map, Value, json};

pub(crate) const FREE_SLOTS: &str = "calendar.free_slots";
pub(crate) const SHARED: &str = "messages.shared";
pub(crate) const SHARED_KINDS: &[&str] = &[
    "any", "image", "video", "audio", "pdf", "document", "file", "link",
];
const DECLINED: i64 = 3;
const CANCELED: i64 = 3;
const FREE: i64 = 1;

impl Call<'_> {
    pub(crate) async fn free_slots(&self, arguments: &Map<String, Value>) -> Step<Value> {
        let zone_name = mac_zone();
        let zone = TimeZone::get(&zone_name).unwrap_or(TimeZone::UTC);
        let now = Timestamp::now();
        let start = match text(arguments, "start") {
            Some(raw) => instant("start", raw)?,
            None => now,
        };
        let end = match text(arguments, "end") {
            Some(raw) => instant("end", raw)?,
            None => start + SignedDuration::from_hours(24 * 7),
        };
        if end <= start || end.duration_since(start) > SignedDuration::from_hours(24 * 62) {
            return Err(
                Problem::new("invalid", "end must be after start and within 62 days").field("end"),
            );
        }
        let minutes = arguments
            .get("minutes")
            .and_then(Value::as_i64)
            .unwrap_or(30);
        let opens = clock("day_start", text(arguments, "day_start").unwrap_or("09:00"))?;
        let closes = clock("day_end", text(arguments, "day_end").unwrap_or("17:00"))?;
        if closes <= opens {
            return Err(Problem::new("invalid", "day_end must be after day_start").field("day_end"));
        }
        let weekends = arguments.get("weekends").and_then(Value::as_bool) == Some(true);
        let limit = usize::try_from(arguments.get("limit").and_then(Value::as_u64).unwrap_or(10))
            .unwrap_or(10);
        let mut agenda = json!({"start": start.to_string(), "end": end.to_string()});
        let mut calendar_name = None;
        if let Some(wanted) = text(arguments, "calendar") {
            let (id, name) = self.calendar_for(Some(wanted), false).await?;
            agenda["calendar_id"] = json!(id);
            calendar_name = name;
        }
        let found = self
            .invoke(self.operation("calendar.agenda")?, &agenda)
            .await?;
        let mut busy: Vec<(Timestamp, Timestamp)> = found["records"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|record| blocking(&record["data"]))
            .collect();
        busy.sort();
        let mut slots = Vec::new();
        let mut day = start.to_zoned(zone.clone()).date();
        let last = end.to_zoned(zone.clone()).date();
        while day <= last && slots.len() < limit {
            let weekday = day.weekday().to_monday_one_offset();
            if weekends || weekday <= 5 {
                let open = day
                    .to_datetime(opens)
                    .to_zoned(zone.clone())
                    .map_err(|_| range())?;
                let close = day
                    .to_datetime(closes)
                    .to_zoned(zone.clone())
                    .map_err(|_| range())?;
                let mut cursor = open.timestamp().max(start);
                let close = close.timestamp().min(end);
                for (from, until) in busy.iter().filter(|(f, _)| *f < close) {
                    if *until <= cursor {
                        continue;
                    }
                    if from.duration_since(cursor) >= SignedDuration::from_mins(minutes) {
                        slots.push(slot(cursor, *from, &zone));
                    }
                    cursor = cursor.max(*until);
                }
                if close.duration_since(cursor) >= SignedDuration::from_mins(minutes) {
                    slots.push(slot(cursor, close, &zone));
                }
            }
            day = day.tomorrow().map_err(|_| range())?;
        }
        slots.truncate(limit);
        let mut value = json!({
            "slots": slots,
            "minutes": minutes,
            "time_zone": zone_name,
            "searched": {"start": start.to_string(), "end": end.to_string(), "day_start": &opens.to_string()[..5], "day_end": &closes.to_string()[..5], "weekends": weekends},
            "next": "Each slot is a free stretch at least minutes long between timed events that are not free, declined or canceled; all-day events do not block time. To book one, calendar_add with a start inside it.",
        });
        if let Some(name) = calendar_name {
            value["calendar"] = json!(name);
        }
        Ok(value)
    }

    pub(crate) async fn shared(&self, arguments: &Map<String, Value>) -> Step<Value> {
        let kind = text(arguments, "kind").unwrap_or("any");
        if !SHARED_KINDS.contains(&kind) {
            return Err(Problem::new("invalid", "unknown kind")
                .field("kind")
                .expected(json!(SHARED_KINDS)));
        }
        let limit = usize::try_from(arguments.get("limit").and_then(Value::as_u64).unwrap_or(20))
            .unwrap_or(20);
        let scan = usize::try_from(arguments.get("scan").and_then(Value::as_u64).unwrap_or(600))
            .unwrap_or(600);
        let chat_id = self.shared_chat(arguments).await?;
        let thread = self.operation("messages.thread")?;
        let mut found = Vec::new();
        let mut examined = 0;
        let mut before: Option<String> = None;
        let mut complete = true;
        'pages: loop {
            let mut page = json!({"chat_id": chat_id, "limit": 200});
            match &before {
                Some(cursor) => page["before"] = json!(cursor),
                None => page["anchor"] = json!("latest"),
            }
            let result = self.invoke(thread, &page).await?;
            let mut items: Vec<Value> = result["items"].as_array().cloned().unwrap_or_default();
            items.reverse();
            for message in &items {
                if examined >= scan || found.len() >= limit {
                    complete = false;
                    break 'pages;
                }
                examined += 1;
                let summary = message_summary(message);
                for item in Self::shared_in(message, kind) {
                    found.push((summary.clone(), item));
                }
            }
            match result["before"].as_str() {
                Some(cursor) if !items.is_empty() && examined < scan => {
                    before = Some(cursor.to_owned());
                }
                Some(_) => {
                    complete = false;
                    break;
                }
                None => break,
            }
        }
        complete &= found.len() <= limit;
        found.truncate(limit);
        let summaries = self
            .with_names(found.iter().map(|(s, _)| s.clone()).collect())
            .await;
        let items: Vec<Value> = found
            .into_iter()
            .zip(summaries)
            .map(|((_, mut item), summary)| {
                item["message_id"] = summary["id"].clone();
                item["sent_at"] = summary["sent_at"].clone();
                if let Some(local) = local_time_of(summary["sent_at"].as_str()) {
                    item["sent_at_local"] = json!(local);
                }
                item["from"] = summary.get("from").cloned().unwrap_or(Value::Null);
                item
            })
            .collect();
        Ok(json!({
            "chat_id": chat_id,
            "kind": kind,
            "items": items,
            "examined_messages": examined,
            "complete": complete,
            "next": if complete {
                "These are all shared items of this kind in the chat, newest first. To get a file's bytes, apple_read bridge.read_blob with record_id (the attachment_id) and digest."
            } else {
                "Newest first; older messages were not examined. Raise scan or limit to look further back. To get a file's bytes, apple_read bridge.read_blob with record_id (the attachment_id) and digest."
            },
        }))
    }

    async fn shared_chat(&self, arguments: &Map<String, Value>) -> Step<String> {
        Ok(
            match (text(arguments, "chat_id"), text(arguments, "with")) {
                (Some(_), Some(_)) => {
                    return Err(
                        Problem::new("invalid", "give with or chat_id, not both").field("with")
                    );
                }
                (Some(chat), None) => chat.to_owned(),
                (None, Some(who)) => {
                    let person = self.person(who, "with").await?;
                    let chats = self.direct_chats(&person.handles).await?;
                    chats
                        .first()
                        .and_then(|c| c["chat"]["id"].as_str())
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            Problem::new("not_found", format!("no one-to-one chat with {who}"))
                                .field("with")
                                .hint("For a group chat, pass its chat_id from messages_read.")
                        })?
                }
                (None, None) => {
                    return Err(Problem::new("invalid", "with or chat_id is required")
                        .field("with")
                        .expected(json!("a contact name, phone number or email, or a cht_ ID")));
                }
            },
        )
    }

    fn shared_in(message: &Value, kind: &str) -> Vec<Value> {
        let mut found = Vec::new();
        if matches!(kind, "any" | "link") {
            for url in links(&message["data"]) {
                found.push(json!({"kind": "link", "url": url}));
            }
        }
        if kind == "link" {
            return found;
        }
        for file in message["attachments"].as_array().into_iter().flatten() {
            let fields = &file["data"];
            if file["deleted"] == true
                || fields["is_sticker"] == true
                || fields["genmoji"] == true
                || fields["hidden"] == true
            {
                continue;
            }
            let what = file_kind(fields);
            if kind == "any" || kind == what || kind == "file" {
                let mut item = json!({
                    "kind": what,
                    "attachment_id": file["id"],
                    "filename": file_name(fields["filename"].as_str()),
                    "mime_type": fields["mime_type"],
                    "size": fields["size"],
                });
                if let Some(digest) = fields["blob"]["digest"].as_str() {
                    item["digest"] = json!(digest);
                }
                found.push(item);
            }
        }
        found
    }
}

fn range() -> Problem {
    Problem::new("invalid", "date out of range")
}

fn instant(field: &str, raw: &str) -> Step<Timestamp> {
    crate::mcp_compose::moment(field, raw, false)?
        .parse()
        .map_err(|_| {
            Problem::new("invalid", format!("{field} is not a time")).field(field.to_owned())
        })
}

fn clock(field: &str, raw: &str) -> Step<Time> {
    raw.parse::<Time>().map_err(|_| {
        Problem::new("invalid", format!("{field} is not a time of day"))
            .field(field.to_owned())
            .expected(json!("HH:MM, such as 09:00"))
    })
}

fn blocking(event: &Value) -> Option<(Timestamp, Timestamp)> {
    if event["all_day"] == true
        || event["availability"].as_i64() == Some(FREE)
        || event["status"].as_i64() == Some(CANCELED)
        || event["owner_participation_status"].as_i64() == Some(DECLINED)
    {
        return None;
    }
    let start: Timestamp = event["start"].as_str()?.parse().ok()?;
    let end: Timestamp = event["end"].as_str()?.parse().ok()?;
    (end > start).then_some((start, end))
}

fn slot(from: Timestamp, until: Timestamp, zone: &TimeZone) -> Value {
    let local = |t: Timestamp| {
        t.to_zoned(zone.clone())
            .strftime("%a %Y-%m-%d %H:%M")
            .to_string()
    };
    json!({
        "start": from.to_string(),
        "end": until.to_string(),
        "start_local": local(from),
        "end_local": local(until),
        "minutes": until.duration_since(from).as_mins(),
    })
}

fn links(data: &Value) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    if data["balloon"]["kind"] == "link" {
        for key in ["url", "original_url"] {
            if let Some(url) = data["balloon"][key]
                .as_str()
                .or_else(|| data["balloon"]["link"][key].as_str())
            {
                urls.push(url.to_owned());
            }
        }
    }
    for word in data["text"].as_str().unwrap_or_default().split_whitespace() {
        let word = word.trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '<' | '>' | '"' | '\'' | ',' | '.' | '!' | '?'
            )
        });
        if (word.starts_with("https://") || word.starts_with("http://")) && word.len() > 10 {
            urls.push(word.to_owned());
        }
    }
    urls.dedup();
    let mut seen = Vec::new();
    urls.retain(|u| {
        let fresh = !seen.contains(u);
        seen.push(u.clone());
        fresh
    });
    urls
}

fn file_kind(fields: &Value) -> &'static str {
    let mime = fields["mime_type"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    let uti = fields["uti"].as_str().unwrap_or_default().to_lowercase();
    let extension = std::path::Path::new(fields["filename"].as_str().unwrap_or_default())
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if mime == "application/pdf" || uti == "com.adobe.pdf" || extension == "pdf" {
        "pdf"
    } else if mime.starts_with("image/")
        || uti.contains("image")
        || ["heic", "heif", "jpg", "jpeg", "png", "gif", "webp", "tiff"]
            .contains(&extension.as_str())
    {
        "image"
    } else if mime.starts_with("video/")
        || uti.contains("movie")
        || uti.contains("video")
        || ["mov", "mp4", "m4v"].contains(&extension.as_str())
    {
        "video"
    } else if mime.starts_with("audio/")
        || uti.contains("audio")
        || ["caf", "m4a", "mp3", "aac", "wav", "amr"].contains(&extension.as_str())
    {
        "audio"
    } else if mime.starts_with("text/")
        || mime.contains("officedocument")
        || mime.contains("msword")
        || mime.contains("ms-excel")
        || mime.contains("ms-powerpoint")
        || uti.contains("iwork")
        || uti.contains("rtf")
        || uti.contains("spreadsheet")
        || uti.contains("presentation")
    {
        "document"
    } else {
        "file"
    }
}

fn file_name(raw: Option<&str>) -> Value {
    raw.and_then(|path| path.rsplit('/').find(|part| !part.is_empty()))
        .map_or(Value::Null, |name| json!(name))
}
