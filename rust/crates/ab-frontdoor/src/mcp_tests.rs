use crate::{
    mcp_check,
    mcp_facade::{change_result, compact},
    mcp_guides::{GUIDES, INSTRUCTIONS},
    mcp_registry::{COMPACT, registry},
};
use ab_catalog::{OperationKind, VersionRule, undescribed};
use ab_operations::Principal;
use serde_json::{Value, json};
use std::collections::BTreeSet;

type Check = Result<(), Box<dyn std::error::Error>>;

const BANNED: &[&str] = &[
    "operation version",
    "evidence",
    "native readback",
    "transient",
    "admission",
    "bridge-owned",
];

fn principal(level: &str, grants: &[&str]) -> Principal {
    Principal {
        id: "local/agt_test".into(),
        kind: "agent".into(),
        grants: grants.iter().map(|grant| (*grant).to_owned()).collect(),
        level: level.into(),
        resource: String::new(),
        remote: false,
        expires_at: None,
    }
}

fn every_scope() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let registry = registry().ok_or("registry")?;
    let scopes: BTreeSet<String> = registry
        .operations
        .iter()
        .map(|operation| operation.scope.clone())
        .filter(|scope| !scope.is_empty())
        .collect();
    Ok(scopes.into_iter().collect())
}

fn names(tools: &[Value]) -> Vec<String> {
    tools
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn every_catalog_operation_has_an_agent_contract() -> Check {
    let registry = registry().ok_or("registry")?;
    let catalog = ab_catalog::operations()?;
    for descriptor in &catalog {
        let operation = registry
            .find(&descriptor.name)
            .ok_or(descriptor.name.clone())?;
        assert!(!operation.summary.is_empty(), "{}", descriptor.name);
        assert!(!operation.returns.is_empty(), "{}", descriptor.name);
        assert!(operation.example.is_object(), "{}", descriptor.name);
        assert_eq!(operation.scope, descriptor.scope, "{}", descriptor.name);
        assert!(
            registry.find(&descriptor.name.replace('.', "_")).is_some(),
            "{}",
            descriptor.name
        );
    }
    assert_eq!(registry.operations.len(), catalog.len() + 4);
    let summaries: BTreeSet<&str> = registry
        .operations
        .iter()
        .map(|operation| operation.summary.as_str())
        .collect();
    assert_eq!(summaries.len(), registry.operations.len());
    Ok(())
}

#[test]
fn every_parameter_is_described() -> Check {
    let registry = registry().ok_or("registry")?;
    let mut missing = Vec::new();
    for operation in &registry.operations {
        undescribed(&operation.arguments, &operation.name, &mut missing);
    }
    for tool in registry.tools(None) {
        undescribed(
            &tool["inputSchema"],
            tool["name"].as_str().unwrap_or_default(),
            &mut missing,
        );
        assert!(
            tool["description"].as_str().is_some_and(|d| d.len() > 40),
            "{}",
            tool["name"]
        );
    }
    assert!(missing.is_empty(), "undescribed: {missing:#?}");
    Ok(())
}

#[test]
fn agent_text_avoids_internal_words_and_html() -> Check {
    let registry = registry().ok_or("registry")?;
    let mut texts = vec![INSTRUCTIONS.to_owned()];
    texts.push(serde_json::to_string(&registry.tools(None))?);
    for operation in &registry.operations {
        texts.push(serde_json::to_string(&json!([
            operation.summary,
            operation.detail,
            operation.returns,
            operation.arguments,
        ]))?);
    }
    for (name, _, text) in GUIDES {
        assert!(!text.contains("bridge_"), "{name}");
    }
    for text in &texts {
        let lower = text.to_lowercase();
        for word in BANNED {
            assert!(
                !lower.contains(word),
                "{word} in {}",
                &text[..text.len().min(160)]
            );
        }
        for character in ['<', '>', '&'] {
            assert!(
                !text.contains(character),
                "{character} in {}",
                &text[..text.len().min(160)]
            );
        }
    }
    Ok(())
}

#[test]
fn every_example_passes_its_own_contract() -> Check {
    let registry = registry().ok_or("registry")?;
    for operation in &registry.operations {
        let checked = mcp_check::arguments(operation, Some(&operation.example));
        assert!(
            checked.is_ok(),
            "{}: {:?}",
            operation.name,
            checked.err().map(|problem| problem.to_json())
        );
    }
    Ok(())
}

fn validate(name: &str, args: &Value, version: u64) -> bool {
    let command = serde_json::from_value::<ab_operations::Command>(json!({
        "command_id": "check-1",
        "op": name,
        "args": args,
        "expected_version": version,
        "wait_ms": 0,
    }));
    command.is_ok_and(|command| ab_operations::command_contract::validate(&command).is_ok())
}

#[test]
fn command_examples_pass_the_mac_validator_and_version_rules_match_it() -> Check {
    let registry = registry().ok_or("registry")?;
    let mut checked = 0;
    let mut rejected: Vec<String> = Vec::new();
    for operation in registry
        .operations
        .iter()
        .filter(|operation| {
            operation.kind == OperationKind::Command && operation.unavailable.is_none()
        })
        .filter(|operation| {
            ab_catalog::operations().is_ok_and(|all| all.iter().any(|d| d.name == operation.name))
        })
    {
        let mut args = operation.example.clone();
        let version = args
            .as_object_mut()
            .and_then(|fields| fields.remove("expected_version"))
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        if !validate(&operation.name, &args, version) {
            if validate(&operation.name, &args, 1) || validate(&operation.name, &args, 0) {
                return Err(format!(
                    "{}: version rule does not match the validator",
                    operation.name
                )
                .into());
            }
            rejected.push(operation.name.clone());
            continue;
        }
        checked += 1;
        match operation.version {
            VersionRule::Required => {
                assert!(version > 0, "{}", operation.name);
                assert!(
                    !validate(&operation.name, &args, 0),
                    "{} accepts no version",
                    operation.name
                );
            }
            VersionRule::Optional => {
                assert!(validate(&operation.name, &args, 0), "{}", operation.name);
            }
            VersionRule::None => {
                assert_eq!(version, 0, "{}", operation.name);
            }
        }
    }
    assert!(
        rejected.is_empty(),
        "examples the Mac rejects: {rejected:?}"
    );
    assert!(checked > 40, "only {checked} command examples validated");
    Ok(())
}

fn portable(schema: &Value, path: &str, top: bool, problems: &mut Vec<String>) {
    const ALLOWED: &[&str] = &[
        "type",
        "properties",
        "required",
        "items",
        "description",
        "enum",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "additionalProperties",
        "pattern",
        "format",
        "default",
        "title",
        "uniqueItems",
    ];
    let Some(object) = schema.as_object() else {
        return;
    };
    for (key, value) in object {
        if !ALLOWED.contains(&key.as_str()) {
            problems.push(format!("{path}: {key}"));
        }
        if key == "type" && !value.is_string() {
            problems.push(format!("{path}: type list"));
        }
    }
    if top && object.get("type") != Some(&json!("object")) {
        problems.push(format!("{path}: top level is not an object"));
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, child) in properties {
            portable(child, &format!("{path}.{name}"), false, problems);
        }
    }
    if let Some(items) = object.get("items") {
        portable(items, &format!("{path}[]"), false, problems);
    }
    if let Some(extra) = object.get("additionalProperties").filter(|v| v.is_object()) {
        portable(extra, &format!("{path}{{}}"), false, problems);
    }
}

