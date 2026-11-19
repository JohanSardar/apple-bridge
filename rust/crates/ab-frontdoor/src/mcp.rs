use crate::{Frontdoor, problem};
use ab_operations::{Code, Error, Principal};
use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use rmcp::{
    RoleServer, ServerHandler,
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, ErrorData, ListResourcesResult,
        ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
        ReadResourceResult, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;
#[derive(Debug, Clone)]
struct Handler {
    frontdoor: Arc<Frontdoor>,
    principal: Option<Principal>,
    tools: Vec<Tool>,
}
impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        serde_json::from_value(server_info()).unwrap_or_default()
    }
    fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.tools.clone())
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Public)))
    }
    fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> {
        std::future::ready(
            serde_json::from_value(crate::mcp_guides::resources())
                .map_err(|_| ErrorData::internal_error("encoding", None)),
        )
    }
    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> {
        let result = crate::mcp_guides::guide(&request.uri).map_or_else(
            || {
                Err(ErrorData::resource_not_found(
                    format!("unknown resource {}", request.uri),
                    None,
                ))
            },
            |(name, _, text)| {
                serde_json::from_value::<ReadResourceResult>(json!({"contents":[{
                    "uri": crate::mcp_guides::uri(name),
                    "mimeType": "text/markdown",
                    "text": text,
                }]}))
                .map(ReadResourceResponse::Complete)
                .map_err(|_| ErrorData::internal_error("encoding", None))
            },
        );
        std::future::ready(result)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let principal = self.principal.as_ref().ok_or_else(|| {
            ErrorData::new(
                rmcp::model::ErrorCode(0),
                "unauthorized: credential or scope rejected",
                None,
            )
        })?;
        let arguments = request.arguments.clone().unwrap_or_default();
        if crate::mcp_registry::COMPACT.contains(&request.name.as_ref()) {
            let outcome = tokio::select! {
                () = context.ct.cancelled() => return Err(ErrorData::new(rmcp::model::ErrorCode(-32800), "cancelled", None)),
                outcome = crate::mcp_facade::call(&self.frontdoor, principal, &request.name, arguments) => outcome,
            };
            let encoded = self.compact_result(&outcome);
            self.frontdoor.observe_mcp(
                principal,
                &request.name,
                outcome.operation.as_deref(),
                outcome.class().as_deref(),
                encoded.as_ref().map_or(0, |(_, size)| *size),
            );
            return encoded.map(|(response, _)| response);
        }
        Err(unknown_tool(principal, &request.name))
    }
}
impl Handler {
    fn compact_result(
        &self,
        outcome: &crate::mcp_facade::Outcome,
    ) -> Result<(CallToolResponse, usize), ErrorData> {
        let raw = serde_json::to_vec(&outcome.value)
            .map_err(|_| ErrorData::internal_error("encoding", None))?;
        let bytes = self
            .frontdoor
            .encode(&raw)
            .map_err(|_| ErrorData::internal_error("encoding", None))?;
        let text =
            String::from_utf8(bytes).map_err(|_| ErrorData::internal_error("encoding", None))?;
        let structured: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| ErrorData::internal_error("encoding", None))?;
        let mut items = vec![json!({"type":"text","text":text})];
        for image in &outcome.images {
            let mut content = json!({"type":"image","data":image.data,"mimeType":image.mime});
            if outcome.operation.as_deref() == Some("photos.contact_sheet") {
                content["annotations"] = json!({"audience":["assistant"]});
            }
            items.push(content);
        }
        let mut value =
            json!({"isError":outcome.is_error,"content":items,"structuredContent":structured});
        if let Some(elapsed) = outcome.read_model_ms {
            value["_meta"] = json!({"bridge/read_model_ms": elapsed});
        }
        let size = text.len();
        serde_json::from_value::<rmcp::model::CallToolResult>(value)
            .map(|result| (CallToolResponse::Complete(result), size))
            .map_err(|_| ErrorData::internal_error("encoding", None))
    }
}

