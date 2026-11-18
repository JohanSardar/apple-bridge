use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    process::{Command, Output},
};

type Check<T = ()> = Result<T, Box<dyn std::error::Error>>;

const DEFAULT_TOKENS: usize = 2_000;
const MAX_TOKENS: usize = 8_000;
const NOTES: usize = 400;
const TEST_CALENDARS: usize = 22;
const TODAY_EVENTS: usize = 30;
const OVERDUE: usize = 60;
const DUE_TODAY: usize = 4;
const UPCOMING: usize = 12;
const OLD_SPAM: usize = 300;
const NEW_UNKNOWN: usize = 30;
const MAIL: usize = 300;

fn bridge(home: &Path, arguments: &[&str]) -> Check<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_bridge"))
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .args(arguments)
        .arg("--home")
        .arg(home)
        .output()?)
}

struct Running<'a>(&'a Path);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        let _ = bridge(self.0, &["down"]);
    }
}

fn tokens(value: &Value) -> usize {
    (value.to_string().len() * 10).div_ceil(35)
}

fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref": {"collection": collection, "native_key": key}})
}

struct World {
    zone: jiff::tz::TimeZone,
    today: jiff::civil::Date,
    now: jiff::Timestamp,
    records: Vec<Value>,
}

impl World {
    fn put(&mut self, collection: &str, key: &str, data: Value) {
        let mut record = json!({
            "kind": "upsert",
            "collection": collection,
            "native_key": key,
            "observed_at": self.now.to_string(),
        });
        record["data"] = data;
        self.records.push(record);
    }
    fn day(&self, offset: i64) -> jiff::civil::Date {
        self.today
            .checked_add(jiff::Span::new().days(offset))
            .unwrap_or(self.today)
    }
    fn at(&self, offset: i64, hour: i8) -> String {
        self.day(offset)
            .at(hour, 0, 0, 0)
            .to_zoned(self.zone.clone())
            .map(|zoned| zoned.timestamp().to_string())
            .unwrap_or_default()
    }
    fn ago(&self, days: i64) -> String {
        self.now
            .checked_sub(jiff::SignedDuration::from_hours(days * 24))
            .unwrap_or(self.now)
            .to_string()
    }
}

fn calendars(world: &mut World) {
    world.put(
        "calendar.calendar",
        "agent-family",
        json!({"title": "Family", "account": "synthetic", "type": "caldav", "writable": true}),
    );
    for index in 0..TEST_CALENDARS {
        world.put(
            "calendar.calendar",
            &format!("agent-test-{index}"),
            json!({"title": format!("Bridge Test [ab-test {index}]"), "account": "synthetic", "type": "caldav", "writable": true}),
        );
    }
    let today = world.today.to_string();
    world.put(
        "calendar.event",
        "agent-birthday",
        json!({"calendar_id": reference("calendar.calendar", "agent-family"), "title": "Synthetic Birthday", "start": today, "end": today, "all_day": true, "time_zone": null, "status": 1, "recurrence": [], "alarms": [], "attendees": []}),
    );
    world.put(
        "calendar.event",
        "agent-trip",
        json!({"calendar_id": reference("calendar.calendar", "agent-family"), "title": "Synthetic Trip", "start": world.day(-1).to_string(), "end": world.day(1).to_string(), "all_day": true, "time_zone": null, "status": 1, "recurrence": [], "alarms": [], "attendees": []}),
    );
    for index in 0..TODAY_EVENTS {
        let calendar = format!("agent-test-{}", index % TEST_CALENDARS);
        let hour = i8::try_from(7 + index % 14).unwrap_or(7);
        world.put(
            "calendar.event",
            &format!("agent-event-{index}"),
            json!({"calendar_id": reference("calendar.calendar", &calendar), "title": format!("Synthetic meeting {index}"), "start": world.at(0, hour), "end": world.at(0, hour + 1), "all_day": false, "time_zone": "UTC", "status": 1, "location": "Room ".repeat(40), "notes": "Agenda item. ".repeat(200), "recurrence": [], "alarms": [{"relative_offset": -600}], "attendees": (0..12).map(|a| json!({"name": format!("Guest {a}"), "url": format!("mailto:guest{a}@example.com"), "status": 1, "role": 1, "type": 1})).collect::<Vec<_>>()}),
        );
    }
}