#[test]
fn schemas_use_only_keywords_every_client_accepts() -> Check {
    let registry = registry().ok_or("registry")?;
    let mut problems = Vec::new();
    for tool in registry.tools(None) {
        let name = tool["name"].as_str().unwrap_or_default().to_owned();
        portable(&tool["inputSchema"], &name, true, &mut problems);
        portable(
            &tool["outputSchema"],
            &format!("{name} output"),
            true,
            &mut problems,
        );
    }
    for operation in &registry.operations {
        portable(&operation.arguments, &operation.name, true, &mut problems);
    }
    assert!(problems.is_empty(), "{problems:#?}");
    Ok(())
}

#[test]
fn what_an_agent_loads_on_connect_fits_the_budget() -> Check {
    let registry = registry().ok_or("registry")?;
    let tools = serde_json::to_string(&json!({"tools": registry.tools(None)}))?;
    let bytes = tools.len() + INSTRUCTIONS.len();
    let tokens = bytes * 10 / 35;
    assert!(
        INSTRUCTIONS.len() <= 2100,
        "instructions are {} bytes",
        INSTRUCTIONS.len()
    );
    assert!(
        tokens <= 6000,
        "tools/list and instructions are about {tokens} tokens ({bytes} bytes)"
    );
    assert_eq!(names(&registry.tools(None)), COMPACT);
    Ok(())
}