fn unknown_tool(principal: &Principal, name: &str) -> ErrorData {
    let mut tools: Vec<String> = crate::mcp_registry::COMPACT
        .iter()
        .filter(|tool| distance_ok(tool, name))
        .map(|tool| (*tool).to_owned())
        .collect();
    let operations: Vec<serde_json::Value> = crate::mcp_registry::registry()
        .map(|registry| {
            registry
                .closest(name, Some(principal))
                .into_iter()
                .map(|operation| {
                    json!({"tool": crate::mcp_check::tool_for(operation), "operation": operation.public})
                })
                .collect()
        })
        .unwrap_or_default();
    if tools.is_empty() && operations.is_empty() {
        tools.push("apple_describe".into());
    }
    ErrorData::invalid_params(
        format!("unknown tool {name:?}"),
        Some(json!({
            "closest": tools,
            "did_you_mean": operations,
            "hint": "Operations run through apple_read or apple_change with operation set to a name from their operation list.",
        })),
    )
}

fn distance_ok(tool: &str, wanted: &str) -> bool {
    let wanted = wanted.to_lowercase();
    tool.contains(&wanted) || wanted.contains(tool)
}

pub(crate) fn returns_image(operation: &str) -> bool {
    matches!(operation, "photos.preview" | "photos.contact_sheet")
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Image {
    pub(crate) data: String,
    pub(crate) mime: String,
}

pub(crate) fn detach_image(raw: &str) -> Result<(Vec<u8>, Option<Image>), serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(raw)?;
    let result = &mut value["result_json"];
    let image = match (
        result
            .get("image_base64")
            .and_then(serde_json::Value::as_str),
        result.get("mime_type").and_then(serde_json::Value::as_str),
    ) {
        (Some(data), Some(mime)) if mime.starts_with("image/") => Some(Image {
            data: data.to_owned(),
            mime: mime.to_owned(),
        }),
        _ => None,
    };
    if let Some(fields) = result.as_object_mut() {
        fields.remove("image_base64");
        if image.is_some() {
            fields.insert("image".into(), json!("attached as MCP image content"));
        }
    }
    Ok((serde_json::to_vec(&value)?, image))
}

pub fn server_info() -> serde_json::Value {
    json!({
        "protocolVersion":"2025-03-26",
        "capabilities":{"logging":{},"tools":{"listChanged":true},"resources":{}},
        "serverInfo":{"name":"apple-bridge","version":"1"},
        "instructions": crate::mcp_guides::INSTRUCTIONS,
    })
}
pub fn every_tool() -> Vec<serde_json::Value> {
    crate::mcp_registry::registry().map_or_else(Vec::new, |registry| registry.tools(None))
}
pub(crate) async fn handle(State(frontdoor): State<Arc<Frontdoor>>, request: Request) -> Response {
    let principal = request.extensions().get::<Principal>().cloned();
    let Some(registry) = crate::mcp_registry::registry() else {
        return problem(Code::Corrupt.into());
    };
    let Ok(tools) =
        serde_json::from_value(serde_json::Value::Array(registry.tools(principal.as_ref())))
    else {
        return problem(Code::Corrupt.into());
    };
    let mut config = StreamableHttpServerConfig::default();
    config.legacy_session_mode = false;
    config.json_response = !frontdoor.config.public;
    config.allowed_hosts = frontdoor.config.hosts.clone();
    config.allowed_origins = frontdoor.config.origins.clone();
    let handler = Handler {
        frontdoor,
        principal,
        tools,
    };
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let json = request
        .headers()
        .get("content-type")
        .is_some_and(|v| v == "application/json");
    match service.oneshot(request).await {
        Ok(response) => {
            let mut response = response.into_response();
            if json && response.status() == axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE {
                *response.status_mut() = axum::http::StatusCode::BAD_REQUEST;
            }
            normalize(response).await
        }
        Err(_) => problem(Error::from(Code::Unavailable)),
    }
}

