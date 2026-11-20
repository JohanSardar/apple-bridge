#![forbid(unsafe_code)]
mod mcp;
mod mcp_answer;
mod mcp_check;
mod mcp_compose;
mod mcp_facade;
mod mcp_find;
mod mcp_guides;
mod mcp_lookups;
mod mcp_names;
mod mcp_photos;
mod mcp_registry;
#[cfg(test)]
mod mcp_tests;
mod mcp_views;
pub mod web;
pub mod web_connector;
mod web_metadata;
mod web_oauth;
mod web_pages;
use ab_operations::{Code, Cursor, Error, Principal, Service};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{
        DefaultBodyLimit, FromRequest, Path, Query, Request, State, rejection::PathRejection,
        rejection::QueryRejection,
    },
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{any, get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
#[cfg(feature = "lab-harness")]
pub use mcp::golden_router;
pub use mcp::{every_tool as mcp_tools, server_info as mcp_server_info};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    fmt::Debug,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

static READ_MODELS: OnceLock<HashSet<String>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub principal: String,
    pub method: String,
    pub route: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_bytes: Option<u64>,
}
pub trait Audit: Debug + Send + Sync {
    fn record(&self, entry: AuditEntry) -> Result<(), Error>;
}
#[derive(Debug, Clone)]
pub struct Config {
    pub hosts: Vec<String>,
    pub origins: Vec<String>,
    pub public: bool,
    pub requests_per_second: u32,
}
#[derive(Debug)]
pub struct Frontdoor {
    pub service: Arc<Service>,
    pub config: Config,
    pub audit: Arc<dyn Audit>,
    rates: Mutex<HashMap<String, (Instant, u32)>>,
    mcp_failures: Mutex<HashMap<(String, String), Instant>>,
}
impl Frontdoor {
    pub fn new(service: Arc<Service>, config: Config, audit: Arc<dyn Audit>) -> Arc<Self> {
        Arc::new(Self {
            service,
            config,
            audit,
            rates: Mutex::new(HashMap::new()),
            mcp_failures: Mutex::new(HashMap::new()),
        })
    }
    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route(
                "/v1/health",
                get(|| async { raw_response(b"{\"state\":\"alive\"}".to_vec()) }),
            )
            .route("/v1/openapi.json", get(openapi))
            .route("/v1/op/{name}", post(invoke))
            .route("/v1/records/{collection}/{id}", get(record))
            .route("/v1/blobs/{digest}", get(blob))
            .route("/v1/blobs", post(upload_message))
            .route("/v1/changes", get(changes))
            .route("/mcp", any(mcp::handle))
            .route(
                "/mcp/{*path}",
                any(|| async { problem(Code::NotFound.into()) }),
            )
            .route(
                "/v1/{*path}",
                any(|| async { problem(Code::NotFound.into()) }),
            )
            .fallback(|| async { problem(Code::NotFound.into()) })
            .method_not_allowed_fallback(|| async {
                http_problem(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "method_not_allowed",
                    "method not allowed",
                    false,
                )
            })
            .layer(middleware::from_fn_with_state(self.clone(), guard))
            .with_state(self)
    }
    pub(crate) fn observe_mcp(
        &self,
        principal: &Principal,
        tool: &str,
        operation: Option<&str>,
        failure: Option<&str>,
        answer_bytes: usize,
    ) {
        let operation = operation.unwrap_or(tool).to_owned();
        let key = (principal.id.clone(), operation.clone());
        let recovered = self.mcp_failures.lock().is_ok_and(|mut failures| {
            failures.retain(|_, at| at.elapsed() < Duration::from_secs(300));
            if failure.is_some() {
                failures.insert(key, Instant::now());
                false
            } else {
                failures.remove(&key).is_some()
            }
        });
        let class = failure.unwrap_or("ok");
        let mut route = format!("tools/call {tool} {operation} {class}");
        if recovered {
            route.push_str(" recovered");
        }
        let _ = self.audit.record(AuditEntry {
            principal: principal.id.clone(),
            method: "MCP".into(),
            route,
            status: if failure.is_some() { 422 } else { 200 },
            answer_bytes: u64::try_from(answer_bytes).ok(),
        });
    }
    pub fn encode(&self, raw: &[u8]) -> Result<Vec<u8>, Error> {
        self.service.codec.json(raw)
    }
}
pub fn raw_response(mut bytes: Vec<u8>) -> Response {
    bytes.push(b'\n');
    ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
}
#[derive(Serialize, Deserialize)]
struct FieldError {
    field: String,
    expected: String,
    hint: String,
}

