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
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeSet,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

const READ_TIMEOUT: Duration = Duration::from_secs(60);
const LIMIT_CAP: usize = 200;
const ALL_CAP: usize = 10_000;
const DEFAULT_LIMIT: usize = 10;
const DEFAULT_DAYS: i64 = 7;
const READ_RETRIES: u32 = 3;
const RETRY_CEILING: Duration = Duration::from_secs(10);
const MAX_PAGES: usize = 200;
const EMPTY_PAGES: usize = 3;
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
    Refused(Failure, Retry),
    Unknown(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Retry {
    Never,
    After(Option<Duration>),
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let when = jiff::fmt::rfc2822::parse(value.trim()).ok()?.timestamp();
    let left = when.as_second() - jiff::Timestamp::now().as_second();
    Some(Duration::from_secs(u64::try_from(left).unwrap_or(0)))
}

fn rate_limited(operation: &str, wait: Option<Duration>) -> Failure {
    let mut failure = Failure::new(
        Reason::RateLimited,
        format!("Apple Bridge is busy and asked to slow down, so {operation} didn't run."),
    );
    if let Some(wait) = wait {
        failure = failure
            .data("retryAfterSeconds", wait.as_secs())
            .row("Retry after", super::commands::friendly(wait));
    }
    failure.sentence("Nothing was changed. Wait a moment, then try again.")
}

impl Session {
    async fn open(home: &Path) -> Outcome<Self> {
        let address = gateway::running_address(home).await?;
        Self::temporary(home, address)
    }

    fn temporary(home: &Path, address: String) -> Outcome<Self> {
        let client = client()?;
        let authority = access::open(home)?;
        let mut grants = Grant::agent_scopes()
            .into_iter()
            .filter(|scope| scope.ends_with(":read"))
            .map(|scope| Grant::parse(&scope))
            .collect::<Result<Vec<_>, _>>()?;
        grants.push(Grant::parse("system:read")?);
        let expires = jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::from_hours(1))
            .map_err(|error| Failure::internal(&error))?;
        let token = authority
            .create_token_until(grants, false, Some(expires))
            .map_err(|_| {
                Failure::new(
                    Reason::PermissionDenied,
                    "Can't read from this home on its own.",
                )
                .sentence("Replicas read through the access they were paired with.")
            })?;
        Ok(Self {
            authority: Some(authority),
            client,
            address,
            token,
        })
    }

    async fn with_token(home: &Path, token: String) -> Outcome<Self> {
        let address = gateway::running_address(home).await?;
        Ok(Self {
            authority: None,
            client: client()?,
            address,
            token,
        })
    }

    async fn send(&self, operation: &str, arguments: &Value, subject: Option<&str>) -> Sent {
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
                    Retry::After(None),
                );
            }
            Err(error) => return Sent::Unknown(self.redact(&error.to_string())),
        };
        let status = response.status();
        let wait = retry_after(response.headers());
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return Sent::Unknown(self.redact(&error.to_string())),
        };
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Sent::Refused(rate_limited(operation, wait), Retry::After(wait));
        }
        let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
            if status.is_success() {
                return Sent::Unknown(format!(
                    "Apple Bridge answered {status}, but the answer wasn't JSON."
                ));
            }
            let retry = if status.is_server_error() {
                Retry::After(wait)
            } else {
                Retry::Never
            };
            return Sent::Refused(
                Failure::new(
                    Reason::ApiError,
                    format!("Apple Bridge answered {status} without JSON."),
                )
                .data("httpStatus", status.as_u16()),
                retry,
            );
        };
        if status.is_success() {
            Sent::Answer(body)
        } else {
            let retry = if body["error"]["retryable"] == true {
                Retry::After(wait)
            } else {
                Retry::Never
            };
            Sent::Refused(
                problem(&body, operation, &self.token, subject).data("httpStatus", status.as_u16()),
                retry,
            )
        }
    }

    fn redact(&self, text: &str) -> String {
        ui::clean(&hide(text, &self.token))
    }

    async fn read(
        &self,
        operation: &str,
        arguments: &Value,
        subject: Option<&str>,
    ) -> Outcome<Value> {
        let mut attempt = 0;
        loop {
            let pause = Duration::from_millis(300 * 2_u64.pow(attempt));
            match self.send(operation, arguments, subject).await {
                Sent::Answer(answer) => return Ok(answer),
                Sent::Refused(failure, Retry::After(wait)) if attempt < READ_RETRIES => {
                    attempt += 1;
                    let wait = wait.unwrap_or(pause);
                    if wait > RETRY_CEILING {
                        return Err(failure);
                    }
                    if failure.reason == Reason::RateLimited {
                        ui::warn(
                            "retrying",
                            &format!(
                                "Apple Bridge asked to slow down; reading {operation} again in {}.",
                                super::commands::friendly(wait)
                            ),
                        );
                    }
                    tokio::time::sleep(wait).await;
                }
                Sent::Refused(failure, _) => return Err(failure),
                Sent::Unknown(detail) if attempt < READ_RETRIES => {
                    attempt += 1;
                    ui::warn(
                        "retrying",
                        &format!("The answer to {operation} broke off; reading it again."),
                    );
                    tracing_detail(&detail);
                    tokio::time::sleep(pause).await;
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
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(authority) = &self.authority
            && let Ok(principal) = authority.authenticate_token(&self.token)
        {
            let _ = authority.revoke(&principal.id);
        }
    }
}

fn tracing_detail(detail: &str) {
    if Modes::get().debug {
        ui::err_line(&format!("debug: {}", ui::clean_input(detail)));
    }
}

fn hide(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_owned()
    } else {
        text.replace(secret, "[redacted]")
    }
}

fn problem(body: &Value, operation: &str, secret: &str, subject: Option<&str>) -> Failure {
    let code = body["error"]["code"].as_str().unwrap_or("unknown");
    let message = body["error"]["message"]
        .as_str()
        .unwrap_or("request failed");
    let remote = hide(&format!("{message} ({code})"), secret);
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
        "not_found" => Failure::new(
            Reason::NotFound,
            subject.map_or_else(
                || format!("Couldn't find what {operation} asked for."),
                |subject| format!("Couldn't find {}.", ui::clean(subject)),
            ),
        )
        .remote("Apple Bridge", remote),
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
        "indeterminate" => Failure::new(
            Reason::UnknownOutcome,
            format!("Can't confirm whether {operation} took effect."),
        )
        .remote("Apple Bridge", remote),
        "deadline" => Failure::new(
            Reason::Timeout,
            format!("{operation} ran out of time in Apple Bridge."),
        )
        .remote("Apple Bridge", remote),
        "id_reused" | "conflict" => Failure::new(
            Reason::Conflict,
            format!("Can't run {operation}. It conflicts with an earlier request."),
        )
        .remote("Apple Bridge", remote),
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
                "Can't use --days {}. It must be a whole number of days, 1 or more.",
                ui::clean(text)
            ),
        )
    };
    let value: i64 = text.trim().parse().map_err(|_| invalid())?;
    if value >= 1 {
        Ok(value)
    } else {
        Err(invalid())
    }
}