#[test]
fn each_access_level_sees_only_the_tools_it_can_use() -> Check {
    let registry = registry().ok_or("registry")?;
    let scopes = every_scope()?;
    let scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
    let full = names(&registry.tools(Some(&principal("full", &scopes))));
    let approve = names(&registry.tools(Some(&principal("approve", &scopes))));
    let read = names(&registry.tools(Some(&principal("read", &scopes))));
    assert_eq!(full, COMPACT);
    assert_eq!(approve, COMPACT);
    for tool in [
        "apple_change",
        "messages_send",
        "reminders_add",
        "notes_create",
    ] {
        assert!(!read.contains(&tool.to_owned()), "{tool}");
    }
    for tool in [
        "apple_status",
        "apple_describe",
        "apple_read",
        "messages_read",
        "calendar_events",
    ] {
        assert!(read.contains(&tool.to_owned()), "{tool}");
    }
    let narrow = names(&registry.tools(Some(&principal(
        "full",
        &["reminders:read", "reminders:write"],
    ))));
    assert!(narrow.contains(&"reminders_add".to_owned()));
    assert!(narrow.contains(&"apple_change".to_owned()));
    assert!(!narrow.contains(&"messages_send".to_owned()));
    assert!(!narrow.contains(&"calendar_events".to_owned()));
    let reader = principal("read", &["reminders:read"]);
    assert!(
        registry
            .visible(Some(&reader))
            .all(|operation| !operation.changes || operation.scope.is_empty())
    );
    let admin = principal("full", &["admin"]);
    assert_eq!(names(&registry.tools(Some(&admin))), COMPACT);
    Ok(())
}

