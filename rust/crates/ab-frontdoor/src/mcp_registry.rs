use ab_catalog::{AgentOperation, OperationKind, VersionRule};
use ab_operations::Principal;
use serde_json::{Value, json};
use std::sync::OnceLock;

pub(crate) const COMPACT: &[&str] = &[
    "apple_status",
    "apple_describe",
    "apple_search",
    "apple_read",
    "apple_change",
    "apple_command_status",
    "apple_guide",
    "messages_read",
    "messages_send",
    "reminders_add",
    "calendar_events",
    "calendar_add",
    "contacts_find",
    "notes_create",
    "screen_time",
    "photos_find",
    "photos_show",
];

pub(crate) const BLOB_READ: &str = "blobs.read";
pub(crate) const BLOB_STAGE: &str = "blobs.stage";
pub(crate) const USAGE_SUMMARY: &str = "usage.summary";
pub(crate) const CALENDARS: &str = "calendar.calendars";
pub(crate) const PHOTOS_FIND_MAX: u64 = 30;
pub(crate) const BY_TITLE: &[&str] = &[
    "reminders.complete",
    "reminders.uncomplete",
    "reminders.update",
    "reminders.delete",
];

#[derive(Debug)]
pub(crate) struct Registry {
    pub operations: Vec<AgentOperation>,
    pub domains: Vec<(String, String)>,
    tools: Vec<Value>,
    examples: Vec<(String, Value)>,
}

pub(crate) fn registry() -> Option<&'static Registry> {
    static REGISTRY: OnceLock<Option<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(load).as_ref()
}

fn load() -> Option<Registry> {
    let descriptors = ab_catalog::operations().ok()?;
    let mut operations = ab_catalog::agent_operations(&descriptors).ok()?;
    for operation in &mut operations {
        patch(operation);
    }
    operations.extend(blob_operations());
    operations.extend(composite_operations());
    let mut domains = ab_catalog::agent_domains().ok()?;
    domains.push((
        "usage".into(),
        "Screen Time: app usage per device and day, estimated from app focus.".into(),
    ));
    domains.sort();
    let mut tools: Vec<Value> = serde_json::from_str(include_str!("mcp_tools.json")).ok()?;
    let change = change_schema();
    let mut examples = Vec::new();
    for tool in &mut tools {
        if tool["outputSchema"]["$ref"] == "#change" {
            tool["outputSchema"] = change.clone();
        }
        let name = tool["name"].as_str().unwrap_or_default().to_owned();
        if let Some(example) = tool.as_object_mut().and_then(|t| t.remove("example")) {
            examples.push((name, example));
        }
    }
    Some(Registry {
        operations,
        domains,
        tools,
        examples,
    })
}

fn patch(operation: &mut AgentOperation) {
    if operation.kind != OperationKind::Command
        && operation.arguments["properties"].get("time_zone").is_some()
    {
        if let Some(required) = operation.arguments["required"].as_array_mut() {
            required.retain(|field| field != "time_zone");
        }
        operation.arguments["properties"]["time_zone"]["description"] =
            json!("Optional. IANA time zone; default the Mac's.");
    }
    if operation.name == "reminders.items" {
        operation.arguments["properties"]["list"]["description"] =
            json!("A list's name, such as Groceries, or its lst_ ID.");
    }
    if operation.name == "calendar.create_event" {
        operation.arguments["properties"]["calendar"] = json!({"type":"string","description":"Calendar name, such as Work, instead of calendar_id. A name that matches no calendar or several returns the candidates."});
        operation.arguments["properties"]["calendar_id"]["description"] = json!(
            "Calendar to add it to: a cal_ ID from calendar.calendars. Or give calendar with its name."
        );
        if let Some(required) = operation.arguments["required"].as_array_mut() {
            required.retain(|field| field != "calendar_id");
        }
        operation.detail = "Give calendar (a name) or calendar_id. Without either, the first writable calendar is used.".into();
    }
    if operation.name == "calendar.agenda" {
        operation.arguments["properties"]["start"]["description"] = json!(
            "Window start: RFC 3339 timestamp, or YYYY-MM-DD for midnight in the Mac's time zone."
        );
        operation.arguments["properties"]["end"]["description"] = json!(
            "Window end, after start: RFC 3339 timestamp, or YYYY-MM-DD for midnight in the Mac's time zone."
        );
        operation.arguments["properties"]["calendar_id"]["description"] = json!(
            "Only events in this calendar (a cal_ ID from calendar.calendars). Omit for all calendars."
        );
    }
    if BY_TITLE.contains(&operation.name.as_str()) {
        operation.arguments["properties"]["reminder"] = json!({"type":"string","description":"The reminder's title, instead of id and expected_version. It must match one reminder; otherwise you get the candidates."});
        operation.arguments["properties"]["id"]["description"] =
            json!("The reminder's rmd_ ID from reminders.items. Or give reminder with its title.");
        if let Some(required) = operation.arguments["required"].as_array_mut() {
            required.retain(|field| field != "id" && field != "expected_version");
        }
        operation.detail = "Give id with expected_version from your read, or only reminder with its title; the current version is then used.".into();
    }
}

