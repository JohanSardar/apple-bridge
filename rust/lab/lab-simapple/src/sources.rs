use super::{
    Request, Result, Shared, Value, enqueue, invalid, json, lock, mpsc, now, observation, source,
    string,
};

pub(super) fn dataset_alias(record: &mut Value) {
    let collection = string(record, "collection");
    if matches!(source(&collection), "mail" | "files")
        && (source(&collection) != "mail" || string(record, "scoped_alias").is_empty())
    {
        record["scoped_alias"] = json!(format!("{collection}/{}", string(record, "native_key")));
    }
}

pub(super) fn control_source(
    state: &Shared,
    tx: &mpsc::Sender<Request>,
    route: &str,
    input: &Value,
) -> Result<Value> {
    if route == "/usage" {
        return usage(state, tx, input);
    }
    let mut records = if route == "/safari" {
        input
            .as_array()
            .cloned()
            .ok_or_else(|| invalid("safari rows"))?
    } else {
        let collection = if route == "/phone" {
            "phone.call".to_owned()
        } else {
            string(input, "collection")
        };
        if route == "/recordings"
            && ![
                "voice_memos.folder",
                "voice_memos.recording",
                "voicemail.message",
            ]
            .contains(&collection.as_str())
        {
            return Err(invalid("recording collection"));
        }
        validate_recording(&collection, &input["data"])?;
        vec![observation(
            &collection,
            &string(input, "key"),
            input["data"].clone(),
        )]
    };
    for record in &mut records {
        let collection = string(record, "collection");
        let key = string(record, "native_key");
        if key.is_empty()
            || (route == "/safari" && !collection.starts_with("safari."))
            || (route != "/safari" && !record["data"].is_object())
        {
            return Err(invalid("source observation"));
        }
        record["observed_at"] = json!(now());
        record["scoped_alias"] = json!(match route {
            "/phone" => format!("phone/fixture/{key}"),
            "/recordings" => format!("{collection}/fixture/{key}"),
            _ => format!("safari/{collection}/{key}"),
        });
    }
    let src = records.first().map_or("safari".to_owned(), |r| {
        source(r["collection"].as_str().unwrap_or_default()).to_owned()
    });
    let mut s = lock(state)?;
    for record in &records {
        let key = format!(
            "{}/{}",
            string(record, "collection"),
            string(record, "native_key")
        );
        let storage_key = if route == "/phone" {
            string(record, "native_key")
        } else {
            key
        };
        s.data["records"][storage_key] = record.clone();
    }
    s.save()?;
    drop(s);
    enqueue(
        tx,
        json!({"t":"Capture","source":src,"observations":records}),
        true,
    )?;
    Ok(json!({"effect_done":true}))
}