pub fn home_tag(home: &Path) -> String {
    use sha2::Digest;
    let canonical = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_owned());
    let digest = sha2::Sha256::digest(canonical.to_string_lossy().as_bytes());
    digest[..8].iter().fold(String::new(), |mut text, byte| {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[derive(Clone, Copy)]
struct Pager {
    items: &'static str,
    next: &'static str,
    argument: &'static str,
    older_first: bool,
    nested: bool,
}

impl Pager {
    fn body<'a>(&self, page: &'a Value) -> &'a Value {
        if self.nested {
            &page["result_json"]
        } else {
            page
        }
    }

    fn body_mut<'a>(&self, page: &'a mut Value) -> &'a mut Value {
        if self.nested {
            &mut page["result_json"]
        } else {
            page
        }
    }

    fn arguments<'a>(&self, arguments: &'a mut Value) -> &'a mut Value {
        if self.nested {
            &mut arguments["args"]
        } else {
            arguments
        }
    }
}

fn pager(operation: &str) -> Option<Pager> {
    let pager = |items, next, argument, older_first| Pager {
        items,
        next,
        argument,
        older_first,
        nested: false,
    };
    match operation {
        "photos.search" => Some(Pager {
            nested: true,
            ..pager("assets", "next_cursor", "cursor", false)
        }),
        "messages.chats" => Some(pager("chats", "next", "after", false)),
        "messages.thread" => Some(pager("items", "before", "before", true)),
        "messages.search" | "messages.collection" => Some(pager("items", "next", "after", false)),
        "search" | "records.query" => Some(pager("records", "next", "after", false)),
        _ => None,
    }
}

struct Plan {
    kind: &'static str,
    operation: &'static str,
    arguments: Value,
    options: Map<String, Value>,
    collections: &'static [&'static str],
    source: Option<&'static str>,
    subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Cursor {
    kind: String,
    operation: String,
    arguments: Value,
    options: Map<String, Value>,
    after: Option<String>,
    offset: usize,
    limit: Option<usize>,
    home: String,
    text: String,
}

impl Cursor {
    fn encode(&self) -> String {
        let bound = json!({
            "v": 2,
            "kind": self.kind,
            "op": self.operation,
            "args": self.arguments,
            "options": self.options,
            "after": self.after,
            "offset": self.offset,
            "limit": self.limit,
            "home": self.home,
        });
        BASE64URL_NOPAD.encode(bound.to_string().as_bytes())
    }

    fn decode(text: &str) -> Outcome<Self> {
        let invalid = || {
            Failure::new(
                Reason::InvalidValue,
                "Can't use that --next cursor. Copy it whole from the previous page.",
            )
        };
        let bytes = BASE64URL_NOPAD
            .decode(text.trim().as_bytes())
            .map_err(|_| invalid())?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let string = |key: &str| value[key].as_str().map(str::to_owned);
        match value["v"].as_u64() {
            Some(1) => Ok(Self {
                kind: String::new(),
                operation: string("op").ok_or_else(invalid)?,
                arguments: value["filters"].clone(),
                options: Map::new(),
                after: Some(string("after").ok_or_else(invalid)?),
                offset: 0,
                limit: None,
                home: String::new(),
                text: text.trim().to_owned(),
            }),
            Some(2) => Ok(Self {
                kind: string("kind").ok_or_else(invalid)?,
                operation: string("op").ok_or_else(invalid)?,
                arguments: value["args"].clone(),
                options: value["options"].as_object().cloned().unwrap_or_default(),
                after: string("after"),
                offset: value["offset"]
                    .as_u64()
                    .and_then(|offset| usize::try_from(offset).ok())
                    .unwrap_or(0),
                limit: value["limit"]
                    .as_u64()
                    .and_then(|limit| usize::try_from(limit).ok()),
                home: string("home").unwrap_or_default(),
                text: text.trim().to_owned(),
            }),
            _ => Err(invalid()),
        }
    }

