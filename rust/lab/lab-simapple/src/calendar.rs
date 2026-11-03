use super::{
    BTreeMap, Duration, Instant, Request, Result, Session, Shared, State, Ticket, Value, enqueue,
    id, invalid, json, lock, mpsc, now, observation, send, string,
};

#[derive(Debug, Default)]
pub(super) struct Live {
    stamps: BTreeMap<String, (Value, bool)>,
    verified: Option<jiff::Timestamp>,
}

fn clock(s: &State) -> jiff::Zoned {
    let zone = jiff::tz::TimeZone::system();
    s.data["calendar_clock"]
        .as_str()
        .and_then(|text| text.parse::<jiff::Timestamp>().ok())
        .unwrap_or_else(jiff::Timestamp::now)
        .to_zoned(zone)
}

pub(super) fn in_window(s: &State, record: &Value) -> bool {
    if record["collection"] != "calendar.event" || record["kind"] == "absent" {
        return true;
    }
    let now = clock(s);
    let Ok(today) = now.date().to_zoned(now.time_zone().clone()) else {
        return true;
    };
    let (Ok(left), Ok(right)) = (
        today.checked_sub(jiff::Span::new().days(730)),
        today.checked_add(jiff::Span::new().days(730)),
    ) else {
        return true;
    };
    let start = string(&record["data"], "start");
    let end = string(&record["data"], "end");
    if start.len() == 10 {
        return start < right.date().to_string() && end > left.date().to_string();
    }
    let (Ok(a), Ok(b)) = (
        start.parse::<jiff::Timestamp>(),
        end.parse::<jiff::Timestamp>(),
    ) else {
        return false;
    };
    a < right.timestamp() && b > left.timestamp()
}

pub(super) fn view(s: &State, records: Vec<Value>) -> Vec<Value> {
    records
        .into_iter()
        .map(|mut record| {
            let key = string(&record, "native_key");
            if let Some((cached, forced)) = s.calendar.stamps.get(&key)
                && (*forced
                    || (cached["data"]["location"] == record["data"]["location"]
                        && cached["data"]["alarms"] == record["data"]["alarms"]))
            {
                record = cached.clone();
            }
            if !in_window(s, &record) {
                record["kind"] = json!("evict");
                record["revalidated"] = json!(true);
                record["observed_at"] = json!(now());
            }
            record
        })
        .collect()
}

fn reference(key: &str) -> Value {
    json!({"$ref":{"collection":"calendar.calendar","native_key":key}})
}

fn scope(s: &State, calendar: &str) -> String {
    format!(
        "{}/{calendar}/",
        string(&s.data["records"][calendar]["data"], "account")
    )
}

fn writable(record: &Value) -> bool {
    record["collection"] == "calendar.calendar"
        && record["kind"] != "absent"
        && record["data"]["writable"] == true
        && record["data"]["type"] != "subscribed"
}

fn put_event(s: &mut State, key: &str, data: Value, calendar: &str) {
    let mut record = observation("calendar.event", key, data);
    record["scoped_alias"] = json!(format!("{}{key}", scope(s, calendar)));
    s.put(record);
}