#[derive(Serialize)]
struct RestError<'a> {
    code: &'a str,
    message: &'a str,
    retryable: bool,
    #[serde(flatten)]
    field_error: Option<FieldError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<&'a serde_json::value::RawValue>,
}

impl RestError<'_> {
    fn response(self, status: StatusCode) -> Response {
        #[derive(Serialize)]
        struct Envelope<'a> {
            error: RestError<'a>,
        }
        let bytes = serde_json::to_vec(&Envelope { error: self }).unwrap_or_else(|_| {
            b"{\"error\":{\"code\":\"corrupt\",\"message\":\"error encoding failed\",\"retryable\":false}}".to_vec()
        });
        let mut response = raw_response(bytes);
        *response.status_mut() = status;
        response
    }
}

fn http_problem(status: StatusCode, code: &str, message: &str, retryable: bool) -> Response {
    RestError {
        code,
        message,
        retryable,
        field_error: None,
        current: None,
    }
    .response(status)
}

fn rate_limited() -> Response {
    let mut response = http_problem(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "rate limited",
        true,
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
    response
}

#[allow(clippy::needless_pass_by_value)]
pub fn problem(error: Error) -> Response {
    let status = match error.code {
        Code::Unauthorized => StatusCode::UNAUTHORIZED,
        Code::Forbidden => StatusCode::FORBIDDEN,
        Code::NotFound => StatusCode::NOT_FOUND,
        Code::Conflict | Code::Reset | Code::IdReused => StatusCode::CONFLICT,
        Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Code::Capacity => StatusCode::INSUFFICIENT_STORAGE,
        Code::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        Code::Unsupported => StatusCode::UNPROCESSABLE_ENTITY,
        Code::Corrupt => StatusCode::INTERNAL_SERVER_ERROR,
        Code::Deadline => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_REQUEST,
    };
    let current = error.current.as_deref();
    let field_error = if matches!(
        error.code,
        Code::Invalid | Code::TooLarge | Code::Unsupported | Code::Expired
    ) {
        current.and_then(|raw| serde_json::from_str::<FieldError>(raw.get()).ok())
    } else {
        None
    };
    let current = if field_error.is_none() { current } else { None };
    let mut response = RestError {
        code: &error.code.to_string(),
        message: &error.message,
        retryable: error.retryable,
        field_error,
        current,
    }
    .response(status);
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer"),
        );
    }
    response
}

fn extraction_problem(status: StatusCode) -> Response {
    problem(
        if status.is_server_error() {
            Code::Corrupt
        } else {
            Code::Invalid
        }
        .into(),
    )
}

