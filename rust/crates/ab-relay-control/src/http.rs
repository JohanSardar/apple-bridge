use crate::{Control, Error, IdentityProvider, Result, auth::text, random};
use axum::{
    Json, Router,
    extract::Request,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct App {
    control: Arc<Control>,
    provider: Arc<dyn IdentityProvider>,
}
impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("App([redacted])")
    }
}
pub fn router(control: Arc<Control>, provider: Arc<dyn IdentityProvider>) -> Router {
    Router::new()
        .route("/", get(ui))
        .route("/device", get(ui))
        .route("/ui.js", get(script))
        .route("/v1/oauth/callback", get(callback))
        .route("/v1/{action}", post(action))
        .layer(DefaultBodyLimit::max(16384))
        .layer(middleware::from_fn(boundary))
        .with_state(App { control, provider })
}
async fn boundary(request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers.get("host").and_then(|v| v.to_str().ok());
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    if host != Some("app.lexaai.tech")
        || origin.is_some_and(|v| v != "https://app.lexaai.tech")
        || (request.method() == "POST"
            && origin != Some("https://app.lexaai.tech")
            && !request.uri().path().starts_with("/v1/device-"))
    {
        return Error::Forbidden.into_response();
    }
    let mut response = next.run(request).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
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
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|s| s.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, v)| v)
}
fn session_response(mut value: Value) -> Result<Response> {
    let secret = text(&value, "session")?.to_owned();
    if let Some(map) = value.as_object_mut() {
        map.remove("session");
    }
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        "set-cookie",
        HeaderValue::from_str(&format!(
            "__Host-ab-control={secret}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=3600"
        ))
        .map_err(|_| Error::Invalid)?,
    );
    Ok(response)
}
async fn callback(
    State(app): State<App>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<Response> {
    let state = query.get("state").ok_or(Error::Invalid)?;
    let code = query.get("code").ok_or(Error::Invalid)?;
    let browser = cookie(&headers, "__Host-ab-oauth").ok_or(Error::Unauthorized)?;
    let value = app
        .control
        .oauth_finish(app.provider.as_ref(), state, code, browser, now()?)
        .await?;
    let csrf = text(&value, "csrf")?.to_owned();
    let account = text(&value, "account")?.to_owned();
    let session = session_response(value)?;
    let mut response=Html(include_str!("ui.html").replace("<title>",&format!("<meta name=\"csrf\" content=\"{csrf}\"><meta name=\"account\" content=\"{account}\"><title>"))).into_response();
    response.headers_mut().insert(
        "set-cookie",
        session
            .headers()
            .get("set-cookie")
            .ok_or(Error::Unavailable)?
            .clone(),
    );
    Ok(response)
}
async fn action(
    State(app): State<App>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Path(action): Path<String>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Result<Response> {
    let time = now()?;
    app.control.throttle(&source.ip().to_string(), time)?;
    let c = &app.control;
    let result = match action.as_str() {
        "oauth-start" => {
            let browser = random(32)?;
            let value = c.oauth_start(
                app.provider.as_ref(),
                input["invite"].as_str().unwrap_or_default(),
                &browser,
                time,
            )?;
            let mut response = Json(value).into_response();
            response.headers_mut().insert(
                "set-cookie",
                HeaderValue::from_str(&format!(
                    "__Host-ab-oauth={browser}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=300"
                ))
                .map_err(|_| Error::Invalid)?,
            );
            return Ok(response);
        }
        "device-start" => c.start_grant(
            text(&input, "public_key")?,
            input["slot"]
                .as_u64()
                .and_then(|v| v.try_into().ok())
                .ok_or(Error::Invalid)?,
            time,
        )?,
        "device-poll" => c.poll(text(&input, "polling_secret")?, time)?,
        "device-complete" => c.complete(
            text(&input, "polling_secret")?,
            text(&input, "signature")?,
            time,
        )?,
        "passkey-login-start" => c.passkey_start(text(&input, "account")?, false, time)?,
        "passkey-login-finish" => {
            return session_response(c.passkey_finish(None, false, &input, time)?);
        }
        "recover" => {
            return session_response(c.recover(
                text(&input, "account")?,
                text(&input, "code")?,
                time,
            )?);
        }
        _ => authenticated(c, &action, &headers, &input, time)?,
    };
    Ok(Json(result).into_response())
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Invalid => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Capacity | Self::SlowDown => StatusCode::TOO_MANY_REQUESTS,
            Self::Pending => StatusCode::ACCEPTED,
            Self::Expired => StatusCode::GONE,
            Self::Unavailable | Self::Fenced => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, Json(json!({"error":self.to_string()}))).into_response()
    }
}
async fn ui() -> Html<&'static str> {
    Html(include_str!("ui.html"))
}

async fn script() -> impl IntoResponse {
    (
        [("content-type", "application/javascript")],
        include_str!("ui.js"),
    )
}

fn authenticated(
    c: &Control,
    action: &str,
    headers: &HeaderMap,
    input: &Value,
    time: i64,
) -> Result<Value> {
    let secret = cookie(headers, "__Host-ab-control").ok_or(Error::Unauthorized)?;
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .ok_or(Error::Forbidden)?;
    let account = c.session(secret, Some(csrf), action != "list", time)?;
    Ok(match action {
        "list" => c.list(&account)?,
        "delete-account" => c.delete_account(&account)?,
        "confirm-view" => c.confirm_view(text(input, "code")?, time)?,
        "confirm" => c.confirm(
            &account,
            text(input, "code")?,
            text(input, "fingerprint")?,
            time,
        )?,
        "passkey-register-start" => c.passkey_start(&account, true, time)?,
        "passkey-register-finish" => c.passkey_finish(Some(&account), true, input, time)?,
        "rotate-start" => {
            let nonce = c.challenge(
                "rotation",
                &json!({"account":account,"device":text(input,"device")?}),
                time,
            )?;
            json!({"nonce":nonce})
        }
        "suspend" | "resume" | "revoke" | "rotate" => {
            let device = text(input, "device")?;
            let nonce = input["nonce"].as_str().unwrap_or_default();
            if action == "rotate" {
                let challenge = c.take_challenge(nonce, "rotation", time)?;
                if challenge["account"] != account || challenge["device"] != device {
                    return Err(Error::Forbidden);
                }
            }
            c.change(
                &account,
                device,
                action,
                input["public_key"].as_str().unwrap_or_default(),
                input["signature"].as_str().unwrap_or_default(),
                nonce,
            )?
        }
        _ => Err(Error::Invalid)?,
    })
}