#[test]
fn unknown_fields_and_bad_dates_get_actionable_errors() -> Check {
    let registry = registry().ok_or("registry")?;
    let create = registry.find("reminders.create").ok_or("create")?;
    let problem = mcp_check::arguments(create, Some(&json!({"list_id": "lst_01J9Z8Y7X6W5V4T3S2R1Q0P9N8", "title": "Milk", "deu": "2026-11-12"})))
        .err()
        .ok_or("accepted an unknown field")?
        .to_json();
    assert_eq!(problem["error"]["code"], "invalid");
    assert_eq!(problem["error"]["field"], "deu");
    assert!(problem["error"]["example"].is_object());
    let problem = mcp_check::arguments(create, Some(&json!({"list_id": "lst_01J9Z8Y7X6W5V4T3S2R1Q0P9N8", "title": "Milk", "due": "next tuesday"})))
        .err()
        .ok_or("accepted a bad date")?
        .to_json();
    assert_eq!(problem["error"]["field"], "due");
    assert!(
        problem["error"]["expected"]
            .to_string()
            .contains("YYYY-MM-DD")
    );
    let update = registry.find("reminders.update").ok_or("update")?;
    let problem = mcp_check::arguments(
        update,
        Some(&json!({"id": "rmd_01J9Z8Y7X6W5V4T3S2R1Q0P9N8", "title": "Oat milk"})),
    )
    .err()
    .ok_or("accepted a missing version")?
    .to_json();
    assert_eq!(problem["error"]["field"], "expected_version");
    assert!(
        problem["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("version"))
    );
    Ok(())
}

#[test]
fn legacy_args_wrappers_are_accepted() -> Check {
    let registry = registry().ok_or("registry")?;
    let ocr = registry.find("vision.ocr").ok_or("ocr")?;
    let flat =
        mcp_check::arguments(ocr, Some(&ocr.example)).map_err(|p| p.to_json().to_string())?;
    let wrapped = mcp_check::arguments(ocr, Some(&json!({"args": ocr.example})))
        .map_err(|p| p.to_json().to_string())?;
    assert_eq!(flat, wrapped);
    Ok(())
}

#[test]
fn reads_are_compact_unless_verbose() {
    let record = json!({
        "collection": "reminders.reminder",
        "id": "rmd_1",
        "version": 3,
        "deleted": false,
        "coverage": {"fields": "complete", "missing": []},
        "data": {"title": "Milk", "due": "2026-11-12", "store_coverage": {"x": 1}},
    });
    let page = json!({"records": [record], "next": "abc", "cursor": {"epoch": "e", "seq": 1}, "coverage_notes": []});
    let small = compact(&page, false, None);
    assert_eq!(
        small,
        json!({"records": [{"id": "rmd_1", "version": 3, "data": {"title": "Milk", "due": "2026-11-12"}}], "next": "abc"})
    );
    let fields = vec!["title".to_owned()];
    assert_eq!(
        compact(&page, false, Some(&fields))["records"][0]["data"],
        json!({"title": "Milk"})
    );
    assert_eq!(compact(&page, true, None), page);
}

#[test]
fn a_change_returns_a_finished_compact_result() {
    let done = json!({
        "state": "observed",
        "disposition": "locally_observed",
        "command": {"command_id": "req-1", "op": "messages.send", "args": {"chat_id": "cht_1", "text": "hi"}},
        "observed_record_ids": ["msg_1"],
        "evidence": {"fingerprint": "x", "native_key": "y"},
        "history": [{"state": "admitted"}, {"state": "observed"}],
    });
    let result = change_result(&done, false);
    assert_eq!(result["status"], "done");
    assert_eq!(result["request_id"], "req-1");
    assert_eq!(result["ids"], json!(["msg_1"]));
    assert!(result.get("details").is_none());
    assert!(result.get("result").is_none());
    let said = result["verification"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    assert!(said.contains("does not prove"));
    assert!(!said.contains("delivered"));
    assert!(change_result(&done, true)["details"]["history"].is_array());
    for (state, status) in [
        ("admitted", "pending"),
        ("dispatched", "pending"),
        ("delivered", "done"),
        ("awaiting_approval", "awaiting_approval"),
        ("indeterminate", "indeterminate"),
        ("rejected", "failed"),
        ("expired", "failed"),
    ] {
        let record = json!({"state": state, "command": {"command_id": "req-2", "op": "reminders.create", "args": {}}});
        assert_eq!(change_result(&record, false)["status"], status, "{state}");
    }
}

#[test]
fn guides_are_served_and_named_in_the_instructions() {
    assert!(INSTRUCTIONS.contains("apple_status"));
    assert!(INSTRUCTIONS.contains("apple_describe"));
    assert!(INSTRUCTIONS.contains("apple-bridge://guides"));
    let resources = crate::mcp_guides::resources();
    assert_eq!(
        resources["resources"].as_array().map(Vec::len),
        Some(GUIDES.len())
    );
    for (name, _, text) in GUIDES {
        assert!(
            crate::mcp_guides::guide(&crate::mcp_guides::uri(name))
                .is_some_and(|(_, _, t)| t == text)
        );
    }
    assert!(GUIDES[0].2.contains("apple_status"));
}

fn surface() -> Result<Value, Box<dyn std::error::Error>> {
    let registry = registry().ok_or("registry")?;
    let scopes = every_scope()?;
    let all: Vec<&str> = scopes.iter().map(String::as_str).collect();
    let mut levels = serde_json::Map::new();
    for level in ["full", "approve", "read"] {
        levels.insert(
            level.into(),
            json!(names(&registry.tools(Some(&principal(level, &all))))),
        );
    }
    let mut by_scope = serde_json::Map::new();
    for scope in &scopes {
        by_scope.insert(
            scope.clone(),
            json!(names(&registry.tools(Some(&principal("full", &[scope]))))),
        );
    }
    Ok(json!({
        "instructions": INSTRUCTIONS,
        "tools": registry.tools(None),
        "levels": levels,
        "full_agent_with_one_scope": by_scope,
        "guides": GUIDES.iter().map(|(name, summary, _)| json!({"name": name, "summary": summary})).collect::<Vec<_>>(),
    }))
}

#[test]
fn the_served_surface_matches_its_golden() -> Check {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../contracts/golden/mcp-surface.json"
    ))?;
    let actual = surface()?;
    assert!(
        actual == expected,
        "{}",
        serde_json::to_string_pretty(&actual)?
    );
    for raw in [
        include_str!("../../../../contracts/golden/mcp-json.json"),
        include_str!("../../../../contracts/golden/mcp-sse.json"),
    ] {
        let transcript: Value = serde_json::from_str(raw)?;
        let payload = |index: usize| -> Result<Value, Box<dyn std::error::Error>> {
            let response = transcript["exchanges"][index]["response"]
                .as_str()
                .ok_or("response")?;
            let body = response
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap_or(response);
            Ok(serde_json::from_str(body)?)
        };
        assert_eq!(payload(0)?["result"]["instructions"], INSTRUCTIONS);
        assert_eq!(payload(2)?["result"]["tools"], expected["tools"]);
    }
    Ok(())
}

