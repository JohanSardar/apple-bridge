#![forbid(unsafe_code)]
mod mcp;
pub mod web;
pub mod web_connector;
mod web_metadata;
use ab_operations::{Code, Cursor, Error, Principal, Service};
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, Query, Request, State},
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
use serde::Serialize;
use serde_json::json;
use std::{
    collections::HashMap,
    convert::Infallible,
    fmt::Debug,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub principal: String,
    pub method: String,
    pub route: String,
    pub status: u16,
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
}
impl Frontdoor {
    pub fn new(service: Arc<Service>, config: Config, audit: Arc<dyn Audit>) -> Arc<Self> {
        Arc::new(Self {
            service,
            config,
            audit,
            rates: Mutex::new(HashMap::new()),
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
            .route(
                "/v1/blobs",
                post(upload_message).layer(axum::extract::DefaultBodyLimit::max(100 << 20)),
            )
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
            .layer(middleware::from_fn_with_state(self.clone(), guard))
            .with_state(self)
    }
    pub fn encode(&self, raw: &[u8]) -> Result<Vec<u8>, Error> {
        self.service.codec.json(raw)
    }
}
pub fn raw_response(mut bytes: Vec<u8>) -> Response {
    bytes.push(b'\n');
    ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
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
        _ => StatusCode::BAD_REQUEST,
    };
    let raw = serde_json::to_vec(&ab_operations::Problem {error:&error}).unwrap_or_else(|_| b"{\"error\":{\"code\":\"unavailable\",\"message\":\"peer unavailable\",\"retryable\":true}}".to_vec());
    let mut response = raw_response(raw);
    *response.status_mut() = status;
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer"),
        );
    }
    response
}
fn authentication_problem(error: Error, public: bool, host: Option<&str>) -> Response {
    let mut response = problem(error);
    if response.status() == StatusCode::UNAUTHORIZED {
        let scheme = if public { "https" } else { "http" };
        if let Ok(value) = format!(
            "Bearer resource_metadata=\"{scheme}://{}/.well-known/oauth-protected-resource\"",
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
async fn guard(
    State(frontdoor): State<Arc<Frontdoor>>,
    mut request: Request,
    next: Next,
) -> Response {
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
    let principal = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| Error::from(Code::Unauthorized))
        .and_then(|token| frontdoor.service.identity.authenticate(token));
    let principal = match principal {
        Ok(p) => p,
        Err(error) => return authentication_problem(error, frontdoor.config.public, host),
    };
    let scheme = if frontdoor.config.public {
        "https"
    } else {
        "http"
    };
    if (frontdoor.config.public && !principal.remote)
        || (!principal.resource.is_empty()
            && principal.resource != format!("{scheme}://{}/mcp", host.unwrap_or_default()))
    {
        return authentication_problem(Code::Unauthorized.into(), frontdoor.config.public, host);
    }
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
        return (StatusCode::TOO_MANY_REQUESTS, "rate limited\n").into_response();
    }
    let method = request.method().to_string();
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or_else(|| "unmatched".into(), |path| path.as_str().to_owned());
    let principal_id = principal.id.clone();
    request.extensions_mut().insert(principal);
    let (parts, body) = request.into_parts();
    let limit = if route == "/v1/blobs" && method == "POST" {
        100 << 20
    } else {
        1 << 20
    };
    let Ok(bytes) = to_bytes(body, limit).await else {
        return problem(Code::Invalid.into());
    };
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    if let Err(error) = frontdoor.audit.record(AuditEntry {
        principal: principal_id,
        method,
        route,
        status: response.status().as_u16(),
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
    Path(name): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
    body: axum::body::Bytes,
) -> Response {
    let started = Instant::now();
    let result = frontdoor
        .service
        .invoke(&principal, &name, &body)
        .await
        .and_then(|value| frontdoor.encode(value.get().as_bytes()));
    let mut response = result.map_or_else(problem, raw_response);
    if ab_catalog::operations().is_ok_and(|ds| ds.iter().any(|d| d.name == name && d.read_model))
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
    body: axum::body::Bytes,
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
    Path((collection, id)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Response {
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
    Query(query): Query<HashMap<String, String>>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
) -> Response {
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
                let result = frontdoor
                    .service
                    .identity
                    .current(&principal)
                    .and_then(|current| {
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

async fn blob(
    State(frontdoor): State<Arc<Frontdoor>>,
    Path(digest): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(record) = query.get("record") else {
        return problem(Code::Unauthorized.into());
    };
    let current = match frontdoor.service.identity.current(&principal) {
        Ok(p) => p,
        Err(e) => return problem(e),
    };
    let result = if record.starts_with("qry_") {
        match &frontdoor.service.target {
            ab_operations::CommandTarget::Remote(target) => {
                target.query_blob(&current, record, &digest).await
            }
            ab_operations::CommandTarget::Local(_) => {
                frontdoor
                    .service
                    .store
                    .cached_query_blob(&current, record, &digest)
                    .await
            }
        }
    } else {
        frontdoor
            .service
            .store
            .blob(&current.visibility(), record, &digest)
            .await
    };
    match result {
        Ok(bytes) => blob_response(bytes, &headers),
        Err(e) => problem(e),
    }
}

fn blob_response(bytes: Vec<u8>, headers: &axum::http::HeaderMap) -> Response {
    if let Some(range) = headers.get(header::RANGE) {
        let total = bytes.len();
        if let Some((start, end)) = range.to_str().ok().and_then(|s| byte_range(s, total)) {
            return (
                StatusCode::PARTIAL_CONTENT,
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                    (header::ACCEPT_RANGES, "bytes".into()),
                    (
                        header::CONTENT_RANGE,
                        format!("bytes {start}-{end}/{total}"),
                    ),
                ],
                bytes[start..=end].to_vec(),
            )
                .into_response();
        }
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{total}"))],
            "invalid range\n",
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::ACCEPT_RANGES, "bytes"),
        ],
        bytes,
    )
        .into_response()
}
fn byte_range(raw: &str, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let (start, end) = raw.strip_prefix("bytes=")?.trim().split_once('-')?;
    if start.is_empty() {
        let length = end.parse::<usize>().ok()?.min(total);
        return (length > 0).then_some((total - length, total - 1));
    }
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<usize>().ok()?.min(total - 1)
    };
    (start <= end && start < total).then_some((start, end))
}

#[cfg(test)]
mod oauth_discovery_tests {
    use super::*;

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
                    "Bearer resource_metadata=\"{scheme}://{host}/.well-known/oauth-protected-resource\""
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
