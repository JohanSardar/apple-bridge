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
    for operation in &mut operations {
        if let Some(public) = crate::mcp_names::renamed(&operation.name) {
            public.clone_into(&mut operation.public);
        }
        operation.domain = operation
            .public
            .split('.')
            .next()
            .unwrap_or_default()
            .to_owned();
    }
    let mut domains = ab_catalog::agent_domains().ok()?;
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
            "Calendar to add it to: a cal_ ID from calendar.list_calendars. Or give calendar with its name."
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
            "Only events in this calendar (a cal_ ID from calendar.list_calendars). Omit for all calendars."
        );
    }
    if BY_TITLE.contains(&operation.name.as_str()) {
        operation.arguments["properties"]["reminder"] = json!({"type":"string","description":"The reminder's title, instead of id and expected_version. It must match one reminder; otherwise you get the candidates."});
        operation.arguments["properties"]["id"]["description"] =
            json!("The reminder's rmd_ ID from reminders.list. Or give reminder with its title.");
        if let Some(required) = operation.arguments["required"].as_array_mut() {
            required.retain(|field| field != "id" && field != "expected_version");
        }
        operation.detail = "Give id with expected_version from your read, or only reminder with its title; the current version is then used.".into();
    }
}

fn text_search(name: &str, scope: &str, summary: &str, example: &str) -> AgentOperation {
    AgentOperation {
        name: name.into(),
        public: name.into(),
        domain: name.split('.').next().unwrap_or_default().into(),
        kind: OperationKind::Read,
        changes: false,
        scope: scope.into(),
        summary: summary.into(),
        detail: String::new(),
        returns: "records with id, version and data, plus next for the next page.".into(),
        arguments: json!({"type":"object","additionalProperties":false,"properties":{
            "text":{"type":"string","minLength":1,"description":"Words to find."},
            "limit":{"type":"integer","minimum":1,"maximum":100,"description":"Most matches to return. Default 20."},
            "after":{"type":"string","description":"The next value from the previous page."}
        },"required":["text"]}),
        example: json!({"text": example}),
        version: VersionRule::None,
        unavailable: None,
        annotations: json!({"readOnlyHint":true,"openWorldHint":false}),
    }
}

pub(crate) fn preset(name: &str, arguments: &Value) -> Option<(&'static str, Value)> {
    let mut mapped = arguments.as_object().cloned().unwrap_or_default();
    let (target, collection) = match name {
        crate::mcp_names::NOTES_SEARCH => ("search", "notes.note"),
        crate::mcp_names::MAIL_SEARCH => ("search", "mail.message"),
        crate::mcp_names::NOTES_GET => ("records.get", "notes.note"),
        _ => return None,
    };
    if target == "search" {
        mapped.entry("limit").or_insert(json!(20));
    }
    mapped.insert("collection".into(), json!(collection));
    Some((target, Value::Object(mapped)))
}

fn search_operations() -> Vec<AgentOperation> {
    vec![
        text_search(
            crate::mcp_names::NOTES_SEARCH,
            "notes:read",
            "Find notes whose title or text contains these words, including notes written on the iPhone.",
            "passport",
        ),
        text_search(
            crate::mcp_names::MAIL_SEARCH,
            "mail:read",
            "Find mail messages whose sender, subject or text contains these words.",
            "invoice",
        ),
        AgentOperation {
            name: crate::mcp_names::NOTES_GET.into(),
            public: crate::mcp_names::NOTES_GET.into(),
            domain: "notes".into(),
            kind: OperationKind::Read,
            changes: false,
            scope: "notes:read".into(),
            summary: "Read one note in full: its title and whole plain text.".into(),
            detail: String::new(),
            returns: "records: the note with id, version and data (title, plain_text, folder_id, modified_at).".into(),
            arguments: json!({"type":"object","additionalProperties":false,"properties":{
                "id":{"type":"string","minLength":1,"description":"The note's nte_ ID from notes.list or notes.search."}
            },"required":["id"]}),
            example: json!({"id":"nte_01J9Z8Y7X6W5V4T3S2R1Q0P9N8"}),
            version: VersionRule::None,
            unavailable: None,
            annotations: json!({"readOnlyHint":true,"openWorldHint":false}),
        },
    ]
}

