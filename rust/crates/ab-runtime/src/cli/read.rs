use super::{
    Failure, Outcome, access,
    failure::{Reason, Step},
    gateway,
    modes::{self, Modes},
    stats,
    tree::{CallArgs, ReadArgs, ReadKind, SearchArgs},
    ui,
};
use ab_catalog::OperationKind;
use ab_trust::{Authority, Grant};
use data_encoding::BASE64URL_NOPAD;
use serde_json::{Value, json};
use std::{io::Read, path::Path, time::Duration};

const READ_TIMEOUT: Duration = Duration::from_secs(60);
const LIMIT_CAP: usize = 200;
const ALL_CAP: usize = 10_000;
const READ_RETRIES: u32 = 2;
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
const PAGE_KEYS: &[&str] = &["chats", "items", "records", "results"];

struct Session {
    authority: Option<Authority>,
    client: reqwest::Client,
    address: String,
    token: String,
    principal: String,
}

fn client() -> Outcome<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(READ_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|error| Failure::internal(&error))
}

enum Sent {
    Answer(Value),
    Refused(Failure),
    Unknown(String),
}

impl Session {
    async fn open(home: &Path) -> Outcome<Self> {
        let address = gateway::running_address(home).await?;
        let authority = access::open(home)?;
        let mut grants = Grant::agent_scopes()
            .into_iter()
            .filter(|scope| scope.ends_with(":read"))
            .map(|scope| Grant::parse(&scope))
            .collect::<Result<Vec<_>, _>>()?;
        grants.push(Grant::parse("system:read")?);
        let token = authority.create_token(grants, false).map_err(|_| {
            Failure::new(
                Reason::PermissionDenied,
                "Can't read from this home on its own.",
            )
            .sentence("Replicas read through the access they were paired with.")
        })?;
        let principal = authority.authenticate_token(&token)?.id;
        Ok(Self {
            authority: Some(authority),
            client: client()?,
            address,
            token,
            principal,
        })
    }

    async fn with_token(home: &Path, token: String) -> Outcome<Self> {
        let address = gateway::running_address(home).await?;
        Ok(Self {
            authority: None,
            client: client()?,
            address,
            token,
            principal: String::new(),
        })
    }

    async fn send(&self, operation: &str, arguments: &Value) -> Sent {
        let response = self
            .client
            .post(format!("http://{}/v1/op/{operation}", self.address))
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .body(arguments.to_string())
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) if error.is_connect() => {
                return Sent::Refused(
                    Failure::new(
                        Reason::NetworkError,
                        "Couldn't reach Apple Bridge. Nothing was sent.",
                    )
                    .then("Check that it's running", ["bridge", "up"]),
                );
            }
            Err(error) => return Sent::Unknown(ui::clean(&error.to_string())),
        };
        let status = response.status();
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return Sent::Unknown(ui::clean(&error.to_string())),
        };
        let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
            return Sent::Refused(Failure::new(
                Reason::ApiError,
                format!("Apple Bridge answered {status} without JSON."),
            ));
        };
        if status.is_success() {
            Sent::Answer(body)
        } else {
            Sent::Refused(problem(&body, operation))
        }
    }

    async fn read(&self, operation: &str, arguments: &Value) -> Outcome<Value> {
        let mut attempt = 0;
        loop {
            match self.send(operation, arguments).await {
                Sent::Answer(answer) => return Ok(answer),
                Sent::Refused(failure)
                    if failure.reason == Reason::NetworkError && attempt < READ_RETRIES =>
                {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(300 * u64::from(attempt))).await;
                }
                Sent::Refused(failure) => return Err(failure),
                Sent::Unknown(detail) if attempt < READ_RETRIES => {
                    attempt += 1;
                    ui::warn(
                        "retrying",
                        &format!("The answer to {operation} broke off; reading it again."),
                    );
                    tracing_detail(&detail);
                    tokio::time::sleep(Duration::from_millis(300 * u64::from(attempt))).await;
                }
                Sent::Unknown(detail) => {
                    return Err(Failure::new(
                        Reason::NetworkError,
                        format!("Couldn't read {operation}. The answer broke off."),
                    )
                    .row("Detail", detail)
                    .then("Try again", ["bridge", "call", operation]));
                }
            }
        }
    }

    fn close(self) {
        if let Some(authority) = self.authority {
            let _ = authority.revoke(&self.principal);
        }
    }
}

