#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]
use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::{
    RoleServer, ServerHandler,
    model::{
        CacheScope, CallToolRequestParams, CallToolResponse, ErrorCode, ErrorData, ListToolsResult,
        PaginatedRequestParams, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use webauthn_rs_core::{
    WebauthnCore,
    proto::{COSEKey, PublicKeyCredential, UserVerificationPolicy},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Debug)]
struct Handler {
    tools: Vec<Tool>,
    active: Arc<AtomicUsize>,
}
#[derive(Debug)]
struct InFlight(Arc<AtomicUsize>);
impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        serde_json::from_value(json!({"protocolVersion":"2025-03-26","capabilities":{"logging":{},"tools":{"listChanged":true}},"serverInfo":{"name":"apple-bridge","version":"1"}})).unwrap_or_default()
    }
    fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListToolsResult, ErrorData>> {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.tools.clone())
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Public)))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        if request.name == "spike_wait" {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _in_flight = InFlight(self.active.clone());
            context.ct.cancelled().await;
            return Err(ErrorData::new(ErrorCode(-32800), "cancelled", None));
        }
        if !self.tools.iter().any(|tool| tool.name == request.name) {
            return Err(ErrorData::invalid_params(
                format!("unknown tool {:?}", request.name),
                None,
            ));
        }
        Err(ErrorData::new(
            ErrorCode(0),
            "unauthorized: credential or scope rejected",
            None,
        ))
    }
}
async fn malformed_json_status(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let json = request
        .headers()
        .get("content-type")
        .is_some_and(|value| value == "application/json");
    let mut response = next.run(request).await;
    if json && response.status() == axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE {
        *response.status_mut() = axum::http::StatusCode::BAD_REQUEST;
    }
    response
}
fn core() -> Result<WebauthnCore> {
    Ok(WebauthnCore::new_unsafe_experts_only(
        "Bridge",
        "localhost",
        vec![url::Url::parse("http://localhost:7780")?],
        Duration::from_secs(300),
        Some(false),
        Some(false),
    ))
}
fn replace_challenge<T: serde::Serialize + serde::de::DeserializeOwned>(
    state: T,
    challenge: &Value,
) -> Result<T> {
    let mut state = serde_json::to_value(state)?;
    state["challenge"] = challenge.clone();
    Ok(serde_json::from_value(state)?)
}
fn webauthn(input: &Value) -> Result<Value> {
    let wa = core()?;
    match input["op"].as_str().unwrap_or("") {
        "start" => {
            let builder = wa
                .new_challenge_register_builder(
                    b"apple-bridge-owner",
                    "Bridge owner",
                    "Bridge owner",
                )?
                .require_resident_key(true)
                .user_verification_policy(UserVerificationPolicy::Required);
            let (options, state) = wa.generate_challenge_register(builder)?;
            Ok(json!({"options": options, "state":state}))
        }
        "auth-start" => {
            let builder = wa.new_challenge_authenticate_builder(
                vec![serde_json::from_value(input["credential"].clone())?],
                Some(UserVerificationPolicy::Required),
            )?;
            let (options, state) = wa.generate_challenge_authenticate(builder)?;
            Ok(json!({"options":options,"state":state}))
        }
        "auth-finish" => {
            let response: PublicKeyCredential = serde_json::from_value(input["response"].clone())?;
            let valid_handle = response
                .response
                .user_handle
                .as_ref()
                .is_some_and(|h| h.as_slice() == b"apple-bridge-owner");
            if !valid_handle {
                return Err("wrong user handle".into());
            }
            let result = wa.authenticate_credential(
                &response,
                &serde_json::from_value(input["state"].clone())?,
            );
            Ok(json!({"accepted":result.is_ok(),"result":format!("{result:?}")}))
        }
        "finish" => {
            let credential = wa.register_credential(
                &serde_json::from_value(input["response"].clone())?,
                &serde_json::from_value(input["state"].clone())?,
                None,
            )?;
            Ok(serde_json::to_value(credential)?)
        }
        "golden" => {
            let fixture = &input["fixture"];
            let builder = wa
                .new_challenge_register_builder(
                    b"apple-bridge-owner",
                    "Bridge owner",
                    "Bridge owner",
                )?
                .require_resident_key(true)
                .user_verification_policy(UserVerificationPolicy::Required);
            let (_, state) = wa.generate_challenge_register(builder)?;
            let state =
                replace_challenge(state, &fixture["enroll_options"]["publicKey"]["challenge"])?;
            let registered = wa.register_credential(
                &serde_json::from_value(fixture["enroll_credential"].clone())?,
                &state,
                None,
            )?;
            let mut imported = registered.clone();
            let stored = &fixture["stored_credential"];
            imported.cred_id = STANDARD.decode(stored["id"].as_str().ok_or("id")?)?;
            let cose: serde_cbor_2::Value = serde_cbor_2::from_slice(
                &STANDARD.decode(stored["publicKey"].as_str().ok_or("key")?)?,
            )?;
            imported.cred = COSEKey::try_from(&cose)?;
            imported.counter =
                serde_json::from_value(stored["authenticator"]["signCount"].clone())?;
            imported.user_verified = stored["flags"]["userVerified"] == true;
            imported.backup_eligible = stored["flags"]["backupEligible"] == true;
            imported.backup_state = stored["flags"]["backupState"] == true;
            if serde_json::to_value(&imported)? != serde_json::to_value(&registered)? {
                return Err("credential conversion mismatch".into());
            }
            let builder = wa.new_challenge_authenticate_builder(
                vec![imported],
                Some(UserVerificationPolicy::Required),
            )?;
            let (_, state) = wa.generate_challenge_authenticate(builder)?;
            let state =
                replace_challenge(state, &fixture["login_options"]["publicKey"]["challenge"])?;
            let result = wa.authenticate_credential(
                &serde_json::from_value(fixture["login_credential"].clone())?,
                &state,
            )?;
            Ok(json!({"result":result,"credential":registered}))
        }
        _ => Err("unknown operation".into()),
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("serve") {
        let fixture: Value =
            serde_json::from_slice(&std::fs::read("contracts/golden/mcp-json.json")?)?;
        let response: Value = serde_json::from_str(
            fixture["exchanges"][2]["response"]
                .as_str()
                .ok_or("tools")?,
        )?;
        let active = Arc::new(AtomicUsize::new(0));
        let handler = Handler {
            active: active.clone(),
            tools: serde_json::from_value(response["result"]["tools"].clone())?,
        };
        let mut config = StreamableHttpServerConfig::default();
        config.legacy_session_mode = false;
        config.json_response = args.get(2).map(String::as_str) == Some("json");
        config.allowed_hosts = vec![
            "localhost".into(),
            "127.0.0.1".into(),
            "fixture.example.com".into(),
        ];
        config.allowed_origins = vec![
            "http://localhost:7780".into(),
            "https://fixture.example.com:443".into(),
        ];
        let service = StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        println!("{}", listener.local_addr()?);
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/active",
                    axum::routing::get(move || {
                        let active = active.clone();
                        async move { active.load(Ordering::SeqCst).to_string() }
                    }),
                )
                .nest_service("/mcp", service)
                .layer(axum::middleware::from_fn(malformed_json_status)),
        )
        .await?;
    } else if args.get(1).map(String::as_str) == Some("client") {
        use rmcp::{ServiceExt, transport::StreamableHttpClientTransport};
        let service = ()
            .serve(StreamableHttpClientTransport::from_uri(
                args.get(2).ok_or("url")?.clone(),
            ))
            .await?;
        let tools = service.list_all_tools().await?;
        let result = service
            .call_tool(
                CallToolRequestParams::new("records_query").with_arguments(serde_json::Map::new()),
            )
            .await;
        println!(
            "{}",
            json!({"tools":tools.len(), "call_error":format!("{result:?}")})
        );
        service.cancel().await?;
    } else {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        println!("{}", webauthn(&serde_json::from_str(&input)?)?);
    }
    Ok(())
}
