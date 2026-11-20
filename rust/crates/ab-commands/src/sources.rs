use crate::{
    Args, Command, Error, Timestamp, Value, Violation, arr, b, base64, check, decode, n, p, s,
    timestamp,
};
use sha2::{Digest, Sha256};
fn require(ok: bool, field: impl Into<String>, expected: &'static str) -> Result<(), Violation> {
    if ok {
        Ok(())
    } else {
        Err(Violation::invalid(
            field,
            "field does not meet the operation requirements",
            expected,
        ))
    }
}

pub(crate) fn validate(c: &Command, now: Timestamp) -> Result<Args, Violation> {
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
    } else if op.starts_with("mail.") {
        mail(c, &a)?;
    } else if op.starts_with("files.") {
        files(c, &a)?;
    } else if op == "pdf.annotate" {
        pdf(c, &a)?;
    } else if op == "focus.set" {
        require(p(&a, "on"), "args.on", "a boolean")?;
        require(
            s(&a, "mode") == "Do Not Disturb",
            "args.mode",
            "Do Not Disturb",
        )?;
        if !s(&a, "until").is_empty() {
            require(
                b(&a, "on") && s(&a, "until").parse::<Timestamp>().is_ok_and(|t| t > now),
                "args.until",
                "a future timestamp when enabling Do Not Disturb",
            )?;
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
        require(
            !p(&a, "volume") || (0.0..=1.0).contains(&n(&a, "volume")),
            "args.volume",
            "a number from 0 to 1",
        )?;
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
        return Err(Error::Unsupported.into());
    }
    Ok(a)
}
fn document_input(a: &Args) -> Result<(), Violation> {
    require(
        !a.contains_key("file"),
        "args.file",
        "a blob reference or inline Base64 input",
    )?;
    let inputs = usize::from(!s(a, "data_base64").is_empty()) + usize::from(p(a, "blob"));
    require(
        inputs == 1,
        "args.blob",
        "exactly one of blob or data_base64",
    )?;
    if !s(a, "data_base64").is_empty() {
        let data = base64(s(a, "data_base64")).map_err(|_| {
            Violation::invalid(
                "args.data_base64",
                "invalid Base64",
                "Base64-encoded document bytes",
            )
        })?;
        require(
            !data.is_empty() && data.len() <= 8 << 20,
            "args.data_base64",
            "1 byte to 8 MiB of document content",
        )?;
    }
    if p(a, "blob") {
        let reference = a["blob"].as_object().ok_or_else(|| {
            Violation::invalid(
                "args.blob",
                "field has the wrong type",
                "a blob reference object",
            )
        })?;
        let query = !s(reference, "query_id").is_empty();
        let record =
            !s(reference, "record_id").is_empty() && !s(reference, "collection").is_empty();
        require(
            digest(s(reference, "digest")),
            "args.blob.digest",
            "a SHA-256 digest",
        )?;
        require(
            query != record,
            "args.blob",
            "a query ID or a record ID and collection",
        )?;
    }
    Ok(())
}
fn pdf(c: &Command, a: &Args) -> Result<(), Violation> {
    let annotations = arr(a, "annotations");
    require(
        c.expected_version == 0,
        "expected_version",
        "omit for a new document",
    )?;
    require(
        (1..=100).contains(&annotations.len()),
        "args.annotations",
        "1 to 100 annotations",
    )?;
    document_input(a)?;
    for (index, annotation) in annotations.iter().enumerate() {
        let a = annotation.as_object().ok_or(Error::Invalid)?;
        let path = |field| format!("args.annotations[{index}].{field}");
        require(
            (1.0..=1000.0).contains(&n(a, "page")),
            path("page"),
            "a page number from 1 to 1000",
        )?;
        require(
            ["note", "highlight"].contains(&s(a, "type")),
            path("type"),
            "note or highlight",
        )?;
        require(
            s(a, "text").len() <= 4096 && (s(a, "type") != "note" || !s(a, "text").is_empty()),
            path("text"),
            "at most 4096 bytes of text, required for notes",
        )?;
        for field in ["x", "y"] {
            require(
                (0.0..=100_000.0).contains(&n(a, field)),
                path(field),
                "0 to 100000",
            )?;
        }
        for field in ["width", "height"] {
            let size = n(a, field);
            require(
                size > 0.0 && size <= 100_000.0,
                path(field),
                "greater than 0 and at most 100000",
            )?;
        }
    }
    Ok(())
}
pub(crate) fn shape(op: &str) -> &'static str {
    if op == "pdf.annotate" {
        "PDFAnnotateArgs"
    } else if op.starts_with("reminders.") {
        "ReminderArgs"
    } else if op.starts_with("calendar.") {
        "CalendarArgs"
    } else if op.starts_with("notes.") {
        "NoteArgs"
    } else if op.starts_with("messages.") {
        "MessagesArgs"
    } else if op.starts_with("mail.") {
        "MailArgs"
    } else if op.starts_with("files.") {
        "FileArgs"
    } else if op == "mac.screen_record"
        || op.starts_with("capture.") && op != "capture.play_sound"
        || op == "print.document"
    {
        "CaptureArgs"
    } else if op == "focus.set" {
        "FocusSetArgs"
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
fn allowed(c: &Command, keys: &[&str], reject_null: bool) -> Result<(), Violation> {
    let v: Value = serde_json::from_str(c.args.get()).map_err(|_| Error::Invalid)?;
    for (k, v) in v.as_object().ok_or(Error::Invalid)? {
        if !keys.contains(&k.as_str()) {
            return Err(Violation::invalid(
                format!("args.{k}"),
                "field is unsupported for this operation",
                "a field supported by the operation",
            ));
        }
        if reject_null && v.is_null() {
            return Err(Violation::invalid(
                format!("args.{k}"),
                "field must not be null",
                "a value or an omitted field",
            ));
        }
    }
    Ok(())
}
fn reminders(c: &Command, a: &Args) -> Result<(), Violation> {
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
            "alarms",
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
            "alarms",
        ],
        _ => vec![],
    };
    allowed(c, &fields, true)?;
    if fields.is_empty() {
        return Err(Error::Unsupported.into());
    }
    if p(a, "title") && s(a, "title").trim().is_empty() {
        return Err(Violation::invalid(
            "args.title",
            "title is empty",
            "a nonempty title",
        ));
    }
    if fields.contains(&"id") && s(a, "id").is_empty() {
        return Err(Violation::invalid(
            "args.id",
            "target is required",
            "a reminder or list ID",
        ));
    }
    if fields.contains(&"id") && c.expected_version == 0 {
        return Err(Violation::invalid(
            "expected_version",
            "target version is required",
            "the current version of the target",
        ));
    }
    let patch = [
        "title",
        "notes",
        "url",
        "due",
        "due_all_day",
        "start",
        "priority",
        "alarms",
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
    reminder_alarms(a)?;
    if p(a, "priority") && !(0.0..=9.0).contains(&n(a, "priority")) {
        return Err(Violation::invalid(
            "args.priority",
            "priority is outside the supported range",
            "0 to 9",
        ));
    }
    Ok(())
}
fn reminder_alarms(a: &Args) -> Result<(), Violation> {
    check(arr(a, "alarms").len() <= 8)?;
    for alarm in arr(a, "alarms") {
        let alarm = alarm.as_object().ok_or(Error::Invalid)?;
        let place = p(alarm, "latitude") || p(alarm, "longitude");
        let timed = !s(alarm, "absolute_date").is_empty();
        let relative = p(alarm, "relative_offset");
        check(u8::from(place) + u8::from(timed) + u8::from(relative) == 1)?;
        if place {
            check(
                p(alarm, "latitude")
                    && p(alarm, "longitude")
                    && (-90.0..=90.0).contains(&n(alarm, "latitude"))
                    && (-180.0..=180.0).contains(&n(alarm, "longitude"))
                    && ["arrive", "leave"].contains(&s(alarm, "proximity"))
                    && (!p(alarm, "radius_m") || (1.0..=100_000.0).contains(&n(alarm, "radius_m"))),
            )?;
        } else {
            check(
                !p(alarm, "proximity")
                    && s(alarm, "location_title").is_empty()
                    && !p(alarm, "radius_m"),
            )?;
        }
        if timed {
            check(timestamp(s(alarm, "absolute_date")))?;
        }
    }
    Ok(())
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
fn messages(c: &Command, a: &Args) -> Result<(), Violation> {
    let fields: &[&str] = match c.op.as_str() {
        "messages.send" => &[
            "chat_id",
            "handles",
            "service",
            "text",
            "files",
            "asset_ids",
            "resend_of",
        ],
        "messages.reply" => &["id", "part_index", "text", "files"],
        "messages.react" => &["id", "part_index", "reaction", "emoji", "remove"],
        "messages.edit" => &["id", "part_index", "text"],
        "messages.unsend" => &["id", "part_index"],
        _ => &["id"],
    };
    allowed(c, fields, true)?;
    require(
        n(a, "part_index") >= 0.0,
        "args.part_index",
        "a nonnegative index",
    )?;
    require(
        s(a, "text").len() <= 65536,
        "args.text",
        "at most 65536 bytes of text",
    )?;
    let files = arr(a, "files");
    message_files(files)?;

    if c.op == "messages.send" {
        send_message(c, a, files)?;
    } else {
        require(!s(a, "id").is_empty(), "args.id", "a message ID")?;
        if matches!(c.op.as_str(), "messages.edit" | "messages.unsend")
            || c.op.starts_with("messages.delete_")
        {
            require(
                c.expected_version != 0,
                "expected_version",
                "the current version of the message",
            )?;
        }
    }
    if matches!(c.op.as_str(), "messages.edit" | "messages.reply") {
        require(
            !s(a, "text").is_empty(),
            "args.text",
            "nonempty message text",
        )?;
    }
    if c.op == "messages.react" {
        require(
            ["love", "like", "dislike", "laugh", "emphasize", "question"]
                .contains(&s(a, "reaction"))
                || s(a, "reaction") == "emoji" && !s(a, "emoji").is_empty(),
            "args.reaction",
            "a supported reaction or an emoji with its value",
        )?;
    }
    Ok(())
}
fn send_message(c: &Command, a: &Args, files: &[Value]) -> Result<(), Violation> {
    if p(a, "asset_ids")
        && (arr(a, "asset_ids").is_empty()
            || arr(a, "asset_ids").len() != arr(a, "files").len()
            || arr(a, "asset_ids").iter().any(|id| {
                id.as_str()
                    .is_none_or(|id| id.trim().is_empty() || id.len() > 256)
            }))
    {
        return Err(Violation::invalid(
            "args.asset_ids",
            "invalid photo attachment binding",
            "one nonempty asset ID for each staged photo",
        ));
    }
    let chat = !s(a, "chat_id").is_empty();
    require(
        chat == arr(a, "handles").is_empty(),
        "args.chat_id",
        "exactly one of chat_id or handles",
    )?;
    require(
        s(a, "resend_of").is_empty() || chat,
        "args.resend_of",
        "chat_id when resending",
    )?;
    require(
        !chat || s(a, "service").is_empty(),
        "args.service",
        "omit when chat_id is given",
    )?;
    require(
        !s(a, "text").is_empty() || !files.is_empty(),
        "args.text",
        "message text, files, or both",
    )?;
    require(
        c.expected_version == 0,
        "expected_version",
        "omit for a new message",
    )?;
    if !chat {
        require(
            arr(a, "handles").len() == 1,
            "args.handles",
            "one recipient",
        )?;
        require(
            ["iMessage", "SMS", "RCS"].contains(&s(a, "service")),
            "args.service",
            "iMessage, SMS, or RCS",
        )?;
    }
    Ok(())
}
fn message_files(files: &[Value]) -> Result<(), Violation> {
    require(files.len() <= 10, "args.files", "at most 10 files")?;
    for (index, file) in files.iter().enumerate() {
        let file = file.as_object().ok_or(Error::Invalid)?;
        let data = base64(s(file, "data")).map_err(|_| {
            Violation::invalid(
                format!("args.files[{index}].data"),
                "invalid Base64",
                "Base64-encoded file bytes",
            )
        })?;
        if data.len() > 100 << 20 {
            return Err(Violation {
                code: "too_large",
                ..Violation::invalid(
                    format!("args.files[{index}].data"),
                    "file exceeds upload limit",
                    "at most 100 MiB",
                )
            });
        }
        require(
            s(file, "data").is_empty() != s(file, "upload_id").is_empty(),
            format!("args.files[{index}]"),
            "exactly one of data or upload_id",
        )?;
        let name = s(file, "name");
        require(
            !name.is_empty() && ![".", ".."].contains(&name) && !name.contains('/'),
            format!("args.files[{index}].name"),
            "a filename without path components",
        )?;
    }
    Ok(())
}
fn calendar(c: &Command, a: &Args) -> Result<(), Violation> {
    if s(a, "span") == "all" {
        return Err(Error::Unsupported.into());
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
        _ => return Err(Error::Unsupported.into()),
    })?;
    calendar_times(c, a)?;
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
    check(!p(a, "availability") || (-1.0..=3.0).contains(&n(a, "availability")))?;
    Ok(())
}
fn calendar_times(c: &Command, a: &Args) -> Result<(), Violation> {
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
                return Err(Violation::invalid(
                    format!("args.{k}"),
                    "invalid event date or time",
                    "an ISO date for an all-day event or a timestamp with offset",
                ));
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
        if !ordered {
            return Err(Violation::invalid(
                "args.end",
                "event ends before or at its start",
                "an end after start",
            ));
        }
    }
    Ok(())
}
fn mail(c: &Command, a: &Args) -> Result<(), Error> {
    match c.op.as_str() {
        "mail.create_draft" | "mail.send" => {
            check(!arr(a, "to").is_empty() && !s(a, "subject").is_empty())?;
            for address in arr(a, "to").iter().chain(arr(a, "cc")) {
                check(
                    address
                        .as_str()
                        .is_some_and(ab_wire::native::mail_recipient),
                )?;
            }
            if !arr(a, "attachments").is_empty()
                || !s(a, "reply_to").is_empty()
                || !s(a, "forward_of").is_empty()
            {
                return Err(Error::Unsupported);
            }
            Ok(())
        }
        "mail.move" => check(
            !s(a, "id").is_empty() && !s(a, "mailbox").is_empty() && !s(a, "account").is_empty(),
        ),
        "mail.set_flag" => check(!s(a, "id").is_empty() && p(a, "flagged")),
        "mail.mark_read" => check(!s(a, "id").is_empty() && p(a, "read")),
        "mail.delete" => check(!s(a, "id").is_empty()),
        _ => Err(Error::Unsupported),
    }
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
fn capture(op: &str, a: &Args) -> Result<(), Violation> {
    if op == "print.document" {
        document_input(a)?;
    } else {
        check(!p(a, "blob") && s(a, "data_base64").is_empty())?;
    }
    require(
        match op {
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
                    && ["file", "printer"].iter().all(|k| s(a, k).is_empty())
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
                !s(a, "printer").trim().is_empty()
                    && !p(a, "display")
                    && !p(a, "window")
                    && n(a, "seconds") == 0.0
            }
            _ => false,
        },
        "args",
        "valid capture options for the operation",
    )
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[allow(clippy::too_many_lines)]
fn contacts(c: &Command) -> Result<Args, Violation> {
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
        _ => return Err(Error::Unsupported.into()),
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod mail_tests {
    use crate::{ActionContext, Command, Error, native_action};
    use serde_json::{Value, json};

    fn command(op: &str, args: &Value) -> Command {
        Command {
            id: "cmd".into(),
            op: op.into(),
            args: serde_json::value::RawValue::from_string(args.to_string()).unwrap(),
            expected_version: 0,
            wait_ms: 0,
        }
    }

    fn checked(op: &str, args: &Value) -> Result<(), Error> {
        crate::validate(&command(op, args), jiff::Timestamp::now()).map(|_| ())
    }

    #[test]
    fn a_plain_draft_passes_and_reaches_the_native_action() {
        let args =
            json!({"to": ["a@example.com"], "cc": ["b@example.com"], "subject": "s", "body": "b"});
        checked("mail.create_draft", &args).unwrap();
        checked("mail.send", &args).unwrap();
        let context = ActionContext {
            origin: String::new(),
            ..ActionContext::default()
        };
        let action: Value = serde_json::from_slice(
            &native_action(&command("mail.create_draft", &args), &context).unwrap(),
        )
        .unwrap();
        assert_eq!(action["op"], "mail.create_draft");
        assert_eq!(action["args"], args);
    }

    #[test]
    fn who_mail_can_reach_is_refused_before_any_effect() {
        for op in ["mail.create_draft", "mail.send"] {
            for args in [
                json!({"subject": "s"}),
                json!({"to": [], "subject": "s"}),
                json!({"to": ["a@example.com"]}),
                json!({"to": ["a@example.com"], "subject": ""}),
                json!({"to": ["plain"], "subject": "s"}),
                json!({"to": ["a@example.com, b@example.com"], "subject": "s"}),
                json!({"to": ["a@example.com\nbcc: b@example.com"], "subject": "s"}),
                json!({"to": ["Name <a@example.com>"], "subject": "s"}),
                json!({"to": ["a@example.com"], "cc": ["nope"], "subject": "s"}),
                json!({"to": ["a@example.com"], "bcc": ["b@example.com"], "subject": "s"}),
            ] {
                assert_eq!(checked(op, &args), Err(Error::Invalid), "{op} {args}");
            }
            for args in [
                json!({"to": ["a@example.com"], "subject": "s", "attachments": ["x"]}),
                json!({"to": ["a@example.com"], "subject": "s", "reply_to": "1"}),
                json!({"to": ["a@example.com"], "subject": "s", "forward_of": "1"}),
            ] {
                assert_eq!(checked(op, &args), Err(Error::Unsupported), "{op} {args}");
            }
        }
    }

    #[test]
    fn message_commands_name_the_message_and_the_change() {
        checked("mail.delete", &json!({"id": "m"})).unwrap();
        checked("mail.set_flag", &json!({"id": "m", "flagged": false})).unwrap();
        checked("mail.mark_read", &json!({"id": "m", "read": true})).unwrap();
        checked(
            "mail.move",
            &json!({"id": "m", "mailbox": "A", "account": "B"}),
        )
        .unwrap();
        for (op, args) in [
            ("mail.delete", json!({})),
            ("mail.set_flag", json!({"id": "m"})),
            ("mail.mark_read", json!({"id": "m"})),
            ("mail.move", json!({"id": "m", "mailbox": "A"})),
        ] {
            assert_eq!(checked(op, &args), Err(Error::Invalid), "{op}");
        }
    }
}
