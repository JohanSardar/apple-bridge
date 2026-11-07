use super::{
    Failure, Outcome, gateway,
    tree::{CallArgs, ReadArgs, ReadKind, SearchArgs},
};
use ab_catalog::OperationKind;
use ab_trust::{Authority, Grant};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

const READ_TIMEOUT: Duration = Duration::from_secs(60);
const HEADLINE_FIELDS: &[&str] = &[
    "title",
    "display_name",
    "subject",
    "name",
    "text",
    "snippet",
    "plain_text",
    "filename",
];
const WHEN_FIELDS: &[&str] = &[
    "start",
    "sent_at",
    "received_at",
    "due",
    "modified_at",
    "date",
];

struct Session {
    authority: Option<Authority>,
    client: reqwest::Client,
    address: String,
    token: String,
    principal: String,
}

impl Session {
    async fn open(home: &Path) -> Outcome<Self> {
        let address = gateway::running_address(home).await?;
        let authority = Authority::open(home).map_err(crate::Error::from)?;
        let mut grants = Grant::agent_scopes()
            .into_iter()
            .filter(|scope| scope.ends_with(":read"))
            .map(|scope| Grant::parse(&scope))
            .collect::<Result<Vec<_>, _>>()?;
        grants.push(Grant::parse("system:read")?);
        let token = authority.create_token(grants, false).map_err(|_| {
            Failure::new("This home cannot read on its own.")
                .next("Replicas read through the access they were paired with.")
        })?;
        let principal = authority.authenticate_token(&token)?.id;
        let client = reqwest::Client::builder()
            .timeout(READ_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|error| Failure::new(format!("Could not start a request: {error}.")))?;
        Ok(Self {
            authority: Some(authority),
            client,
            address,
            token,
            principal,
        })
    }

    async fn with_token(home: &Path, token: String) -> Outcome<Self> {
        let address = gateway::running_address(home).await?;
        let client = reqwest::Client::builder()
            .timeout(READ_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|error| Failure::new(format!("Could not start a request: {error}.")))?;
        Ok(Self {
            authority: None,
            client,
            address,
            token,
            principal: String::new(),
        })
    }

    async fn call(&self, operation: &str, arguments: &Value) -> Outcome<Value> {
        let response = self
            .client
            .post(format!("http://{}/v1/op/{operation}", self.address))
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .body(arguments.to_string())
            .send()
            .await
            .map_err(|error| {
                Failure::new(format!("The gateway did not answer: {error}."))
                    .next("Run `bridge up`, then try again.")
            })?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| Failure::new(format!("The gateway's answer broke off: {error}.")))?;
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Failure::new(format!("The gateway answered {status} without JSON.")))?;
        if status.is_success() {
            return Ok(body);
        }
        Err(problem(&body, operation))
    }

    fn close(self) {
        if let Some(authority) = self.authority {
            let _ = authority.revoke(&self.principal);
        }
    }
}

fn problem(body: &Value, operation: &str) -> Failure {
    let code = body["error"]["code"].as_str().unwrap_or("unknown");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or("request failed");
    let failure = Failure::new(format!("{operation} failed: {message} ({code})."));
    match code {
        "unavailable" => failure.next("Check the source with `bridge stats`."),
        "invalid" => failure.next(
            "Check its arguments in `bridge skills get core`, or list operations with `bridge call --list`.",
        ),
        _ => failure,
    }
}

fn time_zone() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .map_or_else(|| "UTC".to_owned(), str::to_owned)
}