fn composite_operations() -> Vec<AgentOperation> {
    let read = json!({"readOnlyHint":true,"openWorldHint":false});
    let mut operations = search_operations();
    operations.extend([
        AgentOperation {
            name: USAGE_SUMMARY.into(),
            public: USAGE_SUMMARY.into(),
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
            public: CALENDARS.into(),
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
            annotations: read.clone(),
        },
        AgentOperation {
            name: crate::mcp_lookups::FREE_SLOTS.into(),
            public: crate::mcp_lookups::FREE_SLOTS.into(),
            domain: "calendar".into(),
            kind: OperationKind::Read,
            changes: false,
            scope: "calendar:read".into(),
            summary: "Find free time: open slots of at least some minutes between events, within working hours, over the next days.".into(),
            detail: "Timed events block time unless they are marked free, declined or canceled; all-day events do not. Days are in the Mac's time zone; weekends are skipped unless weekends is true.".into(),
            returns: "slots with start, end, start_local, end_local and minutes, earliest first.".into(),
            arguments: json!({"type":"object","additionalProperties":false,"properties":{
                "start":{"type":"string","description":"Search from: RFC 3339, a local time such as 2026-11-12T09:00, or YYYY-MM-DD. Default now."},
                "end":{"type":"string","description":"Search until, up to 62 days after start. Default 7 days after start."},
                "minutes":{"type":"integer","minimum":5,"maximum":1440,"description":"Shortest slot to return, in minutes. Default 30."},
                "day_start":{"type":"string","description":"Working day start, HH:MM. Default 09:00."},
                "day_end":{"type":"string","description":"Working day end, HH:MM. Default 17:00."},
                "weekends":{"type":"boolean","description":"Include Saturdays and Sundays. Default false."},
                "calendar":{"type":"string","description":"Only events in this calendar, by name or cal_ ID. Default all calendars."},
                "limit":{"type":"integer","minimum":1,"maximum":50,"description":"Most slots to return. Default 10."}
            },"required":[]}),
            example: json!({"minutes":60,"start":"2026-11-12","end":"2026-11-15"}),
            version: VersionRule::None,
            unavailable: None,
            annotations: read.clone(),
        },
        AgentOperation {
            name: crate::mcp_lookups::SHARED.into(),
            public: crate::mcp_lookups::SHARED.into(),
            domain: "messages".into(),
            kind: OperationKind::Read,
            changes: false,
            scope: "messages:read".into(),
            summary: "Find what was shared in a chat: attachments by kind (photos, videos, PDFs, documents, audio) and links, with who sent each.".into(),
            detail: "Give with (a person) or chat_id (also group chats). Looks through the newest messages first, up to scan messages.".into(),
            returns: "items, newest first, each with kind, from, sent_at, message_id and either url (links) or attachment_id, filename, mime_type, size and digest (files); complete says whether the whole chat was examined.".into(),
            arguments: json!({"type":"object","additionalProperties":false,"properties":{
                "with":{"type":"string","description":"The person: contact name, phone number or email."},
                "chat_id":{"type":"string","description":"A cht_ chat ID, for group chats. Use instead of with."},
                "kind":{"type":"string","enum":crate::mcp_lookups::SHARED_KINDS,"description":"What to find: any, image, video, audio, pdf, document, file (any attachment) or link. Default any."},
                "limit":{"type":"integer","minimum":1,"maximum":100,"description":"Most items to return. Default 20."},
                "scan":{"type":"integer","minimum":1,"maximum":5000,"description":"Most messages to look through, newest first. Default 600."}
            },"required":[]}),
            example: json!({"with":"Sam","kind":"pdf"}),
            version: VersionRule::None,
            unavailable: None,
            annotations: read,
        },
    ]);
    operations
}

pub(crate) fn change_schema() -> Value {
    json!({"type":"object","properties":{
        "status":{"type":"string","enum":["done","partial","pending","awaiting_approval","failed","indeterminate"]}
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
            public: BLOB_READ.into(),
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
            public: BLOB_STAGE.into(),
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
        self.operations
            .iter()
            .find(|operation| operation.name == name)
    }

    pub fn named(&self, name: &str) -> Option<&AgentOperation> {
        let name = name.trim();
        self.operations
            .iter()
            .find(|operation| operation.public == name)
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
                let changes = match tool["name"].as_str() {
                    Some("apple_read") => Some(false),
                    Some("apple_change") => Some(true),
                    _ => None,
                };
                if let Some(changes) = changes {
                    tool["inputSchema"]["properties"]["operation"]["enum"] =
                        json!(self.names(principal, changes));
                }
                tool
            })
            .collect()
    }

    pub fn names<'a>(&'a self, principal: Option<&'a Principal>, changes: bool) -> Vec<&'a str> {
        let mut names: Vec<&str> = self
            .visible(principal)
            .filter(|operation| (operation.changes && !operation.read_only()) == changes)
            .map(|operation| operation.public.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    pub fn visible<'a>(
        &'a self,
        principal: Option<&'a Principal>,
    ) -> impl Iterator<Item = &'a AgentOperation> + 'a {
        self.operations
            .iter()
            .filter(move |operation| principal.is_none_or(|p| allowed(p, operation)))
    }

    pub fn closest<'a>(
        &'a self,
        name: &str,
        principal: Option<&'a Principal>,
    ) -> Vec<&'a AgentOperation> {
        let mut found: Vec<&AgentOperation> = Vec::new();
        let mut add = |operation: &'a AgentOperation| {
            if !found.iter().any(|seen| seen.public == operation.public) {
                found.push(operation);
            }
        };
        if let Some(public) = crate::mcp_names::retired(name)
            && let Some(operation) = self.named(public)
        {
            add(operation);
        }
        let wanted = crate::mcp_names::key(name);
        for operation in self.visible(principal) {
            if !wanted.is_empty() && crate::mcp_names::key(&operation.public) == wanted {
                add(operation);
            }
        }
        let lower = name.trim().to_lowercase();
        let mut scored: Vec<(usize, &AgentOperation)> = self
            .visible(principal)
            .map(|operation| (distance(&lower, &operation.public), operation))
            .filter(|(score, _)| *score <= lower.len().max(4) / 3 + 1)
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.public.cmp(&b.1.public)));
        for (_, operation) in scored {
            add(operation);
        }
        let words = lower.replace(['.', '_', '-', '/'], " ");
        let (hits, _) = crate::mcp_find::find(self, principal, &words);
        for hit in hits {
            if let crate::mcp_find::Hit::Operation(operation) = hit {
                add(operation);
            }
        }
        found.truncate(3);
        found
    }
}

pub(crate) fn internal_name(public: &str) -> String {
    registry()
        .and_then(|registry| registry.named(public))
        .map_or_else(String::new, |operation| operation.name.clone())
}

pub(crate) fn public_name(internal: &str) -> String {
    registry()
        .and_then(|registry| registry.find(internal))
        .map_or_else(|| internal.to_owned(), |operation| operation.public.clone())
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