    fn check(&self, kind: &str, home: &Path) -> Outcome {
        if !self.kind.is_empty() && self.kind != kind {
            return Err(Failure::new(
                Reason::InvalidValue,
                format!(
                    "Can't use that --next cursor here. It continues bridge {}, not bridge {kind}.",
                    self.command_name()
                ),
            )
            .then("Continue it", self.continue_argv()));
        }
        if !self.home.is_empty() && self.home != home_tag(home) {
            return Err(Failure::new(
                Reason::InvalidValue,
                "Can't use that --next cursor here. It belongs to another home.",
            )
            .sentence("Pass the same --home as the read that printed it."));
        }
        Ok(())
    }

    fn command_name(&self) -> String {
        if self.kind == "search" {
            "search".to_owned()
        } else {
            format!("read {}", self.kind)
        }
    }

    fn continue_argv(&self) -> Vec<String> {
        let mut argv = vec!["bridge".to_owned()];
        argv.extend(self.command_name().split(' ').map(str::to_owned));
        argv.extend(["--next".to_owned(), self.text.clone()]);
        argv
    }
}

fn filters(arguments: &Value) -> Value {
    let mut copy = arguments.clone();
    if let Some(object) = copy.as_object_mut() {
        object.remove("limit");
        object.remove("after");
        object.remove("before");
    }
    if let Some(object) = copy.get_mut("args").and_then(Value::as_object_mut) {
        object.remove("limit");
        object.remove("cursor");
    }
    copy
}

fn inherit(
    option: &str,
    given: Option<&str>,
    cursor: Option<&Cursor>,
    key: &str,
) -> Outcome<Option<String>> {
    let carried = cursor.and_then(|cursor| cursor.options.get(key).and_then(Value::as_str));
    match (given, carried) {
        (Some(given), Some(carried)) if given != carried => Err(Failure::new(
            Reason::ConflictingOptions,
            format!(
                "Can't use {option} {} with that --next cursor. It continues {option} {}.",
                ui::clean(given),
                ui::clean(carried)
            ),
        )
        .sentence("Leave the option out to continue, or drop --next to start a new read.")),
        (Some(given), _) => Ok(Some(given.to_owned())),
        (None, carried) => Ok(carried.map(str::to_owned)),
    }
}

#[allow(clippy::too_many_lines)]
fn plan(args: &ReadArgs, limit: usize, cursor: Option<&Cursor>) -> Outcome<Plan> {
    let kind = args.kind.as_str();
    let mut issues = Vec::new();
    if args.kind != ReadKind::Messages {
        if args.query.is_some() {
            issues.push("--query works only with bridge read messages.");
        }
        if args.chat.is_some() {
            issues.push("--chat works only with bridge read messages.");
        }
    }
    if args.kind != ReadKind::Calendar && args.days.is_some() {
        issues.push("--days works only with bridge read calendar.");
    }
    if args.next.is_some() && args.all {
        issues.push("--next and --all can't be used together.");
    }
    if !issues.is_empty() {
        let mut failure = Failure::new(
            Reason::ConflictingOptions,
            if issues.len() == 1 {
                format!("Can't read {kind}. {}", issues[0])
            } else {
                format!("Can't read {kind}. {} options don't fit:", issues.len())
            },
        );
        if issues.len() > 1 {
            for issue in issues {
                failure = failure.issue(issue);
            }
        }
        return Err(failure);
    }
    let chat = inherit("--chat", args.chat.as_deref(), cursor, "chat")?;
    let query = inherit("--query", args.query.as_deref(), cursor, "query")?;
    let days_text = inherit("--days", args.days.as_deref(), cursor, "days")?;
    if chat.is_some() && query.is_some() {
        ui::warn(
            "ignored",
            "--query is ignored with --chat; it reads the whole chat.",
        );
    }
    let mut options = Map::new();
    let (operation, arguments, collections, source, subject): (
        &'static str,
        Value,
        &'static [&'static str],
        Option<&'static str>,
        Option<String>,
    ) = match args.kind {
        ReadKind::Photos => (
            "photos.search",
            json!({"args":{"limit":limit}}),
            &[],
            None,
            None,
        ),
        ReadKind::Messages => {
            if let Some(chat) = chat {
                options.insert("chat".into(), json!(chat));
                (
                    "messages.thread",
                    json!({"chat_id": chat, "limit": limit}),
                    &["messages.message"],
                    Some("messages"),
                    Some(format!("chat {chat}")),
                )
            } else if let Some(text) = query {
                options.insert("query".into(), json!(text));
                (
                    "messages.search",
                    json!({"text": text, "limit": limit}),
                    &["messages.message"],
                    Some("messages"),
                    None,
                )
            } else {
                (
                    "messages.chats",
                    json!({"limit": limit}),
                    &["messages.chat"],
                    Some("messages"),
                    None,
                )
            }
        }
        ReadKind::Chats => (
            "messages.chats",
            json!({"limit": limit}),
            &["messages.chat"],
            Some("messages"),
            None,
        ),
        ReadKind::Contacts => (
            "contacts.list",
            json!({}),
            &["contacts.contact"],
            Some("contacts"),
            None,
        ),
        ReadKind::Calendar => {
            let days = days_text
                .as_deref()
                .map(days)
                .transpose()?
                .unwrap_or(DEFAULT_DAYS);
            options.insert("days".into(), json!(days.to_string()));
            let arguments = if let Some(cursor) =
                cursor.filter(|cursor| cursor.operation == "calendar.agenda")
            {
                cursor.arguments.clone()
            } else {
                let now = jiff::Timestamp::now();
                let end = days
                    .checked_mul(24)
                    .and_then(|hours| jiff::Span::new().try_hours(hours).ok())
                    .and_then(|span| now.checked_add(span).ok())
                    .ok_or_else(|| Failure::invalid("Can't use that --days. It's too large."))?;
                json!({"start": now.to_string(), "end": end.to_string()})
            };
            (
                "calendar.agenda",
                arguments,
                &["calendar.event"],
                Some("calendar"),
                None,
            )
        }
        ReadKind::Reminders => {
            let arguments = cursor
                .filter(|cursor| cursor.operation == "reminders.items")
                .map_or_else(
                    || json!({"time_zone": time_zone(), "smart": "all"}),
                    |cursor| cursor.arguments.clone(),
                );
            (
                "reminders.items",
                arguments,
                &["reminders.reminder"],
                Some("reminders"),
                None,
            )
        }
        ReadKind::Notes => (
            "notes.list",
            json!({}),
            &["notes.note"],
            Some("notes"),
            None,
        ),
        ReadKind::Mail => (
            "records.query",
            json!({"collection": "mail.message", "limit": limit}),
            &["mail.message"],
            Some("mail"),
            None,
        ),
        ReadKind::Voicemail => (
            "records.query",
            json!({"collection": "voicemail.message", "limit": limit}),
            &["voicemail.message"],
            Some("voicemail"),
            None,
        ),
    };
    Ok(Plan {
        kind,
        operation,
        arguments,
        options,
        collections,
        source,
        subject,
    })
}