fn request(args: &ReadArgs) -> Outcome<(&'static str, Value, &'static [&'static str])> {
    if (args.text.is_some() || args.chat.is_some()) && args.kind != ReadKind::Messages {
        return Err(Failure::new(
            "--text and --chat only work with `bridge read messages`.",
        ));
    }
    let limit = args.limit.min(200);
    Ok(match args.kind {
        ReadKind::Photos => ("photos.search", json!({"args":{"limit":limit.max(1)}}), &[]),
        ReadKind::Messages => {
            if let Some(chat) = &args.chat {
                (
                    "messages.thread",
                    json!({"chat_id": chat, "limit": limit}),
                    &["messages.message"],
                )
            } else if let Some(text) = &args.text {
                (
                    "messages.search",
                    json!({"text": text, "limit": limit}),
                    &["messages.message"],
                )
            } else {
                (
                    "messages.chats",
                    json!({"limit": limit}),
                    &["messages.chat"],
                )
            }
        }
        ReadKind::Chats => (
            "messages.chats",
            json!({"limit": limit}),
            &["messages.chat"],
        ),
        ReadKind::Contacts => ("contacts.list", json!({}), &["contacts.contact"]),
        ReadKind::Calendar => {
            let now = jiff::Timestamp::now();
            let end = now
                .checked_add(jiff::Span::new().hours(i64::from(args.days) * 24))
                .map_err(|_| Failure::new("--days is too large."))?;
            (
                "calendar.agenda",
                json!({"start": now.to_string(), "end": end.to_string()}),
                &["calendar.event"],
            )
        }
        ReadKind::Reminders => (
            "reminders.items",
            json!({"time_zone": time_zone(), "smart": "all"}),
            &["reminders.reminder"],
        ),
        ReadKind::Notes => ("notes.list", json!({}), &["notes.note"]),
        ReadKind::Mail => (
            "records.query",
            json!({"collection": "mail.message", "limit": limit}),
            &["mail.message"],
        ),
        ReadKind::Voicemail => (
            "records.query",
            json!({"collection": "voicemail.message", "limit": limit}),
            &["voicemail.message"],
        ),
    })
}

fn collect<'a>(value: &'a Value, wanted: &[&str], out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(fields) => {
            if fields
                .get("collection")
                .and_then(Value::as_str)
                .is_some_and(|collection| wanted.is_empty() || wanted.contains(&collection))
                && fields.contains_key("data")
            {
                out.push(value);
            }
            for child in fields.values() {
                collect(child, wanted, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect(child, wanted, out);
            }
        }
        _ => {}
    }
}

fn text_field<'a>(data: &'a Value, fields: &[&str]) -> Option<&'a str> {
    fields
        .iter()
        .find_map(|field| data[*field].as_str().filter(|text| !text.trim().is_empty()))
}

