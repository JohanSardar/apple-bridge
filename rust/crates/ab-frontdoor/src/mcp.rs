use crate::{Frontdoor, problem};
use ab_operations::{Code, Error, Principal};
use axum::{
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use rmcp::{
    RoleServer, ServerHandler,
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, ErrorData, ListToolsResult,
        PaginatedRequestParams, ServerConfig, Tool,
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
        serde_json::from_value(json!({"protocolVersion":"2025-03-26","capabilities":{"logging":{},"tools":{"listChanged":true}},"serverInfo":{"name":"apple-bridge","version":"1"}})).unwrap_or_default()
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
        let descriptors =
            tool_descriptors().map_err(|_| ErrorData::internal_error("catalog", None))?;
        let descriptor = descriptors
            .iter()
            .find(|d| d.name.replace('.', "_") == request.name)
            .ok_or_else(|| {
                ErrorData::invalid_params(format!("unknown tool {:?}", request.name), None)
            })?;
        let input = serde_json::to_vec(&request.arguments.unwrap_or_default())
            .map_err(|_| ErrorData::invalid_params("invalid input", None))?;
        let started = std::time::Instant::now();
        let result = tokio::select! {
            () = context.ct.cancelled() => return Err(ErrorData::new(rmcp::model::ErrorCode(-32800), "cancelled", None)),
            result = self.frontdoor.service.invoke(principal, &descriptor.name, &input) => result,
        };
        let is_error = result.is_err();
        let raw = match result {
            Ok(value) => value.get().as_bytes().to_vec(),
            Err(error) => serde_json::to_vec(&ab_operations::Problem { error: &error })
                .map_err(|_| ErrorData::internal_error("encoding", None))?,
        };
        let bytes = self
            .frontdoor
            .encode(&raw)
            .map_err(|_| ErrorData::internal_error("encoding", None))?;
        let text =
            String::from_utf8(bytes).map_err(|_| ErrorData::internal_error("encoding", None))?;
        let structured: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| ErrorData::internal_error("encoding", None))?;
        let mut value = json!({"isError":is_error,"content":[{"type":"text","text":text}],"structuredContent":structured});
        if descriptor.read_model {
            value["_meta"] =
                json!({"bridge/read_model_ms": started.elapsed().as_secs_f64()*1000.0});
        }
        serde_json::from_value::<rmcp::model::CallToolResult>(value)
            .map(CallToolResponse::Complete)
            .map_err(|_| ErrorData::internal_error("encoding", None))
    }
}
pub async fn handle(State(frontdoor): State<Arc<Frontdoor>>, request: Request) -> Response {
    let principal = request.extensions().get::<Principal>().cloned();
    let Ok(descriptors) = tool_descriptors() else {
        return problem(Code::Corrupt.into());
    };
    let Ok(tools) = serde_json::from_value(ab_catalog::mcp_tools(&descriptors)["tools"].clone())
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

fn tool_descriptors() -> Result<Vec<ab_catalog::OperationDescriptor>, Error> {
    let descriptors = ab_catalog::operations().map_err(|_| Code::Corrupt)?;
    #[cfg(feature = "lab-queries")]
    let descriptors = {
        let mut descriptors = descriptors;
        for name in ["test.echo", "test.sleep", "test.blob"] {
            descriptors.push(ab_operations::descriptor_for(name)?);
        }
        descriptors
    };
    Ok(descriptors)
}