fn tracing_detail(detail: &str) {
    if Modes::get().debug {
        ui::err_line(&format!("debug: {detail}"));
    }
}

fn problem(body: &Value, operation: &str) -> Failure {
    let code = body["error"]["code"].as_str().unwrap_or("unknown");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or("request failed");
    let remote = format!("{message} ({code})");
    match code {
        "unavailable" => Failure::new(
            Reason::SourceUnavailable,
            format!("Can't run {operation}. Its source isn't available."),
        )
        .remote("Apple Bridge", remote)
        .then("See which sources have synced", ["bridge", "stats"]),
        "invalid" => Failure::new(
            Reason::InvalidValue,
            format!("Can't run {operation} with those arguments."),
        )
        .remote("Apple Bridge", remote)
        .then("See its arguments", ["bridge", "call", "--list", "--json"]),
        "unauthorized" | "forbidden" | "permission" | "denied" => Failure::new(
            Reason::PermissionDenied,
            format!("Can't run {operation}. This credential isn't allowed to."),
        )
        .remote("Apple Bridge", remote),
        "awaiting_approval" => Failure::new(
            Reason::PermissionMissing,
            format!("{operation} is waiting for the owner's approval."),
        )
        .remote("Apple Bridge", remote)
        .sentence("After approval, send the identical request again with the same id."),
        _ => Failure::new(Reason::ApiError, format!("Failed to run {operation}."))
            .remote("Apple Bridge", remote),
    }
}

fn time_zone() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .map_or_else(|| "UTC".to_owned(), str::to_owned)
}

pub fn limit(text: &str) -> Outcome<usize> {
    let invalid = || {
        Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --limit {}. It must be a whole number from 1 to {LIMIT_CAP}.",
                ui::clean(text)
            ),
        )
    };
    let value: i64 = text.trim().parse().map_err(|_| invalid())?;
    if value < 1 {
        return Err(invalid());
    }
    let value = usize::try_from(value).map_err(|_| invalid())?;
    if value > LIMIT_CAP {
        ui::warn(
            "limit_clamped",
            &format!(
                "--limit {value} is more than {LIMIT_CAP}, so it shows {LIMIT_CAP}. Use --all or --next for more."
            ),
        );
        return Ok(LIMIT_CAP);
    }
    Ok(value)
}

fn days(text: &str) -> Outcome<i64> {
    let invalid = || {
        Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --days {}. It must be a whole number from 1 to 366.",
                ui::clean(text)
            ),
        )
    };
    let value: i64 = text.trim().parse().map_err(|_| invalid())?;
    if (1..=366).contains(&value) {
        Ok(value)
    } else {
        Err(invalid())
    }
}

struct Plan {
    operation: &'static str,
    arguments: Value,
    collections: &'static [&'static str],
    source: Option<&'static str>,
}

