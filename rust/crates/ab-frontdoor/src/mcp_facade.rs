use crate::{
    Frontdoor,
    mcp_check::{self, Problem, example_call, from_error, mac_zone},
    mcp_registry::{
        self, BLOB_READ, BLOB_STAGE, BY_TITLE, CALENDARS, Registry, USAGE_SUMMARY, allowed, can,
        read_only, registry,
    },
};
use ab_catalog::{AgentOperation, OperationKind, VersionRule};
use ab_operations::{Code, Error, Principal};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};

#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub value: Value,
    pub is_error: bool,
    pub image: Option<(String, String)>,
    pub read_model_ms: Option<f64>,
    pub operation: Option<String>,
}

impl Outcome {
    pub(crate) fn from_value(value: Value) -> Self {
        Self::ok(value)
    }
    pub(crate) fn ok(value: Value) -> Self {
        let is_error = matches!(value["status"].as_str(), Some("failed" | "partial"));
        Self {
            value,
            is_error,
            ..Self::default()
        }
    }
    fn fail(problem: &Problem) -> Self {
        Self {
            value: problem.to_json(),
            is_error: true,
            operation: problem.operation.clone(),
            ..Self::default()
        }
    }
    pub(crate) fn named(mut self, operation: &str) -> Self {
        if self.operation.is_none() {
            self.operation = Some(operation.into());
        }
        self
    }
    pub(crate) fn class(&self) -> Option<String> {
        if let Some(code) = self.value["error"]["code"].as_str() {
            return Some(code.to_owned());
        }
        match self.value["status"].as_str() {
            Some(status @ ("failed" | "partial" | "indeterminate")) => Some(status.to_owned()),
            _ => None,
        }
    }
}

pub(crate) type Step<T> = Result<T, Problem>;

pub(crate) struct Call<'a> {
    pub frontdoor: &'a Frontdoor,
    pub principal: &'a Principal,
    pub registry: &'a Registry,
}

const TOOL_SCOPES: &[(&str, &str)] = &[
    ("messages_read", "messages:read"),
    ("messages_send", "messages:write"),
    ("reminders_add", "reminders:write"),
    ("calendar_events", "calendar:read"),
    ("calendar_add", "calendar:write"),
    ("contacts_find", "contacts:read"),
    ("notes_create", "notes:write"),
    ("screen_time", "usage:read"),
    ("photos_find", "photos:read"),
    ("photos_show", "photos:read"),
];

pub(crate) async fn call(
    frontdoor: &Frontdoor,
    principal: &Principal,
    tool: &str,
    mut input: Map<String, Value>,
) -> Outcome {
    let Some(registry) = registry() else {
        return Outcome::fail(&Problem::new(
            "corrupt",
            "operation registry failed to load",
        ));
    };
    let call = Call {
        frontdoor,
        principal,
        registry,
    };
    if tool == "calendar_events"
        && let Some(id) = input.remove("calendar_id")
    {
        input.entry("calendar").or_insert(id);
    }
    if let Err(problem) = call.check_tool(tool, &input) {
        return Outcome::fail(&problem);
    }
    let result = match tool {
        "apple_status" => call.status().await,
        "apple_describe" => call.describe(&input),
        "apple_search" => call.search(&input).await,
        "apple_read" => call.read(&input).await,
        "apple_change" => call.change(&input).await,
        "apple_command_status" => call.command_status(&input).await,
        "apple_guide" => crate::mcp_guides::guide_tool(input.get("name").and_then(Value::as_str)),
        "messages_read" => call.messages_read(&input).await,
        "messages_send" => call.messages_send(&input).await,
        "reminders_add" => call.reminders_add(&input).await,
        "calendar_events" => call.calendar_events(&input).await,
        "calendar_add" => call.calendar_add(&input).await,
        "contacts_find" => call.contacts_find(&input).await,
        "notes_create" => call.notes_create(&input).await,
        "screen_time" => call.screen_time(&input).await,
        "photos_find" => call.photos_find(&input).await,
        "photos_show" => call.photos_show(&input).await,
        _ => Err(Problem::new("invalid", format!("unknown tool {tool}"))),
    };
    result.unwrap_or_else(|problem| Outcome::fail(&problem))
}

pub(crate) fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn content<'a>(
    input: &'a Map<String, Value>,
    key: &str,
    operation: &str,
) -> Step<Option<&'a str>> {
    match input.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Err(Problem::new(
            "invalid",
            format!("{key} is blank"),
        )
        .operation(operation)
        .field(key)
        .problem(format!("{key} has no visible characters"))
        .expected(json!("text with at least one visible character; it is kept exactly as given, including spaces and new lines"))),
        Some(value) => Ok(Some(value)),
    }
}

pub(crate) fn flag(input: &Map<String, Value>, key: &str) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) fn request_id(input: &Map<String, Value>) -> Step<String> {
    match input.get("request_id") {
        None | Some(Value::Null) => Ok(format!("req-{}", ulid::Ulid::new())),
        Some(Value::String(id))
            if !id.is_empty()
                && id.len() <= 120
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b)) =>
        {
            Ok(id.clone())
        }
        Some(_) => Err(Problem::new("invalid", "request_id has an invalid format")
            .field("request_id")
            .expected(json!("1 to 120 letters, digits or . _ : -"))
            .example(json!("groceries-2026-11-12"))),
    }
}

pub(crate) fn scoped_id(principal: &str, request_id: &str) -> String {
    let digest = ab_trust::fingerprint(format!("{principal}\n{request_id}").as_bytes());
    let readable: String = request_id.chars().take(100).collect();
    format!("{readable}.{}", &digest[..16])
}

pub(crate) fn legacy_ids(request_id: &str) -> Vec<String> {
    let mut ids = vec![request_id.to_owned()];
    if !request_id.starts_with("qry_") {
        ids.push(format!("qry_{request_id}"));
    }
    ids.retain(|id| mcp_registry::valid_id(id));
    ids
}

pub(crate) fn status_ids(principal: &str, request_id: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for id in [scoped_id(principal, request_id), request_id.to_owned()] {
        if !id.starts_with("qry_") {
            ids.push(format!("qry_{id}"));
        }
        ids.push(id);
    }
    ids.retain(|id| mcp_registry::valid_id(id));
    ids
}

pub(crate) fn source_of(collection: &str) -> &str {
    collection.split('.').next().unwrap_or(collection)
}