fn expand(base: &Value, key: &str, prefix: &str) -> Result<Vec<(String, Value)>> {
    let rules = base["recurrence"].as_array().cloned().unwrap_or_default();
    let Some(rule) = rules.first() else {
        return Ok(vec![(key.to_owned(), base.clone())]);
    };
    let interval = rule["interval"].as_i64().unwrap_or_default();
    if interval < 1 {
        return Err(invalid("interval"));
    }
    let zone =
        jiff::tz::TimeZone::get(&string(base, "time_zone")).unwrap_or(jiff::tz::TimeZone::UTC);
    let all_day = base["all_day"] == true;
    let parse = |field: &str| -> Result<jiff::Zoned> {
        let text = string(base, field);
        if all_day {
            let date: jiff::civil::Date = text.parse().map_err(|_| invalid("date"))?;
            date.to_zoned(zone.clone()).map_err(|_| invalid("date"))
        } else {
            let at: jiff::Timestamp = text.parse().map_err(|_| invalid("time"))?;
            Ok(at.to_zoned(zone.clone()))
        }
    };
    let (start, end) = (parse("start")?, parse("end")?);
    if end.timestamp() <= start.timestamp() {
        return Err(invalid("range"));
    }
    let count = match rule["count"].as_i64().unwrap_or_default() {
        0 => 1462,
        n if n > 10000 => return Err(invalid("capacity")),
        n => n,
    };
    let until = rule["until"]
        .as_str()
        .and_then(|u| u.parse::<jiff::Timestamp>().ok());
    let frequency = string(rule, "frequency");
    let mut result = Vec::new();
    for i in 0..count * 12 {
        if i64::try_from(result.len()).unwrap_or(i64::MAX) >= count {
            break;
        }
        let steps = i.checked_mul(interval).ok_or_else(|| invalid("range"))?;
        let span = match frequency.as_str() {
            "daily" => jiff::Span::new().days(steps),
            "weekly" => jiff::Span::new().days(steps * 7),
            "monthly" => jiff::Span::new().months(steps),
            "yearly" => jiff::Span::new().years(steps),
            _ => return Err(invalid("frequency")),
        };
        let (Ok(occurrence), Ok(finish)) = (start.checked_add(span), end.checked_add(span)) else {
            break;
        };
        if matches!(frequency.as_str(), "monthly" | "yearly") && occurrence.day() != start.day() {
            continue;
        }
        if until.is_some_and(|limit| occurrence.timestamp() > limit) {
            break;
        }
        let format = |at: &jiff::Zoned| {
            if all_day {
                at.date().to_string()
            } else {
                at.timestamp().to_string()
            }
        };
        let original = format(&occurrence);
        let mut value = base.clone();
        value["start"] = json!(original);
        value["end"] = json!(format(&finish));
        value["series_id"] = json!(key);
        value["occurrence_date"] = json!(original);
        result.push((format!("{key}|{original}"), value));
    }
    let _ = prefix;
    if result.is_empty() {
        return Err(invalid("recurrence"));
    }
    Ok(result)
}

fn defaults(data: &mut Value) {
    for (key, value) in [
        ("all_day", json!(false)),
        ("time_zone", json!("America/New_York")),
        ("is_exception", json!(false)),
        ("attendees", json!([])),
        ("alarms", json!([])),
        ("availability", json!(0)),
        ("status", json!(1)),
    ] {
        if data[key].is_null() {
            data[key] = value;
        }
    }
}

