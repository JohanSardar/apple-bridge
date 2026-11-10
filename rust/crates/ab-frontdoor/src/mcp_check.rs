use ab_catalog::AgentOperation;
use ab_operations::{Code, Error, Principal};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Default)]
pub(crate) struct Problem(Box<Details>);

#[derive(Debug, Clone, Default)]
pub(crate) struct Details {
    pub code: String,
    pub message: String,
    pub operation: Option<String>,
    pub field: Option<String>,
    pub issue: Option<String>,
    pub expected: Option<Value>,
    pub example: Option<Value>,
    pub hint: Option<String>,
    pub retryable: bool,
    pub current: Option<Value>,
    pub candidates: Option<Value>,
}

impl std::ops::Deref for Problem {
    type Target = Details;
    fn deref(&self) -> &Details {
        &self.0
    }
}

impl std::ops::DerefMut for Problem {
    fn deref_mut(&mut self) -> &mut Details {
        &mut self.0
    }
}

impl Problem {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self(Box::new(Details {
            code: code.into(),
            message: message.into(),
            ..Details::default()
        }))
    }
    pub fn operation(mut self, name: &str) -> Self {
        self.operation = Some(name.into());
        self
    }
    pub fn field(mut self, name: impl Into<String>) -> Self {
        self.field = Some(name.into());
        self
    }
    pub fn problem(mut self, text: impl Into<String>) -> Self {
        self.issue = Some(text.into());
        self
    }
    pub fn expected(mut self, value: Value) -> Self {
        self.expected = Some(value);
        self
    }
    pub fn example(mut self, value: Value) -> Self {
        self.example = Some(value);
        self
    }
    pub fn hint(mut self, text: impl Into<String>) -> Self {
        self.hint = Some(text.into());
        self
    }
    pub fn candidates(mut self, value: Value) -> Self {
        self.candidates = Some(value);
        self
    }
    pub fn to_json(&self) -> Value {
        let mut error = Map::new();
        error.insert("code".into(), json!(self.code));
        error.insert("message".into(), json!(self.message));
        let optional = [
            ("operation", self.operation.clone().map(Value::String)),
            ("field", self.field.clone().map(Value::String)),
            ("problem", self.issue.clone().map(Value::String)),
            ("expected", self.expected.clone()),
            ("example", self.example.clone()),
            ("hint", self.hint.clone().map(Value::String)),
            ("current", self.current.clone()),
            ("candidates", self.candidates.clone()),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                error.insert(key.into(), value);
            }
        }
        error.insert("retryable".into(), json!(self.retryable));
        json!({ "error": error })
    }
}

pub(crate) fn example_call(tool: &str, operation: &AgentOperation) -> Value {
    if tool == "apple_read" || tool == "apple_change" {
        json!({"tool": tool, "arguments": {"operation": operation.name, "arguments": operation.example}})
    } else {
        json!({"tool": tool, "arguments": operation.example})
    }
}

pub(crate) fn tool_for(operation: &AgentOperation) -> &'static str {
    if operation.changes && !operation.read_only() {
        "apple_change"
    } else {
        "apple_read"
    }
}

pub(crate) fn mac_zone() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_owned()
}

