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
    principal: Principal,
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
        let descriptors =
            ab_catalog::operations().map_err(|_| ErrorData::internal_error("catalog", None))?;
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
            result = self.frontdoor.service.invoke(&self.principal, &descriptor.name, &input) => result,
        };
        let is_error = result.is_err();
        let raw = match result {
            Ok(value) => value.get().as_bytes().to_vec(),
            Err(error) => serde_json::to_vec(&json!({"error":error}))
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
    let Some(principal) = request.extensions().get::<Principal>().cloned() else {
        return problem(Code::Unauthorized.into());
    };
    let Ok(tools) = ab_catalog::operations()
        .and_then(|ds| serde_json::from_value(ab_catalog::mcp_tools(&ds)["tools"].clone()))
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
            response
        }
        Err(_) => problem(Error::from(Code::Unavailable)),
    }
}