fn reminders(world: &mut World) {
    world.put(
        "reminders.list",
        "agent-errands",
        json!({"title": "Synthetic Errands", "sections": [], "is_group": false}),
    );
    let add = |world: &mut World, key: String, title: String, due: String| {
        world.put(
            "reminders.reminder",
            &key,
            json!({"title": title, "list_id": reference("reminders.list", "agent-errands"), "due": due, "due_all_day": true, "completed": false, "priority": 0, "flagged": false, "notes": "Long note. ".repeat(50), "tags": []}),
        );
    };
    for index in 0..OVERDUE {
        let due = world
            .day(-10 - i64::try_from(index).unwrap_or(0))
            .to_string();
        add(
            world,
            format!("agent-overdue-{index}"),
            format!("Overdue errand {index}"),
            due,
        );
    }
    for index in 0..DUE_TODAY {
        let due = world.today.to_string();
        add(
            world,
            format!("agent-today-{index}"),
            format!("Today errand {index}"),
            due,
        );
    }
    for index in 0..UPCOMING {
        let due = world
            .day(1 + i64::try_from(index % 6).unwrap_or(0))
            .to_string();
        add(
            world,
            format!("agent-soon-{index}"),
            format!("Upcoming errand {index}"),
            due,
        );
    }
}

fn notes(world: &mut World) {
    world.put(
        "notes.folder",
        "agent-notes",
        json!({"title": "Notes", "account": "synthetic", "account_name": "iCloud", "parent_id": "", "read_only": false, "recently_deleted": false}),
    );
    for index in 0..NOTES {
        let body = format!("Synthetic note {index}. ").repeat(400);
        world.put(
            "notes.note",
            &format!("agent-note-{index}"),
            json!({"title": format!("Synthetic note {index}"), "plain_text": body, "html": format!("<p>{body}</p>"), "blocks": (0..40).map(|b| json!({"type": "paragraph", "text": format!("block {b} of note {index}")})).collect::<Vec<_>>(), "folder_id": reference("notes.folder", "agent-notes"), "modified_at": world.ago(i64::try_from(index).unwrap_or(0)), "created_at": world.ago(400), "locked": false, "recently_deleted": false, "read_only": false, "editable": "plain", "pinned": false, "tags": []}),
        );
    }
}

fn messages(world: &mut World) {
    let chat = |world: &mut World, key: &str, filter: &str, spam: bool, handle: &str| {
        world.put(
            "messages.chat",
            key,
            json!({"display_name": "", "service": "SMS", "style": "direct", "participants": [reference("messages.handle", handle)], "is_archived": false, "capabilities": [], "filter": filter, "is_spam": spam, "read_receipts": "default", "muted_until": null, "pinned_index": null}),
        );
    };
    let message = |world: &mut World, key: &str, chat: &str, handle: &str, sent: String| {
        world.put(
            "messages.message",
            key,
            json!({"chat_id": reference("messages.chat", chat), "is_from_me": false, "sent_at": sent, "text": "You won a prize, reply now to claim it.", "parts": [], "kind": "message", "decode_status": "decoded", "error_code": 0, "edit_history": [], "retracted_parts": [], "sender_handle_id": reference("messages.handle", handle)}),
        );
    };
    for index in 0..OLD_SPAM + NEW_UNKNOWN {
        let handle = format!("agent-stranger-{index}");
        world.put(
            "messages.handle",
            &handle,
            json!({"address": format!("+1 900 555 {index:04}"), "service": "SMS"}),
        );
        let key = format!("agent-stranger-chat-{index}");
        let old = index < OLD_SPAM;
        chat(world, &key, "unknown", old && index % 3 == 0, &handle);
        let sent = if old {
            world.ago(400 + i64::try_from(index).unwrap_or(0))
        } else {
            world.ago(1)
        };
        message(
            world,
            &format!("agent-stranger-message-{index}"),
            &key,
            &handle,
            sent,
        );
    }
    for index in 0..5_usize {
        let handle = lab_demo::handle_key(index + 1);
        let key = format!("agent-friend-chat-{index}");
        chat(world, &key, "known", false, &handle);
        message(
            world,
            &format!("agent-friend-message-{index}"),
            &key,
            &handle,
            world.ago(0),
        );
    }
}