pub(crate) fn arguments(
    operation: &AgentOperation,
    given: Option<&Value>,
) -> Result<Map<String, Value>, Problem> {
    let tool = tool_for(operation);
    let mut fields = match given {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(fields)) => fields.clone(),
        Some(other) => {
            return Err(Problem::new("invalid", "arguments must be an object")
                .operation(&operation.name)
                .field("arguments")
                .problem(format!("got {}", kind(other)))
                .expected(json!("an object of the operation's arguments"))
                .example(example_call(tool, operation)));
        }
    };
    let properties = operation.arguments["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if !properties.contains_key("args")
        && let Some(Value::Object(inner)) = fields.remove("args")
    {
        let outer = std::mem::take(&mut fields);
        fields = inner;
        if let Some(version) = outer.get("expected_version") {
            fields.insert("expected_version".into(), version.clone());
        }
    }
    if operation.kind != ab_catalog::OperationKind::Command
        && properties.contains_key("time_zone")
        && !fields.contains_key("time_zone")
    {
        fields.insert("time_zone".into(), Value::String(mac_zone()));
    }
    let schema = &operation.arguments;
    check_object(&Value::Object(fields.clone()), schema, "", operation, tool)?;
    if operation.version == ab_catalog::VersionRule::Required
        && fields.contains_key("id")
        && !fields.contains_key("expected_version")
    {
        return Err(Problem::new(
            "invalid",
            format!("{} has a problem with expected_version", operation.name),
        )
        .operation(&operation.name)
        .field("expected_version")
        .problem("expected_version is required with id")
        .example(example_call(tool, operation))
        .hint(format!(
            "Read the record first ({}) and pass its version field as expected_version.",
            lookup_hint(operation)
        )));
    }
    Ok(fields)
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn check_object(
    value: &Value,
    schema: &Value,
    path: &str,
    operation: &AgentOperation,
    tool: &str,
) -> Result<(), Problem> {
    let Some(fields) = value.as_object() else {
        return Ok(());
    };
    let properties = schema["properties"].as_object();
    let closed = schema["additionalProperties"] == false;
    let invalid = |field: &str| {
        Problem::new(
            "invalid",
            format!("{} has a problem with {field}", operation.name),
        )
        .operation(&operation.name)
        .field(field.to_owned())
        .example(example_call(tool, operation))
    };
    if let Some(properties) = properties {
        for (name, child) in fields {
            let here = join(path, name);
            let Some(child_schema) = properties.get(name) else {
                if closed {
                    let valid: Vec<&String> = properties.keys().collect();
                    return Err(invalid(&here)
                        .problem(format!("{here} is not an argument of {}", operation.name))
                        .expected(json!({ "valid_fields": valid }))
                        .hint(format!(
                            "Remove {here}. apple_describe with operation {} shows every argument.",
                            operation.name
                        )));
                }
                continue;
            };
            check_value(child, child_schema, &here, operation, tool)?;
        }
    }
    for required in schema["required"].as_array().into_iter().flatten() {
        let Some(name) = required.as_str() else {
            continue;
        };
        if !fields.contains_key(name) {
            let here = join(path, name);
            let description = properties
                .and_then(|p| p.get(name))
                .and_then(|s| s["description"].as_str())
                .unwrap_or("required");
            let mut problem = invalid(&here)
                .problem(format!("{here} is required"))
                .expected(json!(description));
            if name == "expected_version" {
                problem = problem.hint(format!(
                    "Read the record first ({}) and pass its version field as expected_version.",
                    lookup_hint(operation)
                ));
            }
            return Err(problem);
        }
    }
    Ok(())
}

fn lookup_hint(operation: &AgentOperation) -> String {
    let description = operation.arguments["properties"]["id"]["description"]
        .as_str()
        .unwrap_or_default();
    description.find(" from ").map_or_else(
        || "records.get with the record's collection and id".to_owned(),
        |at| description[at + 6..].trim_end_matches('.').to_owned(),
    )
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path}.{name}")
    }
}

