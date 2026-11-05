use crate::{
    Accounts, Control, DeviceAction, Error, RelayAdmin, Result, accounts::Session, pages,
    store::VerifiedGrant,
};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{any, get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const FRESH_SECONDS: i64 = 3600;

#[derive(Clone)]
struct App {
    control: Arc<Control>,
    accounts: Arc<dyn Accounts>,
    relay: Arc<dyn RelayAdmin>,
    accounts_url: String,
    proxy: reqwest::Client,
    host: String,
    origin: String,
}

pub fn router(
    control: Arc<Control>,
    accounts: Arc<dyn Accounts>,
    relay: Arc<dyn RelayAdmin>,
    accounts_url: &str,
) -> Result<Router> {
    let url = url::Url::parse(control.public_url()).map_err(|_| Error::Invalid)?;
    let host = url.authority().to_owned();
    let origin = url.origin().ascii_serialization();
    let app = App {
        control,
        accounts,
        relay,
        accounts_url: accounts_url.trim_end_matches('/').to_owned(),
        proxy: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| Error::Unavailable)?,
        host,
        origin,
    };
    Ok(Router::new()
        .route("/", get(dashboard))
        .route("/sign-in", get(|| async { Html(pages::sign_in()) }))
        .route("/invite", get(|| async { Html(pages::invite()) }))
        .route("/device", get(device))
        .route("/health", get(|| async { "ok\n" }))
        .route(ab_pages::STYLESHEET_PATH, get(stylesheet))
        .route("/assets/account.js", get(script))
        .route("/api/auth/{*path}", any(proxy_accounts))
        .route("/v1/{action}", post(action))
        .layer(DefaultBodyLimit::max(16384))
        .layer(middleware::from_fn_with_state(app.clone(), boundary))
        .with_state(app))
}

async fn boundary(State(app): State<App>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let path = request.uri().path();
    let from_mac = path.starts_with("/v1/device-");
    let writes = !matches!(request.method().as_str(), "GET" | "HEAD");
    if path != "/health"
        && (host != Some(app.host.as_str())
            || (writes && !from_mac && origin != Some(app.origin.as_str())))
    {
        return Error::Forbidden.into_response();
    }
    let proxied = path.starts_with("/api/auth/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if !proxied {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    for (name, value) in [
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("strict-transport-security", "max-age=31536000"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Unavailable)?
        .as_secs()
        .try_into()
        .map_err(|_| Error::Unavailable)
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Invalid => StatusCode::BAD_REQUEST,
            Self::Unauthorized | Self::SignInAgain => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Capacity | Self::SlowDown => StatusCode::TOO_MANY_REQUESTS,
            Self::Pending => StatusCode::ACCEPTED,
            Self::Expired => StatusCode::GONE,
            Self::Unavailable | Self::Fenced => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, Json(json!({"error": self.to_string()}))).into_response()
    }
}

async fn stylesheet() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], ab_pages::STYLESHEET)
}
async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../assets/account.js"),
    )
}

async fn current(app: &App, headers: &HeaderMap) -> Result<Session> {
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or(Error::Unauthorized)?;
    let source = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map_or("127.0.0.1", str::trim);
    app.accounts.session(cookie, source).await
}

async fn dashboard(State(app): State<App>, headers: HeaderMap) -> Response {
    match current(&app, &headers).await {
        Ok(session) => Html(pages::dashboard(&session.email)).into_response(),
        Err(Error::Unauthorized) => Redirect::to("/sign-in").into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Deserialize)]
struct DeviceQuery {
    #[serde(default)]
    code: String,
}
async fn device(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<DeviceQuery>,
) -> Response {
    match current(&app, &headers).await {
        Ok(session) => Html(pages::device(&session.email, &query.code)).into_response(),
        Err(Error::Unauthorized) => {
            let next = if query.code.is_empty() {
                "/device".to_owned()
            } else {
                format!(
                    "/device?code={}",
                    url::form_urlencoded::byte_serialize(query.code.as_bytes()).collect::<String>()
                )
            };
            let target = format!(
                "/sign-in?next={}",
                url::form_urlencoded::byte_serialize(next.as_bytes()).collect::<String>()
            );
            Redirect::to(&target).into_response()
        }
        Err(error) => error.into_response(),
    }
}

fn client_address(headers: &HeaderMap, peer: SocketAddr) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map_or_else(|| peer.ip().to_string(), |value| value.trim().to_owned())
}

