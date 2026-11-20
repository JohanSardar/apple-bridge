use crate::{Frontdoor, problem};
use ab_operations::{Code, Error, Principal};
use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use rmcp::{
    RoleServer, ServerHandler,
    model::{
        Annotations, CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult,
        ContentBlock, ErrorData, ImageContent, ListResourcesResult, ListToolsResult, MetaObject,
        PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
        ReadResourceResult, Role, ServerConfig,
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
        let result = crate::mcp_registry::registry()
            .map(|registry| {
                ListToolsResult::with_all_items(registry.typed_tools(self.principal.as_ref()))
                    .with_ttl_ms(0)
                    .with_cache_scope(CacheScope::Private)
            })
            .ok_or_else(|| ErrorData::internal_error("operation registry failed to load", None));
        std::future::ready(result)
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
        let size = text.len();
        let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
        result.structured_content = Some(outcome.value.clone());
        result.is_error = outcome.is_error.then_some(true);
        for image in &outcome.images {
            let mut content = ImageContent::new(&image.data, &image.mime);
            if outcome.operation.as_deref() == Some("photos.contact_sheet") {
                content = content
                    .with_annotations(Annotations::default().with_audience(vec![Role::Assistant]));
            }
            result.content.push(ContentBlock::Image(content));
        }
        if let Some(elapsed) = outcome.read_model_ms {
            result.meta = Some(MetaObject(serde_json::Map::from_iter([(
                "bridge/read_model_ms".into(),
                json!(elapsed),
            )])));
        }
        Ok((CallToolResponse::Complete(result), size))
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
        "capabilities":{"logging":{},"tools":{},"resources":{}},
        "serverInfo":{"name":"apple-bridge","version":"1"},
        "instructions": crate::mcp_guides::INSTRUCTIONS,
    })
}
pub fn every_tool() -> Vec<serde_json::Value> {
    crate::mcp_registry::registry().map_or_else(Vec::new, |registry| registry.tools(None))
}
pub(crate) async fn handle(State(frontdoor): State<Arc<Frontdoor>>, request: Request) -> Response {
    let content_type = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let json = content_type == Some("application/json");
    let request = if request.method() == axum::http::Method::POST
        && content_type.is_some_and(|value| {
            let media_type = value
                .split_once(';')
                .map_or(value, |(media_type, _)| media_type);
            media_type.trim().eq_ignore_ascii_case("application/json")
        }) {
        let (parts, body) = request.into_parts();
        let bytes = match crate::read_body(body, 1 << 20).await {
            Ok(bytes) => bytes,
            Err(error) => return problem(error),
        };
        let valid = std::str::from_utf8(&bytes)
            .is_ok_and(|raw| ab_operations::command_contract::parse_json(raw).is_ok());
        if !valid {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(json!({"jsonrpc":"2.0","id":null,"error":{
                    "code":-32700,"message":"Invalid JSON or duplicate object keys"
                }})),
            )
                .into_response();
        }
        Request::from_parts(parts, axum::body::Body::from(bytes))
    } else {
        request
    };
    let principal = request.extensions().get::<Principal>().cloned();
    let mut config = StreamableHttpServerConfig::default();
    config.legacy_session_mode = false;
    config.json_response = !frontdoor.config.public;
    config.allowed_hosts = frontdoor.config.hosts.clone();
    config.allowed_origins = frontdoor.config.origins.clone();
    let handler = Handler {
        frontdoor,
        principal,
    };
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    match service.oneshot(request).await {
        Ok(response) => {
            let mut response = response.into_response();
            if json && response.status() == axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE {
                *response.status_mut() = axum::http::StatusCode::BAD_REQUEST;
            }
            response
        }
        Err(_) => problem(Error::from(Code::Unavailable)),
    }
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