#[allow(clippy::too_many_lines)]
fn check_value(
    value: &Value,
    schema: &Value,
    path: &str,
    operation: &AgentOperation,
    tool: &str,
) -> Result<(), Problem> {
    let description = schema["description"].as_str().unwrap_or_default();
    let fail = |problem: String| {
        Problem::new(
            "invalid",
            format!("{} has a problem with {path}", operation.name),
        )
        .operation(&operation.name)
        .field(path.to_owned())
        .problem(problem)
        .expected(json!(if description.is_empty() {
            schema["type"].as_str().unwrap_or("a value")
        } else {
            description
        }))
        .example(example_call(tool, operation))
    };
    let actual = kind(value);
    if let Some(expected) = schema["type"].as_str() {
        let fits = match expected {
            "number" => value.is_number(),
            "integer" => actual == "integer" || value.as_f64().is_some_and(|n| n.fract() == 0.0),
            other => actual == other,
        };
        if !fits {
            return Err(fail(format!("{path} must be {expected}, got {actual}")));
        }
    }
    if let Some(options) = schema["enum"].as_array()
        && !options.contains(value)
    {
        return Err(fail(format!("{path} must be one of the listed values"))
            .expected(json!({ "one_of": options })));
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema["minimum"].as_f64()
            && number < minimum
        {
            return Err(fail(format!("{path} must be at least {minimum}")));
        }
        if let Some(maximum) = schema["maximum"].as_f64()
            && number > maximum
        {
            return Err(fail(format!("{path} must be at most {maximum}")));
        }
        if let Some(minimum) = schema["exclusiveMinimum"].as_f64()
            && number <= minimum
        {
            return Err(fail(format!("{path} must be more than {minimum}")));
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count();
        if let Some(minimum) = schema["minLength"].as_u64()
            && (length as u64) < minimum
        {
            return Err(fail(format!("{path} must not be empty")));
        }
        if let Some(maximum) = schema["maxLength"].as_u64()
            && (length as u64) > maximum
        {
            return Err(fail(format!("{path} is longer than {maximum} characters")));
        }
        if let Some(pattern) = schema["pattern"].as_str()
            && pattern.starts_with('^')
            && !pattern.contains(['(', '[', '{', '*', '+', '?', '|', '\\'])
            && !text.starts_with(&pattern[1..])
        {
            return Err(fail(format!("{path} must start with {}", &pattern[1..])));
        }
        if operation.domain != TOOL_DOMAIN {
            check_date(text, path, description).map_err(fail)?;
        }
        if path.ends_with("time_zone")
            && !text.is_empty()
            && text != "Local"
            && jiff::tz::TimeZone::get(text).is_err()
        {
            return Err(fail(format!("{text} is not an IANA time zone"))
                .expected(json!(
                    "an IANA time zone such as Europe/Ljubljana or America/Chicago"
                ))
                .hint(format!("The Mac's time zone is {}.", mac_zone())));
        }
    }
    if let Some(items) = value.as_array() {
        if let Some(maximum) = schema["maxItems"].as_u64()
            && items.len() as u64 > maximum
        {
            return Err(fail(format!("{path} has more than {maximum} items")));
        }
        if let Some(minimum) = schema["minItems"].as_u64()
            && (items.len() as u64) < minimum
        {
            return Err(fail(format!("{path} needs at least {minimum} items")));
        }
        if let Some(item) = schema.get("items") {
            for (index, entry) in items.iter().enumerate() {
                check_value(entry, item, &format!("{path}[{index}]"), operation, tool)?;
            }
        }
    }
    if value.is_object() && schema.get("properties").is_some() {
        check_object(value, schema, path, operation, tool)?;
    }
    Ok(())
}

fn check_date(text: &str, path: &str, description: &str) -> Result<(), String> {
    let timestamp = description.contains("RFC 3339");
    let date = description.contains("YYYY-MM-DD");
    if !(timestamp || date) {
        return Ok(());
    }
    if text.is_empty() && description.contains("empty string clears") {
        return Ok(());
    }
    if timestamp && text.parse::<jiff::Timestamp>().is_ok() {
        return Ok(());
    }
    if date && text.len() == 10 && text.parse::<jiff::civil::Date>().is_ok() {
        return Ok(());
    }
    let forms = match (timestamp, date) {
        (true, true) => {
            "an RFC 3339 timestamp with an offset, such as 2026-11-12T09:00:00-06:00, or a date such as 2026-11-12"
        }
        (true, false) => "an RFC 3339 timestamp with an offset, such as 2026-11-12T09:00:00-06:00",
        _ => "a date such as 2026-11-12",
    };
    Err(format!("{path} must be {forms}; got {text:?}"))
}

pub(crate) fn from_error(
    error: &Error,
    operation: Option<&AgentOperation>,
    principal: &Principal,
) -> Problem {
    let code = serde_json::to_value(&error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "failed".into());
    let mut problem = Problem::new(&code, error.message.clone());
    problem.retryable = error.retryable;
    if let Some(operation) = operation {
        problem = problem.operation(&operation.name);
    }
    problem = match error.code {
        Code::Unauthorized | Code::Forbidden => access_problem(problem, operation, principal),
        Code::Invalid => invalid_problem(problem, operation),
        Code::Conflict => conflict_problem(problem, error),
        Code::NotFound => problem
            .problem("no visible record has that ID")
            .hint(operation.map_or_else(
                || "Get IDs from a read; never make them up.".to_owned(),
                |o| format!("Get the ID from {}; never make IDs up.", lookup_hint(o)),
            )),
        Code::Unavailable => problem
            .problem("the Mac's helper or this source is not reachable right now")
            .hint("Call apple_status to see which sources are ready, and retry in a minute. If it stays unavailable, tell the owner."),
        Code::Unsupported => problem
            .problem(operation.and_then(|o| o.unavailable.clone()).unwrap_or_else(|| {
                "this action is not available on this Mac or for this kind of agent".into()
            }))
            .hint("Do not retry. apple_describe lists what you can use."),
        Code::CapabilityUnproven => problem
            .problem("this action has not been proven to work on this Mac yet")
            .hint("Do not retry. Tell the owner it is not available yet."),
        Code::Indeterminate => problem
            .problem("nobody can tell whether this change happened")
            .hint("Do not retry with a new request_id. Check apple_command_status with the same request_id, or tell the owner."),
        Code::IdReused => problem
            .field("request_id")
            .problem("this request_id was already used for a different request")
            .hint("Use a new request_id for a new change. Reuse a request_id only to repeat exactly the same call."),
        Code::Deadline => problem
            .problem("the Mac did not answer in time")
            .hint("Retry a read. For a change, check apple_command_status before trying again."),
        Code::Expired => expired_problem(problem, operation),
        Code::TooLarge => problem
            .problem("the input is too large")
            .hint("Send a smaller file or split the work."),
        Code::Capacity => problem
            .problem("the Mac is busy or out of space for this request")
            .hint("Wait a little and retry."),
        Code::Reset => problem
            .problem("the paging cursor is no longer valid")
            .hint("Start again from the first page."),
        _ => problem,
    };
    problem
}

fn access_problem(
    problem: Problem,
    operation: Option<&AgentOperation>,
    principal: &Principal,
) -> Problem {
    let scope = operation.map_or("", |o| o.scope.as_str());
    let agent = crate::mcp_registry::agent_id(principal);
    if operation.is_some_and(|o| o.changes) && crate::mcp_registry::read_only(principal) {
        problem.problem("this agent is read-only").hint(format!(
            "Tell the owner. The Mac's owner can allow changes by running: bridge agents update {agent} --level full"
        ))
    } else if !scope.is_empty() && !crate::mcp_registry::can(principal, scope) {
        let mut grants = principal.grants.clone();
        grants.push(scope.to_owned());
        problem
            .problem(format!("this agent does not have the {scope} scope"))
            .expected(json!({ "scope": scope }))
            .hint(format!(
                "Tell the owner. The Mac's owner can grant it by running: bridge agents update {agent} --scope {}",
                grants.join(",")
            ))
    } else {
        problem.problem("the Mac refused this action for this agent").hint(
            "Some actions, such as locking or restarting the Mac, are limited by who asks. Tell the owner what you tried.",
        )
    }
}

fn invalid_problem(problem: Problem, operation: Option<&AgentOperation>) -> Problem {
    let Some(operation) = operation else {
        return problem.problem("Apple Bridge rejected these arguments");
    };
    let name = &operation.name;
    let rules = if operation.detail.is_empty() {
        String::new()
    } else {
        format!(" {}", operation.detail)
    };
    let who = if operation.kind == ab_catalog::OperationKind::Query {
        "the Mac"
    } else {
        "Apple Bridge"
    };
    problem
        .problem(format!("{who} rejected these arguments for {name}"))
        .example(example_call(tool_for(operation), operation))
        .hint(format!(
            "Compare with the example and the rules in apple_describe with operation {name}.{rules}"
        ))
}

fn conflict_problem(mut problem: Problem, error: &Error) -> Problem {
    let current = error
        .current
        .as_ref()
        .and_then(|raw| serde_json::from_str::<Value>(raw.get()).ok());
    let version = current.as_ref().and_then(|c| c["version"].as_u64());
    problem.current = current.map(|record| crate::mcp_facade::compact(&record, false, None));
    problem = problem.problem("the record changed since you read it");
    match version {
        Some(version) => problem.hint(format!(
            "Check current. If your change still applies, retry with expected_version={version}."
        )),
        None => problem.hint("Read the record again and retry with its current version."),
    }
}

fn expired_problem(problem: Problem, operation: Option<&AgentOperation>) -> Problem {
    match operation {
        Some(o) if o.name == "photos.release" => problem
            .problem("that photo query already expired")
            .hint("Nothing is left to release; there is nothing to retry."),
        Some(o) if !o.changes || o.read_only() => problem
            .problem("the earlier result this read refers to expired")
            .hint("Run the read that produced it again, then use its new IDs. If every call fails, the owner must reconnect this app."),
        _ => problem
            .problem("the request or credential expired")
            .hint("This change did not run. Send it again with a new request_id. If every call fails, the owner must reconnect this app."),
    }
}

const TOOL_DOMAIN: &str = "listed tool";

pub(crate) fn check_tool(
    tool: &str,
    schema: &Value,
    example: &Value,
    input: &Map<String, Value>,
) -> Result<(), Problem> {
    let mut schema = schema.clone();
    schema["additionalProperties"] = json!(false);
    let schema = &schema;
    let operation = AgentOperation {
        name: tool.into(),
        domain: TOOL_DOMAIN.into(),
        kind: ab_catalog::OperationKind::Read,
        changes: false,
        scope: String::new(),
        summary: String::new(),
        detail: String::new(),
        returns: String::new(),
        arguments: schema.clone(),
        example: example.clone(),
        version: ab_catalog::VersionRule::None,
        unavailable: None,
        annotations: Value::Null,
    };
    check_object(&Value::Object(input.clone()), schema, "", &operation, tool).map_err(
        |mut problem| {
            problem.operation = None;
            problem.message =
                problem
                    .message
                    .replacen(&format!("{tool} has"), &format!("{tool} call has"), 1);
            if matches!(tool, "apple_read" | "apple_change")
                && problem
                    .issue
                    .as_deref()
                    .is_some_and(|i| i.contains("is not an argument"))
            {
                problem.hint = Some("Put the operation's own arguments inside arguments.".into());
            } else if let Some(hint) = problem.hint.as_mut() {
                *hint = hint.replace(
                    &format!("apple_describe with operation {tool} shows every argument."),
                    "The tool's input schema lists every argument.",
                );
            }
            problem
        },
    )
}

pub(crate) fn refused(principal: &Principal, what: &str, scope: &str, read_only: bool) -> Problem {
    let agent = crate::mcp_registry::agent_id(principal);
    if read_only {
        return Problem::new("unauthorized", format!("{what} changes data and this agent is read-only"))
            .problem("this agent is read-only")
            .hint(format!("Tell the owner. The Mac's owner can allow changes by running: bridge agents update {agent} --level full"));
    }
    let mut grants = principal.grants.clone();
    grants.push(scope.to_owned());
    Problem::new("unauthorized", format!("this agent lacks a scope needed for {what}"))
        .problem(format!("this agent does not have the {scope} scope"))
        .expected(json!({ "scope": scope }))
        .hint(format!(
            "Do not retry. Tell the owner; they can grant it by running: bridge agents update {agent} --scope {}",
            grants.join(",")
        ))
}

const LISTINGS: &[(&str, &str)] = &[
    ("cht", "messages_read with no arguments lists chats"),
    ("msg", "messages_read"),
    ("att", "messages_read, from a message's attachments"),
    ("con", "contacts_find"),
    ("evt", "calendar_events"),
    ("cal", "apple_read calendar.calendars"),
    ("rem", "apple_read reminders.items"),
    ("lst", "apple_read reminders.lists"),
    ("not", "apple_read notes.list or apple_search"),
    ("fld", "apple_read notes.list"),
    ("qry", "the read that returned it"),
    ("dev", "apple_read usage.summary"),
];

pub(crate) fn not_found_hint(problem: Problem, input: &Value) -> Problem {
    let id = [
        "id",
        "chat_id",
        "calendar_id",
        "list_id",
        "folder_id",
        "record_id",
        "query_id",
    ]
    .iter()
    .find_map(|key| {
        input
            .get(*key)
            .or_else(|| input["args"].get(*key))
            .and_then(Value::as_str)
    });
    let Some(id) = id else {
        return problem;
    };
    let prefix = id.split('_').next().unwrap_or_default();
    match LISTINGS.iter().find(|(p, _)| *p == prefix) {
        Some((_, listing)) => problem.hint(format!(
            "{id} is not visible. Get a current ID from {listing}; never make IDs up."
        )),
        None => problem,
    }
}

fn zone() -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::get(&mac_zone()).unwrap_or(jiff::tz::TimeZone::UTC)
}