fn check_cursor(cursor: &Cursor, plan: &Plan) -> Outcome {
    let different = || {
        Failure::new(
            Reason::InvalidValue,
            "Can't use that --next cursor here. It belongs to a different read; repeat the same command and options.",
        )
    };
    if cursor.operation != plan.operation {
        return Err(different());
    }
    if filters(&cursor.arguments) != filters(&plan.arguments) {
        return Err(different());
    }
    Ok(())
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
        .then("Follow the sync", ["bridge", "stats", "--wait"])
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

struct Budget {
    started: Instant,
    limit: Duration,
}

impl Budget {
    fn remaining(&self) -> Option<Duration> {
        self.limit
            .checked_sub(self.started.elapsed())
            .filter(|left| !left.is_zero())
    }

    fn passed(&self) -> Failure {
        Failure::new(
            Reason::Timeout,
            format!(
                "Stopped reading after {}. --timeout passed before the last page.",
                super::commands::friendly(self.limit)
            ),
        )
        .sentence("Nothing was printed. Continue from the last page, or pass a longer --timeout.")
    }

    async fn read(
        &self,
        session: &Session,
        operation: &str,
        arguments: &Value,
        subject: Option<&str>,
    ) -> Outcome<Value> {
        let Some(left) = self.remaining() else {
            return Err(self.passed());
        };
        tokio::time::timeout(left, session.read(operation, arguments, subject))
            .await
            .unwrap_or_else(|_| Err(self.passed()))
    }
}

struct Gathered {
    answer: Value,
    skip: usize,
    total: Option<usize>,
    after: Option<String>,
    offset: Option<usize>,
}

struct Progress<'a> {
    plan: &'a Plan,
    start: Option<&'a Cursor>,
    rows: usize,
    pages: usize,
}

impl Progress<'_> {
    fn annotate(&self, failure: Failure) -> Failure {
        let resume = self.start.map(|cursor| cursor.text.clone());
        let mut failure = failure
            .data("rowsFetched", self.rows)
            .data("pages", self.pages)
            .data(
                "cursor",
                self.start.map_or(Value::Null, |cursor| json!(cursor.text)),
            )
            .data("resume", resume.clone().map_or(Value::Null, Value::from))
            .row(
                "Fetched",
                format!("{} rows on {} pages, not printed", self.rows, self.pages),
            );
        if let Some(resume) = resume {
            failure = failure.step(next_command(self.plan.kind, &resume));
        } else {
            failure = failure.sentence("Run the same read again to start from the first page.");
        }
        failure
    }
}

fn stalled(progress: &Progress<'_>, why: &str) -> Failure {
    progress.annotate(
        Failure::new(
            Reason::PaginationStalled,
            format!(
                "Stopped reading {}. Apple Bridge {why}, so more pages wouldn't help.",
                progress.plan.operation
            ),
        )
        .sentence("This is a fault in the gateway's paging; nothing was changed."),
    )
}

fn merge(answer: &mut Option<Value>, page: Value, pager: Pager) {
    let Some(whole) = answer else {
        *answer = Some(page);
        return;
    };
    let page = pager.body(&page).clone();
    let current = pager.body_mut(whole);
    if pager.nested
        && let (Some(before), Some(added)) = (
            current["examined_count"].as_f64(),
            page["examined_count"].as_f64(),
        )
    {
        current["examined_count"] = json!(before + added);
    }
    let extra = page[pager.items].as_array().cloned().unwrap_or_default();
    if let Some(items) = current[pager.items].as_array_mut() {
        if pager.older_first {
            let mut combined = extra;
            combined.append(items);
            *items = combined;
        } else {
            items.extend(extra);
        }
    } else {
        current[pager.items] = Value::Array(extra);
    }
    current[pager.next] = page[pager.next].clone();
}