impl Call<'_> {
    fn check_tool(&self, tool: &str, input: &Map<String, Value>) -> Step<()> {
        let Some(schema) = self.registry.tool_schema(tool) else {
            return Ok(());
        };
        mcp_check::check_tool(tool, schema, &self.registry.example(tool), input)?;
        if let Some((_, scope)) = TOOL_SCOPES.iter().find(|(name, _)| *name == tool) {
            self.require(scope, tool)?;
        }
        Ok(())
    }

    pub(crate) fn require(&self, scope: &str, what: &str) -> Step<()> {
        if can(self.principal, scope) {
            if scope.ends_with(":write") && read_only(self.principal) {
                return Err(mcp_check::refused(self.principal, what, scope, true));
            }
            return Ok(());
        }
        Err(mcp_check::refused(self.principal, what, scope, false))
    }

    fn write_only_source(&self) -> Option<String> {
        if can(self.principal, "admin") {
            return None;
        }
        self.principal
            .grants
            .iter()
            .filter_map(|grant| grant.strip_suffix(":write"))
            .find(|source| !can(self.principal, &format!("{source}:read")))
            .map(str::to_owned)
    }

    pub(crate) fn operation(&self, name: &str) -> Step<&AgentOperation> {
        self.registry.find(name).ok_or_else(|| {
            let close = self.registry.closest(name, Some(self.principal));
            Problem::new("invalid", format!("unknown operation {name}"))
                .field("operation")
                .problem(format!("{name} is not an operation"))
                .expected(json!({ "closest": close }))
                .hint(if close.is_empty() {
                    "Call apple_describe with a query describing the task.".to_owned()
                } else {
                    format!(
                        "Did you mean {}? apple_describe with a query finds operations.",
                        close[0]
                    )
                })
        })
    }

    pub(crate) async fn invoke(&self, operation: &AgentOperation, input: &Value) -> Step<Value> {
        let raw = self.invoke_raw(operation, input).await?;
        serde_json::from_str(&raw).map_err(|_| Problem::new("corrupt", "unreadable result"))
    }

    pub(crate) async fn invoke_raw(
        &self,
        operation: &AgentOperation,
        input: &Value,
    ) -> Step<String> {
        let bytes =
            serde_json::to_vec(input).map_err(|_| Problem::new("invalid", "unencodable input"))?;
        match self
            .frontdoor
            .service
            .invoke(self.principal, &operation.name, &bytes)
            .await
        {
            Ok(raw) => Ok(raw.get().to_owned()),
            Err(error) => {
                let problem = from_error(&error, Some(operation), self.principal);
                Err(if error.code == Code::NotFound {
                    mcp_check::not_found_hint(problem, input)
                } else {
                    problem
                })
            }
        }
    }

    pub(crate) async fn command_id(&self, request_id: &str) -> Step<String> {
        let status = self.operation("commands.status")?;
        for legacy in legacy_ids(request_id) {
            match self.invoke(status, &json!({"command_id": legacy})).await {
                Ok(_) => return Ok(request_id.to_owned()),
                Err(problem) if problem.code == "not_found" => {}
                Err(problem) => {
                    return Err(problem.field("request_id").hint(
                        "Apple Bridge could not check whether this request_id was already used, so nothing was done. Retry the same call with the same request_id.",
                    ));
                }
            }
        }
        Ok(scoped_id(&self.principal.id, request_id))
    }

    pub(crate) async fn quiet(&self, name: &str, input: Value) -> Option<Value> {
        let operation = self.registry.find(name)?;
        if !allowed(self.principal, operation) {
            return None;
        }
        self.invoke(operation, &input).await.ok()
    }

    async fn status(&self) -> Step<Outcome> {
        let sources = self.quiet("sources.status", json!({})).await;
        let helper = self.quiet("helper.version", json!({"args":{}})).await;
        let mut sources_list = Vec::new();
        for record in sources
            .as_ref()
            .and_then(|s| s["records"].as_array())
            .into_iter()
            .flatten()
        {
            let data = &record["data"];
            let mut entry = json!({"source": data["source"], "state": data["state"]});
            if data["state"] != "ready"
                && let Some(reason) = data["reason"].as_str().filter(|r| !r.is_empty())
            {
                entry["reason"] = json!(reason);
            }
            sources_list.push(entry);
        }
        let mut domains = Map::new();
        for (domain, _) in &self.registry.domains {
            let operations: Vec<&AgentOperation> = self
                .registry
                .operations
                .iter()
                .filter(|o| &o.domain == domain)
                .collect();
            let reads = operations
                .iter()
                .any(|o| !o.changes && allowed(self.principal, o));
            let writes = operations
                .iter()
                .any(|o| o.changes && allowed(self.principal, o));
            let access = match (reads, writes) {
                (true, true) => "read and change",
                (true, false) => "read",
                (false, true) => "change",
                (false, false) => continue,
            };
            domains.insert(domain.clone(), json!(access));
        }
        let level = if matches!(self.principal.kind.as_str(), "agent" | "delegated") {
            self.principal.level.clone()
        } else {
            "owner".into()
        };
        let behaviour = match level.as_str() {
            "read" => "read only: changes are refused",
            "approve" => "asks first: each change waits for the owner's approval",
            _ => "full: changes run at once",
        };
        let mut value = json!({
            "access": {
                "agent": mcp_registry::agent_id(self.principal),
                "level": level,
                "changes": behaviour,
                "scopes": self.principal.grants,
                "remote": self.principal.remote,
                "domains": domains,
            },
            "sources": sources_list,
            "time_zone": mac_zone(),
            "now": jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            "next": "Use the dedicated tools for common tasks, or apple_describe with a query to find any other operation.",
        });
        if let Some(helper) = helper {
            value["helper"] = helper["result_json"].clone();
        }
        if sources.is_none() {
            value["sources_note"] =
                json!("Source status is unavailable right now; the Mac's helper may be stopped.");
        }
        Ok(Outcome::ok(value))
    }

    fn describe(&self, input: &Map<String, Value>) -> Step<Outcome> {
        if let Some(name) = text(input, "operation") {
            let operation = self.operation(name)?;
            return Ok(Outcome::ok(self.contract(operation)));
        }
        if let Some(domain) = text(input, "domain") {
            let domain = domain.to_lowercase();
            let Some((_, about)) = self.registry.domains.iter().find(|(d, _)| *d == domain) else {
                let names: Vec<&String> = self.registry.domains.iter().map(|(d, _)| d).collect();
                return Err(Problem::new("invalid", format!("unknown domain {domain}"))
                    .field("domain")
                    .expected(json!({ "domains": names })));
            };
            let operations: Vec<Value> = self
                .registry
                .visible(Some(self.principal))
                .filter(|o| o.domain == domain)
                .map(|o| {
                    json!({
                        "operation": o.name,
                        "summary": o.summary,
                        "changes_data": o.changes && !o.read_only(),
                        "required": o.required(),
                    })
                })
                .collect();
            let mut value = json!({
                "domain": domain,
                "about": about,
                "operations": operations,
                "next": "Call apple_describe with operation for the full contract.",
            });
            let routes: Vec<String> = crate::mcp_find::ROUTES
                .iter()
                .filter(|r| source_of(r.collection) == domain && can(self.principal, r.scope))
                .map(route_line)
                .collect();
            if !routes.is_empty() {
                value["collections"] = json!(routes);
            }
            return Ok(Outcome::ok(value));
        }
        if let Some(query) = text(input, "query") {
            let (hits, total) = crate::mcp_find::find(self.registry, Some(self.principal), query);
            let matches: Vec<Value> = hits
                .iter()
                .map(|hit| match hit {
                    crate::mcp_find::Hit::Operation(o) => json!(line(o)),
                    crate::mcp_find::Hit::Route(r) => json!(route_line(r)),
                })
                .collect();
            return Ok(Outcome::ok(json!({
                "query": query,
                "operations": matches,
                "total": total,
                "truncated": total > matches.len(),
                "next": if matches.is_empty() { "Nothing matched; try other words, a domain such as messages, or apple_describe with no arguments." } else { "Best first. Call apple_describe with operation for the full contract, or run it." },
            })));
        }
        let mut lines: Vec<Value> = self
            .registry
            .visible(Some(self.principal))
            .map(|o| json!(line(o)))
            .collect();
        lines.extend(
            crate::mcp_find::ROUTES
                .iter()
                .filter(|r| can(self.principal, r.scope))
                .map(|r| json!(route_line(r))),
        );
        Ok(Outcome::ok(json!({
            "count": lines.len(),
            "operations": lines,
            "next": "Read with apple_read, change with apple_change. Call apple_describe with operation for exact arguments.",
        })))
    }

    fn contract(&self, operation: &AgentOperation) -> Value {
        let tool = mcp_check::tool_for(operation);
        let mut access = json!({
            "scope": if operation.scope.is_empty() { Value::from("none: limited to records this agent can see") } else { Value::from(operation.scope.clone()) },
            "you_can_use_it": allowed(self.principal, operation) && operation.unavailable.is_none(),
        });
        if operation.changes {
            access["level"] = json!("full, or approve (waits for the owner)");
        }
        if !can(self.principal, &operation.scope) {
            access["why_not"] = json!(format!("this agent does not have {}", operation.scope));
        } else if operation.changes && read_only(self.principal) {
            access["why_not"] = json!("this agent is read-only");
        }
        let mut value = json!({
            "operation": operation.name,
            "domain": operation.domain,
            "summary": operation.summary,
            "changes_data": operation.changes && !operation.read_only(),
            "tool": tool,
            "access": access,
            "arguments": operation.arguments,
            "example": example_call(tool, operation),
            "returns": operation.returns,
            "annotations": operation.annotations,
        });
        if !operation.detail.is_empty() {
            value["details"] = json!(operation.detail);
        }
        if operation.name == "records.query" || operation.name == "search" {
            value["collections"] = json!(
                crate::mcp_find::ROUTES
                    .iter()
                    .map(route_line)
                    .collect::<Vec<_>>()
            );
        }
        if let Some(reason) = &operation.unavailable {
            value["available"] = json!(false);
            value["unavailable"] = json!(reason);
        }
        if operation.version == VersionRule::Required {
            value["versions"] = json!(if BY_TITLE.contains(&operation.name.as_str()) {
                "Pass id with expected_version, the version field of the record from your latest read, or pass reminder with its title instead."
            } else {
                "expected_version is required: pass the version field of the record from your latest read."
            });
        }
        if operation.changes && !operation.read_only() {
            value["completion"] = json!(
                "Waits up to 15 seconds; returns done, partial, pending, awaiting_approval, failed or indeterminate with request_id."
            );
        }
        value
    }

    async fn read(&self, input: &Map<String, Value>) -> Step<Outcome> {
        if let Some(reads) = input.get("reads").and_then(Value::as_array) {
            if input.contains_key("operation") {
                return Err(Problem::new("invalid", "give operation or reads, not both")
                    .field("reads")
                    .hint("Put each read, including this operation, inside reads."));
            }
            let mut results = Vec::new();
            for (index, entry) in reads.iter().enumerate() {
                let mut single = entry.as_object().cloned().unwrap_or_default();
                if let Some(fields) = input.get("fields") {
                    single.insert("fields".into(), fields.clone());
                }
                if let Some(verbose) = input.get("verbose") {
                    single.insert("verbose".into(), verbose.clone());
                }
                let name = single
                    .get("operation")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if crate::mcp::returns_image(&mcp_registry::normal_name(&name)) {
                    results.push(json!({"operation": name, "error": Problem::new("invalid", format!("{name} returns an image")).field(format!("reads[{index}].operation")).hint("Call it on its own with operation, so the image can be shown.").to_json()["error"]}));
                    continue;
                }
                match Box::pin(self.read_one(&single)).await {
                    Ok(outcome) => {
                        results.push(json!({"operation": name, "result": outcome.value}));
                    }
                    Err(problem) => {
                        results
                            .push(json!({"operation": name, "error": problem.to_json()["error"]}));
                    }
                }
            }
            let failed = results.iter().filter(|r| r.get("error").is_some()).count();
            return Ok(Outcome::ok(json!({
                "results": results,
                "failed": failed,
                "next": if failed == 0 { "All reads finished, in the order given." } else { "Some reads failed; each error says why. The others are complete." },
            }))
            .named("reads"));
        }
        if !input.contains_key("operation") {
            return Err(Problem::new("invalid", "operation is required")
                .field("operation")
                .expected(json!(
                    "a read operation name such as reminders.items, or reads with several"
                ))
                .hint("apple_describe with a query finds the operation for a task."));
        }
        self.read_one(input).await
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) async fn read_one(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let name = text(input, "operation").ok_or_else(|| {
            Problem::new("invalid", "operation is required")
                .field("operation")
                .expected(json!("a read operation name such as reminders.items"))
                .hint("apple_describe lists the operations you can use.")
        })?;
        let operation = self.operation(name)?;
        if operation.changes && !operation.read_only() {
            return Err(
                Problem::new("invalid", format!("{} changes data", operation.name))
                    .operation(&operation.name)
                    .field("operation")
                    .problem("apple_read only runs reads")
                    .example(example_call("apple_change", operation))
                    .hint("Use apple_change for this operation."),
            );
        }
        let mut given = input.get("arguments").cloned();
        if let Some(Value::Object(arguments)) = given.as_mut() {
            mcp_check::infer_collection(operation, arguments);
            mcp_check::local_dates(operation, arguments);
        }
        let mut arguments = mcp_check::arguments(operation, given.as_ref())?;
        if !operation.scope.is_empty() {
            self.require(&operation.scope, &operation.name)?;
        }
        if operation.name == "reminders.items"
            && let Some(list) = text(&arguments, "list").filter(|l| !l.starts_with("lst_"))
        {
            let list = list.to_owned();
            let (id, _) = self.reminder_list(Some(&list)).await?;
            arguments.insert("list".into(), json!(id));
        }
        if matches!(
            operation.name.as_str(),
            "records.query" | "records.get" | "search"
        ) && let Some(collection) = arguments
            .get("collection")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            && !can(self.principal, &format!("{}:read", source_of(collection)))
        {
            return Err(mcp_check::refused(
                self.principal,
                &format!("{} {collection}", operation.name),
                &format!("{}:read", source_of(collection)),
                false,
            ));
        }
        if matches!(
            operation.name.as_str(),
            "records.query" | "records.get" | "search"
        ) && arguments
            .get("collection")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            && let Some(source) = self.write_only_source()
        {
            return Err(mcp_check::refused(
                self.principal,
                &format!("{} across every collection", operation.name),
                &format!("{source}:read"),
                false,
            )
            .field("collection"));
        }
        let verbose = flag(input, "verbose");
        let fields: Option<Vec<String>> = input.get("fields").and_then(Value::as_array).map(|f| {
            f.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        });
        if operation.name == USAGE_SUMMARY {
            return self
                .usage_summary(&arguments)
                .await
                .map(|v| Outcome::ok(v).named(USAGE_SUMMARY));
        }
        if operation.name == crate::mcp_lookups::FREE_SLOTS {
            return self
                .free_slots(&arguments)
                .await
                .map(|v| Outcome::ok(v).named(crate::mcp_lookups::FREE_SLOTS));
        }
        if operation.name == crate::mcp_lookups::SHARED {
            return self
                .shared(&arguments)
                .await
                .map(|v| Outcome::ok(v).named(crate::mcp_lookups::SHARED));
        }
        if operation.name == CALENDARS {
            return self
                .calendars()
                .await
                .map(|list| {
                    Outcome::ok(json!({
                        "calendars": list,
                        "next": "Pass a calendar's title as calendar to calendar_add or calendar_events.",
                    }))
                })
                .map(|o| o.named(CALENDARS));
        }
        if operation.kind == OperationKind::Command {
            return self
                .run_change(operation, arguments, &request_id(input)?, verbose)
                .await;
        }
        if operation.name == BLOB_READ {
            return self
                .blob_read(&arguments)
                .await
                .map(|v| Outcome::ok(v).named(BLOB_READ));
        }
        let started = std::time::Instant::now();
        let body = if operation.kind == OperationKind::Query {
            let mut body = json!({"args": arguments});
            if input.get("request_id").is_some_and(|id| !id.is_null()) {
                let id = self.command_id(&request_id(input)?).await?;
                body["id"] = json!(if id.starts_with("qry_") {
                    id
                } else {
                    format!("qry_{id}")
                });
            }
            body
        } else {
            Value::Object(arguments)
        };
        let raw = self.invoke_raw(operation, &body).await?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let mut image = None;
        let raw = if crate::mcp::returns_image(&operation.name) {
            let (body, attached) = crate::mcp::detach_image(&raw)
                .map_err(|_| Problem::new("corrupt", "unreadable image result"))?;
            image = attached.map(|i| (i.data, i.mime));
            String::from_utf8(body).map_err(|_| Problem::new("corrupt", "unreadable result"))?
        } else {
            raw
        };
        let value: Value =
            serde_json::from_str(&raw).map_err(|_| Problem::new("corrupt", "unreadable result"))?;
        let value = if operation.kind == OperationKind::Query {
            let mut out = query_result(&value, verbose, fields.as_deref());
            if out["status"] == "awaiting_approval"
                && let Some(Value::String(external)) = input.get("request_id")
            {
                out["query_id"] = json!(external);
                out["next"] = json!(format!(
                    "The owner set this agent to ask first. Tell them; after they approve, repeat this exact call with request_id {external} within ten minutes."
                ));
            }
            out
        } else if operation.name == "reminders.lists" && !verbose {
            reminder_lists(&value)
        } else {
            compact(&value, verbose, fields.as_deref())
        };
        let value = match value {
            Value::Object(_) => value,
            other => json!({ "items": other }),
        };
        let read_model = ab_operations::descriptor_for(&operation.name).is_ok_and(|d| d.read_model);
        Ok(Outcome {
            value,
            image,
            read_model_ms: read_model.then_some(elapsed),
            ..Outcome::default()
        }
        .named(&operation.name))
    }

    async fn change(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let name = text(input, "operation").ok_or_else(|| {
            Problem::new("invalid", "operation is required")
                .field("operation")
                .expected(json!("an operation name such as reminders.create"))
                .hint("apple_describe lists the operations you can use.")
        })?;
        let operation = self.operation(name)?;
        if !operation.changes {
            return Err(
                Problem::new("invalid", format!("{} only reads", operation.name))
                    .operation(&operation.name)
                    .field("operation")
                    .problem("apple_change only runs operations that change data")
                    .example(example_call("apple_read", operation))
                    .hint("Use apple_read for this operation."),
            );
        }
        let mut given = input.get("arguments").cloned();
        if let Some(Value::Object(arguments)) = given.as_mut() {
            mcp_check::local_dates(operation, arguments);
        }
        let mut arguments = mcp_check::arguments(operation, given.as_ref())?;
        if !operation.scope.is_empty() {
            self.require(&operation.scope, &operation.name)?;
        }
        let id = request_id(input)?;
        let _turn = if BY_TITLE.contains(&operation.name.as_str()) {
            Some(crate::mcp_compose::request_turn(&self.principal.id, &id).await)
        } else {
            None
        };
        let mut selected = None;
        if BY_TITLE.contains(&operation.name.as_str())
            && !self.bound_target(operation, &id, &mut arguments).await?
        {
            let title = arguments
                .get("reminder")
                .and_then(Value::as_str)
                .map(str::to_owned);
            self.reminder_target(operation, &mut arguments).await?;
            selected = title.zip(
                arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            );
        }
        if operation.name == "calendar.create_event" {
            let name = arguments.remove("calendar");
            if name.is_some() || !arguments.contains_key("calendar_id") {
                let (calendar, _) = self
                    .calendar_for(name.as_ref().and_then(Value::as_str), true)
                    .await?;
                arguments.insert("calendar_id".into(), json!(calendar));
            }
        }
        let outcome = self
            .run_change(operation, arguments, &id, flag(input, "verbose"))
            .await;
        if let Some((title, target)) = selected
            && outcome
                .as_ref()
                .err()
                .is_none_or(|problem| problem.code != "id_reused")
        {
            self.bind_if_admitted(operation, &id, &title, &target).await;
        }
        outcome
    }

    async fn bind_if_admitted(
        &self,
        operation: &AgentOperation,
        request_id: &str,
        title: &str,
        target: &str,
    ) {
        let Ok(status) = self.operation("commands.status") else {
            return;
        };
        let Ok(command_id) = self.command_id(request_id).await else {
            return;
        };
        let Ok(record) = self
            .invoke(status, &json!({"command_id": command_id}))
            .await
        else {
            return;
        };
        let command = &record["command"];
        if command["op"] == operation.name.as_str() && command["args"]["id"] == target {
            crate::mcp_compose::bind_selector(&self.principal.id, request_id, title);
        }
    }

    pub(crate) async fn run_change(
        &self,
        operation: &AgentOperation,
        mut arguments: Map<String, Value>,
        id: &str,
        verbose: bool,
    ) -> Step<Outcome> {
        if let Some(reason) = &operation.unavailable {
            return Err(from_error(
                &Error::from(Code::Unsupported),
                Some(operation),
                self.principal,
            )
            .problem(reason.clone()));
        }
        if operation.name == BLOB_STAGE {
            return self.blob_stage(operation, &arguments).await;
        }
        let external = id;
        let internal = self.command_id(id).await?;
        let id = internal.as_str();
        if operation.photos_write() {
            let query_id = if id.starts_with("qry_") {
                id.to_owned()
            } else {
                format!("qry_{id}")
            };
            let value = self
                .invoke(operation, &json!({"id": query_id, "args": arguments}))
                .await?;
            return Ok(Outcome::ok(photos_change(
                &operation.name,
                external,
                &arguments,
                &value,
                verbose,
            ))
            .named(&operation.name));
        }
        let version = arguments.remove("expected_version");
        let mut envelope = json!({"command_id": id, "args": arguments, "wait_ms": 15000});
        if let Some(version) = version {
            envelope["expected_version"] = version;
        }
        let record = self.invoke(operation, &envelope).await?;
        let mut value = change_result(&record, verbose);
        value["request_id"] = json!(external);
        self.attach_record(&mut value, operation).await;
        Ok(Outcome::ok(value).named(&operation.name))
    }

    async fn attach_record(&self, value: &mut Value, operation: &AgentOperation) {
        if !matches!(value["status"].as_str(), Some("done" | "partial"))
            || operation.name.contains("delete")
        {
            return;
        }
        let Some(id) = value["ids"]
            .as_array()
            .and_then(|ids| ids.first())
            .and_then(Value::as_str)
        else {
            return;
        };
        let Some(collection) = collection_of(id) else {
            return;
        };
        if !can(self.principal, &format!("{}:read", source_of(&collection))) {
            if value["status"] == "done" {
                value["next"] = json!(format!(
                    "Done. This agent cannot read {}, so the changed record is not shown.",
                    source_of(&collection)
                ));
            }
            return;
        }
        if let Some(found) = self
            .quiet("records.get", json!({"collection": collection, "id": id}))
            .await
            && let Some(record) = found["records"].as_array().and_then(|r| r.first())
        {
            value["record"] = if collection == "messages.message" {
                message_summary(record)
            } else {
                compact(record, false, None)
            };
        }
    }

    pub(crate) async fn command_status(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let id = text(input, "request_id").ok_or_else(|| {
            Problem::new("invalid", "request_id is required")
                .field("request_id")
                .expected(json!("the request_id an earlier change returned"))
        })?;
        let operation = self.operation("commands.status")?;
        let mut record = Err(Problem::new("not_found", "no such change"));
        for candidate in status_ids(&self.principal.id, id) {
            record = self
                .invoke(operation, &json!({"command_id": candidate}))
                .await;
            if !record
                .as_ref()
                .is_err_and(|problem| problem.code == "not_found")
            {
                break;
            }
        }
        let record = record.map_err(|problem| {
            if problem.code == "not_found" {
                problem
                    .field("request_id")
                    .problem("no change with this request_id was made by this agent")
                    .hint("Use the request_id from the change's result. A change that returned an error before it started has no status.")
            } else {
                problem
            }
        })?;
        let verbose = flag(input, "verbose");
        let op = record["command"]["op"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if crate::mcp_registry::registry()
            .and_then(|r| r.find(&op))
            .is_some_and(AgentOperation::photos_write)
        {
            return Ok(Outcome::ok(photos_status(&op, id, &record, verbose)).named(&op));
        }
        let mut value = change_result(&record, verbose);
        value["request_id"] = json!(id);
        if let Some(op) = self.registry.find(&op) {
            self.attach_record(&mut value, op).await;
        }
        Ok(Outcome::ok(value).named("commands.status"))
    }

    async fn messages_send(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let operation = self.operation("messages.send")?;
        let photos = input.get("photos").and_then(Value::as_array);
        let body = content(input, "text", &operation.name)?;
        if body.is_none() && photos.is_none() {
            return Err(Problem::new("invalid", "text is required")
                .operation(&operation.name)
                .field("text")
                .hint("Give text, photos (asset_id values from photos_find), or both."));
        }
        let id = request_id(input)?;
        if photos.is_some()
            && let Ok(sent) = self
                .command_status(&Map::from_iter([("request_id".to_owned(), json!(id))]))
                .await
        {
            return Ok(sent);
        }
        let mut arguments = Map::new();
        if let Some(chat) = text(input, "chat_id") {
            if input.contains_key("to") {
                return Err(Problem::new("invalid", "give to or chat_id, not both")
                    .operation(&operation.name)
                    .field("to")
                    .hint("chat_id already names the chat; remove to."));
            }
            arguments.insert("chat_id".into(), json!(chat));
        } else if let Some(who) = text(input, "to") {
            let person = self.person(who, "to").await?;
            let chats = if can(self.principal, "messages:read") {
                self.direct_chats(&person.handles).await?
            } else {
                Vec::new()
            };
            let same_people = chats.windows(2).all(|pair| {
                pair[0]["chat"]["data"]["participants"] == pair[1]["chat"]["data"]["participants"]
                    && pair[0]["chat"]["data"]["service"] == pair[1]["chat"]["data"]["service"]
            });
            if chats.len() > 1 && !same_people {
                return Err(
                    Problem::new("ambiguous", format!("{who} has {} chats", chats.len()))
                        .operation(&operation.name)
                        .field("to")
                        .problem("more than one chat matches")
                        .candidates(json!(chats.iter().map(chat_summary).collect::<Vec<_>>()))
                        .hint("Ask the owner which chat to use, then pass its chat_id."),
                );
            }
            if let Some(chat) = chats.first().and_then(|c| c["chat"]["id"].as_str()) {
                arguments.insert("chat_id".into(), json!(chat));
            } else if let [handle] = person.handles.as_slice() {
                arguments.insert("handles".into(), json!([handle]));
                arguments.insert(
                    "service".into(),
                    json!(text(input, "service").unwrap_or("iMessage")),
                );
            } else {
                return Err(Problem::new(
                    "ambiguous",
                    format!("{who} has several numbers or emails and no existing chat"),
                )
                .operation(&operation.name)
                .field("to")
                .candidates(json!(person.handles))
                .hint("Ask the owner which one to use, then pass it as to."));
            }
        } else {
            return Err(Problem::new("invalid", "to or chat_id is required")
                .operation(&operation.name)
                .field("to")
                .expected(json!("a phone number, email or contact name, or a chat_id"))
                .example(json!({"tool": "messages_send", "arguments": {"to": "+38640123456", "text": "On my way"}})));
        }
        if let Some(body) = body {
            arguments.insert("text".into(), json!(body));
        }
        if let Some(photos) = photos {
            arguments.insert("files".into(), json!(self.photo_attachments(photos).await?));
        }
        mcp_check::arguments(operation, Some(&Value::Object(arguments.clone())))?;
        self.run_change(operation, arguments, &id, false).await
    }

    pub(crate) async fn reminder_list(&self, list: Option<&str>) -> Step<(String, Option<String>)> {
        Ok(match list {
            Some(list) if list.starts_with("lst_") => (list.to_owned(), None),
            _ => {
                if list.is_some() {
                    self.require("reminders:read", "finding a reminder list by name")
                        .map_err(|problem| problem.field("list").hint("Pass the list's lst_ ID instead, or ask the owner to grant reminders:read."))?;
                }
                let lists = self
                    .invoke(
                        self.operation("reminders.lists")?,
                        &json!({"time_zone": mac_zone()}),
                    )
                    .await?;
                let entries: Vec<(String, String)> = lists["lists"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| {
                        Some((
                            l["list"]["id"].as_str()?.to_owned(),
                            l["list"]["data"]["title"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                        ))
                    })
                    .collect();
                pick("list", list, &entries, "reminder list")?
            }
        })
    }

    async fn reminders_add(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let operation = self.operation("reminders.create")?;
        let title = content(input, "title", &operation.name)?.ok_or_else(|| {
            Problem::new("invalid", "title is required")
                .operation(&operation.name)
                .field("title")
        })?;
        let id = request_id(input)?;
        let (list_id, list_name) = self.reminder_list(text(input, "list")).await?;
        let mut arguments = Map::new();
        arguments.insert("list_id".into(), json!(list_id));
        arguments.insert("title".into(), json!(title));
        if let Some(due) = text(input, "due") {
            let due = mcp_check::local_time(due).unwrap_or_else(|| due.to_owned());
            if due.len() == 10 {
                arguments.insert("due_all_day".into(), json!(true));
            }
            arguments.insert("due".into(), json!(due));
        }
        if let Some(notes) = content(input, "notes", &operation.name)? {
            arguments.insert("notes".into(), json!(notes));
        }
        if let Some(priority) = input.get("priority").filter(|p| !p.is_null()) {
            arguments.insert("priority".into(), priority.clone());
        }
        mcp_check::arguments(operation, Some(&Value::Object(arguments.clone())))?;
        let mut outcome = self.run_change(operation, arguments, &id, false).await?;
        if let Some(name) = list_name.filter(|_| can(self.principal, "reminders:read")) {
            outcome.value["list"] = json!(name);
        }
        Ok(outcome)
    }

    async fn contacts_find(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let query = text(input, "query").ok_or_else(|| {
            Problem::new("invalid", "query is required")
                .field("query")
                .expected(json!("part of a name, a phone number or an email"))
        })?;
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(10);
        let operation = self.operation("contacts.lookup")?;
        mcp_check::arguments(operation, Some(&json!({"q": query, "limit": limit})))?;
        let found = self
            .invoke(operation, &json!({"q": query, "limit": limit}))
            .await?;
        let contacts = compact(&found["records"], false, None);
        let count = contacts.as_array().map_or(0, Vec::len);
        Ok(Outcome::ok(json!({
            "contacts": contacts,
            "next": if count == 0 { "Nobody matches. Try part of the name, or a phone number or email." } else { "Use a contact's name, phone or email with messages_send or messages_read." },
        }))
        .named("contacts.lookup"))
    }

    async fn notes_create(&self, input: &Map<String, Value>) -> Step<Outcome> {
        let operation = self.operation("notes.create")?;
        let title = content(input, "title", &operation.name)?.ok_or_else(|| {
            Problem::new("invalid", "title is required")
                .operation(&operation.name)
                .field("title")
        })?;
        let body = input.get("text").and_then(Value::as_str);
        let id = request_id(input)?;
        let folder = text(input, "folder");
        let (folder_id, folder_name) = match folder {
            Some(folder) if folder.starts_with("fld_") => (folder.to_owned(), None),
            _ => {
                if folder.is_some() {
                    self.require("notes:read", "finding a notes folder by name")
                        .map_err(|problem| problem.field("folder").hint("Pass the folder's fld_ ID instead, or ask the owner to grant notes:read."))?;
                }
                let notes = self
                    .invoke(
                        self.operation("notes.list")?,
                        &json!({"time_zone": mac_zone()}),
                    )
                    .await?;
                let mut entries: Vec<(String, String)> = notes["folders"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|f| f["read_only"] != true && f["recently_deleted"] != true)
                    .filter_map(|f| {
                        Some((
                            f["folder"]["id"].as_str()?.to_owned(),
                            f["folder"]["data"]["title"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                        ))
                    })
                    .collect();
                if folder.is_none()
                    && let Some(at) = entries.iter().position(|(_, name)| name == "Notes")
                {
                    entries.swap(0, at);
                }
                pick("folder", folder, &entries, "notes folder").map_err(|problem| {
                    if entries.is_empty() {
                        problem.hint("Create a folder first with apple_change operation notes.create_folder, then pass its fld_ ID as folder.")
                    } else {
                        problem
                    }
                })?
            }
        };
        let mut arguments = Map::new();
        arguments.insert("folder_id".into(), json!(folder_id));
        arguments.insert("title".into(), json!(title));
        if let Some(body) = body {
            arguments.insert("plain_text".into(), json!(body));
        }
        mcp_check::arguments(operation, Some(&Value::Object(arguments.clone())))?;
        let mut outcome = self.run_change(operation, arguments, &id, false).await?;
        if let Some(name) = folder_name.filter(|_| can(self.principal, "notes:read")) {
            outcome.value["folder"] = json!(name);
        }
        Ok(outcome)
    }

    async fn blob_read(&self, arguments: &Map<String, Value>) -> Step<Value> {
        let digest = arguments
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let record = arguments
            .get("query_id")
            .or_else(|| arguments.get("record_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Problem::new("invalid", "query_id or record_id is required")
                    .operation(BLOB_READ)
                    .field("record_id")
                    .hint("Use the query_id or record_id that came with the digest.")
            })?;
        let bytes = crate::blobs::blob_bytes(self.frontdoor, self.principal, record, digest)
            .await
            .map_err(|error| from_error(&error, self.registry.find(BLOB_READ), self.principal))?;
        let offset = usize::try_from(arguments.get("offset").and_then(Value::as_u64).unwrap_or(0))
            .unwrap_or(usize::MAX);
        let length = usize::try_from(
            arguments
                .get("length")
                .and_then(Value::as_u64)
                .unwrap_or(524_288),
        )
        .unwrap_or(524_288)
        .min(524_288);
        let start = offset.min(bytes.len());
        let end = start.saturating_add(length).min(bytes.len());
        Ok(json!({
            "digest": digest,
            "size": bytes.len(),
            "offset": start,
            "length": end - start,
            "done": end == bytes.len(),
            "data_base64": STANDARD.encode(&bytes[start..end]),
        }))
    }

    async fn blob_stage(
        &self,
        operation: &AgentOperation,
        arguments: &Map<String, Value>,
    ) -> Step<Outcome> {
        let data = arguments
            .get("data_base64")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let bytes = STANDARD
            .decode(data.replace(['\r', '\n'], ""))
            .map_err(|_| {
                Problem::new("invalid", "data_base64 is not base64")
                    .operation(&operation.name)
                    .field("data_base64")
            })?;
        match self
            .frontdoor
            .service
            .upload_message(self.principal, &bytes)
            .await
        {
            Ok(raw) => {
                let value: Value = serde_json::from_str(raw.get()).unwrap_or(Value::Null);
                Ok(Outcome::ok(json!({
                    "status": "done",
                    "upload_id": value["upload_id"],
                    "size": bytes.len(),
                    "next": "Pass upload_id as files[].upload_id with a name in messages.send.",
                }))
                .named(BLOB_STAGE))
            }
            Err(error) => Err(from_error(&error, Some(operation), self.principal)),
        }
    }
}

fn line(operation: &AgentOperation) -> String {
    let kind = if operation.changes && !operation.read_only() {
        "change"
    } else {
        "read"
    };
    format!("{} ({kind}): {}", operation.name, operation.summary)
}

fn route_line(route: &crate::mcp_find::Route) -> String {
    format!(
        "records.query {{\"collection\":\"{}\"}} (read): {}",
        route.collection, route.summary
    )
}

pub(crate) fn pick(
    field: &str,
    wanted: Option<&str>,
    entries: &[(String, String)],
    what: &str,
) -> Step<(String, Option<String>)> {
    let Some(wanted) = wanted else {
        return entries
            .first()
            .map(|(id, name)| (id.clone(), Some(name.clone())))
            .ok_or_else(|| {
                Problem::new("not_found", format!("there is no {what} yet")).field(field.to_owned())
            });
    };
    if let Some((id, name)) = entries.iter().find(|(id, _)| id == wanted) {
        return Ok((id.clone(), Some(name.clone())));
    }
    let lower = fold(wanted);
    let exact: Vec<&(String, String)> = entries.iter().filter(|(_, n)| fold(n) == lower).collect();
    let partial: Vec<&(String, String)> = entries
        .iter()
        .filter(|(_, n)| fold(n).contains(&lower))
        .collect();
    let chosen = if exact.is_empty() { partial } else { exact };
    match chosen.as_slice() {
        [(id, name)] => Ok((id.clone(), Some(name.clone()))),
        [] => Err(
            Problem::new("not_found", format!("no {what} is called {wanted}"))
                .field(field.to_owned())
                .candidates(json!(
                    entries
                        .iter()
                        .map(|(id, name)| json!({"name": name, "id": id}))
                        .collect::<Vec<_>>()
                ))
                .hint(format!(
                    "Use one of the candidates' names or ids for {field}."
                )),
        ),
        many => Err(Problem::new(
            "ambiguous",
            format!("{} entries match {wanted}", many.len()),
        )
        .field(field.to_owned())
        .candidates(json!(
            many.iter()
                .map(|(id, name)| json!({"name": name, "id": id}))
                .collect::<Vec<_>>()
        ))
        .hint(format!("Pass the exact name or id for {field}."))),
    }
}

pub(crate) fn fold(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn collection_of(id: &str) -> Option<String> {
    let (prefix, _) = id.split_once('_')?;
    let collection = ab_operations::domain::Collection::ALL
        .iter()
        .find(|collection| collection.prefix() == prefix)?;
    serde_json::to_value(collection)
        .ok()?
        .as_str()
        .map(str::to_owned)
}

pub(crate) fn contact_name(person: &Value) -> String {
    let data = &person["data"];
    let parts: Vec<&str> = ["given_name", "middle_name", "family_name"]
        .iter()
        .filter_map(|k| data[*k].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        data["organization"]
            .as_str()
            .or_else(|| data["display_name"].as_str())
            .unwrap_or_default()
            .to_owned()
    } else {
        parts.join(" ")
    }
}

pub(crate) fn contact_handles(person: &Value) -> Vec<String> {
    let data = &person["data"];
    let mut handles = Vec::new();
    for key in ["phones", "emails"] {
        for entry in data[key].as_array().into_iter().flatten() {
            let value = entry["value"]
                .as_str()
                .or_else(|| entry.as_str())
                .unwrap_or_default();
            if value.is_empty() {
                continue;
            }
            let handle = if key == "phones" {
                value
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '+')
                    .collect()
            } else {
                value.to_owned()
            };
            if !handles.contains(&handle) {
                handles.push(handle);
            }
        }
    }
    handles
}

pub(crate) fn chat_summary(chat: &Value) -> Value {
    let mut value = json!({
        "chat_id": chat["chat"]["id"],
        "title": chat["title"],
        "group": chat["chat"]["data"]["style"] == "group",
        "unread_count": chat["unread_count"],
        "last_activity_at": chat["last_activity_at"],
        "last_message": chat["last_message"],
        "service": chat["chat"]["data"]["service"],
        "participants": chat["chat"]["data"]["participants"].as_array().map_or(Value::Null, |p| json!(p)),
    });
    if let Some(fields) = value.as_object_mut() {
        fields.retain(|_, v| !v.is_null());
    }
    value
}

pub(crate) fn message_summary(message: &Value) -> Value {
    let data = &message["data"];
    let mut value = json!({
        "id": message["id"],
        "version": message["version"],
        "chat_id": data["chat_id"],
        "sent_at": data["sent_at"],
        "from_me": data["is_from_me"],
        "sender_handle_id": data["sender_handle_id"],
        "text": data["text"],
    });
    if let Some(local) = local_time_of(data["sent_at"].as_str()) {
        value["sent_at_local"] = json!(local);
    }
    for key in [
        "read_at",
        "delivered_at",
        "thread_originator_id",
        "reply_count",
    ] {
        if let Some(found) = data
            .get(key)
            .or_else(|| message.get(key))
            .filter(|v| !v.is_null())
        {
            value[key] = found.clone();
        }
    }
    if data["decode_status"]
        .as_str()
        .is_some_and(|status| status != "decoded")
    {
        value["decode_status"] = data["decode_status"].clone();
    }
    if data["error_code"].as_i64().is_some_and(|code| code != 0) {
        value["error_code"] = data["error_code"].clone();
        value["delivery"] = json!("failed");
    }
    if data["is_edited"] == true
        || data["edit_history"]
            .as_array()
            .is_some_and(|h| !h.is_empty())
    {
        value["edited"] = json!(true);
    }
    if data["retracted_parts"]
        .as_array()
        .is_some_and(|r| !r.is_empty())
    {
        value["unsent_parts"] = data["retracted_parts"].clone();
    }
    if let Some(attachments) = message["attachments"].as_array().filter(|a| !a.is_empty()) {
        value["attachments"] = json!(
            attachments
                .iter()
                .map(attachment_summary)
                .collect::<Vec<_>>()
        );
        value["attachments_next"] = json!(
            "Fetch an attachment with apple_read blobs.read, passing its digest and its id as record_id."
        );
    }
    if let Some(reactions) = message["reactions"].as_array().filter(|r| !r.is_empty()) {
        value["reactions"] = json!(
            reactions
                .iter()
                .map(|r| {
                    let data = &r["data"];
                    let mut out = json!({
                        "id": r["id"],
                        "kind": data["kind"].as_str().or_else(|| data["type"].as_str()).map_or(Value::Null, |k| json!(k)),
                        "emoji": data["emoji"],
                        "from_me": data["is_from_me"],
                        "sender_handle_id": data["sender_handle_id"],
                    });
                    if let Some(fields) = out.as_object_mut() {
                        fields.retain(|_, v| !v.is_null());
                    }
                    out
                })
                .collect::<Vec<_>>()
        );
    }
    if let Some(fields) = value.as_object_mut() {
        fields.retain(|_, v| !v.is_null());
    }
    value
}

fn attachment_summary(attachment: &Value) -> Value {
    let data = &attachment["data"];
    let mut value = json!({
        "id": attachment["id"],
        "version": attachment["version"],
        "filename": data["filename"].as_str().or_else(|| data["name"].as_str()).map_or(Value::Null, |n| json!(n)),
        "mime_type": data["mime_type"],
        "size": data["size"],
        "digest": data["blob"]["digest"].as_str().or_else(|| data["digest"].as_str()).map_or(Value::Null, |d| json!(d)),
        "state": data["blob"]["state"],
    });
    if data["hidden"] == true {
        value["hidden"] = json!(true);
    }
    if data["is_sticker"] == true {
        value["sticker"] = json!(true);
    }
    if let Some(fields) = value.as_object_mut() {
        fields.retain(|_, v| !v.is_null());
    }
    value
}

const INTERNAL: &[&str] = &[
    "fingerprint",
    "native_key",
    "capture_checkpoint",
    "prepared",
    "ticket",
    "observed",
    "absent",
    "resolved",
    "candidate_ids",
    "candidate_count",
    "ambiguous",
    "route",
    "t",
    "command_id",
    "expires_at",
    "request_hash",
    "owner_fence",
    "handle",
];

fn without_internal(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(k, _)| !INTERNAL.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), without_internal(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_internal).collect()),
        other => other.clone(),
    }
}