async fn read_body(body: Body, limit: usize) -> Result<Bytes, Error> {
    let mut request = Request::new(body);
    DefaultBodyLimit::max(limit).apply(&mut request);
    Bytes::from_request(request, &())
        .await
        .map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                Error {
                    message: "request body exceeds limit".into(),
                    ..Code::TooLarge.into()
                }
            } else {
                Code::Invalid.into()
            }
        })
}
fn authentication_problem(error: Error, public: bool, host: Option<&str>) -> Response {
    let mut response = problem(error);
    if response.status() == StatusCode::UNAUTHORIZED {
        let scheme = if public { "https" } else { "http" };
        if let Ok(value) = format!(
            "Bearer resource_metadata=\"{scheme}://{}/.well-known/oauth-protected-resource/mcp\"",
            host.unwrap_or_default()
        )
        .parse()
        {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
    }
    response
}
fn authenticate_bearer(
    identity: &dyn ab_operations::Identity,
    public: bool,
    token: &str,
    host: Option<&str>,
) -> Result<Principal, Error> {
    let principal = identity.authenticate(token)?;
    let scheme = if public { "https" } else { "http" };
    if (public && !principal.remote)
        || (!principal.resource.is_empty()
            && principal.resource != format!("{scheme}://{}/mcp", host.unwrap_or_default()))
        || principal
            .expires_at
            .is_some_and(|expiry| expiry <= jiff::Timestamp::now())
    {
        return Err(Code::Unauthorized.into());
    }
    Ok(principal)
}

async fn authenticated_body(
    body: Body,
    limit: usize,
    identity: &dyn ab_operations::Identity,
    public: bool,
    host: Option<&str>,
    token: &str,
    principal_id: &str,
) -> Result<(Bytes, Principal), Box<Response>> {
    let bytes = read_body(body, limit)
        .await
        .map_err(|error| Box::new(problem(error)))?;
    let principal = authenticate_bearer(identity, public, token, host)
        .map_err(|error| Box::new(authentication_problem(error, public, host)))?;
    if principal.id != principal_id {
        return Err(Box::new(authentication_problem(
            Code::Unauthorized.into(),
            public,
            host,
        )));
    }
    Ok((bytes, principal))
}

async fn guard(State(frontdoor): State<Arc<Frontdoor>>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    if !host.is_some_and(|h| frontdoor.config.hosts.iter().any(|allowed| allowed == h)) {
        return problem(Code::Unauthorized.into());
    }
    if let Some(origin) = request.headers().get(header::ORIGIN)
        && !origin
            .to_str()
            .is_ok_and(|o| frontdoor.config.origins.iter().any(|allowed| allowed == o))
    {
        return problem(Code::Unauthorized.into());
    }
    if request.uri().path() == "/v1/health" && !frontdoor.config.public {
        return next.run(request).await;
    }
    let authorization = request.headers().get(header::AUTHORIZATION).cloned();
    let Some(token) = authorization
        .as_ref()
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    else {
        return authentication_problem(Code::Unauthorized.into(), frontdoor.config.public, host);
    };
    let public = frontdoor.config.public;
    let identity = frontdoor.service.identity.as_ref();
    let principal = match authenticate_bearer(identity, public, token, host) {
        Ok(principal) => principal,
        Err(error) => return authentication_problem(error, frontdoor.config.public, host),
    };
    let allowed = match frontdoor.rates.lock() {
        Ok(mut rates) => {
            rates.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(1));
            let rate = rates
                .entry(principal.id.clone())
                .or_insert((Instant::now(), 0));
            rate.1 += 1;
            rate.1 <= frontdoor.config.requests_per_second
        }
        Err(_) => return problem(Code::Unavailable.into()),
    };
    if !allowed {
        return rate_limited();
    }
    let method = request.method().to_string();
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or_else(|| "unmatched".into(), |path| path.as_str().to_owned());
    let limit = if route == "/v1/blobs" && method == "POST" {
        100 << 20
    } else {
        1 << 20
    };
    let (mut parts, body) = request.into_parts();
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    let (bytes, principal) =
        match authenticated_body(body, limit, identity, public, host, token, &principal.id).await {
            Ok(result) => result,
            Err(response) => return *response,
        };
    let principal_id = principal.id.clone();
    parts.extensions.insert(principal);
    let body = if route == "/v1/blobs" || route == "/v1/op/{name}" {
        parts.extensions.insert(bytes);
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let response = next.run(Request::from_parts(parts, body)).await;
    if let Err(error) = frontdoor.audit.record(AuditEntry {
        principal: principal_id,
        method,
        route,
        status: response.status().as_u16(),
        answer_bytes: None,
    }) {
        return problem(error);
    }
    response
}
async fn openapi(State(frontdoor): State<Arc<Frontdoor>>) -> Response {
    let result = ab_catalog::operations()
        .map(|descriptors| ab_catalog::openapi(&descriptors))
        .map_err(|_| Error::from(Code::Corrupt))
        .and_then(|value| serde_json::to_vec(&value).map_err(|_| Error::from(Code::Corrupt)))
        .and_then(|bytes| frontdoor.encode(&bytes));
    result.map_or_else(problem, raw_response)
}
async fn invoke(
    State(frontdoor): State<Arc<Frontdoor>>,
    name: Result<Path<String>, PathRejection>,
    axum::Extension(principal): axum::Extension<Principal>,
    axum::Extension(body): axum::Extension<Bytes>,
) -> Response {
    let name = match name {
        Ok(Path(name)) => name,
        Err(rejection) => return extraction_problem(rejection.status()),
    };
    let started = Instant::now();
    let result = frontdoor
        .service
        .invoke(&principal, &name, &body)
        .await
        .and_then(|value| frontdoor.encode(value.get().as_bytes()));
    let mut response = result.map_or_else(problem, raw_response);
    if READ_MODELS
        .get_or_init(|| {
            ab_catalog::operations()
                .unwrap_or_default()
                .into_iter()
                .filter(|descriptor| descriptor.read_model)
                .map(|descriptor| descriptor.name)
                .collect()
        })
        .contains(&name)
        && let Ok(value) = format!(
            "readmodel;dur={:.6}",
            started.elapsed().as_secs_f64() * 1000.0
        )
        .parse()
    {
        response.headers_mut().insert("server-timing", value);
    }
    response
}
async fn upload_message(
    State(frontdoor): State<Arc<Frontdoor>>,
    axum::Extension(principal): axum::Extension<Principal>,
    axum::Extension(body): axum::Extension<Bytes>,
) -> Response {
    frontdoor
        .service
        .upload_message(&principal, &body)
        .await
        .and_then(|value| frontdoor.encode(value.get().as_bytes()))
        .map_or_else(problem, raw_response)
}
async fn record(
    State(frontdoor): State<Arc<Frontdoor>>,
    path: Result<Path<(String, String)>, PathRejection>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Response {
    let (collection, id) = match path {
        Ok(Path(path)) => path,
        Err(rejection) => return extraction_problem(rejection.status()),
    };
    match serde_json::to_vec(&json!({"collection":collection,"id":id})) {
        Ok(input) => frontdoor
            .service
            .invoke(&principal, "records.get", &input)
            .await
            .and_then(|value| frontdoor.encode(value.get().as_bytes()))
            .map_or_else(problem, raw_response),
        Err(_) => problem(Code::Invalid.into()),
    }
}
fn event_id(cursor: &Cursor) -> Result<String, Error> {
    let raw = serde_json::to_vec(cursor).map_err(|_| Error::from(Code::Corrupt))?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}
fn parse_event_id(value: &str) -> Result<Cursor, Error> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| Error::from(Code::Reset))?;
    let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| Error::from(Code::Reset))?;
    if cursor.epoch.is_empty() || cursor.subscription_revision == 0 {
        return Err(Code::Reset.into());
    }
    Ok(cursor)
}
async fn changes(
    State(frontdoor): State<Arc<Frontdoor>>,
    query: Result<Query<HashMap<String, String>>, QueryRejection>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(error) = principal.authorize_reads() {
        return problem(error);
    }
    let query = match query {
        Ok(Query(query)) => query,
        Err(rejection) => return extraction_problem(rejection.status()),
    };
    let after = query
        .get("after")
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .or_else(|| {
            headers
                .get("last-event-id")
                .and_then(|h| h.to_str().ok())
                .filter(|id| !id.is_empty())
        });
    let cursor = match after.map_or_else(|| frontdoor.service.store.head(), parse_event_id) {
        Ok(c) => c,
        Err(e) => return problem(e),
    };
    let visibility = principal.visibility();
    if let Err(error) = frontdoor.service.store.changes(&visibility, &cursor) {
        return problem(error);
    }
    let stream = futures_util::stream::unfold(
        (frontdoor, principal, cursor, false),
        |(frontdoor, principal, mut cursor, ended)| async move {
            if ended {
                return None;
            }
            loop {
                if principal
                    .expires_at
                    .is_some_and(|expiry| expiry <= jiff::Timestamp::now())
                {
                    return None;
                }
                let result = frontdoor
                    .service
                    .identity
                    .current(&principal)
                    .and_then(|current| {
                        current.authorize_reads()?;
                        frontdoor
                            .service
                            .store
                            .changes(&current.visibility(), &cursor)
                    });
                match result {
                    Ok(Some(batch)) => {
                        let value: serde_json::Value = match serde_json::from_str(batch.get()) {
                            Ok(v) => v,
                            Err(_) => return None,
                        };
                        cursor.sequence = value["through"].as_u64()?;
                        let id = event_id(&cursor).ok()?;
                        let bytes = frontdoor.encode(batch.get().as_bytes()).ok()?;
                        let event = Event::default()
                            .event("batch")
                            .id(id)
                            .data(String::from_utf8(bytes).ok()?);
                        return Some((
                            Ok::<_, Infallible>(event),
                            (frontdoor, principal, cursor, false),
                        ));
                    }
                    Ok(None) => tokio::time::sleep(Duration::from_millis(100)).await,
                    Err(Error {
                        code: Code::Reset, ..
                    }) => {
                        return Some((
                            Ok(Event::default()
                                .event("reset")
                                .data("{\"error\":\"reset\"}")),
                            (frontdoor, principal, cursor, true),
                        ));
                    }
                    Err(_) => return None,
                }
            }
        },
    );
    let stream = futures_util::StreamExt::chain(
        futures_util::stream::once(async { Ok(Event::default().comment("connected")) }),
        stream,
    );
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(5))
                .text("heartbeat"),
        )
        .into_response()
}