fn validate_recording(collection: &str, data: &Value) -> Result<()> {
    let fields: &[(&str, char)] = match collection {
        "phone.call" => &[
            ("direction", 's'),
            ("handle", 's'),
            ("name", 's'),
            ("date", 't'),
            ("duration", 'n'),
            ("service", 's'),
            ("answered", 'b'),
        ],
        "voice_memos.folder" => &[("name", 's'), ("count", 'n')],
        "voice_memos.recording" => &[
            ("title", 's'),
            ("date", 't'),
            ("duration", 'n'),
            ("folder", 's'),
            ("recently_deleted", 'b'),
            ("transcript_state", 's'),
            ("audio_state", 's'),
        ],
        "voicemail.message" => &[
            ("sender", 's'),
            ("date", 't'),
            ("duration", 'n'),
            ("read", 'b'),
            ("mailbox", 's'),
            ("type", 's'),
            ("transcript_state", 's'),
            ("audio_state", 's'),
        ],
        _ => return Err(invalid("source collection")),
    };
    let object = data.as_object().ok_or_else(|| invalid("source data"))?;
    for (name, kind) in fields {
        let value = &data[name];
        let valid = match kind {
            's' => value.is_string(),
            'n' => value.is_number(),
            'b' => value.is_boolean(),
            _ => value
                .as_str()
                .is_some_and(|v| v.contains('T') && v.parse::<jiff::Timestamp>().is_ok()),
        };
        if !valid {
            return Err(invalid("source field"));
        }
    }
    for (name, value) in object {
        if fields.iter().any(|(field, _)| field == name) {
            continue;
        }
        if collection != "voice_memos.recording"
            || !match name.as_str() {
                "transcript" => value.is_null() || value.is_array(),
                "audio" => value.is_null() || value.is_object(),
                _ => false,
            }
        {
            return Err(invalid("unknown source field"));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
struct Event {
    #[serde(default)]
    at: f64,
    #[serde(default)]
    starting: bool,
    #[serde(default)]
    bundle: String,
}
fn timestamp(at: f64) -> Result<String> {
    if !at.is_finite() || !(0.0..=2.1e9).contains(&at) {
        return Err(invalid("usage timestamp"));
    }
    #[allow(clippy::cast_possible_truncation)]
    let nanos = (at * 1e9) as i128;
    Ok(jiff::Timestamp::from_nanosecond(nanos)
        .map_err(|_| invalid("usage timestamp"))?
        .to_string())
}
fn close_session(
    rows: &mut std::collections::BTreeMap<String, Value>,
    current: Option<&Event>,
    end: f64,
    locked: f64,
    device: &str,
) -> Result<()> {
    let Some(current) = current.filter(|c| end > c.at) else {
        return Ok(());
    };
    let capped = end.min(current.at + 14400.0);
    let credited = if locked > 0.0 {
        capped.min(locked)
    } else {
        capped
    };
    let mut cursor = current.at;
    while cursor < capped {
        let at = timestamp(cursor)?
            .parse::<jiff::Timestamp>()
            .map_err(|_| invalid("usage time"))?
            .to_zoned(jiff::tz::TimeZone::system());
        let next = at
            .date()
            .tomorrow()
            .map_err(|_| invalid("usage day"))?
            .to_zoned(at.time_zone().clone())
            .map_err(|_| invalid("usage midnight"))?;
        #[allow(clippy::cast_precision_loss)]
        let until = capped.min(next.timestamp().as_second() as f64);
        let zone = at.time_zone().iana_name().unwrap_or("Local");
        let day = format!("{}@{zone}", at.date());
        let key = format!("{device}/{day}/{}", current.bundle);
        let row = rows.entry(key).or_insert_with(|| json!({"device_id":{"$ref":{"collection":"usage.device","native_key":device}},"day":day,"bundle_id":current.bundle,"seconds_estimated":0.0,"clipped_seconds":0.0,"sessions":0.0,"provenance":"biome.app_infocus","decoder_version":"infocus-segb-1","uncertainty":"focus estimates; remote batches may lag; settings_parity: unproven"}));
        let seconds = if [
            "com.apple.universalcontrol",
            "com.apple.UserNotificationCenter",
            "com.apple.systempreferences",
            "com.apple.controlcenter",
            "com.apple.notificationcenterui",
        ]
        .contains(&current.bundle.as_str())
        {
            0.0
        } else {
            (until.min(credited) - cursor).max(0.0)
        };
        row["seconds_estimated"] =
            json!(row["seconds_estimated"].as_f64().unwrap_or_default() + seconds);
        row["clipped_seconds"] =
            json!(row["clipped_seconds"].as_f64().unwrap_or_default() + until - cursor - seconds);
        row["sessions"] = json!(row["sessions"].as_f64().unwrap_or_default() + 1.0);
        row["latest_event_at"] = json!(timestamp(end)?);
        cursor = until;
    }
    Ok(())
}
fn usage(state: &Shared, tx: &mpsc::Sender<Request>, input: &Value) -> Result<Value> {
    let device = string(input, "device");
    let platform = string(input, "platform");
    if device.is_empty() {
        return Err(invalid("usage device"));
    }
    let batch: Vec<Event> = serde_json::from_value(input["events"].clone())?;
    let mut s = lock(state)?;
    let mut events: Vec<Event> = if s.data["usage_events"][&device].is_null() {
        vec![]
    } else {
        serde_json::from_value(s.data["usage_events"][&device].clone())?
    };
    events.extend(batch);
    if s.data["usage_events"].is_null() {
        s.data["usage_events"] = json!({});
    }
    s.data["usage_events"][&device] = serde_json::to_value(&events)?;
    events.sort_by(|a, b| {
        a.at.total_cmp(&b.at)
            .then(a.starting.cmp(&b.starting))
            .then(a.bundle.cmp(&b.bundle))
    });
    events.dedup();
    let mut rows = std::collections::BTreeMap::new();
    let mut current = None;
    let mut locked = 0.0;
    let mut latest: f64 = 0.0;
    for event in &events {
        if !(1.6e9..=2.1e9).contains(&event.at) || event.bundle.is_empty() {
            continue;
        }
        latest = latest.max(event.at);
        if platform == "mac"
            && [
                "loginwindow",
                "com.apple.ScreenSaver.Engine",
                "bridge.power.sleep",
            ]
            .contains(&event.bundle.as_str())
        {
            if event.starting && current.is_some() && locked == 0.0 {
                locked = event.at;
            }
            continue;
        }
        if event.starting {
            close_session(&mut rows, current, event.at, locked, &device)?;
            current = Some(event);
            locked = 0.0;
        } else if current.is_some_and(|c| c.bundle == event.bundle) {
            close_session(&mut rows, current, event.at, locked, &device)?;
            current = None;
            locked = 0.0;
        }
    }
    if locked > 0.0 {
        close_session(&mut rows, current, locked, locked, &device)?;
    }
    let mut records = vec![observation(
        "usage.device",
        &device,
        json!({"name":format!("Synthetic {platform} {device}"),"platform":platform,"build":"sim","is_local":device=="local","latest_event_at":timestamp(latest)?,"provenance":"biome.app_infocus","decoder_version":"infocus-segb-1"}),
    )];
    records.extend(
        rows.into_iter()
            .map(|(key, data)| observation("usage.app_day", &key, data)),
    );
    for record in &mut records {
        let collection = string(record, "collection");
        let key = string(record, "native_key");
        record["scoped_alias"] = json!(format!("usage/{collection}/{key}"));
        s.data["records"][format!("{collection}/{key}")] = record.clone();
    }
    s.save()?;
    drop(s);
    enqueue(
        tx,
        json!({"t":"Capture","source":"usage","observations":records}),
        true,
    )?;
    Ok(json!({"effect_done":true}))
}

fn model_ref(collection: &str, key: &str) -> Value {
    json!({"$ref":{"collection":collection,"native_key":key}})
}
pub(super) fn readmodels(
    state: &Shared,
    tx: &mpsc::Sender<Request>,
    input: &Value,
) -> Result<Value> {
    let count = input["count"].as_u64().unwrap_or_default();
    if count > 170_000 {
        return Err(invalid("readmodel count"));
    }
    let mut records = vec![];
    let mut frame = json!({"t":"Capture","source":"messages"});
    match string(input, "action").as_str() {
        "seed" => {
            for i in 0..500 {
                records.push(observation(
                    "messages.chat",
                    &format!("rm-chat-{i:03}"),
                    json!({"display_name":format!("Bridge Test {i:03}"),"participants":[],"style":"direct","service":"iMessage"}),
                ));
            }
            for i in 0..count {
                let chat = if i < 499 {
                    format!("rm-chat-{:03}", i + 1)
                } else {
                    "rm-chat-000".to_owned()
                };
                let sent = jiff::Timestamp::from_second(
                    1_767_225_600 + i64::try_from(i / 2).map_err(|_| invalid("readmodel index"))?,
                )
                .map_err(|_| invalid("readmodel time"))?;
                records.push(observation(
                    "messages.message",
                    &format!("rm-message-{i:06}"),
                    json!({"chat_id":model_ref("messages.chat",&chat),"sent_at":sent.to_string(),"is_from_me":true,"kind":"message","text":format!("[ab-test] synthetic {i:06}"),"parts":[]}),
                ));
            }
            frame["scan_id"] = json!(super::id("scan")?);
            frame["scope"] = json!(["messages.chat", "messages.message"]);
            frame["scan_started_at"] = json!(now());
            frame["completion_evidence"] = json!("complete");
        }
        "put" => {
            for record in input["records"]
                .as_array()
                .ok_or_else(|| invalid("readmodel records"))?
            {
                let collection = string(record, "collection");
                let key = string(record, "native_key");
                if source(&collection) != "messages" || key.is_empty() || record["kind"] != "upsert"
                {
                    return Err(invalid("readmodel fixture"));
                }
                let mut record = record.clone();
                record["observed_at"] = json!(now());
                record["scoped_alias"] = json!(format!("{collection}/{key}"));
                records.push(record);
            }
        }
        _ => return Err(invalid("readmodel action")),
    }
    let mut s = lock(state)?;
    for record in &records {
        s.put(record.clone());
    }
    s.save()?;
    drop(s);
    frame["observations"] = json!(records);
    enqueue(tx, frame, true)?;
    Ok(json!({"committed":true}))
}