#[allow(clippy::too_many_lines)]
fn plan(args: &ReadArgs, limit: usize) -> Outcome<Plan> {
    let mut issues = Vec::new();
    if args.kind != ReadKind::Messages {
        if args.query.is_some() {
            issues.push("--query works only with bridge read messages.");
        }
        if args.chat.is_some() {
            issues.push("--chat works only with bridge read messages.");
        }
    }
    if args.kind != ReadKind::Calendar && args.days != "7" {
        issues.push("--days works only with bridge read calendar.");
    }
    if args.next.is_some() && args.all {
        issues.push("--next and --all can't be used together.");
    }
    if !issues.is_empty() {
        let mut failure = Failure::new(
            Reason::ConflictingOptions,
            if issues.len() == 1 {
                format!("Can't read {}. {}", args.kind.as_str(), issues[0])
            } else {
                format!(
                    "Can't read {}. {} options don't fit.",
                    args.kind.as_str(),
                    issues.len()
                )
            },
        );
        if issues.len() > 1 {
            for issue in issues {
                failure = failure.issue(issue);
            }
        }
        return Err(failure);
    }
    if args.chat.is_some() && args.query.is_some() {
        ui::warn(
            "ignored",
            "--query is ignored with --chat; it reads the whole chat.",
        );
    }
    let days = days(&args.days)?;
    Ok(match args.kind {
        ReadKind::Photos => Plan {
            operation: "photos.search",
            arguments: json!({"args":{"limit":limit}}),
            collections: &[],
            source: None,
        },
        ReadKind::Messages => {
            if let Some(chat) = &args.chat {
                Plan {
                    operation: "messages.thread",
                    arguments: json!({"chat_id": chat, "limit": limit}),
                    collections: &["messages.message"],
                    source: Some("messages"),
                }
            } else if let Some(text) = &args.query {
                Plan {
                    operation: "messages.search",
                    arguments: json!({"text": text, "limit": limit}),
                    collections: &["messages.message"],
                    source: Some("messages"),
                }
            } else {
                Plan {
                    operation: "messages.chats",
                    arguments: json!({"limit": limit}),
                    collections: &["messages.chat"],
                    source: Some("messages"),
                }
            }
        }
        ReadKind::Chats => Plan {
            operation: "messages.chats",
            arguments: json!({"limit": limit}),
            collections: &["messages.chat"],
            source: Some("messages"),
        },
        ReadKind::Contacts => Plan {
            operation: "contacts.list",
            arguments: json!({}),
            collections: &["contacts.contact"],
            source: Some("contacts"),
        },
        ReadKind::Calendar => {
            let now = jiff::Timestamp::now();
            let end = now
                .checked_add(jiff::Span::new().hours(days * 24))
                .map_err(|_| Failure::invalid("Can't use that --days. It's too large."))?;
            Plan {
                operation: "calendar.agenda",
                arguments: json!({"start": now.to_string(), "end": end.to_string()}),
                collections: &["calendar.event"],
                source: Some("calendar"),
            }
        }
        ReadKind::Reminders => Plan {
            operation: "reminders.items",
            arguments: json!({"time_zone": time_zone(), "smart": "all"}),
            collections: &["reminders.reminder"],
            source: Some("reminders"),
        },
        ReadKind::Notes => Plan {
            operation: "notes.list",
            arguments: json!({}),
            collections: &["notes.note"],
            source: Some("notes"),
        },
        ReadKind::Mail => Plan {
            operation: "records.query",
            arguments: json!({"collection": "mail.message", "limit": limit}),
            collections: &["mail.message"],
            source: Some("mail"),
        },
        ReadKind::Voicemail => Plan {
            operation: "records.query",
            arguments: json!({"collection": "voicemail.message", "limit": limit}),
            collections: &["voicemail.message"],
            source: Some("voicemail"),
        },
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
    let joined = ui::clean(&text.split_whitespace().collect::<Vec<_>>().join(" "));
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
        let when = ui::clean(entry["last_activity_at"].as_str().unwrap_or_default());
        let preview = entry["last_message"]["preview"]["text"]
            .as_str()
            .unwrap_or_default();
        let id = ui::clean(chat["id"].as_str().unwrap_or_default());
        ui::outln(&format!(
            "{when}  {}: {}  [{id}]",
            one_line(name, 40),
            one_line(preview, 80)
        ));
    }
    ui::next_steps(&[Step::new(
        "Read one chat",
        ["bridge", "read", "messages", "--chat", "<CHAT>"],
    )]);
}

fn render(answer: &Value, wanted: &[&str], limit: usize) -> usize {
    if let Some(chats) = answer["chats"].as_array().filter(|chats| !chats.is_empty()) {
        render_chats(chats, limit);
        return chats.len();
    }
    let mut records = Vec::new();
    collect(answer, wanted, &mut records);
    let total = records.len();
    for record in records.iter().take(limit) {
        let data = &record["data"];
        let headline = text_field(data, HEADLINE_FIELDS)
            .or_else(|| data["sender"].as_str())
            .map(str::to_owned)
            .or_else(|| contact_name(data))
            .unwrap_or_else(|| "(no title)".to_owned());
        let when = ui::clean(text_field(data, WHEN_FIELDS).unwrap_or_default());
        let id = ui::clean(record["id"].as_str().unwrap_or_default());
        if when.is_empty() {
            ui::outln(&format!("{}  [{id}]", one_line(&headline, 100)));
        } else {
            ui::outln(&format!("{when}  {}  [{id}]", one_line(&headline, 100)));
        }
        if record["collection"] == "voicemail.message" {
            if let Some(text) = data["transcript"]["text"].as_str() {
                ui::outln(&format!("  Transcript: {}", ui::clean(text)));
            } else {
                ui::outln(&format!(
                    "  Transcript: {}",
                    ui::clean(data["transcript_state"].as_str().unwrap_or("unavailable"))
                ));
            }
            if let Some(digest) = data["audio"]["digest"].as_str() {
                ui::outln(&format!(
                    "  Audio: {} ({}, {} bytes)",
                    ui::clean(digest),
                    ui::clean(data["audio"]["mime_type"].as_str().unwrap_or("unknown")),
                    data["audio"]["size"]
                ));
            } else {
                ui::outln(&format!(
                    "  Audio: {}",
                    ui::clean(data["audio_state"].as_str().unwrap_or("unavailable"))
                ));
            }
        }
    }
    if total > limit {
        ui::warn(
            "truncated",
            &format!("Showing {limit} of {total}. Use --limit, --all or --json for the rest."),
        );
    }
    total
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

fn encode_cursor(operation: &str, filters: &Value, after: &str) -> String {
    let bound = json!({"v": 1, "op": operation, "filters": filters, "after": after});
    BASE64URL_NOPAD.encode(bound.to_string().as_bytes())
}

fn filters(arguments: &Value) -> Value {
    let mut copy = arguments.clone();
    if let Some(object) = copy.as_object_mut() {
        object.remove("limit");
        object.remove("after");
    }
    copy
}

fn decode_cursor(cursor: &str, operation: &str, arguments: &Value) -> Outcome<String> {
    let invalid = || {
        Failure::new(
            Reason::InvalidValue,
            "Can't use that --next cursor. Copy it whole from the previous page.",
        )
    };
    let bytes = BASE64URL_NOPAD
        .decode(cursor.trim().as_bytes())
        .map_err(|_| invalid())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if value["v"] != 1 {
        return Err(invalid());
    }
    if value["op"] != operation || value["filters"] != filters(arguments) {
        return Err(Failure::new(
            Reason::InvalidValue,
            "Can't use that --next cursor here. It belongs to a different read; repeat the same command and options.",
        ));
    }
    value["after"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(invalid)
}

fn page_key(answer: &Value) -> Option<&'static str> {
    PAGE_KEYS
        .iter()
        .copied()
        .find(|key| answer[*key].is_array())
}

struct Coverage {
    state: Option<String>,
    synced: Option<String>,
}

fn coverage(home: &Path, source: &str) -> Outcome<Coverage> {
    let Some(connection) = stats::open(home)? else {
        return Ok(Coverage {
            state: None,
            synced: None,
        });
    };
    let states = stats::read_states(&connection)?;
    let state = states.get(source).cloned().or_else(|| {
        (source == "voicemail")
            .then(|| states.get("phone").cloned())
            .flatten()
    });
    let synced: Option<String> = connection
        .query_row(
            "SELECT max(observed_at) FROM records WHERE collection LIKE ?1 || '.%' AND deleted=0",
            [source],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    Ok(Coverage { state, synced })
}

fn label(source: &str) -> String {
    let mut characters = source.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

fn judge(kind: &str, source: &str, coverage: &Coverage, empty: bool) -> Outcome {
    let name = label(source);
    let state = coverage.state.as_deref().unwrap_or("never");
    let permission = || {
        Failure::new(
            Reason::PermissionMissing,
            format!("Can't read {kind}. Apple Bridge doesn't have permission for {name}."),
        )
        .then("Allow it", ["bridge", "permissions", "guide"])
    };
    match (state, empty) {
        ("never" | "unknown", true) => Err(Failure::new(
            Reason::SourceUnavailable,
            format!("Can't read {kind}. {name} hasn't synced on this Mac yet, so this isn't a verified empty list."),
        )
        .sentence("Syncing needs the Apple Bridge Helper app and its permissions.")
        .then("Follow the sync", ["bridge", "stats", "--follow"])
        .then("Allow the Mac permissions", ["bridge", "permissions", "guide"])),
        ("permission_missing", true) => Err(permission()),
        ("unavailable" | "capture_failed", true) => Err(Failure::new(
            Reason::SourceUnavailable,
            format!("Can't read {kind}. {name} isn't available on this Mac right now."),
        )
        .then("Check it", ["bridge", "stats", "--all"])),
        ("ready" | "degraded", true) => {
            if !Modes::get().json {
                ui::note(&format!("No {kind}. {name} synced and has none."));
            }
            Ok(())
        }
        ("never" | "unknown", false) => {
            let when = coverage.synced.as_deref().map(ui::age).unwrap_or_default();
            ui::warn("unconfirmed", &format!("{name} hasn't reported its sync status; showing the cached copy from {when}."));
            Ok(())
        }
        ("ready", false) => {
            if let Some(synced) = &coverage.synced {
                ui::note(&format!("Cached copy; {name} last changed {}.", ui::age(synced)));
            }
            Ok(())
        }
        (other, false) => {
            let when = coverage.synced.as_deref().map(ui::age).unwrap_or_default();
            ui::warn(
                "stale",
                &format!("{name} is {} now; showing the cached copy from {when}. It may be out of date.", other.replace('_', " ")),
            );
            Ok(())
        }
        (other, true) => Err(Failure::new(
            Reason::SourceUnavailable,
            format!("Can't read {kind}. {name} is {}.", other.replace('_', " ")),
        )
        .then("Check it", ["bridge", "stats", "--all"])),
    }
}

async fn fetch_pages(
    session: &Session,
    plan: &Plan,
    after: Option<String>,
    all: bool,
    limit: usize,
) -> Outcome<(Value, Option<String>)> {
    let mut arguments = plan.arguments.clone();
    if let Some(after) = &after {
        arguments["after"] = json!(after);
    }
    let mut answer = session.read(plan.operation, &arguments).await?;
    let mut next = answer["next"]
        .as_str()
        .filter(|next| !next.is_empty())
        .map(str::to_owned);
    if !all {
        return Ok((answer, next));
    }
    let Some(key) = page_key(&answer) else {
        return Ok((answer, next));
    };
    let page = if limit < LIMIT_CAP { LIMIT_CAP } else { limit };
    while let Some(after) = next.clone() {
        let gathered = answer[key].as_array().map_or(0, Vec::len);
        if gathered >= ALL_CAP {
            ui::warn(
                "truncated",
                &format!("Stopped at {ALL_CAP} items. Continue with --next."),
            );
            break;
        }
        arguments["after"] = json!(after);
        arguments["limit"] = json!(page.min(ALL_CAP - gathered));
        let more = session.read(plan.operation, &arguments).await?;
        if let (Some(items), Some(extra)) = (answer[key].as_array_mut(), more[key].as_array()) {
            items.extend(extra.iter().cloned());
        }
        next = more["next"]
            .as_str()
            .filter(|next| !next.is_empty())
            .map(str::to_owned);
        answer["next"] = more["next"].clone();
    }
    Ok((answer, next))
}

pub async fn read(home: &Path, args: &ReadArgs) -> Outcome {
    let limit = limit(&args.limit)?;
    let mut plan = plan(args, limit)?;
    let after = args
        .next
        .as_deref()
        .map(|cursor| decode_cursor(cursor, plan.operation, &plan.arguments))
        .transpose()?;
    if args.all && plan.arguments.get("limit").is_some() {
        plan.arguments["limit"] = json!(LIMIT_CAP);
    }
    let session = Session::open(home).await?;
    let result = fetch_pages(&session, &plan, after, args.all, limit).await;
    session.close();
    let (mut answer, next) = result?;
    let shown = if args.all { ALL_CAP } else { limit };
    let mut records = Vec::new();
    collect(&answer, plan.collections, &mut records);
    let empty = records.is_empty() && answer["chats"].as_array().is_none_or(Vec::is_empty);
    if let Some(source) = plan.source {
        judge(args.kind.as_str(), source, &coverage(home, source)?, empty)?;
    }
    let cursor = next
        .as_deref()
        .map(|after| encode_cursor(plan.operation, &filters(&plan.arguments), after));
    if Modes::get().json {
        if args.kind != ReadKind::Photos {
            limit_records(&mut answer, plan.collections, &mut shown.clone());
        }
        if let Some(cursor) = &cursor {
            answer["next_cursor"] = json!(cursor);
        }
        ui::json_document(answer);
        return Ok(());
    }
    if args.kind == ReadKind::Photos {
        ui::outln(&serde_json::to_string_pretty(&answer["result_json"])?);
        if answer["result_json"]["next_cursor"].is_string() {
            ui::warn(
                "truncated",
                "More photos match. Use bridge call photos.search with next_cursor to continue.",
            );
        }
        return Ok(());
    }
    render(&answer, plan.collections, shown);
    if let Some(cursor) = cursor {
        let mut words = vec![
            "bridge".to_owned(),
            "read".to_owned(),
            args.kind.as_str().to_owned(),
        ];
        if let Some(chat) = &args.chat {
            words.extend(["--chat".to_owned(), chat.clone()]);
        } else if let Some(query) = &args.query {
            words.extend(["--query".to_owned(), query.clone()]);
        }
        if limit != 10 {
            words.extend(["--limit".to_owned(), limit.to_string()]);
        }
        words.extend(["--next".to_owned(), cursor]);
        ui::next_steps(&[Step::new("See the next page", words)]);
    }
    Ok(())
}

pub async fn search(home: &Path, args: &SearchArgs) -> Outcome {
    if args.text.trim().is_empty() {
        return Err(
            Failure::new(Reason::MissingArgument, "Can't search for empty text.")
                .then("Search for a word", ["bridge", "search", "<TEXT>"]),
        );
    }
    let limit = limit(&args.limit)?;
    if args.next.is_some() && args.all {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't use --next and --all together.",
        ));
    }
    let plan = Plan {
        operation: "search",
        arguments: json!({"text": args.text, "limit": if args.all { LIMIT_CAP } else { limit }, "collection": ""}),
        collections: &[],
        source: None,
    };
    let after = args
        .next
        .as_deref()
        .map(|cursor| decode_cursor(cursor, plan.operation, &plan.arguments))
        .transpose()?;
    if stats::open(home)?.is_none() {
        return Err(Failure::not_set_up(home));
    }
    let session = Session::open(home).await?;
    let result = fetch_pages(&session, &plan, after, args.all, limit).await;
    session.close();
    let (mut answer, next) = result?;
    let cursor = next
        .as_deref()
        .map(|after| encode_cursor(plan.operation, &filters(&plan.arguments), after));
    if Modes::get().json {
        if let Some(cursor) = &cursor {
            answer["next_cursor"] = json!(cursor);
        }
        ui::json_document(answer);
        return Ok(());
    }
    let shown = if args.all { ALL_CAP } else { limit };
    if render(&answer, &[], shown) == 0 {
        ui::note(&format!(
            "No results for \"{}\" in what has synced.",
            ui::clean(&args.text)
        ));
        ui::next_steps(&[Step::new("See what has synced", ["bridge", "stats"])]);
    }
    if let Some(cursor) = cursor {
        ui::next_steps(&[Step::new(
            "See the next page",
            [
                "bridge".to_owned(),
                "search".to_owned(),
                args.text.clone(),
                "--next".to_owned(),
                cursor,
            ],
        )]);
    }
    Ok(())
}

fn list_operations(writes: bool) -> Outcome {
    let descriptors: Vec<_> = ab_catalog::operations()
        .map_err(|error| Failure::internal(&error))?
        .into_iter()
        .filter(|descriptor| {
            descriptor.kind != OperationKind::Command
                && (!descriptor.scope.ends_with(":write")
                    || writes && descriptor.scope == "photos:write")
        })
        .collect();
    if Modes::get().json {
        ui::json_document(Value::Array(
            descriptors
                .iter()
                .map(|descriptor| {
                    json!({"name": descriptor.name, "scope": descriptor.scope, "description": descriptor.description, "input": descriptor.input})
                })
                .collect(),
        ));
        return Ok(());
    }
    let rows: Vec<Vec<String>> = descriptors
        .iter()
        .map(|descriptor| {
            vec![
                descriptor.name.clone(),
                descriptor.scope.clone(),
                first_sentence(&descriptor.description),
            ]
        })
        .collect();
    ui::table(&["Operation", "Scope", "Summary"], &rows);
    Ok(())
}

fn first_sentence(text: &str) -> String {
    text.split_once(". ")
        .map_or(text, |(first, _)| first)
        .trim_end_matches('.')
        .to_owned()
}

fn read_stdin(what: &str) -> Outcome<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return Err(Failure::new(
            Reason::MissingInput,
            format!("Can't read {what} from a terminal. Pipe the JSON in."),
        )
        .then(
            "Pass it inline instead",
            ["bridge", "call", "<OPERATION>", "--data", "<JSON>"],
        ));
    }
    modes::mark_stdin_payload();
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    Ok(text)
}