fn stamp(zoned: &jiff::Zoned) -> String {
    zoned.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

pub(crate) fn local_time(text: &str) -> Option<String> {
    if text.len() <= 10 || text.parse::<jiff::Timestamp>().is_ok() {
        return None;
    }
    let civil: jiff::civil::DateTime = text.trim_end_matches('Z').parse().ok()?;
    civil.to_zoned(zone()).ok().map(|z| stamp(&z))
}

pub(crate) fn day_start(text: &str, next: bool) -> Option<String> {
    if text.len() != 10 {
        return None;
    }
    let mut date: jiff::civil::Date = text.parse().ok()?;
    if next {
        date = date.tomorrow().ok()?;
    }
    date.to_zoned(zone()).ok().map(|z| stamp(&z))
}

pub(crate) fn infer_collection(operation: &AgentOperation, arguments: &mut Map<String, Value>) {
    if operation.name != "records.get" || arguments.contains_key("collection") {
        return;
    }
    let Some(prefix) = arguments
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.rsplit_once('_'))
        .map(|(prefix, _)| prefix)
    else {
        return;
    };
    if let Some(collection) = ab_trust::domain::Collection::ALL
        .iter()
        .find(|collection| collection.prefix() == prefix)
        .and_then(|collection| serde_json::to_value(collection).ok())
    {
        arguments.insert("collection".to_owned(), collection);
    }
}

pub(crate) fn local_dates(operation: &AgentOperation, arguments: &mut Map<String, Value>) {
    let properties = operation.arguments["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let keys: Vec<String> = arguments.keys().cloned().collect();
    for key in keys {
        let (target, local) = match key.strip_suffix("_local") {
            Some(base) if properties.contains_key(base) && !properties.contains_key(&key) => {
                (base.to_owned(), true)
            }
            _ => (key.clone(), false),
        };
        let description = properties
            .get(&target)
            .and_then(|p| p["description"].as_str())
            .unwrap_or_default();
        if !description.contains("RFC 3339") {
            continue;
        }
        let Some(text) = arguments
            .get(&key)
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let date_allowed = description.contains("YYYY-MM-DD")
            && operation.kind != ab_catalog::OperationKind::Query;
        let converted = local_time(&text).or_else(|| {
            if date_allowed {
                None
            } else {
                day_start(&text, false)
            }
        });
        if local {
            arguments.remove(&key);
            arguments
                .entry(target)
                .or_insert_with(|| json!(converted.unwrap_or(text)));
        } else if let Some(converted) = converted {
            arguments.insert(key, json!(converted));
        }
    }
}