fn part_outcome(disposition: &str) -> &'static str {
    match disposition {
        "observed" | "delivered" => "sent",
        "delivery_failed" => "failed",
        "failed_no_effect" => "not_sent",
        _ => "unknown",
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn change_result(record: &Value, verbose: bool) -> Value {
    let state = record["state"].as_str().unwrap_or_default();
    let disposition = record["disposition"].as_str().unwrap_or_default();
    let request_id = record["command"]["command_id"].as_str().unwrap_or_default();
    let operation = record["command"]["op"].as_str().unwrap_or_default();
    let sends = matches!(operation, "messages.send" | "messages.reply" | "mail.send");
    let parts: Vec<Value> = record["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|part| {
            let mut out = json!({
                "index": part["index"],
                "kind": part["kind"],
                "outcome": part_outcome(part["disposition"].as_str().unwrap_or_default()),
            });
            if let Some(id) = part["record_id"].as_str().filter(|id| !id.is_empty()) {
                out["id"] = json!(id);
            }
            out
        })
        .collect();
    let succeeded = parts.iter().filter(|p| p["outcome"] == "sent").count();
    let failed_parts = parts.len() - succeeded;
    let base = match state {
        "awaiting_approval" => "awaiting_approval",
        "admitted" | "prepared" | "dispatched" => "pending",
        "observed" | "delivered" => "done",
        "indeterminate" => "indeterminate",
        _ => "failed",
    };
    let status = match base {
        "failed" if succeeded > 0 => "partial",
        "done" if failed_parts > 0 && parts.len() > 1 => "partial",
        other => other,
    };
    let mut ids: Vec<Value> = record["observed_record_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for part in &parts {
        if let Some(id) = part["id"].as_str()
            && !ids.iter().any(|known| known == id)
        {
            ids.push(json!(id));
        }
    }
    if ids.is_empty()
        && let Some(id) = record["command"]["args"]["id"].as_str()
    {
        ids.push(json!(id));
    }
    let no_effect = matches!(
        disposition,
        "rejected_before_dispatch"
            | "rejected_by_owner"
            | "failed_no_effect"
            | "expired"
            | "absent_confirmed"
    );
    let verification = match disposition {
        _ if status == "partial" => "Some parts happened and some did not; see parts.",
        "locally_observed" if sends => {
            "The Mac's app shows it sent. That does not prove the recipient got it."
        }
        "locally_observed" => "Read back from the Apple app on the Mac.",
        "delivered" => "Delivery was reported.",
        "delivery_failed" => "The Mac's app shows it, marked as not delivered.",
        "rejected_before_dispatch" => "Refused before anything was sent to the Apple app.",
        "rejected_by_owner" => "The owner declined it.",
        "failed_no_effect" => "It failed without changing anything.",
        "expired" => "It expired before it ran.",
        "absent_confirmed" => "Confirmed it did not happen.",
        "indeterminate" => "Nobody can tell whether it happened.",
        "awaiting_approval" => "Waiting for the owner.",
        _ => "",
    };
    let described: Vec<String> = parts
        .iter()
        .map(|p| {
            format!(
                "part {} ({}) {}",
                p["index"].as_u64().unwrap_or_default() + 1,
                p["kind"].as_str().unwrap_or("item"),
                p["outcome"].as_str().unwrap_or("unknown").replace('_', " ")
            )
        })
        .collect();
    let next = match status {
        "done" if sends => "Done. Do not send it again.".to_owned(),
        "done" => "Done. Use record.version as expected_version for the next change to this record.".to_owned(),
        "partial" => format!(
            "Part of this happened: {}. Do not repeat this request, with this or a new request_id: that would repeat the parts that worked. Tell the owner which part failed; if they want it, send only that part as a new request.",
            described.join(", ")
        ),
        "pending" => format!("Still running on the Mac. Call apple_command_status with request_id {request_id} in a few seconds. Do not repeat it with a new request_id."),
        "awaiting_approval" => format!("The owner set this agent to ask first. Tell them what you asked for; after they approve it in Apple Bridge, call apple_command_status with request_id {request_id}."),
        "indeterminate" => "Nobody can tell whether it happened. Do not retry with a new request_id; tell the owner.".to_owned(),
        _ if no_effect => "It did not happen and nothing changed. Fix the cause before trying again with a new request_id.".to_owned(),
        _ if disposition == "delivery_failed" => "The Mac's app shows it as not delivered. Sending it again with a new request_id makes a second copy; tell the owner and let them decide.".to_owned(),
        _ => "It failed, but that does not prove nothing changed. Read the record or ask the owner before trying again with a new request_id.".to_owned(),
    };
    let mut value = json!({
        "status": status,
        "request_id": request_id,
        "operation": operation,
        "ids": ids,
        "next": next,
    });
    if parts.len() > 1 || failed_parts > 0 {
        value["parts"] = json!(parts);
    }
    if !verification.is_empty() {
        value["verification"] = json!(verification);
    }
    if let Some(error) = record.get("error").filter(|e| !e.is_null()) {
        value["error"] = json!({"code": error["code"], "message": error["message"]});
    }
    let evidence = &record["evidence"];
    if let Some(blob) = evidence["blob_id"].as_str() {
        value["files"] = json!([{"query_id": blob, "digest": evidence["digest"], "size": evidence["size"], "fetch": "apple_read blobs.read"}]);
    }
    if let Some(fields) = evidence.as_object() {
        let result: Map<String, Value> = fields
            .iter()
            .filter(|(k, _)| !INTERNAL.contains(&k.as_str()) && k.as_str() != "blob_id")
            .map(|(k, v)| (k.clone(), without_internal(v)))
            .collect();
        if !result.is_empty() {
            value["result"] = Value::Object(result);
        }
    }
    if verbose {
        value["details"] = record.clone();
    }
    value
}