fn composite_operations() -> Vec<AgentOperation> {
    let read = json!({"readOnlyHint":true,"openWorldHint":false});
    vec![
        AgentOperation {
            name: USAGE_SUMMARY.into(),
            domain: "usage".into(),
            kind: OperationKind::Read,
            changes: false,
            scope: "usage:read".into(),
            summary: "Summarise Screen Time: total app usage and top apps per device (Mac, iPhone, iPad) for a day or a range of days.".into(),
            detail: "Times are estimates from app focus on each device, so they can differ from the Screen Time app. Days are in the Mac's time zone.".into(),
            returns: "total_seconds and top_apps overall, and devices with name, platform, total_seconds, top_apps and per-day totals.".into(),
            arguments: json!({"type":"object","additionalProperties":false,"properties":{
                "date":{"type":"string","description":"One day as YYYY-MM-DD, such as yesterday's date."},
                "start":{"type":"string","description":"First day of a range, YYYY-MM-DD."},
                "end":{"type":"string","description":"Last day of a range, YYYY-MM-DD, inclusive."},
                "device":{"type":"string","description":"Only this device: a dev_ ID or part of its name, such as iPhone."},
                "top":{"type":"integer","minimum":1,"maximum":50,"description":"Top apps to list. Default 10."}
            },"required":[]}),
            example: json!({"date":"2026-11-09"}),
            version: VersionRule::None,
            unavailable: None,
            annotations: read.clone(),
        },
        AgentOperation {
            name: CALENDARS.into(),
            domain: "calendar".into(),
            kind: OperationKind::Read,
            changes: false,
            scope: "calendar:read".into(),
            summary: "List the calendars, such as Work or Family, with their cal_ IDs and whether events can be added.".into(),
            detail: String::new(),
            returns: "calendars with id (cal_), title, writable, type and account.".into(),
            arguments: json!({"type":"object","additionalProperties":false,"properties":{},"required":[]}),
            example: json!({}),
            version: VersionRule::None,
            unavailable: None,
            annotations: read,
        },
    ]
}

pub(crate) fn change_schema() -> Value {
    json!({"type":"object","properties":{
        "status":{"type":"string","enum":["done","partial","pending","awaiting_approval","failed","indeterminate"]},
        "request_id":{"type":"string"},
        "operation":{"type":"string"},
        "ids":{"type":"array","items":{"type":"string"}},
        "record":{"type":"object"},
        "parts":{"type":"array","items":{"type":"object"}},
        "verification":{"type":"string"},
        "next":{"type":"string"}
    }})
}

fn blob_operations() -> Vec<AgentOperation> {
    let reference = json!({
        "digest":{"type":"string","minLength":64,"maxLength":64,"description":"SHA-256 of the bytes, 64 lowercase hex characters, from the result that mentioned the file."},
        "query_id":{"type":"string","description":"The qry_ ID of the read or capture that returned the file. Give this, or record_id."},
        "record_id":{"type":"string","description":"The record the file belongs to, such as an att_ message attachment. Give this, or query_id."},
        "offset":{"type":"integer","minimum":0,"description":"Byte offset to start from. Default 0."},
        "length":{"type":"integer","minimum":1,"maximum":524_288,"description":"Bytes to return, up to 524288. Default 524288."}
    });
    vec![
        AgentOperation {
            name: BLOB_READ.into(),
            domain: "blobs".into(),
            kind: OperationKind::Read,
            changes: false,
            scope: String::new(),
            summary: "Fetch the bytes of a file a result referred to, such as an attachment, capture or query output, in chunks.".into(),
            detail: "Use the digest with the query_id or record_id that came with it. Repeat with a larger offset until done is true. Access follows the record's source or the query's owner.".into(),
            returns: "data_base64 with the requested bytes, size of the whole file, and done when the last byte was returned.".into(),
            arguments: json!({"type":"object","additionalProperties":false,"properties":reference,"required":["digest"]}),
            example: json!({"digest":"4f0c0f5e8f6a1c6b2a7d6f1e3b9c0d2e4f6a8b0c1d3e5f7a9b1c3d5e7f9a1b3c","record_id":"att_01J9Z8Y7X6W5V4T3S2R1Q0P9N8"}),
            version: VersionRule::None,
            unavailable: None,
            annotations: json!({"readOnlyHint":true,"openWorldHint":false}),
        },
        AgentOperation {
            name: BLOB_STAGE.into(),
            domain: "blobs".into(),
            kind: OperationKind::Command,
            changes: true,
            scope: "messages:write".into(),
            summary: "Upload a file's bytes once and get an upload_id to attach to messages.send.".into(),
            detail: "For files up to about 700 KB through MCP; larger files can be sent inline as files[].data in smaller messages or uploaded by the owner through POST /v1/blobs.".into(),
            returns: "upload_id to use as files[].upload_id in messages.send.".into(),
            arguments: json!({"type":"object","additionalProperties":false,"properties":{
                "data_base64":{"type":"string","minLength":1,"description":"The file's bytes, base64 encoded."}
            },"required":["data_base64"]}),
            example: json!({"data_base64":"aGVsbG8K"}),
            version: VersionRule::None,
            unavailable: None,
            annotations: json!({"readOnlyHint":false,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}),
        },
    ]
}

