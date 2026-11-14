use crate::{OperationDescriptor, OperationKind};
use serde::Serialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionRule {
    Required,
    Optional,
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentOperation {
    pub name: String,
    pub domain: String,
    pub kind: OperationKind,
    pub changes: bool,
    pub scope: String,
    pub summary: String,
    pub detail: String,
    pub returns: String,
    pub arguments: Value,
    pub example: Value,
    pub version: VersionRule,
    pub unavailable: Option<String>,
    pub annotations: Value,
}

impl AgentOperation {
    pub fn photos_write(&self) -> bool {
        self.kind == OperationKind::Query && self.scope == "photos:write"
    }
    pub fn read_only(&self) -> bool {
        self.annotations["readOnlyHint"] == true
    }
    pub fn required(&self) -> Vec<&str> {
        self.arguments["required"]
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
}

pub fn agent_domains() -> Result<Vec<(String, String)>, serde_json::Error> {
    let overlay: Value = serde_json::from_str(include_str!("agent.json"))?;
    Ok(overlay["domains"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, text)| (name.clone(), text.as_str().unwrap_or_default().to_owned()))
        .collect())
}

pub fn domain_of(name: &str) -> &str {
    match name {
        "search" | "records.get" | "records.query" => "records",
        "sources.status" | "replica.status" => "status",
        "mac.screen_record" => "capture",
        _ => name.split('.').next().unwrap_or(name),
    }
}

pub fn agent_operations(
    descriptors: &[OperationDescriptor],
) -> Result<Vec<AgentOperation>, serde_json::Error> {
    let overlay: Value = serde_json::from_str(include_str!("agent.json"))?;
    let shared = overlay["params"].as_object().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for descriptor in descriptors {
        let entry = &overlay["operations"][descriptor.name.as_str()];
        if !entry.is_object() {
            continue;
        }
        out.push(build(descriptor, entry, &shared));
    }
    Ok(out)
}

fn text(entry: &Value, key: &str) -> String {
    entry[key].as_str().unwrap_or_default().to_owned()
}

fn build(
    descriptor: &OperationDescriptor,
    entry: &Value,
    shared: &Map<String, Value>,
) -> AgentOperation {
    let base = if descriptor.kind == OperationKind::Read {
        descriptor.input.clone()
    } else {
        descriptor.input["properties"]["args"].clone()
    };
    let mut properties = if let Some(own) = entry.get("a") {
        own.clone()
    } else {
        base.get("properties").cloned().unwrap_or_else(|| json!({}))
    };
    let mut required: Vec<Value> = if entry.get("a").is_some() {
        entry["q"].as_array().cloned().unwrap_or_default()
    } else {
        base["required"].as_array().cloned().unwrap_or_default()
    };
    if let Some(omit) = entry["o"].as_array()
        && let Some(fields) = properties.as_object_mut()
    {
        for name in omit.iter().filter_map(Value::as_str) {
            fields.remove(name);
            required.retain(|field| field != name);
        }
    }
    let own = entry["p"].as_object().cloned().unwrap_or_default();
    if let Some(fields) = properties.as_object_mut() {
        for (name, schema) in fields.iter_mut() {
            describe(schema, name, name, &own, shared);
        }
    }
    let version = match entry["v"].as_str() {
        Some("required") => VersionRule::Required,
        Some("optional") => VersionRule::Optional,
        _ => VersionRule::None,
    };
    if version != VersionRule::None
        && let Some(fields) = properties.as_object_mut()
    {
        fields.insert(
            "expected_version".into(),
            json!({"type":"integer","minimum":1,"description":shared["expected_version"]}),
        );
        if version == VersionRule::Required {
            required.push(json!("expected_version"));
        }
    }
    let changes = descriptor.kind == OperationKind::Command || descriptor.scope == "photos:write";
    let read_like = entry["ro"] == true;
    let name = descriptor.name.as_str();
    let destructive = changes
        && !read_like
        && (name.contains("delete")
            || matches!(name.rsplit('.').next(), Some("trash" | "unsend" | "send")));
    let annotations = if changes && !read_like {
        json!({"readOnlyHint":false,"destructiveHint":destructive,"idempotentHint":true,"openWorldHint":matches!(name, "messages.send" | "messages.reply" | "mail.send")})
    } else {
        json!({"readOnlyHint":true,"openWorldHint":false})
    };
    AgentOperation {
        name: descriptor.name.clone(),
        domain: domain_of(name).to_owned(),
        kind: descriptor.kind,
        changes,
        scope: descriptor.scope.clone(),
        summary: text(entry, "s"),
        detail: text(entry, "d"),
        returns: text(entry, "r"),
        arguments: json!({"type":"object","additionalProperties":false,"properties":properties,"required":required}),
        example: entry["x"].clone(),
        version,
        unavailable: entry["u"].as_str().map(str::to_owned),
        annotations,
    }
}

fn describe(
    schema: &mut Value,
    path: &str,
    name: &str,
    own: &Map<String, Value>,
    shared: &Map<String, Value>,
) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    object.remove("nullable");
    let found = [
        own.get(path),
        shared.get(path),
        own.get(name),
        shared.get(name),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .map(str::to_owned);
    if let Some(text) = found {
        object.insert("description".into(), Value::String(text));
    }
    if let Some(Value::Object(fields)) = object.get_mut("properties") {
        for (child, value) in fields.iter_mut() {
            describe(value, &format!("{path}.{child}"), child, own, shared);
        }
    }
    if let Some(items) = object.get_mut("items")
        && let Some(Value::Object(fields)) = items.get_mut("properties")
    {
        for (child, value) in fields.iter_mut() {
            describe(value, &format!("{path}.{child}"), child, own, shared);
        }
    }
    if let Some(Value::Object(extra)) = object.get_mut("additionalProperties") {
        extra.remove("nullable");
    }
}

pub fn undescribed(schema: &Value, path: &str, out: &mut Vec<String>) {
    let Some(fields) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, child) in fields {
        let here = format!("{path}.{name}");
        if child
            .get("description")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            out.push(here.clone());
        }
        undescribed(child, &here, out);
        if let Some(items) = child.get("items") {
            undescribed(items, &here, out);
        }
    }
}