#[derive(serde::Serialize)]
struct ToolResult<'a> {
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    meta: Option<&'a serde_json::value::RawValue>,
    content: &'a serde_json::value::RawValue,
    #[serde(rename = "structuredContent")]
    structured: &'a serde_json::value::RawValue,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not")]
    is_error: bool,
}
#[derive(serde::Serialize)]
struct Envelope<'a> {
    jsonrpc: &'a serde_json::value::RawValue,
    id: &'a serde_json::value::RawValue,
    result: &'a serde_json::value::RawValue,
}

fn normalize_json(bytes: &[u8]) -> Option<Vec<u8>> {
    use serde_json::value::RawValue;
    let mut envelope: std::collections::BTreeMap<String, Box<RawValue>> =
        serde_json::from_slice(bytes).ok()?;
    let result: std::collections::BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(envelope.get("result")?.get()).ok()?;
    let content = result.get("content")?;
    let items: serde_json::Value = serde_json::from_str(content.get()).ok()?;
    let structured = RawValue::from_string(items[0]["text"].as_str()?.to_owned()).ok()?;
    let result = ToolResult {
        meta: result.get("_meta").map(AsRef::as_ref),
        content,
        structured: &structured,
        is_error: result.get("isError").is_some_and(|v| v.get() == "true"),
    };
    envelope.insert(
        "result".into(),
        serde_json::value::to_raw_value(&result).ok()?,
    );
    serde_json::to_vec(&Envelope {
        jsonrpc: envelope.get("jsonrpc")?,
        id: envelope.get("id")?,
        result: envelope.get("result")?,
    })
    .ok()
}
async fn normalize(response: Response) -> Response {
    let (parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 16 << 20).await else {
        return problem(Code::Corrupt.into());
    };
    let json = parts
        .headers
        .get("content-type")
        .is_some_and(|v| v.to_str().is_ok_and(|s| s.starts_with("application/json")));
    let normalized = if json {
        normalize_json(&bytes).unwrap_or_else(|| bytes.to_vec())
    } else {
        String::from_utf8_lossy(&bytes)
            .split_inclusive('\n')
            .map(|line| {
                if let Some(data) = line.strip_prefix("data:")
                    && let Some(result) = normalize_json(data.trim().as_bytes())
                {
                    return format!("data: {}\n", String::from_utf8_lossy(&result));
                }
                line.to_owned()
            })
            .collect::<String>()
            .into_bytes()
    };
    let mut response = Response::from_parts(parts, axum::body::Body::from(normalized));
    response.headers_mut().remove("content-length");
    response
}

#[cfg(feature = "lab-harness")]
pub fn golden_router(frontdoor: Arc<Frontdoor>) -> axum::Router {
    axum::Router::new()
        .route("/golden-mcp", axum::routing::any(handle))
        .with_state(frontdoor)
}

#[cfg(test)]
mod image_tests {
    #[test]
    fn photo_images_move_from_text_into_image_content() -> Result<(), Box<dyn std::error::Error>> {
        let raw = r#"{"id":"qry_1","blobs":[],"result_json":{"asset_id":"a","mime_type":"image/jpeg","width":2,"height":1,"image_base64":"/9j/AA=="}}"#;
        let (body, image) = super::detach_image(raw)?;
        let body: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(
            image,
            Some(super::Image {
                data: "/9j/AA==".into(),
                mime: "image/jpeg".into()
            })
        );
        assert!(body["result_json"].get("image_base64").is_none());
        assert_eq!(body["result_json"]["asset_id"], "a");
        let (_, image) =
            super::detach_image(r#"{"result_json":{"image_base64":"x","mime_type":"text/html"}}"#)?;
        assert_eq!(image, None);
        assert!(super::returns_image("photos.contact_sheet"));
        assert!(!super::returns_image("photos.fetch"));
        Ok(())
    }
}