pub(super) fn control_calendar(
    state: &Shared,
    tx: &mpsc::Sender<Request>,
    input: &Value,
) -> Result<Value> {
    let mut s = lock(state)?;
    let key = string(input, "key");
    match string(input, "action").as_str() {
        "calendar" => {
            let data = if input["data"].is_null() {
                json!({"title":"Bridge Test","writable":true,"account":"test","type":"local","color":"#00aaff"})
            } else {
                input["data"].clone()
            };
            s.put(observation("calendar.calendar", &key, data));
        }
        "create" => {
            let mut data = input["data"].clone();
            data["calendar_id"] = reference("test-calendar");
            defaults(&mut data);
            for (event, value) in expand(&data, &key, "")? {
                put_event(&mut s, &event, value, "test-calendar");
            }
        }
        "create_many" => {
            let count = input["count"].as_u64().unwrap_or_default();
            if !(1..=3000).contains(&count) {
                return Err(invalid("invalid"));
            }
            for index in 0..count {
                let mut data = json!({"calendar_id":reference("test-calendar"),"title":format!("{} {index}",string(&input["data"],"title")),"start":input["data"]["start"],"end":input["data"]["end"],"all_day":false,"time_zone":"UTC","is_exception":false});
                if !input["data"]["notes"].is_null() {
                    data["notes"] = input["data"]["notes"].clone();
                }
                put_event(&mut s, &format!("{key}/{index}"), data, "test-calendar");
            }
        }
        "delete" => {
            let mut record = s.data["records"][&key].clone();
            if record.is_null() {
                return Err(invalid("not found"));
            }
            record["kind"] = json!("absent");
            record["revalidated"] = json!(true);
            record["observed_at"] = json!(now());
            s.put(record);
        }
        "churn" => {
            let moved: Vec<Value> = s
                .records()
                .into_iter()
                .filter(|record| {
                    record["collection"] == "calendar.event" && record["kind"] != "absent"
                })
                .collect();
            for mut record in moved {
                if let Some(records) = s.data["records"].as_object_mut() {
                    records.remove(&string(&record, "native_key"));
                }
                record["native_key"] = json!(format!("churn/{}", string(&record, "native_key")));
                s.put(record);
            }
        }
        "clock" => {
            let at = string(input, "now")
                .parse::<jiff::Timestamp>()
                .map_err(|_| invalid("invalid"))?;
            s.data["calendar_clock"] = json!(at.to_string());
            let due = s.calendar.verified.is_none_or(|last| {
                at >= last
                    .checked_add(jiff::SignedDuration::from_mins(15))
                    .unwrap_or(last)
            });
            if due {
                s.calendar.stamps.clear();
                s.calendar.verified = Some(at);
            }
        }
        "server_update" => {
            let mut record = s.data["records"][&key].clone();
            if record["collection"] != "calendar.event" {
                return Err(invalid("not found"));
            }
            let forced = input["force_stamp_match"] == true;
            s.calendar
                .stamps
                .insert(key.clone(), (record.clone(), forced));
            if let Some(changes) = input["data"].as_object() {
                for (field, value) in changes {
                    record["data"][field] = value.clone();
                }
            }
            s.data["records"][&key] = record;
            s.save()?;
            return Ok(json!({"effect_done":true,"stamp_unchanged":forced}));
        }
        "move" => {
            let mut record = s.data["records"][&key].clone();
            if record["collection"] != "calendar.event" || record["kind"] == "absent" {
                return Err(invalid("not found"));
            }
            let parse = |text: &str| {
                text.parse::<jiff::Timestamp>()
                    .map_err(|_| invalid("invalid"))
            };
            let start = parse(&string(&input["data"], "start"))?;
            let old = parse(&string(&record["data"], "start"))?;
            let old_end = parse(&string(&record["data"], "end"))?;
            let mut finish = start
                .checked_add(old_end.duration_since(old))
                .map_err(|_| invalid("invalid"))?;
            if let Some(requested) = input["data"]["end"].as_str() {
                let parsed = parse(requested)?;
                if parsed <= start {
                    return Err(invalid("invalid"));
                }
                finish = parsed;
            }
            record["data"]["start"] = json!(start.to_string());
            record["data"]["end"] = json!(finish.to_string());
            record["data"]["is_exception"] = json!(true);
            if let Some(title) = input["data"]["new_title"].as_str() {
                record["data"]["title"] = json!(title);
            }
            record["observed_at"] = json!(now());
            s.data["records"][&key] = record;
        }
        _ => return Err(invalid("unsupported calendar control")),
    }
    s.save()?;
    let denied = s.denied.get("calendar") == Some(&true);
    let mut frame = s.scan("calendar")?;
    frame["live"] = json!(true);
    let bytes = serde_json::to_vec(&frame["observations"])?.len();
    drop(s);
    let effect_at = now();
    if !denied {
        enqueue(tx, frame, true)?;
    }
    Ok(json!({"effect_done":true,"effect_at":effect_at,"capture_bytes":bytes}))
}