fn arguments(args: &CallArgs) -> Outcome<(Value, &'static str)> {
    let given = [
        args.data.is_some(),
        args.args.is_some(),
        args.file.is_some(),
    ]
    .iter()
    .filter(|given| **given)
    .count();
    if given > 1 {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't combine --data and --file. Pass the arguments once.",
        ));
    }
    let (text, option) = if let Some(data) = &args.data {
        (
            if data == "-" {
                read_stdin("--data")?
            } else {
                data.clone()
            },
            "--data",
        )
    } else if let Some(old) = &args.args {
        ui::deprecated("--args", "--data");
        (old.clone(), "--data")
    } else if let Some(file) = &args.file {
        if file.as_os_str() == "-" {
            (read_stdin("--file")?, "--file")
        } else {
            let text = std::fs::read_to_string(file).map_err(|_| {
                Failure::new(
                    Reason::NotFound,
                    format!("Couldn't read --file {}.", file.display()),
                )
            })?;
            (text, "--file")
        }
    } else {
        ("{}".to_owned(), "--data")
    };
    let value = serde_json::from_str(&text).map_err(|error| {
        Failure::new(
            Reason::InvalidValue,
            format!("Can't use {option}. It isn't valid JSON."),
        )
        .row("Detail", ui::clean(&error.to_string()))
        .then(
            "Pass an object",
            ["bridge", "call", "<OPERATION>", "--data", "{\"limit\": 5}"],
        )
    })?;
    Ok((value, option))
}