#[cfg(test)]
mod oauth_discovery_tests {
    #![allow(clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn buffering_rechecks_the_presented_access_tokens_expiry_and_resource()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let authority = Arc::new(ab_trust::Authority::open(home.path())?);
        let (agent, _) = authority.create_oauth_agent(
            "Synthetic",
            "full",
            &[ab_trust::Grant::parse("calendar:read")?],
            false,
        )?;
        let token = "synthetic-oauth-access";
        let identity = ab_operations::TrustIdentity(authority.clone());
        let host = Some("bridge.test");
        let resource = "http://bridge.test/mcp";
        let live_until = ab_trust::oauth::expiry(3600);
        let install_access = move |authority: &ab_trust::Authority,
                                   principal: &str,
                                   resource: &str,
                                   expires_at: &str| {
            authority.update(|credentials| {
                credentials.extra.insert(
                    "oauth_access".into(),
                    json!([{
                        "hash": ab_trust::fingerprint(token.as_bytes()),
                        "principal": principal,
                        "resource": resource,
                        "expires_at": expires_at,
                    }]),
                );
                Ok(())
            })
        };
        for (updated_resource, updated_expiry) in [
            (resource, "2000-01-01T00:00:00Z"),
            ("http://other.test/mcp", live_until.as_str()),
        ] {
            install_access(&authority, &agent.id, resource, &live_until)?;
            let initial = authenticate_bearer(&identity, false, token, host)?;
            let fixture = authority.clone();
            let principal_id = agent.id.clone();
            let updated_expiry = updated_expiry.to_owned();
            let body = Body::from_stream(futures_util::stream::once(async move {
                install_access(&fixture, &principal_id, updated_resource, &updated_expiry)?;
                Ok::<_, ab_trust::Error>(Bytes::from_static(b"{}"))
            }));
            let denied = authenticated_body(body, 1024, &identity, false, host, token, &initial.id)
                .await
                .expect_err("the original access token became unusable during buffering");
            assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
            assert!(authority.principal(&agent.id)?.active());
        }
        Ok(())
    }

    #[test]
    fn challenges_use_the_validated_transport_and_host() {
        for (public, host, scheme) in [
            (false, "127.0.0.1:7780", "http"),
            (true, "bridge.example", "https"),
        ] {
            let response = authentication_problem(Code::Unauthorized.into(), public, Some(host));
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response.headers()[header::WWW_AUTHENTICATE],
                format!(
                    "Bearer resource_metadata=\"{scheme}://{host}/.well-known/oauth-protected-resource/mcp\""
                )
            );
        }
    }

    #[test]
    fn other_errors_do_not_offer_an_authentication_challenge() {
        let response =
            authentication_problem(Code::Unavailable.into(), false, Some("localhost:7780"));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
    }
}

mod blobs;
use blobs::blob;