impl Session {
    pub(super) fn calendar_prepare(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let action = &frame["action"];
        let s = lock(state)?;
        let args = &action["calendar_args"];
        if s.denied.get("calendar") == Some(&true) {
            drop(s);
            return self.reject(frame, "unavailable");
        }
        if args.is_null() || args["span"] == "all" {
            drop(s);
            return self.reject(frame, "unsupported");
        }
        let key = string(action, "native_key");
        let mut before = Value::Null;
        if !key.is_empty() {
            let record = &s.data["records"][&key];
            if record.is_null() || record["kind"] == "absent" {
                drop(s);
                return self.reject(frame, "not_found");
            }
            before = record["data"].clone();
            let mut expected = frame["expected_state"].clone();
            let mut native = before.clone();
            for value in [&mut expected, &mut native] {
                if value.is_null() {
                    *value = json!({});
                }
            }
            let membership = string(&native["calendar_id"]["$ref"], "native_key");
            if membership != string(action, "expected_list_key") {
                drop(s);
                return self.calendar_conflict(state, frame, &key);
            }
            for value in [&mut expected, &mut native] {
                if let Some(object) = value.as_object_mut() {
                    object.remove("calendar_id");
                }
            }
            if expected != native {
                drop(s);
                return self.calendar_conflict(state, frame, &key);
            }
        }
        for field in ["list_key", "expected_list_key"] {
            let list = string(action, field);
            if !list.is_empty() && !writable(&s.data["records"][&list]) {
                drop(s);
                return self.reject(frame, "unsupported");
            }
        }
        drop(s);
        let ticket = id("ticket")?;
        let requested = string(frame, "expires_at")
            .parse::<jiff::Timestamp>()
            .map_err(|_| invalid("expiry"))?;
        let seconds = (requested.as_second() - jiff::Timestamp::now().as_second()).clamp(0, 30);
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: before.clone(),
                deadline: Instant::now() + Duration::from_secs(seconds.cast_unsigned()),
            },
        );
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":command,"ticket":ticket,"native_state":before,"route":"sim-calendar","fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    fn calendar_conflict(&mut self, state: &Shared, frame: &Value, key: &str) -> Result<()> {
        let record = lock(state)?.data["records"][key].clone();
        let mut rejected = json!({"t":"Rejected","command_id":frame["command_id"],"disposition":"rejected_before_dispatch","error":{"code":"conflict","message":"conflict"}});
        if !record.is_null() {
            let mut record = record;
            record["observed_at"] = json!(now());
            self.emit(
                state,
                json!({"t":"Capture","source":"calendar","observations":[record],"scan_id":id("scan")?,"scope":["calendar.calendar","calendar.event"],"scan_started_at":now(),"completion_evidence":"complete"}),
            )?;
            rejected["evidence"] = json!({"native_key":key,"capture_checkpoint":self.checkpoints.get("calendar").cloned().unwrap_or_default()});
        }
        send(&mut self.stream, &rejected)?;
        Ok(())
    }

    pub(super) fn calendar_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let Some(ticket) = self.tickets.remove(&string(frame, "ticket")) else {
            return self.reject(frame, "expired");
        };
        if ticket.prepare["command_id"] != command
            || frame["owner_fence"].as_u64() != Some(self.fence)
            || Instant::now() >= ticket.deadline
        {
            return self.reject(frame, "expired");
        }
        let action = ticket.prepare["action"].clone();
        let op = string(&action, "op");
        let mut s = lock(state)?;
        if s.denied.get("calendar") == Some(&true) {
            drop(s);
            return self.reject(frame, "unavailable");
        }
        let requested = string(&action, "native_key");
        if !requested.is_empty() && s.data["records"][&requested]["data"] != ticket.before {
            drop(s);
            return self.reject(frame, "conflict");
        }
        for field in ["list_key", "expected_list_key"] {
            let list = string(&action, field);
            if !list.is_empty() && !writable(&s.data["records"][&list]) {
                drop(s);
                return self.reject(frame, "unsupported");
            }
        }
        s.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        let plan = Plan::new(&action, &op)?;
        let native = apply(&mut s, plan, &op)?;
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        s.save()?;
        let capture = s.scan("calendar")?;
        let primary = s.data["records"][&native].clone();
        let actual = primary["data"].clone();
        drop(s);
        let large = capture["observations"]
            .as_array()
            .is_some_and(|items| items.len() > 64);
        self.emit(state, capture)?;
        if large {
            self.emit(
                state,
                json!({"t":"Capture","source":"calendar","observations":[primary],"scan_id":id("scan")?,"scope":["calendar.calendar","calendar.event"],"scan_started_at":now(),"completion_evidence":"complete"}),
            )?;
        }
        let args = &action["calendar_args"];
        let mut matched = true;
        if !args["recurrence"].is_null()
            && matches!(
                op.as_str(),
                "calendar.update_event" | "calendar.create_event"
            )
        {
            matched = rules_match(&actual["recurrence"], &args["recurrence"]);
        }
        let absent = op.ends_with("delete_event");
        let result = if absent { Value::Null } else { actual };
        let mut evidence = json!({"source":"calendar","native_key":native,"native_result":result,"fingerprint":ticket.prepare["request_hash"],"observed":matched,"absent":absent});
        let disposition = if matched {
            evidence["capture_checkpoint"] = json!(self.checkpoints.get("calendar"));
            "locally_observed"
        } else {
            evidence["reason"] = json!("native_postcondition_mismatch");
            "indeterminate"
        };
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":disposition,"evidence":evidence});
        let mut s = lock(state)?;
        s.data["evidence"][&command] = outcome.clone();
        s.save()?;
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}

struct Plan {
    patch: serde_json::Map<String, Value>,
    span: String,
    list: String,
    native: String,
    calendar_op: bool,
    recurrence: bool,
    series_id: String,
}

