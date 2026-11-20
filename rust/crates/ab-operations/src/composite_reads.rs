use crate::{Code, Error, Principal, ReadQuery, Service, Visibility};
use jiff::{
    SignedDuration, Timestamp,
    civil::{Date, Time},
    tz::TimeZone,
};
use serde::Serialize;
use serde_json::{Value, json, value::RawValue};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct UsageSummaryQuery {
    pub first: Date,
    pub last: Date,
    pub device: Option<String>,
    pub top: usize,
}

#[derive(Debug, Serialize)]
pub struct UsageSummary {
    pub first: String,
    pub last: String,
    pub total_seconds: u64,
    pub top_apps: Vec<AppUsage>,
    pub devices: Vec<DeviceUsage>,
    pub complete: bool,
}

#[derive(Debug, Serialize)]
pub struct AppUsage {
    pub bundle_id: String,
    pub seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct DeviceUsage {
    pub id: String,
    pub name: Value,
    pub platform: Value,
    pub latest_data_at: Value,
    pub total_seconds: u64,
    pub top_apps: Vec<AppUsage>,
    pub days: BTreeMap<String, u64>,
}

#[derive(Debug, Serialize)]
pub struct Calendar {
    pub id: String,
    pub title: String,
    pub writable: bool,
    #[serde(rename = "type", skip_serializing_if = "Value::is_null")]
    pub kind: Value,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub account: Value,
}

#[derive(Debug, Clone)]
pub struct FreeSlotsQuery {
    pub start: Timestamp,
    pub end: Timestamp,
    pub zone: TimeZone,
    pub day_start: Time,
    pub day_end: Time,
    pub weekends: bool,
    pub minutes: u32,
    pub limit: usize,
    pub calendar: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FreeSlot {
    pub start: String,
    pub end: String,
    pub minutes: i64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedKind {
    #[default]
    Any,
    Image,
    Video,
    Audio,
    Pdf,
    Document,
    File,
    Link,
}

#[derive(Debug, Clone)]
pub enum SharedChat {
    Id(String),
    DirectHandles(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct SharedQuery {
    pub chat: SharedChat,
    pub kind: SharedKind,
    pub limit: usize,
    pub scan: usize,
}

#[derive(Debug, Serialize)]
pub struct SharedItems {
    pub chat_id: String,
    pub items: Vec<Value>,
    pub examined_messages: usize,
    pub complete: bool,
}

impl Service {
    fn read_visibility(&self, principal: &Principal, scope: &str) -> Result<Visibility, Error> {
        let principal = self.identity.current(principal)?;
        principal.require_explicit_scope(scope)?;
        Ok(principal.visibility())
    }

    fn all_records(
        &self,
        visibility: &Visibility,
        collection: &str,
    ) -> Result<(Vec<Value>, bool), Error> {
        let mut query = ReadQuery {
            collection: collection.into(),
            limit: 1000,
            ..ReadQuery::default()
        };
        let mut records = Vec::new();
        for _ in 0..200 {
            let page = decode(&self.store.read(visibility, &query)?)?;
            records.extend(
                items(&page, "records")
                    .iter()
                    .filter(|row| row["deleted"] != true)
                    .cloned(),
            );
            let next = text(&page, "next");
            if next.is_empty() {
                return Ok((records, true));
            }
            if next <= query.after.as_str() {
                return Err(Code::Corrupt.into());
            }
            query.after = next.into();
        }
        Ok((records, false))
    }

    pub fn usage_summary(
        &self,
        principal: &Principal,
        query: &UsageSummaryQuery,
    ) -> Result<UsageSummary, Error> {
        let visibility = self.read_visibility(principal, "usage:read")?;
        if query.last < query.first
            || query
                .first
                .until(query.last)
                .map_err(|_| Code::Invalid)?
                .get_days()
                > 91
            || !(1..=100).contains(&query.top)
        {
            return Err(Code::Invalid.into());
        }
        let (mut devices, complete_devices) = self.all_records(&visibility, "usage.device")?;
        if let Some(wanted) = &query.device {
            let folded = wanted.to_lowercase();
            devices.retain(|device| {
                device["id"] == *wanted
                    || text(&device["data"], "name")
                        .to_lowercase()
                        .contains(&folded)
                    || text(&device["data"], "platform").to_lowercase() == folded
            });
            if devices.is_empty() {
                return Err(Code::NotFound.into());
            }
        }
        let (rows, complete_rows) = self.all_records(&visibility, "usage.app_day")?;
        let (first, last) = (query.first.to_string(), query.last.to_string());
        let mut totals = BTreeMap::new();
        let mut devices: BTreeMap<_, _> = devices
            .into_iter()
            .map(|device| {
                (
                    text(&device, "id").to_owned(),
                    (device, BTreeMap::new(), BTreeMap::new()),
                )
            })
            .collect();
        for row in &rows {
            let data = &row["data"];
            let stored = text(data, "day");
            let day = stored.split_once('@').map_or(stored, |(day, _)| day);
            if day < first.as_str() || day > last.as_str() {
                continue;
            }
            if let Some((_, apps, days)) = devices.get_mut(text(data, "device_id")) {
                let seconds = usage_seconds(data);
                let bundle = data["bundle_id"].as_str().unwrap_or("unknown");
                add(apps, bundle, seconds);
                add(days, day, seconds);
                add(&mut totals, bundle, seconds);
            }
        }
        let mut devices: Vec<_> = devices
            .into_iter()
            .map(|(id, (device, apps, days))| DeviceUsage {
                id,
                name: device["data"]["name"].clone(),
                platform: device["data"]["platform"].clone(),
                latest_data_at: device["data"]["latest_event_at"].clone(),
                total_seconds: sum(&apps),
                top_apps: top(apps, query.top),
                days,
            })
            .collect();
        devices.sort_by(|a, b| b.total_seconds.cmp(&a.total_seconds).then(a.id.cmp(&b.id)));
        Ok(UsageSummary {
            first,
            last,
            total_seconds: sum(&totals),
            top_apps: top(totals, query.top),
            devices,
            complete: complete_devices && complete_rows,
        })
    }

    pub fn calendar_calendars(&self, principal: &Principal) -> Result<Vec<Calendar>, Error> {
        let visibility = self.read_visibility(principal, "calendar:read")?;
        self.calendars_for(&visibility)
    }

    fn calendars_for(&self, visibility: &Visibility) -> Result<Vec<Calendar>, Error> {
        let (rows, complete) = self.all_records(visibility, "calendar.calendar")?;
        if !complete {
            return Err(Code::Capacity.into());
        }
        Ok(rows
            .into_iter()
            .map(|row| Calendar {
                id: text(&row, "id").into(),
                title: text(&row["data"], "title").into(),
                writable: row["data"]["writable"] == true,
                kind: row["data"]["type"].clone(),
                account: row["data"]["account"].clone(),
            })
            .collect())
    }

    pub fn calendar_free_slots(
        &self,
        principal: &Principal,
        query: &FreeSlotsQuery,
    ) -> Result<Vec<FreeSlot>, Error> {
        let visibility = self.read_visibility(principal, "calendar:read")?;
        if query.start >= query.end
            || query.end.duration_since(query.start) > SignedDuration::from_hours(24 * 62)
            || query.day_start >= query.day_end
            || !(1..=1440).contains(&query.minutes)
            || !(1..=100).contains(&query.limit)
        {
            return Err(Code::Invalid.into());
        }
        let mut args =
            json!({"start":query.start.to_string(),"end":query.end.to_string(),"limit":1000});
        if let Some(wanted) = &query.calendar {
            args["calendar_id"] = calendar_id(&self.calendars_for(&visibility)?, wanted)?.into();
        }
        let mut busy = Vec::new();
        let mut cursor = None;
        loop {
            let result = decode(
                &self
                    .store
                    .read_model(&visibility, "calendar.agenda", &args)?,
            )?;
            if cursor
                .as_ref()
                .is_some_and(|previous| previous != &result["cursor"])
            {
                return Err(Code::Conflict.into());
            }
            busy.extend(
                items(&result, "records")
                    .iter()
                    .filter_map(|row| blocking(&row["data"])),
            );
            let Some(next) = result["next_offset"].as_u64() else {
                break;
            };
            if next <= args["offset"].as_u64().unwrap_or(0) {
                return Err(Code::Corrupt.into());
            }
            if next >= 200_000 {
                return Err(Code::Capacity.into());
            }
            cursor = Some(result["cursor"].clone());
            args["offset"] = next.into();
        }
        busy.sort_unstable();
        free_slots(query, &busy)
    }

    pub fn messages_shared(
        &self,
        principal: &Principal,
        query: &SharedQuery,
    ) -> Result<SharedItems, Error> {
        let visibility = self.read_visibility(principal, "messages:read")?;
        if !(1..=100).contains(&query.limit) || !(1..=10_000).contains(&query.scan) {
            return Err(Code::Invalid.into());
        }
        let chat_id = self.shared_chat(&visibility, &query.chat)?;
        let mut out = SharedItems {
            chat_id,
            items: Vec::new(),
            examined_messages: 0,
            complete: true,
        };
        let mut page = json!({"chat_id":out.chat_id,"anchor":"latest","limit":200});
        loop {
            let result = decode(
                &self
                    .store
                    .read_model(&visibility, "messages.thread", &page)?,
            )?;
            for message in items(&result, "items").iter().rev() {
                if out.examined_messages == query.scan || out.items.len() == query.limit {
                    out.complete = false;
                    return Ok(out);
                }
                out.examined_messages += 1;
                for item in shared_in(message, query.kind) {
                    if out.items.len() == query.limit {
                        out.complete = false;
                        return Ok(out);
                    }
                    out.items.push(item);
                }
            }
            let cursor = text(&result, "before");
            if cursor.is_empty() {
                return Ok(out);
            }
            if page["before"] == cursor || items(&result, "items").is_empty() {
                return Err(Code::Corrupt.into());
            }
            page = json!({"chat_id":out.chat_id,"before":cursor,"limit":200});
        }
    }

    fn shared_chat(&self, visibility: &Visibility, chat: &SharedChat) -> Result<String, Error> {
        match chat {
            SharedChat::Id(id) if !id.is_empty() => Ok(id.clone()),
            SharedChat::DirectHandles(handles) if !handles.is_empty() && handles.len() <= 20 => {
                let mut latest = None;
                for handle in handles {
                    let result = decode(&self.store.read_model(
                        visibility,
                        "messages.chats",
                        &json!({"participants":[handle],"limit":1}),
                    )?)?;
                    if let Some(chat) = items(&result, "chats").first() {
                        let activity = text(chat, "last_activity_at").parse::<Timestamp>().ok();
                        let id = text(&chat["chat"], "id");
                        if !id.is_empty()
                            && latest
                                .as_ref()
                                .is_none_or(|best: &(Option<Timestamp>, String)| activity > best.0)
                        {
                            latest = Some((activity, id.to_owned()));
                        }
                    }
                }
                latest
                    .map(|(_, id)| id)
                    .ok_or_else(|| Code::NotFound.into())
            }
            _ => Err(Code::Invalid.into()),
        }
    }
}

fn decode(result: &RawValue) -> Result<Value, Error> {
    serde_json::from_str(result.get()).map_err(|_| Code::Corrupt.into())
}

fn items<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value[field].as_array().map_or(&[], Vec::as_slice)
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field].as_str().unwrap_or_default()
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn usage_seconds(data: &Value) -> u64 {
    data["seconds_estimated"]
        .as_f64()
        .map_or(0, |seconds| seconds.max(0.0).round() as u64)
}

fn add(tallies: &mut BTreeMap<String, u64>, key: &str, seconds: u64) {
    let total = tallies.entry(key.into()).or_default();
    *total = total.saturating_add(seconds);
}

fn sum(tallies: &BTreeMap<String, u64>) -> u64 {
    tallies.values().copied().fold(0, u64::saturating_add)
}

fn top(apps: BTreeMap<String, u64>, count: usize) -> Vec<AppUsage> {
    let mut apps: Vec<_> = apps.into_iter().collect();
    apps.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    apps.into_iter()
        .take(count)
        .map(|(bundle_id, seconds)| AppUsage { bundle_id, seconds })
        .collect()
}

fn calendar_id(calendars: &[Calendar], wanted: &str) -> Result<String, Error> {
    let folded = wanted.trim().to_lowercase();
    if folded.is_empty() {
        return Err(Code::Invalid.into());
    }
    if let Some(calendar) = calendars.iter().find(|calendar| calendar.id == wanted) {
        return Ok(calendar.id.clone());
    }
    for exact in [true, false] {
        let mut matches = calendars.iter().filter(|calendar| {
            let title = calendar.title.to_lowercase();
            if exact {
                title == folded
            } else {
                title.contains(&folded)
            }
        });
        if let Some(calendar) = matches.next() {
            if matches.next().is_some() {
                return Err(Code::Conflict.into());
            }
            return Ok(calendar.id.clone());
        }
    }
    Err(Code::NotFound.into())
}

fn blocking(event: &Value) -> Option<(Timestamp, Timestamp)> {
    if event["all_day"] == true
        || event["availability"] == 1
        || event["status"] == 3
        || event["owner_participation_status"] == 3
    {
        return None;
    }
    let start = text(event, "start").parse().ok()?;
    let end = text(event, "end").parse().ok()?;
    (start < end).then_some((start, end))
}

fn free_slots(
    query: &FreeSlotsQuery,
    busy: &[(Timestamp, Timestamp)],
) -> Result<Vec<FreeSlot>, Error> {
    let mut day = query.start.to_zoned(query.zone.clone()).date();
    let last = query.end.to_zoned(query.zone.clone()).date();
    let mut slots = Vec::new();
    let minimum = SignedDuration::from_mins(i64::from(query.minutes));
    let mut push = |start: Timestamp, end: Timestamp| {
        let duration = end.duration_since(start);
        if duration >= minimum {
            slots.push(FreeSlot {
                start: start.to_string(),
                end: end.to_string(),
                minutes: duration.as_mins(),
            });
        }
        slots.len() == query.limit
    };
    while day <= last {
        if query.weekends || day.weekday().to_monday_one_offset() <= 5 {
            let at = |time| {
                day.to_datetime(time)
                    .to_zoned(query.zone.clone())
                    .map(|time| time.timestamp())
                    .map_err(|_| Code::Invalid)
            };
            let mut cursor = at(query.day_start)?.max(query.start);
            let close = at(query.day_end)?.min(query.end);
            for &(start, end) in busy.iter().take_while(|(start, _)| *start < close) {
                if end > cursor {
                    if push(cursor, start) {
                        return Ok(slots);
                    }
                    cursor = cursor.max(end);
                }
            }
            if push(cursor, close) {
                return Ok(slots);
            }
        }
        if day == last {
            break;
        }
        day = day.tomorrow().map_err(|_| Code::Invalid)?;
    }
    Ok(slots)
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

fn shared_in(message: &Value, kind: SharedKind) -> Vec<Value> {
    let mut found = Vec::new();
    if matches!(kind, SharedKind::Any | SharedKind::Link) {
        found.extend(
            links(&message["data"])
                .into_iter()
                .map(|url| json!({"kind":"link","url":url})),
        );
    }
    if !matches!(kind, SharedKind::Link) {
        for file in items(message, "attachments") {
            let fields = &file["data"];
            if file["deleted"] == true
                || fields["is_sticker"] == true
                || fields["genmoji"] == true
                || fields["hidden"] == true
            {
                continue;
            }
            let what = file_kind(fields);
            if matches!(kind, SharedKind::Any | SharedKind::File) || kind.matches(what) {
                let mut item = json!({
                    "kind":what,
                    "attachment_id":file["id"],
                    "filename":file_name(fields["filename"].as_str()),
                    "mime_type":fields["mime_type"],
                    "size":fields["size"],
                });
                if let Some(digest) = fields["blob"]["digest"].as_str() {
                    item["digest"] = json!(digest);
                }
                found.push(item);
            }
        }
    }
    for item in &mut found {
        item["message_id"] = message["id"].clone();
        item["sent_at"] = message["data"]["sent_at"].clone();
        item["from_handle_id"] = message["data"]["sender_handle_id"].clone();
        item["from_me"] = message["data"]["is_from_me"].clone();
    }
    found
}

impl SharedKind {
    fn matches(self, kind: &str) -> bool {
        matches!(
            (self, kind),
            (Self::Image, "image")
                | (Self::Video, "video")
                | (Self::Audio, "audio")
                | (Self::Pdf, "pdf")
                | (Self::Document, "document")
        )
    }
}