fn photos_ids(arguments: &Map<String, Value>, result: &Value) -> Vec<Value> {
    let mut ids = Vec::new();
    for key in ["album_id", "asset_id"] {
        if let Some(id) = result[key]
            .as_str()
            .or_else(|| arguments.get(key).and_then(Value::as_str))
        {
            ids.push(json!(id));
        }
    }
    for key in ["asset_ids"] {
        for id in arguments
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            ids.push(id.clone());
        }
    }
    ids
}

fn photos_change(
    name: &str,
    request_id: &str,
    arguments: &Map<String, Value>,
    value: &Value,
    verbose: bool,
) -> Value {
    if value["state"] == "awaiting_approval" {
        return json!({
            "status": "awaiting_approval",
            "request_id": request_id,
            "operation": name,
            "ids": [],
            "next": format!("The owner set this agent to ask first. Tell them what you asked for; after they approve it, repeat this exact call with request_id {request_id} once."),
        });
    }
    let replayed = value["replayed"] == true;
    if value["result_kept"] == false {
        return json!({
            "status": "done",
            "request_id": request_id,
            "operation": name,
            "ids": photos_ids(arguments, &Value::Null),
            "verification": "This request had already run; its result was not kept. Nothing ran again.",
            "next": "Done earlier. Read the album or photo to see the result. Do not repeat it with a new request_id.",
        });
    }
    let mut result = json!({
        "status": "done",
        "request_id": request_id,
        "operation": name,
        "ids": photos_ids(arguments, &value["result_json"]),
        "result": compact(&value["result_json"], verbose, None),
        "verification": if replayed { "This request had already run; this is its kept result. Nothing ran again." } else { "Read back from Photos on the Mac." },
        "next": "Done. Do not repeat it with a new request_id.",
    });
    if verbose {
        result["details"] = value.clone();
    }
    result
}

