use super::{
    Duration, Instant, Request, Result, Session, Shared, State, Ticket, Value, enqueue, id,
    invalid, json, lock, mpsc, now, observation, send, string,
};

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
            s.data["calendar_clock"] = input["now"].clone();
        }
        _ => return Err(invalid("unsupported calendar control")),
    }
    s.save()?;
    let denied = s.denied.get("calendar") == Some(&true);
    let mut frame = s.scan("calendar")?;
    frame["live"] = json!(true);
    let bytes = serde_json::to_vec(&frame["observations"])?.len();
    drop(s);
    if !denied {
        enqueue(tx, frame, true)?;
    }
    Ok(json!({"effect_done":true,"effect_at":now(),"capture_bytes":bytes}))
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
        if args.is_null() || args["span"] == "all" || args["span"] == "future" {
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
        let mut key = string(&action, "native_key");
        let mut s = lock(state)?;
        if s.denied.get("calendar") == Some(&true) {
            drop(s);
            return self.reject(frame, "unavailable");
        }
        if !key.is_empty() && s.data["records"][&key]["data"] != ticket.before {
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
        if key.is_empty() {
            key = id("event")?;
        }
        let list = string(&action, "list_key");
        let calendar_op = op == "calendar.create_calendar";
        let mut data = if ticket.before.is_null() {
            json!({})
        } else {
            ticket.before.clone()
        };
        let mut patch = action["calendar_args"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for field in ["id", "calendar_id", "span"] {
            patch.remove(field);
        }
        for (k, v) in patch {
            data[k] = v;
        }
        if calendar_op {
            data["writable"] = json!(true);
            data["account"] = json!("test");
            data["type"] = json!("local");
            data["color"] = json!("#00aaff");
        } else {
            data["calendar_id"] = reference(&list);
            defaults(&mut data);
        }
        let absent = op.ends_with("delete_event");
        let series = string(&data, "series_id");
        if !series.is_empty() && !calendar_op {
            data["is_exception"] = json!(true);
        }
        let mut writes = vec![(key.clone(), data.clone())];
        if op == "calendar.create_event"
            && data["recurrence"].as_array().is_some_and(|r| !r.is_empty())
        {
            writes = expand(&data, &key, &scope(&s, &list))?;
            key.clone_from(&writes[0].0);
            data = writes[0].1.clone();
        }
        for (target, value) in writes {
            let mut record = if calendar_op {
                observation("calendar.calendar", &target, value)
            } else {
                let mut record = observation("calendar.event", &target, value);
                record["scoped_alias"] = json!(format!("{}{target}", scope(&s, &list)));
                record
            };
            if absent {
                record["kind"] = json!("absent");
                record["revalidated"] = json!(true);
            }
            s.put(record);
        }
        let count = s.data["effects"][&command].as_u64().unwrap_or_default() + 1;
        s.data["effects"][&command] = json!(count);
        s.save()?;
        let capture = s.scan("calendar")?;
        drop(s);
        self.emit(state, capture)?;
        let result = if absent { Value::Null } else { data };
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":{"source":"calendar","native_key":key,"native_result":result,"fingerprint":ticket.prepare["request_hash"],"observed":true,"absent":absent,"capture_checkpoint":self.checkpoints.get("calendar")}});
        let mut s = lock(state)?;
        s.data["evidence"][&command] = outcome.clone();
        s.save()?;
        s.guard(&json!({"command_id":command,"outcome":outcome}))?;
        drop(s);
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}