impl Registry {
    pub fn find(&self, name: &str) -> Option<&AgentOperation> {
        let normal = normal_name(name);
        self.operations
            .iter()
            .find(|operation| operation.name == normal || operation.name.replace('.', "_") == name)
    }

    pub fn example(&self, tool: &str) -> Value {
        self.examples
            .iter()
            .find(|(name, _)| name == tool)
            .map_or_else(|| json!({}), |(_, example)| example.clone())
    }

    pub fn tool_schema(&self, tool: &str) -> Option<&Value> {
        self.tools
            .iter()
            .find(|t| t["name"] == tool)
            .map(|t| &t["inputSchema"])
    }

    pub fn tools(&self, principal: Option<&Principal>) -> Vec<Value> {
        self.tools
            .iter()
            .filter(|tool| {
                let name = tool["name"].as_str().unwrap_or_default();
                let scope = tool["scope"].as_str().unwrap_or_default();
                let writes = tool["annotations"]["readOnlyHint"] == false;
                principal.is_none_or(|principal| {
                    if writes && read_only(principal) {
                        return false;
                    }
                    if name == "apple_change" {
                        return self.operations.iter().any(|operation| {
                            operation.changes
                                && !operation.read_only()
                                && allowed(principal, operation)
                        });
                    }
                    scope.is_empty() || can(principal, scope)
                })
            })
            .map(|tool| {
                let mut tool = tool.clone();
                if let Some(fields) = tool.as_object_mut() {
                    fields.remove("scope");
                }
                tool
            })
            .collect()
    }

    pub fn visible<'a>(
        &'a self,
        principal: Option<&'a Principal>,
    ) -> impl Iterator<Item = &'a AgentOperation> + 'a {
        self.operations
            .iter()
            .filter(move |operation| principal.is_none_or(|p| allowed(p, operation)))
    }

    pub fn closest(&self, name: &str, principal: Option<&Principal>) -> Vec<String> {
        let wanted = normal_name(name).to_lowercase();
        let mut scored: Vec<(usize, &str)> = self
            .visible(principal)
            .map(|operation| (distance(&wanted, &operation.name), operation.name.as_str()))
            .collect();
        scored.sort_unstable();
        scored
            .into_iter()
            .take_while(|(score, _)| *score <= wanted.len().max(4) / 2 + 2)
            .take(3)
            .map(|(_, name)| name.to_owned())
            .collect()
    }
}

pub(crate) fn normal_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.contains('.') {
        return trimmed.to_owned();
    }
    let Some(registry) = registry() else {
        return trimmed.to_owned();
    };
    registry
        .operations
        .iter()
        .find(|operation| operation.name.replace('.', "_") == trimmed)
        .map_or_else(|| trimmed.to_owned(), |operation| operation.name.clone())
}

pub(crate) fn valid_id(id: &str) -> bool {
    ab_operations::valid_command_id(id)
}

pub(crate) fn can(principal: &Principal, scope: &str) -> bool {
    scope.is_empty()
        || principal
            .grants
            .iter()
            .any(|grant| grant == "admin" || grant == scope)
}

pub(crate) fn read_only(principal: &Principal) -> bool {
    principal.level == "read" && matches!(principal.kind.as_str(), "agent" | "delegated")
}

pub(crate) fn allowed(principal: &Principal, operation: &AgentOperation) -> bool {
    if !can(principal, &operation.scope) {
        return false;
    }
    !(operation.changes && read_only(principal))
}

pub(crate) fn agent_id(principal: &Principal) -> &str {
    principal
        .id
        .split_once('/')
        .map_or(principal.id.as_str(), |(_, local)| local)
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, right) in b.iter().enumerate() {
            let cost = usize::from(left != *right);
            current.push(
                (previous[j] + cost)
                    .min(previous[j + 1] + 1)
                    .min(current[j] + 1),
            );
        }
        previous = current;
    }
    previous[b.len()]
}