async fn proxy_accounts(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    if request.uri().path().starts_with("/api/auth/delete-user") {
        return Error::Forbidden.into_response();
    }
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, 16384).await else {
        return Error::Invalid.into_response();
    };
    let query = parts
        .uri
        .query()
        .map_or_else(String::new, |q| format!("?{q}"));
    let mut outgoing = app
        .proxy
        .request(
            parts.method,
            format!("{}{}{query}", app.accounts_url, parts.uri.path()),
        )
        .header("x-forwarded-for", client_address(&parts.headers, peer))
        .body(body);
    for name in [
        header::COOKIE,
        header::ORIGIN,
        header::CONTENT_TYPE,
        header::USER_AGENT,
        header::ACCEPT,
    ] {
        if let Some(value) = parts.headers.get(&name) {
            outgoing = outgoing.header(name, value);
        }
    }
    let Ok(reply) = outgoing.send().await else {
        return Error::Unavailable.into_response();
    };
    let status = reply.status();
    let reply_headers = reply.headers().clone();
    let Ok(bytes) = reply.bytes().await else {
        return Error::Unavailable.into_response();
    };
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    for (name, value) in &reply_headers {
        if matches!(
            name.as_str(),
            "set-cookie" | "content-type" | "location" | "cache-control" | "retry-after"
        ) {
            response.headers_mut().append(name, value.clone());
        }
    }
    response
}

fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str> {
    input[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or(Error::Invalid)
}

async fn action(
    State(app): State<App>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(action): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Response> {
    let time = now()?;
    app.control
        .throttle(&client_address(&headers, peer), time)?;
    let c = &app.control;
    let result = match action.as_str() {
        "device-start" => c.start_grant(
            text(&input, "public_key")?,
            input["name"].as_str().unwrap_or_default(),
            &client_address(&headers, peer),
            time,
        )?,
        "device-poll" => c.poll(text(&input, "polling_secret")?, time)?,
        "device-complete" => {
            let grant = c.verify_grant(
                text(&input, "polling_secret")?,
                text(&input, "signature")?,
                time,
            )?;
            complete(&app, &grant, time).await?
        }
        _ => signed_in(&app, &action, &headers, peer, &input, time).await?,
    };
    Ok(Json(result).into_response())
}

async fn complete(app: &App, grant: &VerifiedGrant, time: i64) -> Result<Value> {
    let admitted = match app.relay.admit(&grant.public_key).await {
        Ok(admitted) => admitted,
        Err(error) => {
            let _ = app.control.release_grant(grant);
            return Err(error);
        }
    };
    if let Err(error) = app.control.finish_grant(grant, &admitted.hostname, time) {
        let _ = app.control.release_grant(grant);
        if matches!(error, Error::Capacity) {
            let _ = app.relay.policy(&admitted.hostname, true, true, 0).await;
        }
        return Err(error);
    }
    Ok(json!({"hostname": admitted.hostname, "credential": admitted.credential}))
}

async fn signed_in(
    app: &App,
    action: &str,
    headers: &HeaderMap,
    peer: SocketAddr,
    input: &Value,
    time: i64,
) -> Result<Value> {
    let session = current(app, headers).await?;
    let fresh = || {
        if time - session.signed_in_at <= FRESH_SECONDS {
            Ok(())
        } else {
            Err(Error::SignInAgain)
        }
    };
    let c = &app.control;
    Ok(match action {
        "list" => c.list(&session.account)?,
        "grant-view" => c.grant_view(text(input, "code")?, &client_address(headers, peer), time)?,
        "confirm" => {
            fresh()?;
            c.confirm(
                &session.account,
                text(input, "code")?,
                text(input, "fingerprint")?,
                time,
            )?
        }
        "suspend" | "resume" | "revoke" => {
            let change = match action {
                "suspend" => DeviceAction::Suspend,
                "resume" => DeviceAction::Resume,
                _ => {
                    fresh()?;
                    DeviceAction::Revoke
                }
            };
            c.change(&session.account, text(input, "hostname")?, change)?;
            apply_policy(c, app.relay.as_ref()).await;
            json!({"committed": true})
        }
        "revoke-all" => {
            fresh()?;
            c.revoke_all(&session.account)?;
            apply_policy(c, app.relay.as_ref()).await;
            json!({"committed": true})
        }
        _ => return Err(Error::Invalid),
    })
}

async fn apply_policy(control: &Control, relay: &dyn RelayAdmin) {
    let Some(_serialized) = control.try_lock_policy_sync() else {
        return;
    };
    apply_pending_policy(control, relay).await;
}

async fn apply_pending_policy(control: &Control, relay: &dyn RelayAdmin) {
    let Ok(pending) = control.pending_policy() else {
        return;
    };
    for change in pending {
        if relay
            .policy(
                &change.hostname,
                change.disabled,
                change.revoked,
                change.version(),
            )
            .await
            .is_ok()
        {
            let _ = control.policy_applied(&change);
        }
    }
}

pub async fn sync_policy(control: Arc<Control>, relay: Arc<dyn RelayAdmin>) {
    loop {
        {
            let _serialized = control.lock_policy_sync().await;
            apply_pending_policy(&control, relay.as_ref()).await;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