fn mail(world: &mut World) {
    world.put(
        "mail.account",
        "agent-mail",
        json!({"name": "synthetic@example.com", "kind": "imap", "addresses": ["synthetic@example.com"]}),
    );
    world.put(
        "mail.mailbox",
        "mailbox/agent-inbox",
        json!({"account_id": "agent-mail", "name": "INBOX", "path": "INBOX", "role": "inbox", "unread_count": 10, "message_count": MAIL}),
    );
    for index in 0..MAIL {
        world.put(
            "mail.message",
            &format!("agent-mail-{index}"),
            json!({"subject": format!("Synthetic newsletter {index}"), "from": "news@example.com", "received_at": world.ago(i64::try_from(index).unwrap_or(0)), "read": index > 10, "message_id": format!("<{index}@example.com>"), "mailbox_ids": ["mailbox/agent-inbox"], "account_id": "agent-mail", "to": null, "cc": null, "sent_at": null, "flagged": false, "flag_color": null, "has_attachments": null, "size": null, "snippet": "Read this. ".repeat(100), "body_state": null}),
        );
    }
}

fn seed(home: &Path) -> Check<World> {
    let zone = jiff::tz::TimeZone::system();
    let now = jiff::Timestamp::now();
    let mut world = World {
        today: now.to_zoned(zone.clone()).date(),
        zone,
        now,
        records: lab_demo::generate(lab_demo::Options {
            seed: 7,
            huge: false,
        })
        .records,
    };
    calendars(&mut world);
    reminders(&mut world);
    notes(&mut world);
    messages(&mut world);
    mail(&mut world);
    let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for mut record in std::mem::take(&mut world.records) {
        record["revalidated"] = json!(true);
        let source = record["collection"]
            .as_str()
            .and_then(|c| c.split('.').next())
            .unwrap_or_default()
            .to_owned();
        grouped.entry(source).or_default().push(record);
    }
    let mut state = ab_state::State::<ab_state::Gateway>::open(home)?;
    let mut number = 0_u64;
    for (source, records) in &grouped {
        let mut predecessor = String::new();
        for chunk in records.chunks(100_000) {
            number += 1;
            let checkpoint = number.to_string();
            let mut scope: Vec<&str> = chunk
                .iter()
                .filter_map(|r| r["collection"].as_str())
                .collect();
            scope.sort_unstable();
            scope.dedup();
            let capture: ab_wire::messages::Capture = serde_json::from_value(json!({
                "source": source,
                "generation": "agent-answers",
                "predecessor": predecessor,
                "checkpoint": checkpoint,
                "observations": chunk,
                "scan_id": checkpoint,
                "scope": scope,
                "scan_started_at": world.now.to_string(),
                "completion_evidence": "complete",
            }))?;
            state.commit_capture(&capture)?;
            predecessor = checkpoint;
        }
    }
    Ok(world)
}

struct Agent {
    client: reqwest::Client,
    address: String,
    token: String,
    id: u64,
}

