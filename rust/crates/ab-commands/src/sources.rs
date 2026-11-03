use crate::{
    Args, Command, Error, Timestamp, Value, arr, b, base64, check, decode, n, p, s, timestamp,
};
use sha2::{Digest, Sha256};
pub(crate) fn validate(c: &Command, now: Timestamp) -> Result<Args, Error> {
    let op = c.op.as_str();
    if op.starts_with("contacts.") {
        return contacts(c);
    }
    let shape = shape(op);
    let a = decode(c.args.get(), shape)?;
    if op.starts_with("reminders.") {
        reminders(c, &a)?;
    } else if op.starts_with("calendar.") {
        calendar(c, &a)?;
    } else if op.starts_with("notes.") {
        notes(c, &a)?;
    } else if op.starts_with("messages.") {
        messages(c, &a)?;
    } else if op.starts_with("mac.") && op != "mac.screen_record" {
        mac(c, &a)?;
    } else if op.starts_with("files.") {
        files(c, &a)?;
    } else if op == "focus.set" {
        check(p(&a, "on") && s(&a, "mode") == "Do Not Disturb")?;
        if !s(&a, "until").is_empty() {
            check(b(&a, "on") && s(&a, "until").parse::<Timestamp>().is_ok_and(|t| t > now))?;
        }
    } else if op == "shortcuts.run" {
        check(
            !s(&a, "shortcut").trim().is_empty()
                && s(&a, "shortcut").len() <= 1024
                && !s(&a, "shortcut").starts_with('-')
                && s(&a, "input").len() <= 16384
                && (0.0..=300_000.0).contains(&n(&a, "timeout_ms")),
        )?;
        if !s(&a, "input_base64").is_empty() {
            check(base64(s(&a, "input_base64"))?.len() <= 16384 && s(&a, "input").is_empty())?;
        }
    } else if op.starts_with("iphone.") {
        iphone(c, &a, now)?;
    } else if matches!(
        op,
        "capture.screenshot"
            | "capture.camera_photo"
            | "capture.record_audio"
            | "mac.screen_record"
            | "print.document"
    ) {
        capture(op, &a)?;
    } else if matches!(op, "speech.say" | "capture.play_sound") {
        check(!p(&a, "volume") || (0.0..=1.0).contains(&n(&a, "volume")))?;
        check(if op == "speech.say" {
            !s(&a, "text").trim().is_empty()
                && s(&a, "text").len() <= 2048
                && s(&a, "name").is_empty()
        } else {
            s(&a, "text").is_empty()
                && [
                    "Basso",
                    "Blow",
                    "Bottle",
                    "Frog",
                    "Funk",
                    "Glass",
                    "Hero",
                    "Morse",
                    "Ping",
                    "Pop",
                    "Purr",
                    "Sosumi",
                    "Submarine",
                    "Tink",
                ]
                .contains(&s(&a, "name"))
        })?;
    } else {
        return Err(Error::Unsupported);
    }
    Ok(a)
}
pub(crate) fn shape(op: &str) -> &'static str {
    if op.starts_with("reminders.") {
        "ReminderArgs"
    } else if op.starts_with("calendar.") {
        "CalendarArgs"
    } else if op.starts_with("notes.") {
        "NoteArgs"
    } else if op.starts_with("messages.") {
        "MessagesArgs"
    } else if op.starts_with("files.") {
        "FileArgs"
    } else if op == "mac.screen_record"
        || op.starts_with("capture.") && op != "capture.play_sound"
        || op == "print.document"
    {
        "CaptureArgs"
    } else if op.starts_with("mac.") {
        "MacArgs"
    } else if op == "focus.set" {
        "FocusSetArgs"
    } else if op == "shortcuts.run" {
        "ShortcutRunArgs"
    } else if matches!(op, "speech.say" | "capture.play_sound") {
        "PlaybackArgs"
    } else if op.starts_with("iphone.alarm.") || op.starts_with("iphone.timer.") {
        "IPhoneAlarmArgs"
    } else if op.starts_with("iphone.live.") {
        "IPhoneLiveArgs"
    } else {
        match op {
            "iphone.motion.history" => "IPhoneMotionHistoryArgs",
            "iphone.screentime.schedule" => "IPhoneScheduleArgs",
            "iphone.health.log" => "IPhoneHealthArgs",
            "iphone.notify" => "IPhoneNotifyArgs",
            "iphone.capture.audio" => "IPhoneCaptureAudioArgs",
            "iphone.geofence.add" => "IPhoneGeofence",
            "iphone.geofence.remove" => "IPhoneGeofenceName",
            "iphone.location.significant" => "IPhoneSignificant",
            _ => "IPhoneTarget",
        }
    }
}
fn allowed(c: &Command, keys: &[&str], reject_null: bool) -> Result<(), Error> {
    let v: Value = serde_json::from_str(c.args.get()).map_err(|_| Error::Invalid)?;
    for (k, v) in v.as_object().ok_or(Error::Invalid)? {
        check(keys.contains(&k.as_str()) && (!reject_null || !v.is_null()))?;
    }
    Ok(())
}
fn reminders(c: &Command, a: &Args) -> Result<(), Error> {
    let fields = match c.op.as_str() {
        "reminders.create" => vec![
            "list_id",
            "title",
            "notes",
            "url",
            "due",
            "due_all_day",
            "start",
            "priority",
        ],
        "reminders.create_list" => vec!["title"],
        "reminders.rename_list" => vec!["id", "title"],
        "reminders.delete_list"
        | "reminders.complete"
        | "reminders.uncomplete"
        | "reminders.delete" => vec!["id"],
        "reminders.update" => vec![
            "id",
            "list_id",
            "title",
            "notes",
            "url",
            "due",
            "due_all_day",
            "start",
            "priority",
        ],
        _ => vec![],
    };
    allowed(c, &fields, true)?;
    if fields.is_empty() {
        return Err(Error::Unsupported);
    }
    check(!p(a, "title") || !s(a, "title").trim().is_empty())?;
    let patch = [
        "title",
        "notes",
        "url",
        "due",
        "due_all_day",
        "start",
        "priority",
    ]
    .iter()
    .any(|k| p(a, k));
    check(match c.op.as_str() {
        "reminders.create" => {
            s(a, "id").is_empty()
                && c.expected_version == 0
                && !s(a, "list_id").is_empty()
                && p(a, "title")
        }
        "reminders.create_list" => c.expected_version == 0 && p(a, "title"),
        "reminders.rename_list" => {
            !s(a, "id").is_empty() && c.expected_version != 0 && p(a, "title")
        }
        "reminders.update" => {
            !s(a, "id").is_empty()
                && c.expected_version != 0
                && (patch || !s(a, "list_id").is_empty())
        }
        _ => !s(a, "id").is_empty() && c.expected_version != 0,
    })?;
    check(!p(a, "priority") || (0.0..=9.0).contains(&n(a, "priority")))
}
fn notes(c: &Command, a: &Args) -> Result<(), Error> {
    let op = c.op.as_str();
    check(
        !p(a, "html")
            || op == "notes.update"
                && !p(a, "plain_text")
                && s(a, "mode") != "append"
                && !s(a, "html").trim().is_empty(),
    )?;
    let create = c.expected_version == 0 && s(a, "id").is_empty();
    let update = c.expected_version != 0 && !s(a, "id").is_empty();
    check(match op {
        "notes.create" => {
            create
                && !s(a, "folder_id").is_empty()
                && p(a, "title")
                && !s(a, "title").trim().is_empty()
                && s(a, "mode").is_empty()
        }
        "notes.create_folder" => {
            create
                && s(a, "folder_id").is_empty()
                && p(a, "title")
                && !s(a, "title").trim().is_empty()
                && !p(a, "plain_text")
                && s(a, "mode").is_empty()
        }
        "notes.update" => {
            update
                && s(a, "folder_id").is_empty()
                && !p(a, "title")
                && (p(a, "plain_text") || p(a, "html"))
                && ["", "replace", "append"].contains(&s(a, "mode"))
        }
        "notes.rename_folder" => {
            update
                && s(a, "folder_id").is_empty()
                && p(a, "title")
                && s(a, "title").starts_with("Bridge Test [ab-test")
                && !p(a, "plain_text")
                && s(a, "mode").is_empty()
        }
        "notes.move" => {
            update
                && !s(a, "folder_id").is_empty()
                && !p(a, "title")
                && !p(a, "plain_text")
                && s(a, "mode").is_empty()
        }
        "notes.delete" => {
            update
                && s(a, "folder_id").is_empty()
                && !p(a, "title")
                && !p(a, "plain_text")
                && s(a, "mode").is_empty()
        }
        _ => return Err(Error::Unsupported),
    })
}
fn messages(c: &Command, a: &Args) -> Result<(), Error> {
    let fields: &[&str] = match c.op.as_str() {
        "messages.send" => &[
            "chat_id",
            "handles",
            "service",
            "text",
            "files",
            "resend_of",
        ],
        "messages.reply" => &["id", "part_index", "text", "files"],
        "messages.react" => &["id", "part_index", "reaction", "emoji", "remove"],
        "messages.edit" => &["id", "part_index", "text"],
        "messages.unsend" => &["id", "part_index"],
        _ => &["id"],
    };
    allowed(c, fields, true)?;
    check(n(a, "part_index") >= 0.0 && arr(a, "files").len() <= 10 && s(a, "text").len() <= 65536)?;
    for file in arr(a, "files") {
        let f = file.as_object().ok_or(Error::Invalid)?;
        let data = base64(s(f, "data"))?;
        if data.len() > 100 << 20 {
            return Err(Error::TooLarge);
        }
        check(
            s(f, "data").is_empty() != s(f, "upload_id").is_empty()
                && !s(f, "name").is_empty()
                && ![".", ".."].contains(&s(f, "name"))
                && !s(f, "name").contains('/'),
        )?;
    }
    if c.op == "messages.send" {
        check(
            (s(a, "resend_of").is_empty() || !s(a, "chat_id").is_empty())
                && (s(a, "chat_id").is_empty() || s(a, "service").is_empty())
                && (s(a, "chat_id").is_empty() != arr(a, "handles").is_empty())
                && (!s(a, "text").is_empty() || !arr(a, "files").is_empty())
                && c.expected_version == 0,
        )?;
        if !arr(a, "handles").is_empty() {
            check(
                arr(a, "handles").len() == 1
                    && ["iMessage", "SMS", "RCS"].contains(&s(a, "service")),
            )?;
        }
    } else {
        check(!s(a, "id").is_empty())?;
        if matches!(c.op.as_str(), "messages.edit" | "messages.unsend")
            || c.op.starts_with("messages.delete_")
        {
            check(c.expected_version != 0)?;
        }
    }
    if matches!(c.op.as_str(), "messages.edit" | "messages.reply") {
        check(!s(a, "text").is_empty())?;
    }
    if c.op == "messages.react" {
        check(
            ["love", "like", "dislike", "laugh", "emphasize", "question"]
                .contains(&s(a, "reaction"))
                || s(a, "reaction") == "emoji" && !s(a, "emoji").is_empty(),
        )?;
    }
    Ok(())
}
fn calendar(c: &Command, a: &Args) -> Result<(), Error> {
    if s(a, "span") == "all" {
        return Err(Error::Unsupported);
    }
    let mut fields = vec![
        "calendar_id",
        "title",
        "notes",
        "location",
        "url",
        "start",
        "end",
        "all_day",
        "time_zone",
        "recurrence",
        "alarms",
        "availability",
    ];
    match c.op.as_str() {
        "calendar.create_calendar" => fields = vec!["title"],
        "calendar.delete_event" => fields = vec!["id", "span"],
        "calendar.update_event" => fields.extend(["id", "span"]),
        _ => {}
    }
    allowed(c, &fields, true)?;
    check(!p(a, "title") || !s(a, "title").trim().is_empty())?;
    check(match c.op.as_str() {
        "calendar.create_calendar" => p(a, "title") && c.expected_version == 0,
        "calendar.create_event" => {
            !s(a, "calendar_id").is_empty()
                && p(a, "title")
                && p(a, "start")
                && p(a, "end")
                && c.expected_version == 0
        }
        "calendar.update_event" | "calendar.delete_event" => {
            !s(a, "id").is_empty()
                && c.expected_version != 0
                && ["this", "future"].contains(&s(a, "span"))
                && (c.op != "calendar.update_event" || a.len() >= 3)
        }
        _ => return Err(Error::Unsupported),
    })?;
    let mut times = Vec::new();
    for k in ["start", "end"] {
        if p(a, k) {
            let text = s(a, k);
            let date = text.parse::<jiff::civil::Date>();
            let t = if b(a, "all_day") {
                date.map(|d| d.to_string())
            } else if timestamp(text) {
                Ok(text.to_string())
            } else if !p(a, "all_day") && c.op == "calendar.update_event" {
                date.map(|d| d.to_string())
            } else {
                return Err(Error::Invalid);
            };
            times.push(t.map_err(|_| Error::Invalid)?);
        }
    }
    if times.len() == 2 {
        let to_time = |text: &str| {
            text.parse::<Timestamp>()
                .or_else(|_| format!("{text}T00:00:00Z").parse())
        };
        let start = to_time(&times[0]).map_err(|_| Error::Invalid)?;
        let end = to_time(&times[1]).map_err(|_| Error::Invalid)?;
        let zero = "0001-01-01T00:00:00Z";
        let ordered = start.to_string() == zero || end.to_string() == zero || end > start;
        check(ordered)?;
    }
    if !s(a, "time_zone").is_empty() {
        check(s(a, "time_zone") == "Local" || jiff::tz::TimeZone::get(s(a, "time_zone")).is_ok())?;
    }
    for rule in arr(a, "recurrence") {
        let r = rule.as_object().ok_or(Error::Invalid)?;
        check(
            n(r, "interval") >= 1.0
                && n(r, "count") >= 0.0
                && ["daily", "weekly", "monthly", "yearly"].contains(&s(r, "frequency"))
                && (s(r, "end").is_empty() || timestamp(s(r, "end"))),
        )?;
    }
    for alarm in arr(a, "alarms") {
        let a = alarm.as_object().ok_or(Error::Invalid)?;
        check(s(a, "absolute_date").is_empty() == p(a, "relative_offset"))?;
        if !s(a, "absolute_date").is_empty() {
            check(timestamp(s(a, "absolute_date")))?;
        }
    }
    check(!p(a, "availability") || (-1.0..=3.0).contains(&n(a, "availability")))
}
fn safe(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains('\0')
        && !path.split('/').any(|s| s == ".." || s == ".")
}
fn files(c: &Command, a: &Args) -> Result<(), Error> {
    check(s(a, "root").starts_with('/') && safe(s(a, "path")) && safe(s(a, "parent")))?;
    match c.op.as_str() {
        "files.create" | "files.write" | "files.move" | "files.rename" => {
            check(!s(a, "name").is_empty() && !s(a, "name").contains('/') && safe(s(a, "name")))?;
        }
        "files.trash" | "files.set_tags" | "files.download" | "files.evict" => {}
        _ => return Err(Error::Unsupported),
    }
    if matches!(c.op.as_str(), "files.create" | "files.write") {
        let data = base64(s(a, "content_base64"))?;
        check(data.len() <= 128 << 10 && format!("{:x}", Sha256::digest(data)) == s(a, "digest"))?;
    } else {
        check(!s(a, "path").is_empty())?;
    }
    check(arr(a, "tags").len() <= 64)
}
fn mac(c: &Command, a: &Args) -> Result<(), Error> {
    let op = c.op.as_str();
    if matches!(
        op,
        "mac.lock_screen"
            | "mac.sleep_display"
            | "mac.restart"
            | "mac.shutdown"
            | "mac.notify"
            | "mac.open_settings"
            | "mac.set_wallpaper"
    ) {
        allowed(
            c,
            match op {
                "mac.notify" => &["title", "body", "action_url"],
                "mac.open_settings" => &["pane"],
                "mac.set_wallpaper" => &["image_blob"],
                _ => &[],
            },
            false,
        )?;
    }
    check(op == "mac.clipboard_write" || !p(a, "text"))?;
    check(match op {
        "mac.lock_screen" | "mac.sleep_display" | "mac.restart" | "mac.shutdown" => true,
        "mac.open_settings" => [
            "general",
            "notifications",
            "network",
            "displays",
            "wallpaper",
            "privacy",
        ]
        .contains(&s(a, "pane")),
        "mac.notify" => {
            !s(a, "title").is_empty()
                && s(a, "title").len() <= 256
                && s(a, "body").len() <= 4096
                && (s(a, "action_url").is_empty() || action_url(s(a, "action_url")))
        }
        "mac.set_wallpaper" => {
            let v = base64(s(a, "image_blob"))?;
            !v.is_empty() && v.len() <= 8 << 20
        }
        "mac.iwork_export" => {
            let v = base64(s(a, "document_blob"))?;
            !v.is_empty()
                && v.len() <= 8 << 20
                && ["pdf", "text"].contains(&s(a, "format"))
                && ["pages", "numbers", "keynote"].contains(&s(a, "app"))
        }
        "mac.clipboard_write" => p(a, "text") && s(a, "text").len() <= 65536,
        "mac.set_volume" => p(a, "volume") && (0.0..=100.0).contains(&n(a, "volume")),
        "mac.set_mute" => p(a, "muted"),
        "mac.set_appearance" => ["light", "dark"].contains(&s(a, "appearance")),
        "mac.launch_app" | "mac.activate_app" | "mac.quit_app" => {
            !s(a, "bundle_id").is_empty() && s(a, "bundle_id").len() <= 255
        }
        "mac.keep_awake" => p(a, "on") && (0.0..=86400.0).contains(&n(a, "duration")),
        _ => false,
    })
}
fn capture(op: &str, a: &Args) -> Result<(), Error> {
    check(op == "print.document" || s(a, "blob").is_empty() && s(a, "blob_id").is_empty())?;
    check(match op {
        "capture.screenshot" => {
            !(p(a, "display") && p(a, "window"))
                && (!p(a, "display") || n(a, "display") > 0.0)
                && (!p(a, "window") || n(a, "window") > 0.0)
                && n(a, "seconds") == 0.0
                && s(a, "file").is_empty()
                && s(a, "printer").is_empty()
        }
        "capture.camera_photo" => {
            !p(a, "display")
                && !p(a, "window")
                && n(a, "seconds") == 0.0
                && ["blob", "blob_id", "file", "printer"]
                    .iter()
                    .all(|k| s(a, k).is_empty())
        }
        "capture.record_audio" | "mac.screen_record" => {
            (1.0..=if op == "mac.screen_record" {
                30.0
            } else {
                60.0
            })
                .contains(&n(a, "seconds"))
                && !p(a, "display")
                && !p(a, "window")
                && s(a, "file").is_empty()
                && s(a, "printer").is_empty()
        }
        "print.document" => {
            (!s(a, "file").trim().is_empty()
                || digest(s(a, "blob")) && s(a, "blob_id").starts_with("qry_"))
                && (s(a, "file").is_empty()
                    || s(a, "blob").is_empty() && s(a, "blob_id").is_empty())
                && !s(a, "printer").trim().is_empty()
                && !p(a, "display")
                && !p(a, "window")
                && n(a, "seconds") == 0.0
        }
        _ => false,
    })
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[allow(clippy::too_many_lines)]
fn contacts(c: &Command) -> Result<Args, Error> {
    if [
        "contacts.create_group",
        "contacts.rename_group",
        "contacts.delete_group",
        "contacts.add_member",
        "contacts.remove_member",
    ]
    .contains(&c.op.as_str())
    {
        let a = decode(c.args.get(), "ContactGroupArgs")?;
        let named = !s(&a, "name").trim().is_empty() && s(&a, "name").len() <= 256;
        check(match c.op.as_str() {
            "contacts.create_group" => {
                s(&a, "id").is_empty()
                    && c.expected_version == 0
                    && named
                    && s(&a, "contact_id").is_empty()
            }
            "contacts.rename_group" => {
                !s(&a, "id").is_empty()
                    && c.expected_version != 0
                    && named
                    && s(&a, "contact_id").is_empty()
            }
            "contacts.delete_group" => {
                !s(&a, "id").is_empty()
                    && c.expected_version != 0
                    && s(&a, "name").is_empty()
                    && s(&a, "contact_id").is_empty()
            }
            _ => {
                !s(&a, "id").is_empty()
                    && c.expected_version != 0
                    && s(&a, "name").is_empty()
                    && !s(&a, "contact_id").is_empty()
            }
        })?;
        return Ok(a);
    }
    let v: Value = serde_json::from_str(c.args.get()).map_err(|_| Error::Invalid)?;
    let a = v.as_object().ok_or(Error::Invalid)?;
    if a.contains_key("id") {
        check(a["id"].is_string() && !s(a, "id").is_empty())?;
    }
    let strings = [
        "previous_family_name",
        "phonetic_organization",
        "given_name",
        "middle_name",
        "family_name",
        "nickname",
        "name_prefix",
        "name_suffix",
        "phonetic_given_name",
        "phonetic_middle_name",
        "phonetic_family_name",
        "organization",
        "job_title",
        "department",
    ];
    for (k, v) in a {
        match k.as_str() {
            "id" => {}
            "image" => {
                if !v.is_null() {
                    let img = v.as_object().ok_or(Error::Invalid)?;
                    check(img.keys().all(|k| k == "base64"))?;
                    let data = base64(s(img, "base64"))?;
                    check(!data.is_empty() && data.len() <= 8 << 20)?;
                }
            }
            "birthday" | "non_gregorian_birthday" => contact_date(v)?,
            "phones" | "emails" | "urls" | "relations" | "dates" | "postal_addresses"
            | "social_profiles" | "instant_messages" => {
                for entry in v.as_array().ok_or(Error::Invalid)? {
                    let e = entry.as_object().ok_or(Error::Invalid)?;
                    check(
                        e.keys().all(|k| k == "label" || k == "value")
                            && (!p(e, "label") || e["label"].is_string()),
                    )?;
                    let val = e.get("value").ok_or(Error::Invalid)?;
                    if matches!(k.as_str(), "phones" | "emails" | "urls" | "relations") {
                        check(val.is_string())?;
                    } else if k == "dates" {
                        check(!val.is_null())?;
                        contact_date(val)?;
                    } else {
                        check(
                            val.as_object()
                                .is_some_and(|o| o.values().all(|v| v.is_string() || v.is_null())),
                        )?;
                    }
                }
            }
            _ => check(strings.contains(&k.as_str()) && v.is_string())?,
        }
    }
    check(match c.op.as_str() {
        "contacts.create" => s(a, "id").is_empty() && c.expected_version == 0 && !a.is_empty(),
        "contacts.update" => !s(a, "id").is_empty() && c.expected_version != 0 && a.len() >= 2,
        "contacts.delete" => !s(a, "id").is_empty() && c.expected_version != 0 && a.len() == 1,
        _ => return Err(Error::Unsupported),
    })?;
    Ok(a.clone())
}
fn contact_date(v: &Value) -> Result<(), Error> {
    if v.is_null() {
        return Ok(());
    }
    let a = v.as_object().ok_or(Error::Invalid)?;
    check(
        a.keys()
            .all(|k| ["year", "is_leap_month", "month", "day", "calendar"].contains(&k.as_str())),
    )?;
    for k in ["year", "month", "day"] {
        check(!p(a, k) || a[k].as_i64().is_some())?;
    }
    check(!p(a, "is_leap_month") || a["is_leap_month"].is_boolean())?;
    check(!p(a, "calendar") || a["calendar"].is_string())?;
    check((1.0..=12.0).contains(&n(a, "month")) && (1.0..=31.0).contains(&n(a, "day")))
}
#[allow(clippy::too_many_lines)]
fn iphone(c: &Command, a: &Args, now: Timestamp) -> Result<(), Error> {
    check(c.args.get().len() <= 1800 && s(a, "device_id").starts_with("iphone:"))?;
    let op = c.op.as_str();
    if op.starts_with("iphone.alarm.") || op.starts_with("iphone.timer.") {
        check(
            [
                "iphone.alarm.create",
                "iphone.alarm.update",
                "iphone.alarm.cancel",
                "iphone.alarm.list",
                "iphone.timer.start",
                "iphone.timer.cancel",
            ]
            .contains(&op),
        )?;
        check(
            (!p(a, "snooze_seconds") || n(a, "snooze_seconds") > 0.0) && s(a, "title").len() <= 200,
        )?;
        if matches!(op, "iphone.alarm.create" | "iphone.alarm.update") {
            check(
                n(a, "fire_at")
                    > now
                        .as_second()
                        .to_string()
                        .parse::<f64>()
                        .map_err(|_| Error::Invalid)?,
            )?;
        }
        if op == "iphone.timer.start" {
            check(n(a, "duration_seconds") > 0.0)?;
        }
        if !s(a, "id").is_empty()
            || matches!(
                op,
                "iphone.alarm.update" | "iphone.alarm.cancel" | "iphone.timer.cancel"
            )
        {
            check(uuid::Uuid::parse_str(s(a, "id")).is_ok())?;
        }
    } else if op.starts_with("iphone.live.") {
        check(
            ["iphone.live.start", "iphone.live.update", "iphone.live.end"].contains(&op)
                && !s(a, "task_id").is_empty()
                && s(a, "task_id") != "_start"
                && s(a, "task_id").len() <= 128
                && !s(a, "title").trim().is_empty()
                && s(a, "title").len() <= 128
                && s(a, "status").len() <= 256
                && (0.0..=1.0).contains(&n(a, "progress")),
        )?;
    } else {
        match op {
            "iphone.motion.history" => check((1.0..=7.0).contains(&n(a, "days")))?,
            "iphone.screentime.schedule" => check(
                (0.0..1440.0).contains(&n(a, "start_minute"))
                    && (0.0..1440.0).contains(&n(a, "end_minute"))
                    && (n(a, "end_minute") - n(a, "start_minute") + 1440.0) % 1440.0 >= 15.0,
            )?,
            "iphone.health.log" => check(
                timestamp(s(a, "start"))
                    && n(a, "value") > 0.0
                    && !s(a, "tag").is_empty()
                    && s(a, "tag").len() <= 128
                    && (["water", "weight", "mindful"].contains(&s(a, "kind"))
                        || s(a, "kind") == "workout"
                            && ["walking", "running", "cycling", "other"]
                                .contains(&s(a, "activity"))),
            )?,
            "iphone.capture.audio" => {
                check(!p(a, "seconds") || (1.0..=10.0).contains(&n(a, "seconds")))?;
            }
            "iphone.location.significant" => check(p(a, "enabled"))?,
            "iphone.geofence.add" => check(
                !s(a, "name").trim().is_empty()
                    && s(a, "name").len() <= 100
                    && p(a, "latitude")
                    && p(a, "longitude")
                    && p(a, "radius")
                    && n(a, "latitude").abs() <= 90.0
                    && n(a, "longitude").abs() <= 180.0
                    && (1.0..=100_000.0).contains(&n(a, "radius")),
            )?,
            "iphone.geofence.remove" => {
                check(!s(a, "name").trim().is_empty() && s(a, "name").len() <= 100)?;
            }
            "iphone.notify" => {
                check(
                    !s(a, "title").trim().is_empty()
                        && s(a, "title").len() <= 200
                        && s(a, "body").len() <= 1000
                        && s(a, "thread").len() <= 100
                        && arr(a, "buttons").len() <= 4,
                )?;
                let mut seen = std::collections::HashSet::new();
                for b in arr(a, "buttons") {
                    let b = b.as_object().ok_or(Error::Invalid)?;
                    check(
                        !s(b, "id").is_empty()
                            && s(b, "id").len() <= 64
                            && !s(b, "id").starts_with("com.apple.")
                            && !s(b, "title").trim().is_empty()
                            && s(b, "title").len() <= 100
                            && seen.insert(s(b, "id")),
                    )?;
                }
            }
            "iphone.device.status"
            | "iphone.ping"
            | "iphone.capture.photo"
            | "iphone.location.current"
            | "iphone.geofence.list"
            | "iphone.motion.activity"
            | "iphone.screentime.shield"
            | "iphone.screentime.unshield"
            | "iphone.screentime.cancel" => {}
            _ => return Err(Error::Invalid),
        }
    }
    Ok(())
}

fn action_url(raw: &str) -> bool {
    if raw.bytes().any(|b| b < 32 || b == 127) {
        return false;
    }
    let Some((scheme, rest)) = raw.split_once("://") else {
        return false;
    };
    if !["http", "https"]
        .iter()
        .any(|value| scheme.eq_ignore_ascii_case(value))
    {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() {
        return false;
    }
    let valid_escapes = |text: &str| {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                if bytes.get(i + 1).is_none_or(|v| !v.is_ascii_hexdigit())
                    || bytes.get(i + 2).is_none_or(|v| !v.is_ascii_hexdigit())
                {
                    return false;
                }
                i += 3;
            } else {
                i += 1;
            }
        }
        true
    };
    let before_fragment = raw.split('#').next().unwrap_or("");
    if !valid_escapes(before_fragment.split('?').next().unwrap_or(""))
        || !valid_escapes(raw.split_once('#').map_or("", |(_, fragment)| fragment))
    {
        return false;
    }
    let port = if let Some(bracket) = host.strip_prefix('[') {
        let Some((address, suffix)) = bracket.split_once(']') else {
            return false;
        };
        let ip = address.split("%25").next().unwrap_or("");
        if ip.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        suffix
    } else {
        host.find(':').map_or("", |i| &host[i..])
    };
    if !port.is_empty()
        && (!port.starts_with(':') || !port[1..].bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    if host
        .bytes()
        .any(|b| b.is_ascii() && !b.is_ascii_alphanumeric() && !b"-._~!$&'()*+,;=:[]%".contains(&b))
    {
        return false;
    }
    let bytes = host.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'%'
            && bytes.get(i + 1).is_some_and(|v| *v < b'8')
            && bytes.get(i..i + 3) != Some(b"%25")
        {
            return false;
        }
    }
    true
}