#[test]
fn plain_requests_find_the_right_operation_first() -> Check {
    let registry = registry().ok_or("registry")?;
    let cases: &[(&str, &str)] = &[
        ("screen time", "usage.summary"),
        (
            "how much time did I spend on my phone yesterday",
            "usage.summary",
        ),
        ("app usage on my iPhone this week", "usage.summary"),
        ("send a text to Ana", "messages.send"),
        ("send an SMS", "messages.send"),
        ("reply to an iMessage", "messages.reply"),
        ("read my unread messages", "messages.chats"),
        ("email my boss", "mail.send"),
        ("check my inbox", "records.query/mail.message"),
        ("get voicemail", "records.query/voicemail.message"),
        ("listen to my voicemails", "records.query/voicemail.message"),
        ("who called me", "records.query/phone.call"),
        ("missed calls", "records.query/phone.call"),
        ("safari browsing history", "records.query/safari.history"),
        ("websites I visited", "records.query/safari.history"),
        ("voice memos", "records.query/voice_memos.recording"),
        ("is the wifi working", "mac.network_check"),
        ("am I online", "mac.network_check"),
        ("internet connection", "mac.network_check"),
        ("where am I", "location.current"),
        ("find a document", "files.search"),
        ("search my files", "files.search"),
        ("add an event to my calendar", "calendar.create_event"),
        ("schedule a meeting", "calendar.create_event"),
        ("what is on my calendar tomorrow", "calendar.agenda"),
        ("am I free on friday", "calendar.agenda"),
        ("move my dentist appointment", "calendar.update_event"),
        ("list my calendars", "calendar.calendars"),
        ("remind me to buy milk", "reminders.create"),
        ("mark the reminder as done", "reminders.complete"),
        ("delete a reminder", "reminders.delete"),
        ("my to do list", "reminders.items"),
        ("write a note", "notes.create"),
        (
            "read the note I wrote on my phone",
            "records.query/notes.note",
        ),
        ("find a photo of a dog", "photos.search"),
        ("show me pictures from the beach", "photos.search"),
        ("look up a phone number", "contacts.lookup"),
        ("turn the volume down", "mac.set_volume"),
        ("set an alarm", "iphone.alarm.create"),
        ("what song is playing", "mac.recognize_song"),
    ];
    let mut misses = Vec::new();
    for (query, wanted) in cases {
        let (hits, _) = crate::mcp_find::find(registry, None, query);
        let names: Vec<String> = hits
            .iter()
            .take(3)
            .map(|hit| match hit {
                crate::mcp_find::Hit::Operation(o) => o.name.clone(),
                crate::mcp_find::Hit::Route(r) => format!("records.query/{}", r.collection),
            })
            .collect();
        if !names.iter().any(|name| name == wanted) {
            misses.push(format!("{query:?}: wanted {wanted}, got {names:?}"));
        }
    }
    assert!(cases.len() >= 30);
    assert!(misses.is_empty(), "{}", misses.join("\n"));
    Ok(())
}