impl Agent {
    async fn call(&mut self, tool: &str, arguments: &Value) -> Check<Value> {
        self.id += 1;
        let body = json!({"jsonrpc": "2.0", "id": self.id, "method": "tools/call", "params": {"name": tool, "arguments": arguments}});
        let response = self
            .client
            .post(format!("http://{}/mcp", self.address))
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-03-26")
            .body(body.to_string())
            .send()
            .await?;
        let response: Value = serde_json::from_slice(&response.bytes().await?)?;
        assert_ne!(
            response["result"]["isError"], true,
            "{tool} {arguments}: {response}"
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .ok_or("text")?;
        Ok(serde_json::from_str(text)?)
    }
}

fn page_is_declared(page: &Value, path: &str, problems: &mut Vec<String>) {
    for key in ["total", "returned", "truncated", "narrow"] {
        if page.get(key).is_none() {
            problems.push(format!("{path} has no {key}"));
        }
    }
    if page["truncated"] == true && page.get("next").is_none() && page.get("gap").is_none() {
        problems.push(format!("{path} is truncated without next"));
    }
    if let Some(arguments) = page["next"]["arguments"].as_object()
        && arguments
            .values()
            .any(|cursor| cursor.as_str().is_some_and(|c| c.contains(' ')))
    {
        problems.push(format!("{path}.next carries a sentence as its cursor"));
    }
    if let (Some(total), Some(returned)) = (page["total"].as_u64(), page["returned"].as_u64())
        && returned < total
        && page["truncated"] != true
        && page.get("offset").is_none()
    {
        problems.push(format!(
            "{path} returned {returned} of {total} without truncated"
        ));
    }
}

fn pages_are_declared(value: &Value, path: &str, problems: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            let pages = fields
                .get("page")
                .into_iter()
                .map(|page| (format!("{path}.page"), page))
                .chain(
                    fields
                        .get("pages")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                        .map(|(key, page)| (format!("{path}.pages.{key}"), page)),
                );
            for (at, page) in pages {
                page_is_declared(page, &at, problems);
            }
            if let Some(cut) = fields.get("cut")
                && cut.get("reason").is_none()
            {
                problems.push(format!("{path}.cut has no reason"));
            }
            for (key, child) in fields {
                pages_are_declared(child, &format!("{path}.{key}"), problems);
            }
        }
        Value::Array(items) => {
            if items.len() > 100 {
                problems.push(format!(
                    "{path} has {} items, over the cap of 100",
                    items.len()
                ));
            }
            for item in items {
                pages_are_declared(item, path, problems);
            }
        }
        _ => {}
    }
}