fn uncertain(operation: &str, arguments: &Value, detail: String, token_env: &str) -> Failure {
    let id = arguments["id"].as_str().unwrap_or_default().to_owned();
    let mut inspect = vec!["bridge".to_owned(), "call".to_owned()];
    if let Some(asset) = arguments["args"]["asset_id"]
        .as_str()
        .or_else(|| arguments["asset_id"].as_str())
    {
        inspect.extend([
            "photos.get".to_owned(),
            "--data".to_owned(),
            json!({"args": {"asset_id": asset}}).to_string(),
        ]);
    } else {
        inspect.push("photos.albums".to_owned());
    }
    inspect.extend(["--token-env".to_owned(), token_env.to_owned()]);
    Failure::new(Reason::UnknownOutcome, format!("Can't confirm whether {operation} finished. The connection closed after the request was sent."))
        .row("Operation ID", id)
        .row("Confirmed", "the request was sent; no answer came back")
        .row("Detail", detail)
        .sentence("It wasn't retried. Don't send it with a new ID; that could apply it twice. Check its result first; if it didn't apply, send the identical request with the same ID.")
        .step(Step::new("Check what changed", inspect))
}

#[allow(clippy::too_many_lines)]
pub async fn call(home: &Path, args: &CallArgs) -> Outcome {
    if args.list {
        if args.operation.is_some() {
            return Err(Failure::new(
                Reason::ConflictingOptions,
                "Can't use --list with an operation.",
            ));
        }
        return list_operations(args.write);
    }
    let Some(operation) = &args.operation else {
        return Err(Failure::new(
            Reason::MissingArgument,
            "Can't call without an <OPERATION>.",
        )
        .then("Check the sources", ["bridge", "call", "sources.status"])
        .then("Or list operations", ["bridge", "call", "--list"]));
    };
    let descriptor = ab_catalog::operations()
        .map_err(|error| Failure::internal(&error))?
        .into_iter()
        .find(|descriptor| &descriptor.name == operation)
        .ok_or_else(|| {
            let names: Vec<String> = ab_catalog::operations()
                .unwrap_or_default()
                .into_iter()
                .map(|descriptor| descriptor.name)
                .collect();
            let mut headline = format!("Unknown operation {}.", ui::clean(operation));
            if let Some(close) = super::help::suggestion(&names, operation) {
                headline = format!("{headline} Did you mean {close}?");
            }
            Failure::new(Reason::InvalidValue, headline)
                .then("List operations", ["bridge", "call", "--list"])
        })?;
    let photos_write = descriptor.scope == "photos:write";
    if descriptor.kind == OperationKind::Command
        || descriptor.scope.ends_with(":write") && !(photos_write && args.write)
    {
        let mut failure = Failure::new(
            Reason::ReadOnly,
            format!("Can't call {operation}. It changes your data, and bridge call only reads."),
        );
        failure = if photos_write {
            failure.sentence(
                "Photos organization writes need --write, --token-env and a stable query ID.",
            )
        } else {
            failure.sentence("Other changes go through MCP, where the owner approves them.")
        };
        return Err(failure);
    }
    if (photos_write || matches!(operation.as_str(), "photos.fetch" | "photos.release"))
        && args.token_env.is_none()
    {
        return Err(Failure::new(
            Reason::MissingArgument,
            format!("Can't call {operation} without --token-env."),
        )
        .sentence("Photos transfers and writes need a stable credential identity.")
        .then(
            "Pass the variable holding the token",
            ["bridge", "call", operation, "--token-env", "<NAME>"],
        ));
    }
    if let Some(name) = &args.token_env
        && (name.is_empty() || name.contains('='))
    {
        return Err(Failure::invalid(
            "Can't use that --token-env. It must name an environment variable.",
        ));
    }
    let (mut arguments, option) = arguments(args)?;
    if operation == "helper.version" && arguments == json!({}) {
        arguments = json!({"args": {}});
    }
    if photos_write
        && arguments["id"]
            .as_str()
            .is_none_or(|id| id.trim().is_empty())
    {
        return Err(Failure::new(Reason::MissingArgument, format!("Can't call {operation} without a stable \"id\" in {option}."))
            .sentence("Use a qry_ ID you keep. If the outcome is unclear, check it and reuse the same ID; never retry with a new one."));
    }
    let answer = if let Some(name) = &args.token_env {
        let token = std::env::var(name)
            .ok()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                Failure::new(
                    Reason::MissingInput,
                    format!(
                        "Can't call with --token-env {}. That variable is empty or unset.",
                        ui::clean(name)
                    ),
                )
            })?;
        let session = Session::with_token(home, token).await?;
        if photos_write {
            match session.send(operation, &arguments).await {
                Sent::Answer(answer) => answer,
                Sent::Refused(failure) => return Err(failure),
                Sent::Unknown(detail) => {
                    return Err(uncertain(operation, &arguments, detail, name));
                }
            }
        } else {
            session.read(operation, &arguments).await?
        }
    } else {
        let session = Session::open(home).await?;
        let result = session.read(operation, &arguments).await;
        session.close();
        result?
    };
    ui::outln(&answer.to_string());
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
    fn cursors_round_trip_and_refuse_other_reads() -> Result<(), Box<dyn std::error::Error>> {
        let arguments = json!({"limit": 5, "text": "hi"});
        let cursor = encode_cursor("messages.search", &filters(&arguments), "token");
        assert_eq!(
            decode_cursor(
                &cursor,
                "messages.search",
                &json!({"limit": 9, "text": "hi"})
            )
            .map_err(|f| f.headline.clone())?,
            "token"
        );
        assert!(decode_cursor(&cursor, "messages.chats", &arguments).is_err());
        assert!(decode_cursor(&cursor, "messages.search", &json!({"text": "other"})).is_err());
        assert!(decode_cursor("garbage!", "messages.search", &arguments).is_err());
        Ok(())
    }

    #[test]
    fn limits_must_be_positive() {
        assert!(limit("0").is_err());
        assert!(limit("-1").is_err());
        assert!(limit("x").is_err());
        assert_eq!(limit("5").ok(), Some(5));
    }

    #[test]
    fn long_text_stays_on_one_line() {
        assert_eq!(one_line("a\n b   c", 10), "a b c");
        assert_eq!(one_line("abcdef", 3), "abc…");
    }
}