async fn gather_pages(
    session: &Session,
    plan: &Plan,
    pager: Pager,
    wanted: usize,
    start: Option<&Cursor>,
    budget: &Budget,
) -> Outcome<Gathered> {
    let mut after = start.and_then(|cursor| cursor.after.clone());
    let mut consumed: BTreeSet<String> = after.iter().cloned().collect();
    let mut answer = None;
    let mut progress = Progress {
        plan,
        start,
        rows: 0,
        pages: 0,
    };
    let mut empty = 0;
    let mut next: Option<String>;
    loop {
        let mut arguments = plan.arguments.clone();
        let target = pager.arguments(&mut arguments);
        if let Some(after) = &after {
            target[pager.argument] = json!(after);
        }
        if target.get("limit").is_some() {
            target["limit"] = json!((wanted - progress.rows).clamp(1, LIMIT_CAP));
        }
        let page = match budget
            .read(session, plan.operation, &arguments, plan.subject.as_deref())
            .await
        {
            Ok(page) => page,
            Err(failure) if progress.pages == 0 && start.is_none() => return Err(failure),
            Err(failure) => return Err(progress.annotate(failure)),
        };
        progress.pages += 1;
        let count = pager.body(&page)[pager.items]
            .as_array()
            .map_or(0, Vec::len);
        next = pager.body(&page)[pager.next]
            .as_str()
            .filter(|token| !token.is_empty())
            .map(str::to_owned);
        merge(&mut answer, page, pager);
        progress.rows += count;
        empty = if count == 0 { empty + 1 } else { 0 };
        let Some(token) = next.clone() else {
            break;
        };
        if !consumed.insert(token.clone()) {
            return Err(stalled(
                &progress,
                "returned a page cursor it had already given",
            ));
        }
        if progress.rows >= wanted {
            break;
        }
        if empty >= EMPTY_PAGES && !pager.nested {
            return Err(stalled(
                &progress,
                &format!("returned {EMPTY_PAGES} empty pages in a row that each claimed more"),
            ));
        }
        if progress.pages >= MAX_PAGES {
            ui::warn(
                "truncated",
                &format!(
                    "Stopped after {MAX_PAGES} pages with {} items. Continue with --next.",
                    progress.rows
                ),
            );
            break;
        }
        if budget.remaining().is_none() {
            return Err(progress.annotate(budget.passed()));
        }
        after = Some(token);
    }
    if progress.rows >= ALL_CAP && next.is_some() {
        ui::warn(
            "truncated",
            &format!("Stopped at {ALL_CAP} items. Continue with --next."),
        );
    }
    Ok(Gathered {
        answer: answer.unwrap_or_else(|| json!({})),
        skip: 0,
        total: None,
        after: next,
        offset: None,
    })
}

async fn gather(
    session: &Session,
    plan: &Plan,
    wanted: usize,
    start: Option<&Cursor>,
    budget: &Budget,
) -> Outcome<Gathered> {
    if let Some(pager) = pager(plan.operation) {
        return gather_pages(session, plan, pager, wanted, start, budget).await;
    }
    let answer = budget
        .read(
            session,
            plan.operation,
            &plan.arguments,
            plan.subject.as_deref(),
        )
        .await?;
    let mut records = Vec::new();
    collect(&answer, plan.collections, &mut records);
    let total = records.len();
    let skip = start.map_or(0, |cursor| cursor.offset).min(total);
    let end = skip.saturating_add(wanted);
    Ok(Gathered {
        answer,
        skip,
        total: Some(total),
        after: None,
        offset: (total > end).then_some(end),
    })
}

fn continuation(plan: &Plan, gathered: &Gathered, limit: usize, home: &Path) -> Option<String> {
    if gathered.after.is_none() && gathered.offset.is_none() {
        return None;
    }
    Some(
        Cursor {
            kind: plan.kind.to_owned(),
            operation: plan.operation.to_owned(),
            arguments: filters(&plan.arguments),
            options: plan.options.clone(),
            after: gathered.after.clone(),
            offset: gathered.offset.unwrap_or(0),
            limit: Some(limit),
            home: home_tag(home),
            text: String::new(),
        }
        .encode(),
    )
}

fn window(value: &mut Value, wanted: &[&str], skip: &mut usize, take: &mut usize) {
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
                    if *skip > 0 {
                        let mut inside = Vec::new();
                        collect(item, wanted, &mut inside);
                        *skip = skip.saturating_sub(inside.len().max(1));
                        return false;
                    }
                    if *take == 0 {
                        return false;
                    }
                    *take -= 1;
                }
                window(item, wanted, skip, take);
                true
            });
        }
        Value::Object(fields) => {
            for child in fields.values_mut() {
                window(child, wanted, skip, take);
            }
        }
        _ => {}
    }
}

fn shape_fields(collections: &[&str]) -> Option<BTreeSet<String>> {
    let shapes: Value = serde_json::from_str(include_str!("../record_shapes.json")).ok()?;
    let mut names = BTreeSet::new();
    for collection in collections {
        names.extend(shapes[*collection].as_object()?.keys().cloned());
    }
    if collections.is_empty() {
        for fields in shapes.as_object()?.values().filter_map(Value::as_object) {
            names.extend(fields.keys().cloned());
        }
    }
    Some(names)
}

fn check_record_fields(collections: &[&str], command: &str) -> Outcome {
    let Some(fields) = &Modes::get().fields else {
        return Ok(());
    };
    let Some(known) = shape_fields(collections) else {
        return Ok(());
    };
    let unknown: Vec<String> = fields
        .iter()
        .filter(|field| !known.contains(*field) && !["id", "collection"].contains(&field.as_str()))
        .map(|field| ui::clean(field))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(Failure::new(
        Reason::InvalidValue,
        format!(
            "Can't use --fields {} with {command}. Record fields here: {}.",
            unknown.join(","),
            known.iter().cloned().collect::<Vec<_>>().join(", ")
        ),
    ))
}