fn titles(items: &Value) -> Vec<String> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["title"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn default_answers_stay_small_and_right_on_a_large_synthetic_mac() -> Check {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    let world = seed(&home)?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    drop(listener);
    let started = bridge(
        &home,
        &[
            "up",
            "--api-addr",
            &address,
            "--peer-addr",
            "127.0.0.1:0",
            "--json",
        ],
    )?;
    let _running = Running(&home);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let token = bridge(
        &home,
        &[
            "token",
            "create",
            "--scope",
            "messages:read,calendar:read,reminders:read,notes:read,contacts:read,mail:read",
        ],
    )?;
    assert!(
        token.status.success(),
        "{}",
        String::from_utf8_lossy(&token.stderr)
    );
    let mut agent = Agent {
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(60))
            .build()?,
        address,
        token: String::from_utf8(token.stdout)?.trim().to_owned(),
        id: 1,
    };
    let initialized = agent
        .client
        .post(format!("http://{}/mcp", agent.address))
        .bearer_auth(&agent.token)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-03-26")
        .body(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"agent-answers","version":"1"}}}).to_string())
        .send()
        .await?;
    assert!(initialized.status().is_success());
    let today = world.today.to_string();
    let defaults = [
        ("apple_status", json!({})),
        ("calendar_events", json!({})),
        ("calendar_events", json!({"date": today})),
        ("messages_read", json!({})),
        ("messages_read", json!({"unread": true})),
        ("messages_read", json!({"text": "prize"})),
        ("contacts_find", json!({"query": "Demo"})),
        ("apple_search", json!({"text": "synthetic"})),
        ("apple_read", json!({"operation": "notes.list"})),
        ("apple_read", json!({"operation": "reminders.lists"})),
        (
            "apple_read",
            json!({"operation": "reminders.items", "arguments": {"smart": "today"}}),
        ),
        (
            "apple_read",
            json!({"operation": "reminders.items", "arguments": {"list": "Synthetic Errands"}}),
        ),
        ("apple_read", json!({"operation": "messages.chats"})),
        (
            "apple_read",
            json!({"operation": "records.query", "arguments": {"collection": "mail.message"}}),
        ),
        (
            "apple_read",
            json!({"operation": "search", "arguments": {"text": "synthetic", "collection": "notes.note"}}),
        ),
    ];
    let mut answers = Vec::new();
    let mut problems = Vec::new();
    for (tool, arguments) in &defaults {
        let answer = agent.call(tool, arguments).await?;
        let size = tokens(&answer);
        eprintln!("default {tool} {arguments}: about {size} tokens");
        if size > DEFAULT_TOKENS {
            problems.push(format!("{tool} {arguments} is about {size} tokens"));
        }
        pages_are_declared(&answer, &format!("{tool} {arguments}"), &mut problems);
        answers.push(answer);
    }
    let everything = [
        ("messages_read", json!({"limit": 100, "senders": "all"})),
        (
            "calendar_events",
            json!({"start": world.day(-7).to_string(), "end": world.day(7).to_string(), "detail": true}),
        ),
        (
            "apple_read",
            json!({"operation": "notes.list", "verbose": true}),
        ),
        (
            "apple_read",
            json!({"operation": "records.query", "arguments": {"collection": "notes.note", "limit": 1000}}),
        ),
        (
            "apple_read",
            json!({"operation": "records.get", "arguments": {"collection": "notes.note", "id": answers[8]["notes"][0]["id"]}}),
        ),
        (
            "apple_read",
            json!({"operation": "reminders.items", "arguments": {"smart": "today"}, "view": "overdue", "offset": 20}),
        ),
    ];
    for (tool, arguments) in &everything {
        let answer = agent.call(tool, arguments).await?;
        let size = tokens(&answer);
        eprintln!("asked for more {tool} {arguments}: about {size} tokens");
        if size > MAX_TOKENS {
            problems.push(format!("{tool} {arguments} is about {size} tokens"));
        }
        pages_are_declared(&answer, &format!("{tool} {arguments}"), &mut problems);
        answers.push(answer);
    }
    assert!(problems.is_empty(), "{problems:#?}");

    let events = titles(&answers[1]["events"]);
    assert!(
        events.contains(&"Synthetic Birthday".to_owned()),
        "today misses the all-day event: {events:?}"
    );
    assert!(
        events.contains(&"Synthetic Trip".to_owned()),
        "today misses the multi-day event: {events:?}"
    );
    assert!(
        answers[1]["page"]["total"]
            .as_u64()
            .is_some_and(|t| t >= u64::try_from(TODAY_EVENTS + 2).unwrap_or(u64::MAX)),
        "{}",
        answers[1]["page"]
    );
    for event in answers[1]["events"].as_array().ok_or("events")? {
        let first = event["start_local"]
            .as_str()
            .or(event["start"].as_str())
            .unwrap_or_default();
        let last = event["end_local"]
            .as_str()
            .or(event["end"].as_str())
            .unwrap_or_default();
        assert!(
            first[..10] <= *today
                && (last[..10] > *today
                    || (last[..10] == *today
                        && (event["all_day"] == false || first[..10] == *today))),
            "{event} is not on {today}"
        );
    }
    assert_eq!(answers[1]["page"]["truncated"], true);
    assert!(answers[1]["events"][0].get("attendees").is_none());
    assert!(titles(&answers[2]["events"]).contains(&"Synthetic Birthday".to_owned()));

    let chats = answers[3]["chats"].as_array().ok_or("chats")?;
    assert!(
        chats
            .iter()
            .all(|c| !c["title"].as_str().unwrap_or_default().contains("900"))
    );
    assert!(
        answers[3]["omitted"]["unknown_senders"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert!(answers[3]["omitted"]["spam"].as_u64().unwrap_or_default() > 0);
    let unread = &answers[4];
    let month_ago = world.ago(31);
    for chat in unread["chats"].as_array().ok_or("unread chats")? {
        assert!(
            chat["last_activity_at"].as_str().unwrap_or_default() > month_ago.as_str(),
            "{chat}"
        );
    }
    assert!(
        unread["chats"].as_array().is_some_and(|c| c.len() >= 5),
        "{unread}"
    );
    assert!(
        unread["omitted"]["unknown_senders"]
            .as_u64()
            .unwrap_or_default()
            >= u64::try_from(NEW_UNKNOWN)?
    );
    assert_eq!(
        unread["omitted"]["show_them"]["arguments"]["senders"],
        "all"
    );

    let people = answers[6]["contacts"].as_array().ok_or("contacts")?;
    assert_eq!(people.len(), 10, "{}", answers[6]);
    assert!(answers[6].get("cut").is_none(), "{}", answers[6]);
    assert!(
        people
            .iter()
            .all(|p| p["name"].is_string() && p["phones"].is_array() && p.get("data").is_none())
    );

    let notes = &answers[8];
    assert_eq!(notes["page"]["total"], NOTES);
    let returned = notes["page"]["returned"].as_u64().unwrap_or_default();
    assert!((15..=20).contains(&returned), "{}", notes["page"]);
    assert!(
        returned == 20 || notes["cut"]["dropped_items"]["notes"].as_u64() == Some(20 - returned),
        "{notes}"
    );
    assert_eq!(notes["page"]["next"]["offset"], returned);
    for note in notes["notes"].as_array().ok_or("notes")? {
        for heavy in ["plain_text", "html", "blocks"] {
            assert!(note.get(heavy).is_none(), "{note}");
        }
        assert!(
            note["preview"]
                .as_str()
                .is_some_and(|p| p.chars().count() <= 101)
        );
    }

    let today_reminders = &answers[10];
    assert_eq!(today_reminders["due_today"]["page"]["total"], DUE_TODAY);
    let overdue = today_reminders["overdue"]["count"]
        .as_u64()
        .ok_or("overdue count")?;
    assert!(overdue >= u64::try_from(OVERDUE)?, "{today_reminders}");
    assert!(
        today_reminders["overdue"]["top"]
            .as_array()
            .is_some_and(|t| t.len() <= 3)
    );
    assert!(
        today_reminders["upcoming"]["count"]
            .as_u64()
            .unwrap_or_default()
            >= u64::try_from(UPCOMING)?
    );
    assert_eq!(answers[20]["page"]["total"], overdue);
    assert_eq!(
        today_reminders["overdue"]["show_all"]["arguments"]["view"],
        "overdue"
    );
    assert_eq!(answers[20]["page"]["offset"], 20);

    let body = &answers[19]["records"][0]["data"];
    assert!(
        body["plain_text"].as_str().is_some_and(|t| t.len() > 1000),
        "one note returns its body"
    );

    let stats = bridge(&home, &["stats", "--answers", "--json"])?;
    assert!(
        stats.status.success(),
        "{}",
        String::from_utf8_lossy(&stats.stderr)
    );
    let stats: Value = serde_json::from_slice(&stats.stdout)?;
    let recorded = stats["answers"].as_array().ok_or("answers")?;
    for tool in [
        "calendar_events",
        "messages_read",
        "apple_read",
        "apple_search",
        "contacts_find",
    ] {
        assert!(
            recorded.iter().any(|a| a["tool"] == tool),
            "{tool} sizes not recorded: {stats}"
        );
    }
    let largest = recorded[0]["largest_bytes"].as_u64().unwrap_or(u64::MAX);
    assert!(largest <= u64::try_from(MAX_TOKENS * 35 / 10)?, "{stats}");
    Ok(())
}