fn photos_status(name: &str, request_id: &str, record: &Value, verbose: bool) -> Value {
    let state = record["state"].as_str().unwrap_or_default();
    let kept = &record["evidence"]["result"]["result_json"];
    let arguments = record["command"]["args"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut value = match state {
        "observed" | "delivered" if !kept.is_null() => json!({
            "status": "done",
            "ids": photos_ids(&arguments, kept),
            "result": compact(kept, false, None),
            "next": "Done. Do not repeat it with a new request_id.",
        }),
        "observed" | "delivered" => json!({
            "status": "done",
            "ids": [],
            "next": "Done. To see its result again, repeat the exact same call with this request_id within ten minutes; it does not run twice. Never repeat it with a new request_id.",
        }),
        "awaiting_approval" => json!({
            "status": "awaiting_approval",
            "ids": [],
            "next": "Still waiting for the owner. After they approve it, repeat the exact call with this request_id once.",
        }),
        "admitted" | "prepared" | "dispatched" => json!({
            "status": "pending",
            "ids": [],
            "next": "Approved and not finished yet. Repeat the exact call with this request_id once to run it, or check again in a few seconds.",
        }),
        "indeterminate" => json!({
            "status": "indeterminate",
            "ids": [],
            "next": "It was sent to Photos but nobody can tell whether it happened. Do not repeat it with a new request_id; check the album or photo, or tell the owner.",
        }),
        _ => json!({
            "status": "failed",
            "ids": [],
            "next": "It did not run. Fix the cause before trying again with a new request_id.",
        }),
    };
    value["request_id"] = json!(request_id);
    value["operation"] = json!(name);
    if verbose {
        value["details"] = record.clone();
    }
    value
}

pub(crate) fn reminder_lists(value: &Value) -> Value {
    let lists: Vec<Value> = value["lists"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let record = &entry["list"];
            let mut list = json!({
                "id": record["id"],
                "version": record["version"],
                "title": record["data"]["title"],
                "count": entry["count"],
            });
            if record["data"]["is_group"] == true {
                list["group"] = json!(true);
            } else if entry["status"] == "unsupported" {
                list["custom_smart_list"] = json!(true);
            }
            if entry["group_id"].is_string() {
                list["in_group"] = entry["group_id"].clone();
            }
            if let Some(fields) = list.as_object_mut() {
                fields.retain(|_, v| !v.is_null());
            }
            list
        })
        .collect();
    let smart: Map<String, Value> = value["smart"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["id"].as_str()?.to_owned(), s.get("count")?.clone())))
        .collect();
    let mut out = json!({
        "count": lists
            .iter()
            .filter(|l| l.get("group").is_none() && l.get("custom_smart_list").is_none())
            .count(),
        "lists": lists,
        "smart_counts": smart,
        "next": "Read a list with reminders.items and list set to its title, or smart set to today.",
    });
    if let Some(tags) = value["tags"].as_array().filter(|t| !t.is_empty()) {
        out["tags"] = json!(tags);
    }
    out
}