#[test]
fn a_partly_sent_message_says_what_happened_and_not_to_repeat() {
    let record = json!({
        "state": "delivery_failed",
        "disposition": "delivery_failed",
        "command": {"command_id": "req-p", "op": "messages.send", "args": {}},
        "parts": [
            {"index": 0, "kind": "text", "record_id": "msg_1", "disposition": "observed"},
            {"index": 1, "kind": "attachment", "record_id": "msg_2", "disposition": "delivery_failed"},
            {"index": 2, "kind": "attachment", "disposition": "failed_no_effect"}
        ],
    });
    let result = change_result(&record, false);
    assert_eq!(result["status"], "partial");
    assert_eq!(result["ids"], json!(["msg_1", "msg_2"]));
    let outcomes: Vec<&str> = result["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["outcome"].as_str())
        .collect();
    assert_eq!(outcomes, ["sent", "failed", "not_sent"]);
    let next = result["next"].as_str().unwrap_or_default();
    assert!(next.contains("Do not repeat"), "{next}");
    assert!(!next.contains("did not happen"), "{next}");
    let failed = change_result(
        &json!({"state": "failed", "disposition": "delivery_failed", "command": {"command_id": "r", "op": "messages.send", "args": {}}}),
        false,
    );
    assert_eq!(failed["status"], "failed");
    assert!(
        !failed["next"]
            .as_str()
            .unwrap_or_default()
            .contains("nothing changed")
    );
    let refused = change_result(
        &json!({"state": "rejected", "disposition": "rejected_before_dispatch", "command": {"command_id": "r", "op": "messages.send", "args": {}}}),
        false,
    );
    assert!(
        refused["next"]
            .as_str()
            .unwrap_or_default()
            .contains("nothing changed")
    );
}

#[test]
fn listed_tools_reject_unknown_fields_and_wrong_types() -> Check {
    let registry = registry().ok_or("registry")?;
    let check = |tool: &str, input: Value| {
        let schema = registry.tool_schema(tool).cloned().unwrap_or_default();
        mcp_check::check_tool(
            tool,
            &schema,
            &registry.example(tool),
            input.as_object().ok_or("object")?,
        )
        .map_err(|problem| problem.to_json())
        .map_err(|e| e.to_string())
    };
    assert!(
        check("contacts_find", json!({"query": "Ana", "limit": "three"}))
            .is_err_and(|e| e.contains("limit"))
    );
    assert!(
        check("contacts_find", json!({"query": "Ana", "limt": 3}))
            .is_err_and(|e| e.contains("limt"))
    );
    assert!(check("apple_search", json!({"text": "x", "limit": 500})).is_err());
    assert!(
        check(
            "calendar_add",
            json!({"title": "Lunch", "start": "2026-11-12T12:30"})
        )
        .is_ok()
    );
    assert!(
        check(
            "apple_read",
            json!({"operation": "notes.list", "arguments": {}})
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn local_dates_become_timestamps_where_the_mac_needs_them() -> Check {
    let registry = registry().ok_or("registry")?;
    let photos = registry.find("photos.search").ok_or("photos.search")?;
    let mut arguments = json!({"start": "2026-07-01", "end": "2026-08-01T09:30"});
    let arguments = arguments.as_object_mut().ok_or("object")?;
    mcp_check::local_dates(photos, arguments);
    for key in ["start", "end"] {
        let value = arguments[key].as_str().unwrap_or_default();
        assert!(value.parse::<jiff::Timestamp>().is_ok(), "{key} {value}");
    }
    assert!(
        arguments["start"]
            .as_str()
            .unwrap_or_default()
            .starts_with("2026-07-01T00:00:00")
    );
    let range = registry.find("calendar.range").ok_or("calendar.range")?;
    let mut kept = json!({"start": "2026-07-01", "end": "2026-07-02"});
    let kept = kept.as_object_mut().ok_or("object")?;
    mcp_check::local_dates(range, kept);
    assert_eq!(kept["start"], "2026-07-01");
    Ok(())
}

#[test]
fn message_results_keep_text_exactly_and_name_attachments() {
    let message = json!({
        "collection": "messages.message", "id": "msg_1", "version": 1,
        "data": {"chat_id": "cht_1", "text": "  Ferry at 9\n", "sent_at": "2026-11-10T12:00:00Z", "is_from_me": false},
        "attachments": [{"id": "att_1", "version": 1, "data": {"filename": "map.png", "mime_type": "image/png", "size": 10, "blob": {"digest": "d", "state": "available"}}}],
    });
    let summary = crate::mcp_facade::message_summary(&message);
    assert_eq!(summary["text"], "  Ferry at 9\n");
    assert_eq!(summary["attachments"][0]["filename"], "map.png");
    assert_eq!(summary["attachments"][0]["digest"], "d");
    let compacted = compact(&message, false, None);
    assert_eq!(compacted["attachments"][0]["id"], "att_1");
}

#[test]
fn write_only_agents_cannot_read_through_name_lookups() {
    let writer = principal("full", &["reminders:write", "notes:write"]);
    assert!(!crate::mcp_registry::can(&writer, "reminders:read"));
    assert!(!crate::mcp_registry::can(&writer, "notes:read"));
    let problem = mcp_check::refused(
        &writer,
        "finding a reminder by its title",
        "reminders:read",
        false,
    )
    .to_json();
    assert_eq!(problem["error"]["code"], "unauthorized");
    assert_eq!(problem["error"]["expected"]["scope"], "reminders:read");
}

#[test]
fn record_ids_name_their_collection() -> Check {
    let registry = registry().ok_or("registry")?;
    let get = registry.find("records.get").ok_or("records.get")?;
    let mut bare = json!({"id": "hdl_01M7347WK24DMMSES7G9E72B6D"});
    let bare = bare.as_object_mut().ok_or("object")?;
    mcp_check::infer_collection(get, bare);
    assert_eq!(bare["collection"], "messages.handle");
    let mut given = json!({"id": "hdl_01M7347WK24DMMSES7G9E72B6D", "collection": "notes.note"});
    let given = given.as_object_mut().ok_or("object")?;
    mcp_check::infer_collection(get, given);
    assert_eq!(given["collection"], "notes.note");
    let mut unknown = json!({"id": "zzz_01M7347WK24DMMSES7G9E72B6D"});
    let unknown = unknown.as_object_mut().ok_or("object")?;
    mcp_check::infer_collection(get, unknown);
    assert!(!unknown.contains_key("collection"));
    Ok(())
}

#[test]
fn a_title_is_bound_to_its_request_id() {
    crate::mcp_compose::bind_selector("agent:bind", "req-1", "  Buy MILK ");
    assert_eq!(
        crate::mcp_compose::selector("agent:bind", "req-1").as_deref(),
        Some("buy milk")
    );
    assert_eq!(crate::mcp_compose::selector("agent:other", "req-1"), None);
    assert_eq!(crate::mcp_compose::selector("agent:bind", "req-2"), None);
}

#[tokio::test]
async fn title_changes_with_one_request_id_take_turns() {
    let first = crate::mcp_compose::request_turn("agent:turns", "req-1").await;
    let other = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        crate::mcp_compose::request_turn("agent:turns", "req-2"),
    )
    .await;
    assert!(other.is_ok());
    let waiting = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        crate::mcp_compose::request_turn("agent:turns", "req-1"),
    )
    .await;
    assert!(waiting.is_err());
    drop(first);
    let next = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        crate::mcp_compose::request_turn("agent:turns", "req-1"),
    )
    .await;
    assert!(next.is_ok());
}