fn contact_name(data: &Value) -> Option<String> {
    let parts: Vec<&str> = ["given_name", "family_name", "organization"]
        .iter()
        .filter_map(|field| data[*field].as_str().filter(|text| !text.is_empty()))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn one_line(text: &str, limit: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= limit {
        return joined;
    }
    let mut short: String = joined.chars().take(limit).collect();
    short.push('…');
    short
}

fn render_chats(chats: &[Value], limit: usize) {
    for entry in chats.iter().take(limit) {
        let chat = &entry["chat"];
        let name = text_field(&chat["data"], HEADLINE_FIELDS).unwrap_or("(unnamed chat)");
        let when = entry["last_activity_at"].as_str().unwrap_or_default();
        let preview = entry["last_message"]["preview"]["text"]
            .as_str()
            .unwrap_or_default();
        let id = chat["id"].as_str().unwrap_or_default();
        println!(
            "{when}  {}: {}  [{id}]",
            one_line(name, 40),
            one_line(preview, 80)
        );
    }
    println!("Read one with: bridge read messages --chat <id>");
}

fn render(answer: &Value, wanted: &[&str], limit: usize) {
    if let Some(chats) = answer["chats"].as_array().filter(|chats| !chats.is_empty()) {
        render_chats(chats, limit);
        return;
    }
    let mut records = Vec::new();
    collect(answer, wanted, &mut records);
    if records.is_empty() {
        println!("Nothing to show. Check what has synced with `bridge stats`.");
        return;
    }
    let total = records.len();
    for record in records.iter().take(limit) {
        let data = &record["data"];
        let headline = text_field(data, HEADLINE_FIELDS)
            .or_else(|| data["sender"].as_str())
            .map(str::to_owned)
            .or_else(|| contact_name(data))
            .unwrap_or_else(|| "(no title)".to_owned());
        let when = text_field(data, WHEN_FIELDS).unwrap_or_default();
        let id = record["id"].as_str().unwrap_or_default();
        if when.is_empty() {
            println!("{}  [{id}]", one_line(&headline, 100));
        } else {
            println!("{when}  {}  [{id}]", one_line(&headline, 100));
        }
        if record["collection"] == "voicemail.message" {
            if let Some(text) = data["transcript"]["text"].as_str() {
                println!("  Transcript: {text}");
            } else {
                println!(
                    "  Transcript: {}",
                    data["transcript_state"].as_str().unwrap_or("unavailable")
                );
            }
            if let Some(digest) = data["audio"]["digest"].as_str() {
                println!(
                    "  Audio: {digest} ({}, {} bytes)",
                    data["audio"]["mime_type"].as_str().unwrap_or("unknown"),
                    data["audio"]["size"]
                );
            } else {
                println!(
                    "  Audio: {}",
                    data["audio_state"].as_str().unwrap_or("unavailable")
                );
            }
        }
    }
    if total > limit {
        println!("… and {} more. Use --limit or --json.", total - limit);
    }
}

async fn run_call(home: &Path, operation: &str, arguments: &Value) -> Outcome<Value> {
    let session = Session::open(home).await?;
    let result = session.call(operation, arguments).await;
    session.close();
    result
}

fn limit_records(value: &mut Value, wanted: &[&str], remaining: &mut usize) {
    let record = |value: &Value| {
        value["collection"]
            .as_str()
            .is_some_and(|collection| wanted.contains(&collection))
            && value.get("data").is_some()
    };
    match value {
        Value::Array(items) => {
            items.retain_mut(|item| {
                let is_item = record(item)
                    || item
                        .as_object()
                        .is_some_and(|fields| fields.values().any(record));
                if is_item {
                    if *remaining == 0 {
                        return false;
                    }
                    *remaining -= 1;
                }
                limit_records(item, wanted, remaining);
                true
            });
        }
        Value::Object(fields) => {
            for child in fields.values_mut() {
                limit_records(child, wanted, remaining);
            }
        }
        _ => {}
    }
}

pub async fn read(home: &Path, args: &ReadArgs) -> Outcome {
    let (operation, arguments, collections) = request(args)?;
    let mut answer = run_call(home, operation, &arguments).await?;
    if args.json {
        limit_records(
            &mut answer,
            collections,
            &mut (args.limit.min(200) as usize),
        );
        println!("{answer}");
    } else if args.kind == ReadKind::Photos {
        println!("{}", serde_json::to_string_pretty(&answer["result_json"])?);
    } else {
        render(&answer, collections, args.limit as usize);
    }
    Ok(())
}

pub async fn search(home: &Path, args: &SearchArgs) -> Outcome {
    let arguments = json!({"text": args.text, "limit": args.limit.min(200), "collection": ""});
    let answer = run_call(home, "search", &arguments).await?;
    if args.json {
        println!("{answer}");
    } else {
        render(&answer, &[], args.limit as usize);
    }
    Ok(())
}

fn list_operations(writes: bool) -> Outcome {
    for descriptor in ab_catalog::operations()? {
        if descriptor.kind != OperationKind::Command
            && (!descriptor.scope.ends_with(":write")
                || writes && descriptor.scope == "photos:write")
        {
            println!("{:<28} {}", descriptor.name, descriptor.description);
        }
    }
    Ok(())
}

pub async fn call(home: &Path, args: &CallArgs) -> Outcome {
    if args.list {
        return list_operations(args.write);
    }
    let Some(operation) = &args.operation else {
        return Err(Failure::new("Say which operation to call.")
            .next("bridge call sources.status, or bridge call --list to see them all."));
    };
    let descriptor = ab_catalog::operations()?
        .into_iter()
        .find(|descriptor| &descriptor.name == operation)
        .ok_or_else(|| {
            Failure::new(format!("There is no operation called `{operation}`."))
                .next("Run `bridge call --list`.")
        })?;
    let photos_write = descriptor.scope == "photos:write";
    if descriptor.kind == OperationKind::Command
        || descriptor.scope.ends_with(":write") && !(photos_write && args.write)
    {
        return Err(Failure::new(format!(
            "`{operation}` changes your data, and this command only reads."
        ))
        .next("Photos organization writes need --write, --token-env and a stable query id. Other changes use MCP."));
    }
    if (photos_write || matches!(operation.as_str(), "photos.fetch" | "photos.release"))
        && args.token_env.is_none()
    {
        return Err(Failure::new(
            "Photos transfers and writes require --token-env for a stable authenticated identity.",
        ));
    }
    let mut arguments: Value = serde_json::from_str(&args.args).map_err(|error| {
        Failure::new(format!("--args is not valid JSON: {error}."))
            .next("For example: --args '{\"limit\": 5}'")
    })?;
    if operation == "helper.version" && arguments == json!({}) {
        arguments = json!({"args": {}});
    }
    if photos_write
        && arguments["id"]
            .as_str()
            .is_none_or(|id| id.trim().is_empty())
    {
        return Err(Failure::new(
            "Photos writes require an explicit stable qry_ id in --args. Never automatically retry an ambiguous write.",
        ));
    }
    let answer = if let Some(name) = &args.token_env {
        let token = std::env::var(name)
            .ok()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                Failure::new(
                    "The requested credential environment variable is empty or unavailable.",
                )
            })?;
        let session = Session::with_token(home, token).await?;
        session.call(operation, &arguments).await?
    } else {
        run_call(home, operation, &arguments).await?
    };
    println!("{answer}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_found_at_any_depth_and_filtered_by_collection() {
        let answer = json!({"sections":[{"items":[{"reminder":{"collection":"reminders.reminder","id":"a","data":{"title":"x"}},"attachments":[{"collection":"reminders.attachment","id":"b","data":{}}]}]}]});
        let mut records = Vec::new();
        collect(&answer, &["reminders.reminder"], &mut records);
        assert_eq!(records.len(), 1);
        let mut every = Vec::new();
        collect(&answer, &[], &mut every);
        assert_eq!(every.len(), 2);
    }

    #[test]
    fn json_limit_counts_nested_reminder_subtasks() {
        let item = |id: &str, subtasks: Value| {
            json!({
                "reminder": {"collection":"reminders.reminder","id":id,"data":{}},
                "subtasks":subtasks,
                "attachments":[{"collection":"reminders.attachment","id":"media","data":{}}]
            })
        };
        let original = json!({"count":6,"sections":[
            {"items":[item("parent", json!([
                item("child", json!([item("grandchild", json!([]))])),
                item("sibling", json!([]))
            ])), item("second", json!([]))]},
            {"items":[item("third", json!([]))]}
        ]});
        let expected = [
            "parent",
            "child",
            "grandchild",
            "sibling",
            "second",
            "third",
        ];
        for limit in 0..=6 {
            let mut answer = original.clone();
            let mut remaining = limit;
            limit_records(&mut answer, &["reminders.reminder"], &mut remaining);
            let mut records = Vec::new();
            collect(&answer, &["reminders.reminder"], &mut records);
            let ids: Vec<_> = records
                .iter()
                .filter_map(|record| record["id"].as_str())
                .collect();
            assert_eq!(ids, expected[..limit], "limit {limit}: {answer}");
            assert_eq!(answer["count"], 6);
            if limit > 0 {
                assert_eq!(
                    answer["sections"][0]["items"][0]["attachments"],
                    original["sections"][0]["items"][0]["attachments"]
                );
            }
        }
    }

    #[test]
    fn json_limits_items_across_sections_without_removing_metadata() {
        for (collection, key, wrapper) in [
            ("contacts.contact", "contacts", "contact"),
            ("notes.note", "notes", "note"),
            ("reminders.reminder", "items", "reminder"),
            ("calendar.event", "events", "event"),
            ("messages.chat", "chats", "chat"),
            ("messages.message", "messages", "message"),
            ("mail.message", "records", "message"),
        ] {
            let item =
                json!({wrapper: {"collection":collection,"id":"a","data":{}},"metadata":"kept"});
            let original = json!({"count":3,"sections":[{key:[item.clone(),item.clone()]},{key:[item]}],"other":[{"collection":"other","data":{}}]});
            for limit in [0, 1, 2, 3] {
                let mut answer = original.clone();
                let mut remaining = limit;
                limit_records(&mut answer, &[collection], &mut remaining);
                let mut records = Vec::new();
                collect(&answer, &[collection], &mut records);
                assert_eq!(records.len(), limit, "{answer}");
                assert_eq!(answer["count"], 3);
                assert_eq!(answer["other"], original["other"]);
                if limit > 0 {
                    assert_eq!(answer["sections"][0][key][0]["metadata"], "kept");
                }
            }
        }
    }

    #[test]
    fn long_text_stays_on_one_line() {
        assert_eq!(one_line("a\n b   c", 10), "a b c");
        assert_eq!(one_line("abcdef", 3), "abc…");
    }
}
