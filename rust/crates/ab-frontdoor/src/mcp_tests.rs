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
    assert_eq!(registry.operations.len(), catalog.len() + 2);
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
    let tokens = bytes / 4;
    assert!(
        INSTRUCTIONS.len() <= 2100,
        "instructions are {} bytes",
        INSTRUCTIONS.len()
    );
    assert!(
        tokens <= 5000,
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
