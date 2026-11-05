#![forbid(unsafe_code)]
mod descriptors;
mod reference;
use ab_domain::Source;
pub use reference::operations_reference_markdown;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Query,
    Read,
    Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDescriptor {
    pub name: String,
    pub version: u32,
    pub kind: OperationKind,
    pub read_model: bool,
    #[serde(with = "optional_source")]
    pub source: Option<Source>,
    pub scope: String,
    pub capability: String,
    pub description: String,
    pub input: Value,
    pub output: Value,
    pub examples: Option<Value>,
}

mod optional_source {
    use ab_domain::Source;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    #[allow(clippy::ref_option, clippy::trivially_copy_pass_by_ref)]
    pub fn serialize<S: Serializer>(
        value: &Option<Source>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(source) => source.serialize(serializer),
            None => "".serialize(serializer),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Source>, D::Error> {
        let raw = String::deserialize(deserializer)?;
        if raw.is_empty() {
            Ok(None)
        } else {
            serde_json::from_value(serde_json::Value::String(raw))
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }
}

pub fn operations() -> Result<Vec<OperationDescriptor>, serde_json::Error> {
    let schemas: Vec<Value> = serde_json::from_str(include_str!("schemas.json"))?;
    Ok(descriptors::descriptors(&schemas))
}

pub fn json_schema(descriptor: &OperationDescriptor) -> &Value {
    &descriptor.input
}

pub fn mcp_tools(descriptors: &[OperationDescriptor]) -> Value {
    let mut tools: Vec<Value> = descriptors.iter().map(|d| json!({"name": d.name.replace('.', "_"), "description": format!("{} (operation version {})", d.description, d.version), "inputSchema": json_schema(d)})).collect();
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json!({"tools": tools, "ttlMs": 0, "cacheScope": "public"})
}

pub fn openapi(descriptors: &[OperationDescriptor]) -> Value {
    let mut paths = serde_json::Map::new();
    paths.insert("/v1/blobs".into(), json!({"post": {"operationId": "stageMessageUpload", "description": "Stage bytes for messages.send and use upload_id in files.", "requestBody": {"required": true, "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}}, "responses": {"200": {"description": "Private upload reference", "content": {"application/json": {"schema": {"type": "object", "properties": {"upload_id": {"type": "string"}}}}}}}}}));
    for d in descriptors {
        paths.insert(format!("/v1/op/{}", d.name), json!({"post": {"operationId": d.name, "description": d.description, "requestBody": {"required": true, "content": {"application/json": {"schema": json_schema(d)}}}, "responses": {"200": {"description": "Operation result", "content": {"application/json": {"schema": d.output}}}}}}));
    }
    json!({"openapi": "3.1.0", "info": {"title": "Bridge", "version": "1"}, "paths": paths})
}