impl Plan {
    fn new(action: &Value, op: &str) -> Result<Self> {
        let mut native = string(action, "native_key");
        if native.is_empty() {
            native = id("event")?;
        }
        let args = &action["calendar_args"];
        let mut patch = args.as_object().cloned().unwrap_or_default();
        for field in ["id", "calendar_id", "span"] {
            patch.remove(field);
        }
        Ok(Self {
            patch,
            span: string(args, "span"),
            list: string(action, "list_key"),
            native,
            calendar_op: op == "calendar.create_calendar",
            recurrence: args["recurrence"].as_array().is_some_and(|r| !r.is_empty()),
            series_id: id("series")?,
        })
    }
}

fn bounded(value: &Value, count: usize) -> Value {
    let Some(rules) = value.as_array() else {
        return value.clone();
    };
    let mut result = Vec::new();
    for rule in rules {
        let Some(object) = rule.as_object() else {
            return value.clone();
        };
        let mut copy = object.clone();
        copy.insert("count".to_owned(), json!(count));
        copy.remove("end");
        result.push(Value::Object(copy));
    }
    Value::Array(result)
}

fn normalized(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), normalized(v)))
                .filter(|(_, v)| {
                    !(v.is_null()
                        || v.as_array().is_some_and(Vec::is_empty)
                        || v.as_str().is_some_and(str::is_empty)
                        || v.as_i64() == Some(0))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
        other => other.clone(),
    }
}

fn rules_match(actual: &Value, requested: &Value) -> bool {
    let left = actual.as_array().cloned().unwrap_or_default();
    let right = requested.as_array().cloned().unwrap_or_default();
    left.len() == right.len()
        && (left.is_empty() || normalized(&Value::Array(left)) == normalized(&Value::Array(right)))
}

fn shifted(old: &str, anchor: &str, replacement: &str, all_day: bool) -> Option<String> {
    if all_day {
        let current: jiff::civil::Date = old.parse().ok()?;
        let before: jiff::civil::Date = anchor.parse().ok()?;
        let after: jiff::civil::Date = replacement.parse().ok()?;
        let days = i64::from(before.until(after).ok()?.get_days());
        return current
            .checked_add(jiff::Span::new().days(days))
            .ok()
            .map(|d| d.to_string());
    }
    let current: jiff::Timestamp = old.parse().ok()?;
    let before: jiff::Timestamp = anchor.parse().ok()?;
    let after: jiff::Timestamp = replacement.parse().ok()?;
    current
        .checked_add(after.duration_since(before))
        .ok()
        .map(|t| t.to_string())
}

fn series_members(
    s: &State,
    series: &str,
    calendar: &Value,
    original: &str,
) -> (Vec<String>, Vec<String>) {
    let (mut later, mut earlier) = (Vec::new(), Vec::new());
    for (key, record) in s.data["records"].as_object().into_iter().flatten() {
        let data = &record["data"];
        if record["kind"] == "absent"
            || data["series_id"] != series
            || data["calendar_id"] != *calendar
        {
            continue;
        }
        if string(data, "occurrence_date").as_str() >= original {
            later.push(key.clone());
        } else {
            earlier.push(key.clone());
        }
    }
    (later, earlier)
}

fn absent(record: &mut Value) {
    record["kind"] = json!("absent");
    record["revalidated"] = json!(true);
    record["observed_at"] = json!(now());
}