fn query_result(value: &Value, verbose: bool, fields: Option<&[String]>) -> Value {
    if verbose {
        return value.clone();
    }
    if value["state"] == "awaiting_approval" {
        let id = value["id"].as_str().unwrap_or_default();
        return json!({
            "status": "awaiting_approval",
            "query_id": id,
            "next": format!("The owner set this agent to ask first. Tell them; after they approve, repeat this exact call with request_id {id} within ten minutes."),
        });
    }
    let mut out = match compact(&value["result_json"], false, fields) {
        Value::Object(fields) => Value::Object(fields),
        Value::Null => json!({}),
        other => json!({ "result": other }),
    };
    if let Some(blobs) = value["blobs"].as_array().filter(|b| !b.is_empty()) {
        out["files"] =
            json!(blobs
            .iter()
            .map(|b| json!({"query_id": value["id"], "digest": b["digest"], "size": b["size"]}))
            .collect::<Vec<_>>());
        out["files_next"] = json!(
            "Fetch the bytes with apple_read operation blobs.read, passing digest and query_id."
        );
    }
    out
}

pub(crate) fn compact(value: &Value, verbose: bool, fields: Option<&[String]>) -> Value {
    if verbose {
        return value.clone();
    }
    match value {
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| compact(v, false, fields)).collect())
        }
        Value::Object(map) => {
            if map.contains_key("collection")
                && map.contains_key("id")
                && map.contains_key("version")
                && map.contains_key("data")
            {
                return record(map, fields);
            }
            let mut out = Map::new();
            for (key, child) in map {
                if key == "store_coverage" {
                    continue;
                }
                if key == "cursor" && child.get("epoch").is_some() {
                    continue;
                }
                if matches!(key.as_str(), "coverage_notes" | "aliases")
                    && child.as_array().is_some_and(Vec::is_empty)
                {
                    continue;
                }
                out.insert(key.clone(), compact(child, false, fields));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

const RECORD_KEYS: &[&str] = &[
    "collection",
    "id",
    "version",
    "data",
    "coverage",
    "deleted",
    "tombstone_reason",
    "observed_at",
    "source",
    "created_at",
    "updated_at",
    "aliases",
    "store_coverage",
];

pub(crate) fn local_time_of(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    if raw.len() <= 10 {
        return None;
    }
    let timestamp: jiff::Timestamp = raw.parse().ok()?;
    let zone = jiff::tz::TimeZone::get(&mac_zone()).unwrap_or(jiff::tz::TimeZone::UTC);
    Some(
        timestamp
            .to_zoned(zone)
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
    )
}

fn record(map: &Map<String, Value>, fields: Option<&[String]>) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), map["id"].clone());
    out.insert("version".into(), map["version"].clone());
    if map.get("deleted") == Some(&Value::Bool(true)) {
        out.insert("deleted".into(), json!(true));
        if let Some(reason) = map.get("tombstone_reason").filter(|r| !r.is_null()) {
            out.insert("tombstone_reason".into(), reason.clone());
        }
    }
    let coverage = &map.get("coverage").cloned().unwrap_or(Value::Null);
    let missing = coverage["missing"].as_array().filter(|m| !m.is_empty());
    if coverage["fields"].as_str().is_some_and(|f| f != "complete") || missing.is_some() {
        out.insert(
            "incomplete".into(),
            json!({"fields": coverage["fields"], "missing": missing.cloned().unwrap_or_default()}),
        );
    }
    let mut data = compact(&map["data"], false, None);
    if let Some(object) = data.as_object_mut() {
        object.retain(|_, value| {
            !(value.is_null()
                || value.as_array().is_some_and(Vec::is_empty)
                || value.as_object().is_some_and(Map::is_empty))
        });
        if map["collection"] == "calendar.event" && object.get("all_day") != Some(&json!(true)) {
            for key in ["start", "end"] {
                if let Some(local) = local_time_of(object.get(key).and_then(Value::as_str)) {
                    object.insert(format!("{key}_local"), json!(local));
                }
            }
        }
        if map["collection"] == "reminders.reminder"
            && let Some(local) = local_time_of(object.get("due").and_then(Value::as_str))
        {
            object.insert("due_local".into(), json!(local));
        }
    }
    if let (Some(keep), Some(object)) = (fields, data.as_object_mut()) {
        object.retain(|key, _| {
            keep.iter().any(|k| {
                k == key
                    || k.strip_suffix("_local").is_none() && key.strip_suffix("_local") == Some(k)
            })
        });
    }
    out.insert("data".into(), data);
    for (key, child) in map {
        if !RECORD_KEYS.contains(&key.as_str())
            && !child.is_null()
            && !child.as_array().is_some_and(Vec::is_empty)
        {
            out.insert(key.clone(), compact(child, false, None));
        }
    }
    Value::Object(out)
}
