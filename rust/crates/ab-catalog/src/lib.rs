#![forbid(unsafe_code)]
mod agent;
mod descriptors;
mod photos;
mod reference;
use ab_domain::Source;
pub use agent::{
    AgentOperation, VersionRule, agent_domains, agent_operations, domain_of, undescribed,
};
pub use reference::operations_reference_markdown;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

static OPERATIONS: OnceLock<Vec<OperationDescriptor>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Query,
    Read,
    Command,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    if let Some(operations) = OPERATIONS.get() {
        return Ok(operations.clone());
    }
    let schemas: Vec<Value> = serde_json::from_str(include_str!("schemas.json"))?;
    Ok(OPERATIONS
        .get_or_init(|| {
            let mut operations = descriptors::descriptors(&schemas);
            operations.extend(photos::descriptors(&schemas[46]));
            operations
        })
        .clone())
}

pub fn openapi(descriptors: &[OperationDescriptor]) -> Value {
    static OPENAPI: OnceLock<Value> = OnceLock::new();
    if OPERATIONS
        .get()
        .is_some_and(|catalog| catalog == descriptors)
    {
        return OPENAPI.get_or_init(|| build_openapi(descriptors)).clone();
    }
    build_openapi(descriptors)
}

fn build_openapi(descriptors: &[OperationDescriptor]) -> Value {
    let (errors, mut responses) = error_responses();
    let mut paths = Map::new();
    let mut upload_responses = responses.clone();
    upload_responses.insert(
        "200".into(),
        json_response("Private upload reference", "UploadStatus"),
    );
    paths.insert(
        "/v1/blobs".into(),
        json!({"post": {
            "operationId": "stageMessageUpload",
            "description": "Stage bytes for messages.send and use upload_id in files.",
            "requestBody": {"required": true, "content": {
                "application/octet-stream": {"schema": {"type": "string", "format": "binary"}}
            }},
            "responses": upload_responses
        }}),
    );

    let mut results = Map::new();
    for descriptor in descriptors {
        let schema_name = if let Some((name, _)) = results
            .iter()
            .find(|(_, schema)| *schema == &descriptor.output)
        {
            name.clone()
        } else {
            let mut name = format!("{}.Result", descriptor.name);
            if results.contains_key(&name) {
                name = format!("{name}.{}", results.len());
            }
            results.insert(name.clone(), descriptor.output.clone());
            name
        };
        let mut input = descriptor.input.clone();
        let required_field = match descriptor.name.as_str() {
            "commands.status" => Some("command_id"),
            "records.get" => Some("id"),
            _ => None,
        };
        if let Some(field) = required_field
            && input["properties"].get(field).is_some()
            && let Some(required) = input.get_mut("required").and_then(Value::as_array_mut)
            && !required.iter().any(|value| value == field)
        {
            required.push(json!(field));
        }
        responses.insert(
            "200".into(),
            json_response("Operation result", &schema_name),
        );
        paths.insert(format!("/v1/op/{}", descriptor.name), json!({"post": {
            "operationId": descriptor.name,
            "description": descriptor.description,
            "requestBody": {"required": true, "content": {"application/json": {"schema": input}}},
            "responses": responses
        }}));
    }
    let mut schemas = base_schemas();
    schemas.extend(results);
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Bridge", "version": "1"},
        "security": [{"bearerAuth": []}],
        "components": {
            "schemas": schemas,
            "responses": errors,
            "securitySchemes": {"bearerAuth": {"type": "http", "scheme": "bearer"}}
        },
        "paths": paths
    })
}

fn base_schemas() -> Map<String, Value> {
    let mut schemas = Map::new();
    schemas.insert(
        "ErrorResponse".into(),
        json!({
            "type": "object",
            "required": ["error"],
            "properties": {
                "error": {
                    "type": "object",
                    "required": ["code", "message", "retryable"],
                    "properties": {
                        "code": {"type": "string"},
                        "message": {"type": "string"},
                        "retryable": {"type": "boolean"},
                        "field": {"type": "string"},
                        "expected": {"type": "string"},
                        "hint": {"type": "string"},
                        "current": {}
                    }
                }
            }
        }),
    );
    schemas.insert(
        "UploadStatus".into(),
        json!({
            "type": "object",
            "required": ["upload_id", "offset", "size", "state", "expires_at"],
            "properties": {
                "upload_id": {"type": "string"},
                "offset": {"type": "integer", "format": "int64", "minimum": 0},
                "size": {"type": "integer", "format": "int64", "minimum": 0},
                "state": {"type": "string", "const": "ready"},
                "expires_at": {"type": "string", "format": "date-time"},
                "digest": {"type": "string"}
            }
        }),
    );
    schemas
}

fn error_responses() -> (Map<String, Value>, Map<String, Value>) {
    let mut errors = Map::new();
    let mut responses = Map::new();
    for (status, name, description) in [
        ("400", "BadRequest", "Invalid request"),
        ("401", "Unauthorized", "Bearer credential or scope rejected"),
        ("403", "Forbidden", "Grant rejected"),
        ("429", "TooManyRequests", "Rate limit exceeded"),
        ("500", "InternalServerError", "Internal server error"),
    ] {
        let mut response = json_response(description, "ErrorResponse");
        if status == "429" {
            response["headers"] = json!({
                "Retry-After": {
                    "description": "Seconds to wait before retrying.",
                    "schema": {"type": "integer", "minimum": 0}
                }
            });
        }
        errors.insert(name.into(), response);
        responses.insert(
            status.into(),
            json!({"$ref": format!("#/components/responses/{name}")}),
        );
    }
    (errors, responses)
}

fn json_response(description: &str, schema: &str) -> Value {
    json!({
        "description": description,
        "content": {"application/json": {
            "schema": {"$ref": format!("#/components/schemas/{schema}")}
        }}
    })
}