fn apply(s: &mut State, mut plan: Plan, op: &str) -> Result<String> {
    let collection = if plan.calendar_op {
        "calendar.calendar"
    } else {
        "calendar.event"
    };
    let target = s.data["records"][&plan.native].clone();
    let data = if target["data"].is_object() {
        target["data"].clone()
    } else {
        json!({})
    };
    let series = string(&data, "series_id");
    let original = string(&data, "occurrence_date");
    let future = plan.span == "future" && !series.is_empty();
    let mut targets = vec![plan.native.clone()];
    if future {
        let (later, earlier) = series_members(s, &series, &data["calendar_id"], &original);
        targets = later;
        for key in &earlier {
            let mut record = s.data["records"][key].clone();
            record["data"]["recurrence"] = bounded(&record["data"]["recurrence"], earlier.len());
            record["observed_at"] = json!(now());
            s.put(record);
        }
    }
    let scope_prefix = scope(s, &plan.list);
    if future && plan.recurrence && matches!(op, "calendar.update_event" | "calendar.create_event")
    {
        let mut base = data.clone();
        for (field, value) in &plan.patch {
            base[field] = value.clone();
        }
        if s.fault == "calendar-rule-mismatch" {
            s.fault.clear();
            if let Some(rule) = base["recurrence"]
                .as_array()
                .and_then(|r| r.first())
                .cloned()
            {
                let mut rule = rule;
                rule["count"] = json!(1);
                base["recurrence"] = json!([rule]);
            }
        }
        base["is_exception"] = json!(false);
        let expanded = expand(&base, &plan.series_id, "")?;
        let mut prior = BTreeMap::new();
        for key in &targets {
            let mut record = s.data["records"][key].clone();
            prior.insert(string(&record["data"], "occurrence_date"), record.clone());
            absent(&mut record);
            s.put(record);
        }
        let mut used = std::collections::BTreeSet::new();
        for (index, (key, value)) in expanded.into_iter().enumerate() {
            let occurrence = string(&value, "occurrence_date");
            let mut value = value;
            if let Some(old) = prior.get(&occurrence) {
                if old["data"]["is_exception"] == true {
                    value = old["data"].clone();
                    value["series_id"] = json!(plan.series_id);
                    value["recurrence"] = base["recurrence"].clone();
                }
                used.insert(occurrence);
            }
            let mut record = observation(collection, &key, value);
            record["scoped_alias"] = json!(format!("{scope_prefix}{key}"));
            s.put(record);
            if index == 0 {
                plan.native = key;
            }
        }
        for (occurrence, mut old) in prior {
            if !used.contains(&occurrence) && old["data"]["is_exception"] == true {
                if let Some(object) = old["data"].as_object_mut() {
                    object.remove("series_id");
                    object.remove("occurrence_date");
                }
                old["data"]["recurrence"] = json!([]);
                old["data"]["is_exception"] = json!(false);
                let key = id("event")?;
                old["native_key"] = json!(key);
                old["scoped_alias"] = json!(format!("{scope_prefix}{key}"));
                old["revalidated"] = json!(false);
                old["kind"] = json!("upsert");
                old["observed_at"] = json!(now());
                s.put(old);
            }
        }
        targets.clear();
    }
    let target_count = targets.len();
    for mut key in targets {
        let record = s.data["records"][&key].clone();
        let mut value = if record["data"].is_object() {
            record["data"].clone()
        } else {
            json!({})
        };
        for (field, patch) in &plan.patch {
            if future && key != plan.native && value["is_exception"] == true {
                continue;
            }
            if matches!(field.as_str(), "start" | "end") && key != plan.native && future {
                let all_day = value["all_day"] == true;
                if let Some(next) = shifted(
                    &string(&value, field),
                    &string(&data, field),
                    patch.as_str().unwrap_or_default(),
                    all_day,
                ) {
                    value[field] = json!(next);
                    continue;
                }
            }
            value[field] = patch.clone();
        }
        if plan.calendar_op {
            value["writable"] = json!(true);
            value["account"] = json!("test");
            value["type"] = json!("local");
            value["color"] = json!("#00aaff");
        } else {
            value["calendar_id"] = reference(&plan.list);
            if !series.is_empty() {
                if plan.span == "this" {
                    value["is_exception"] = json!(true);
                } else {
                    value["series_id"] = json!(plan.series_id);
                    value["recurrence"] = bounded(&value["recurrence"], target_count);
                }
            }
            defaults(&mut value);
        }
        let mut alias = string(&record, "scoped_alias");
        if alias.is_empty() && !plan.calendar_op {
            alias = format!("{scope_prefix}{key}");
        }
        if future && op == "calendar.update_event" {
            let mut old = record.clone();
            absent(&mut old);
            s.put(old);
            let original_key = key.clone();
            value["occurrence_date"] = value["start"].clone();
            key = format!("{}|{}", plan.series_id, string(&value, "occurrence_date"));
            alias = format!("{scope_prefix}{key}");
            if original_key == plan.native {
                plan.native.clone_from(&key);
            }
        }
        let mut next = observation(collection, &key, value);
        next["scoped_alias"] = json!(alias);
        if op.ends_with("delete_event") {
            next["kind"] = json!("absent");
            next["revalidated"] = json!(true);
        }
        if op == "calendar.create_event" && plan.recurrence {
            let expanded = expand(&next["data"], &key, "")?;
            plan.native.clone_from(&expanded[0].0);
            for (event, value) in expanded {
                let mut record = observation(collection, &event, value);
                record["scoped_alias"] = json!(format!("{scope_prefix}{event}"));
                s.put(record);
            }
        } else {
            s.put(next);
        }
    }
    Ok(plan.native)
}