fn select_record_fields(value: &mut Value, collections: &[&str]) {
    let Some(fields) = &Modes::get().fields else {
        return;
    };
    match value {
        Value::Object(map) => {
            let is_record =
                map.get("collection")
                    .and_then(Value::as_str)
                    .is_some_and(|collection| {
                        collections.is_empty() || collections.contains(&collection)
                    })
                    && map.contains_key("data");
            if is_record {
                if let Some(Value::Object(data)) = map.get_mut("data") {
                    data.retain(|key, _| fields.iter().any(|field| field == key));
                }
                return;
            }
            for child in map.values_mut() {
                select_record_fields(child, collections);
            }
        }
        Value::Array(items) => {
            for item in items {
                select_record_fields(item, collections);
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
    if !Modes::get().stdout_table() || joined.chars().count() <= limit {
        return joined;
    }
    let mut short: String = joined.chars().take(limit).collect();
    short.push('…');
    short
}

const CHAT_COLUMNS: &[&str] = &["When", "Chat", "Last message", "ID"];
const RECORD_COLUMNS: &[&str] = &["When", "Title", "ID"];
const VOICEMAIL_COLUMNS: &[&str] = &["When", "Title", "ID", "Transcript", "Audio"];

fn chat_rows(chats: &[Value], take: usize) -> Vec<Vec<String>> {
    chats
        .iter()
        .take(take)
        .map(|entry| {
            let chat = &entry["chat"];
            let name = entry["title"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
                .or_else(|| text_field(&chat["data"], HEADLINE_FIELDS))
                .unwrap_or("(unnamed chat)");
            vec![
                ui::clean(entry["last_activity_at"].as_str().unwrap_or_default()),
                one_line(name, 40),
                one_line(
                    entry["last_message"]["preview"]["text"]
                        .as_str()
                        .unwrap_or_default(),
                    60,
                ),
                ui::clean(chat["id"].as_str().unwrap_or_default()),
            ]
        })
        .collect()
}

fn record_row(record: &Value, voicemail: bool) -> Vec<String> {
    let data = &record["data"];
    let headline = text_field(data, HEADLINE_FIELDS)
        .or_else(|| data["sender"].as_str())
        .map(str::to_owned)
        .or_else(|| contact_name(data))
        .unwrap_or_else(|| "(no title)".to_owned());
    let mut row = vec![
        ui::clean(text_field(data, WHEN_FIELDS).unwrap_or_default()),
        one_line(&headline, 80),
        ui::clean(record["id"].as_str().unwrap_or_default()),
    ];
    if voicemail {
        row.push(one_line(
            data["transcript"]["text"]
                .as_str()
                .or_else(|| data["transcript_state"].as_str())
                .unwrap_or("unavailable"),
            80,
        ));
        row.push(data["audio"]["digest"].as_str().map_or_else(
            || ui::clean(data["audio_state"].as_str().unwrap_or("unavailable")),
            |digest| {
                format!(
                    "{} ({}, {} bytes)",
                    ui::clean(digest),
                    ui::clean(data["audio"]["mime_type"].as_str().unwrap_or("unknown")),
                    data["audio"]["size"]
                )
            },
        ));
    }
    row
}

fn print_rows(headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() && Modes::get().stdout_table() {
        return;
    }
    ui::table(headers, rows);
}

fn render(plan: &Plan, gathered: &Gathered, take: usize) -> usize {
    let answer = &gathered.answer;
    if plan.operation == "messages.chats" {
        let chats = answer["chats"].as_array().cloned().unwrap_or_default();
        let rows = chat_rows(&chats, take);
        print_rows(CHAT_COLUMNS, &rows);
        return rows.len();
    }
    let mut records = Vec::new();
    collect(answer, plan.collections, &mut records);
    let voicemail = plan.collections.contains(&"voicemail.message");
    let rows: Vec<Vec<String>> = records
        .iter()
        .skip(gathered.skip)
        .take(take)
        .map(|record| record_row(record, voicemail))
        .collect();
    print_rows(
        if voicemail {
            VOICEMAIL_COLUMNS
        } else {
            RECORD_COLUMNS
        },
        &rows,
    );
    if let Some(total) = gathered.total
        && gathered.offset.is_some()
    {
        ui::note(&format!(
            "Showing {} to {} of {total}.",
            gathered.skip + 1,
            gathered.skip + rows.len()
        ));
    }
    rows.len()
}

fn next_command(kind: &str, cursor: &str) -> Step {
    let mut argv = vec!["bridge".to_owned()];
    if kind == "search" {
        argv.push("search".to_owned());
    } else {
        argv.extend(["read".to_owned(), kind.to_owned()]);
    }
    argv.extend(["--next".to_owned(), cursor.to_owned()]);
    Step::new("See the next page", argv)
}

struct Started {
    cursor: Option<Cursor>,
    limit: usize,
    wanted: usize,
    budget: Budget,
}

fn start(
    limit_text: Option<&str>,
    next: Option<&str>,
    all: bool,
    timeout: &str,
    kind: &str,
    home: &Path,
) -> Outcome<Started> {
    let cursor = next.map(Cursor::decode).transpose()?;
    if let Some(cursor) = &cursor {
        cursor.check(kind, home)?;
    }
    let limit = match limit_text {
        Some(text) => limit(text)?,
        None => cursor
            .as_ref()
            .and_then(|cursor| cursor.limit)
            .unwrap_or(DEFAULT_LIMIT),
    };
    let budget = Budget {
        started: Instant::now(),
        limit: super::commands::duration("--timeout", timeout)?,
    };
    Ok(Started {
        cursor,
        limit,
        wanted: if all { ALL_CAP } else { limit },
        budget,
    })
}

pub async fn read(home: &Path, args: &ReadArgs) -> Outcome {
    let started = start(
        args.limit.as_deref(),
        args.next.as_deref(),
        args.all,
        &args.timeout,
        args.kind.as_str(),
        home,
    )?;
    let page = started.wanted.min(LIMIT_CAP);
    let plan = plan(args, page, started.cursor.as_ref())?;
    if let Some(cursor) = &started.cursor {
        check_cursor(cursor, &plan)?;
    }
    check_record_fields(plan.collections, "bridge read")?;
    let session = Session::open(home).await?;
    let result = gather(
        &session,
        &plan,
        started.wanted,
        started.cursor.as_ref(),
        &started.budget,
    )
    .await;
    drop(session);
    let gathered = result?;
    let mut records = Vec::new();
    collect(&gathered.answer, plan.collections, &mut records);
    let empty = records.len() <= gathered.skip
        && gathered.answer["chats"]
            .as_array()
            .is_none_or(Vec::is_empty);
    if let Some(source) = plan.source {
        judge(args.kind.as_str(), source, &coverage(home, source)?, empty)?;
    }
    let cursor = continuation(&plan, &gathered, started.limit, home);
    if plan.operation == "photos.search" {
        return print_photos(gathered.answer, cursor);
    }
    if Modes::get().json {
        let mut answer = gathered.answer;
        window(
            &mut answer,
            plan.collections,
            &mut gathered.skip.clone(),
            &mut started.wanted.clone(),
        );
        select_record_fields(&mut answer, plan.collections);
        answer["next_cursor"] = cursor.map_or(Value::Null, Value::from);
        ui::json_document_unfiltered(answer);
        return Ok(());
    }
    render(&plan, &gathered, started.wanted);
    if let Some(cursor) = cursor {
        ui::next_steps(&[next_command(args.kind.as_str(), &cursor)]);
    } else if plan.operation == "messages.chats" && !empty {
        ui::next_steps(&[Step::new(
            "Read one chat",
            ["bridge", "read", "messages", "--chat", "<CHAT>"],
        )]);
    }
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn print_photos(mut answer: Value, cursor: Option<String>) -> Outcome {
    if Modes::get().json {
        answer["next_cursor"] = cursor.map_or(Value::Null, Value::from);
        ui::json_document(answer);
        return Ok(());
    }
    ui::outln(&serde_json::to_string_pretty(&answer["result_json"])?);
    if let Some(cursor) = cursor {
        ui::note("More photos match.");
        ui::next_steps(&[next_command("photos", &cursor)]);
    }
    Ok(())
}

pub async fn search(home: &Path, args: &SearchArgs) -> Outcome {
    if args.next.is_some() && args.all {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't use --next and --all together.",
        ));
    }
    let started = start(
        args.limit.as_deref(),
        args.next.as_deref(),
        args.all,
        &args.timeout,
        "search",
        home,
    )?;
    let text = inherit(
        "TEXT",
        args.text.as_deref(),
        started.cursor.as_ref(),
        "text",
    )?
    .filter(|text| !text.trim().is_empty())
    .ok_or_else(|| {
        Failure::new(Reason::MissingArgument, "Can't search for empty text.")
            .then("Search for a word", ["bridge", "search", "<TEXT>"])
    })?;
    let mut options = Map::new();
    options.insert("text".into(), json!(text));
    let plan = Plan {
        kind: "search",
        operation: "search",
        arguments: json!({"text": text, "limit": started.wanted.min(LIMIT_CAP), "collection": ""}),
        options,
        collections: &[],
        source: None,
        subject: None,
    };
    if let Some(cursor) = &started.cursor {
        check_cursor(cursor, &plan)?;
    }
    check_record_fields(&[], "bridge search")?;
    if stats::open(home)?.is_none() {
        return Err(super::commands::not_set_up(home));
    }
    let session = Session::open(home).await?;
    let result = gather(
        &session,
        &plan,
        started.wanted,
        started.cursor.as_ref(),
        &started.budget,
    )
    .await;
    drop(session);
    let gathered = result?;
    let cursor = continuation(&plan, &gathered, started.limit, home);
    if Modes::get().json {
        let mut answer = gathered.answer;
        select_record_fields(&mut answer, &[]);
        answer["next_cursor"] = cursor.map_or(Value::Null, Value::from);
        ui::json_document_unfiltered(answer);
        return Ok(());
    }
    if render(&plan, &gathered, started.wanted) == 0 {
        ui::note(&format!(
            "No results for \"{}\" in what has synced.",
            ui::clean(&text)
        ));
        ui::next_steps(&[Step::new("See what has synced", ["bridge", "stats"])]);
    }
    if let Some(cursor) = cursor {
        ui::next_steps(&[next_command("search", &cursor)]);
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
                descriptor.description.clone(),
            ]
        })
        .collect();
    ui::table(&["Operation", "Scope", "Description"], &rows);
    Ok(())
}

async fn read_stdin(what: &str) -> Outcome<String> {
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
    tokio::task::spawn_blocking(|| {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        Ok(text)
    })
    .await
    .map_err(|error| Failure::internal(&error))?
}

async fn arguments(args: &CallArgs) -> Outcome<(Value, &'static str)> {
    if args.data.is_some() && args.file.is_some() {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't combine --data and --file. Pass the arguments once.",
        ));
    }
    let (text, option) = if let Some(data) = &args.data {
        (
            if data == "-" {
                read_stdin("--data").await?
            } else {
                data.clone()
            },
            "--data",
        )
    } else if let Some(file) = &args.file {
        if file.as_os_str() == "-" {
            (read_stdin("--file").await?, "--file")
        } else {
            let path = file.clone();
            let text = tokio::task::spawn_blocking(move || std::fs::read_to_string(path))
                .await
                .map_err(|error| Failure::internal(&error))?
                .map_err(|_| {
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

fn uncertain(
    operation: &str,
    arguments: &Value,
    confirmed: &str,
    detail: String,
    token_env: &str,
) -> Failure {
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
    let mut failure = Failure::new(
        Reason::UnknownOutcome,
        format!("Can't confirm whether {operation} took effect."),
    );
    if !id.is_empty() {
        failure = failure
            .row("Operation ID", id.clone())
            .data("operationId", id);
    }
    failure
        .row("Confirmed", confirmed)
        .data("confirmed", confirmed)
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
    let Some(operation) = args.describe.as_ref().or(args.operation.as_ref()) else {
        return Err(Failure::new(
            Reason::MissingArgument,
            "Can't call without an <OPERATION>.",
        )
        .then("Check the sources", ["bridge", "call", "sources.status"])
        .then("List the operations", ["bridge", "call", "--list"]));
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
    if args.describe.is_some() {
        ui::json_document_unfiltered(serde_json::to_value(&descriptor)?);
        return Ok(());
    }
    let photos_write = descriptor.scope == "photos:write";
    let once = photos_write || matches!(operation.as_str(), "photos.fetch" | "photos.release");
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
    let (mut arguments, option) = arguments(args).await?;
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
        if once {
            match session.send(operation, &arguments, None).await {
                Sent::Answer(answer) => answer,
                Sent::Refused(failure, _) if failure.reason == Reason::UnknownOutcome => {
                    let detail = failure
                        .remote
                        .as_ref()
                        .map(|(_, text)| text.clone())
                        .unwrap_or_default();
                    return Err(uncertain(
                        operation,
                        &arguments,
                        "Apple Bridge received the request and started it, but couldn't confirm the result",
                        detail,
                        name,
                    ));
                }
                Sent::Refused(failure, _) => return Err(failure),
                Sent::Unknown(detail) => {
                    return Err(uncertain(
                        operation,
                        &arguments,
                        "the request was sent; no answer came back",
                        detail,
                        name,
                    ));
                }
            }
        } else {
            session.read(operation, &arguments, None).await?
        }
    } else {
        let session = Session::open(home).await?;
        let result = session.read(operation, &arguments, None).await;
        drop(session);
        result?
    };
    ui::outln(&answer.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn canceling_a_read_joins_the_task_and_revokes_its_temporary_token() {
        let home = tempfile::tempdir().expect("temporary home");
        let session = Session::temporary(home.path(), "127.0.0.1:1".into()).expect("session");
        let authority = Authority::open(home.path()).expect("authority");
        let token = session.token.clone();
        assert!(authority.authenticate_token(&token).is_ok());
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _session = session;
            started.send(()).expect("start signal");
            std::future::pending::<Outcome>().await
        });
        ready.await.expect("read started");
        super::super::cancel_task(task).await;
        assert!(authority.authenticate_token(&token).is_err());
    }

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
            window(&mut answer, &["reminders.reminder"], &mut 0, &mut remaining);
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
                window(&mut answer, &[collection], &mut 0, &mut remaining);
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
    fn cursors_round_trip_and_carry_what_the_read_needs() -> Result<(), Box<dyn std::error::Error>>
    {
        let home = std::path::Path::new("/tmp");
        let cursor = Cursor {
            kind: "calendar".into(),
            operation: "calendar.agenda".into(),
            arguments: json!({"start": "2026-01-01T00:00:00Z", "end": "2026-02-01T00:00:00Z"}),
            options: Map::from_iter([("days".to_owned(), json!("31"))]),
            after: None,
            offset: 10,
            limit: Some(10),
            home: home_tag(home),
            text: String::new(),
        };
        let text = cursor.encode();
        let decoded = Cursor::decode(&text).map_err(|f| f.headline.clone())?;
        assert_eq!(decoded.arguments, cursor.arguments);
        assert_eq!(decoded.offset, 10);
        assert!(decoded.check("calendar", home).is_ok());
        assert!(decoded.check("notes", home).is_err());
        assert!(
            decoded
                .check("calendar", std::path::Path::new("/var"))
                .is_err()
        );
        assert!(Cursor::decode("garbage!").is_err());
        let legacy = BASE64URL_NOPAD.encode(
            json!({"v": 1, "op": "messages.search", "filters": {"text": "hi"}, "after": "t"})
                .to_string()
                .as_bytes(),
        );
        assert_eq!(
            Cursor::decode(&legacy)
                .map_err(|f| f.headline.clone())?
                .after
                .as_deref(),
            Some("t")
        );
        Ok(())
    }

    #[test]
    fn omitted_options_come_from_the_cursor_and_conflicts_are_refused() {
        let cursor = Cursor {
            kind: "messages".into(),
            operation: "messages.thread".into(),
            arguments: json!({"chat_id": "c1"}),
            options: Map::from_iter([("chat".to_owned(), json!("c1"))]),
            after: Some("t".into()),
            offset: 0,
            limit: None,
            home: String::new(),
            text: String::new(),
        };
        assert_eq!(
            inherit("--chat", None, Some(&cursor), "chat")
                .ok()
                .flatten()
                .as_deref(),
            Some("c1")
        );
        assert!(inherit("--chat", Some("c1"), Some(&cursor), "chat").is_ok());
        assert!(inherit("--chat", Some("c2"), Some(&cursor), "chat").is_err());
    }

    #[test]
    fn a_window_skips_and_takes_records_in_order() {
        let mut answer = json!({"records": [
            {"collection": "notes.note", "id": "a", "data": {}},
            {"collection": "notes.note", "id": "b", "data": {}},
            {"collection": "notes.note", "id": "c", "data": {}},
        ]});
        window(&mut answer, &["notes.note"], &mut 1, &mut 1);
        assert_eq!(answer["records"].as_array().map(Vec::len), Some(1));
        assert_eq!(answer["records"][0]["id"], "b");
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
        let expected = if Modes::get().stdout_table() {
            "abc…"
        } else {
            "abcdef"
        };
        assert_eq!(one_line("abcdef", 3), expected);
    }
}
