use crate::web_pages;
use ab_trust::{Authority, Principal, devices::Session, oauth, passkeys};
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
#[derive(Clone, Copy, Debug)]
pub struct SecureTransport;
pub trait Approvals: std::fmt::Debug + Send + Sync {
    fn pending(&self) -> Result<Vec<Value>, ab_trust::Error>;
    fn command(&self, id: &str) -> Result<Value, ab_trust::Error>;
    fn attachment(&self, id: &str, index: usize) -> Result<AttachmentFile, ab_trust::Error>;
    fn decide(
        &self,
        owner: &Principal,
        id: &str,
        approve: bool,
        reviewed: &Reviewed<'_>,
    ) -> Result<Value, ab_trust::Error>;
}
#[derive(Debug, Clone)]
pub struct AttachmentFile {
    pub media_type: String,
    pub preview: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Copy)]
pub struct Reviewed<'a> {
    pub binding: &'a str,
    pub accepted_unpreviewable: bool,
}
#[derive(Debug)]
struct Ceremony {
    state: Value,
    origin: String,
    flow: String,
    kind: String,
    enroll: String,
    principal: Option<Principal>,
    expires: Instant,
}
#[derive(Debug, Default)]
struct Rate {
    minute: Option<Instant>,
    day: Option<Instant>,
    second: Option<Instant>,
    minutes: u32,
    days: u32,
    seconds: u32,
    failures: u32,
    failure_at: Option<Instant>,
    retry_at: Option<Instant>,
}
const UNKNOWN_SOURCE: &str = "unknown";
const UNKNOWN_SOURCE_SUFFIX: &str = ":unknown";
const UNKNOWN_SOURCE_PER_MINUTE: u32 = 60;
const OWNER_RECOVERY_PER_SECOND: u32 = 2;
#[derive(Debug, Default)]
pub(crate) struct Memory {
    ceremonies: HashMap<String, Ceremony>,
    rates: HashMap<String, Rate>,
    global: Rate,
    cleanup: Option<Instant>,
}
#[derive(Debug)]
pub struct Web {
    pub authority: Arc<Authority>,
    pub address: std::net::SocketAddr,
    pub public_host: String,
    pub approvals: Option<Arc<dyn Approvals>>,
    pub(crate) memory: Mutex<Memory>,
    pub(crate) metadata: crate::web_metadata::Fetcher,
    pub(crate) connectors: Arc<crate::web_connector::Connectors>,
}
impl Web {
    pub fn new(
        authority: Arc<Authority>,
        address: std::net::SocketAddr,
        public_host: String,
        approvals: Option<Arc<dyn Approvals>>,
        connectors: Arc<crate::web_connector::Connectors>,
    ) -> Arc<Self> {
        Arc::new(Self {
            authority,
            address,
            public_host,
            approvals,
            memory: Mutex::default(),
            metadata: crate::web_metadata::Fetcher::default(),
            connectors,
        })
    }
    pub fn router(self: Arc<Self>) -> Router {
        Router::new().fallback(any(handle)).with_state(self)
    }
    pub(crate) fn allow(&self, key: &str, auth: bool, limit: u32) -> bool {
        self.allow_at(key, auth, limit, Instant::now())
    }
    fn allow_at(&self, key: &str, auth: bool, limit: u32, now: Instant) -> bool {
        let Ok(mut m) = self.memory.lock() else {
            return false;
        };
        let cap = if auth { 4096 } else { 8192 };
        if !m.rates.contains_key(key) && m.rates.len() >= cap {
            if m.cleanup
                .is_none_or(|t| now.duration_since(t) >= Duration::from_mins(1))
            {
                m.cleanup = Some(now);
                m.rates.retain(|_, r| {
                    r.day
                        .is_some_and(|t| now.duration_since(t) < Duration::from_hours(24))
                });
            }
            if m.rates.len() >= cap {
                if !auth {
                    return false;
                }
                let oldest = m
                    .rates
                    .iter()
                    .filter(|(id, _)| id.starts_with("auth:"))
                    .min_by_key(|(_, r)| r.day)
                    .map(|(id, _)| id.clone())
                    .or_else(|| m.rates.keys().next().cloned());
                if let Some(id) = oldest {
                    m.rates.remove(&id);
                }
            }
        }
        if auth {
            if m.global
                .minute
                .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(60))
            {
                m.global = Rate {
                    minute: Some(now),
                    ..Rate::default()
                };
            }
            m.global.minutes += 1;
            if m.global.minutes > 600 {
                return false;
            }
        }
        let r = m.rates.entry(key.into()).or_default();
        if r.minute
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(60))
        {
            r.minute = Some(now);
            r.minutes = 0;
        }
        if r.day
            .is_none_or(|t| now.duration_since(t) >= Duration::from_hours(24))
        {
            r.day = Some(now);
            r.days = 0;
        }
        if r.second
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
        {
            r.second = Some(now);
            r.seconds = 0;
        }
        if auth
            && r.failure_at
                .is_none_or(|t| now.duration_since(t) >= Duration::from_mins(15))
        {
            r.failures = 0;
            r.retry_at = None;
        }
        r.minutes += 1;
        r.days += 1;
        r.seconds += 1;
        if auth {
            if key.ends_with(UNKNOWN_SOURCE_SUFFIX) {
                r.minutes <= UNKNOWN_SOURCE_PER_MINUTE
            } else {
                r.minutes <= 10 && r.days <= 100 && r.retry_at.is_none_or(|t| now >= t)
            }
        } else {
            r.seconds <= limit
        }
    }
    fn owner_recovery(&self, path: &str, body: &[u8]) -> bool {
        if !path.starts_with("/auth/enroll/")
            || !self.allow("owner-recovery", false, OWNER_RECOVERY_PER_SECOND)
        {
            return false;
        }
        let Ok(input) = serde_json::from_slice::<Value>(body) else {
            return false;
        };
        if let Some(token) = input["token"].as_str() {
            return self.authority.check_web_enroll(token).is_ok();
        }
        input["ceremony_id"].as_str().is_some_and(|id| {
            self.memory
                .lock()
                .is_ok_and(|m| m.ceremonies.get(id).is_some_and(|c| c.flow == "enroll"))
        })
    }
    fn failure(&self, ip: &str) -> Response {
        if ip != UNKNOWN_SOURCE
            && let Ok(mut m) = self.memory.lock()
        {
            let r = m.rates.entry(format!("auth:{ip}")).or_default();
            let now = Instant::now();
            if r.failure_at
                .is_none_or(|t| now.duration_since(t) >= Duration::from_mins(15))
            {
                r.failures = 0;
            }
            r.failures += 1;
            r.failure_at = Some(now);
            if r.failures >= 10 {
                r.retry_at = Some(now + Duration::from_secs(u64::from((r.failures - 9).min(60))));
            }
        }
        reject(401, "unauthorized")
    }
}
pub(crate) fn reject(status: u16, code: &str) -> Response {
    respond(status, &json!({"error":{"code":code,"message":code}}))
}
pub(crate) fn respond(status: u16, value: &impl serde::Serialize) -> Response {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let raw = String::from_utf8_lossy(&bytes)
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        + "\n";
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [(header::CONTENT_TYPE, "application/json")],
        raw,
    )
        .into_response()
}
pub(crate) fn fields(raw: &str) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(raw.as_bytes()) {
        m.entry(k.into()).or_default().push(v.into());
    }
    m
}
pub(crate) fn field<'a>(m: &'a HashMap<String, Vec<String>>, k: &str) -> &'a str {
    m.get(k).and_then(|v| v.first()).map_or("", String::as_str)
}
async fn handle(State(web): State<Arc<Web>>, request: Request) -> Response {
    let (mut parts, body) = request.into_parts();
    let no_store = parts.uri.path().starts_with("/oauth/")
        || parts.uri.path().starts_with("/auth/")
        || parts.uri.path().starts_with("/approvals");
    let auth_rate = parts.uri.path().starts_with("/auth/login/")
        || parts.uri.path().starts_with("/auth/enroll/");
    let head = parts.method == axum::http::Method::HEAD;
    if head {
        parts.method = axum::http::Method::GET;
    }
    let cap = if parts.uri.path() == "/oauth/register" || parts.uri.path() == "/oauth/token" {
        65536
    } else {
        1_048_576
    };
    let bytes = to_bytes(body, cap).await;
    let secure = parts.extensions.get::<SecureTransport>().is_some();
    let mut response = match bytes {
        Ok(bytes) => tokio::task::spawn_blocking(move || dispatch(&web, &parts, &bytes, secure))
            .await
            .unwrap_or_else(|_| reject(503, "unavailable")),
        Err(_) => reject(400, "invalid"),
    };
    if head {
        *response.body_mut() = axum::body::Body::empty();
    }
    if no_store {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
    }
    if response.status() == 429 {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            axum::http::HeaderValue::from_static(if auth_rate { "60" } else { "1" }),
        );
    }
    if !response
        .headers()
        .contains_key(header::CONTENT_SECURITY_POLICY)
    {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(CSP),
        );
    }
    for (k, v) in [
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        (
            "permissions-policy",
            "camera=(self), microphone=(self), geolocation=(self)",
        ),
    ] {
        response
            .headers_mut()
            .insert(k, axum::http::HeaderValue::from_static(v));
    }
    if secure {
        response.headers_mut().insert(
            "strict-transport-security",
            axum::http::HeaderValue::from_static("max-age=31536000"),
        );
    }
    response
}
#[allow(clippy::too_many_lines)]
fn dispatch(w: &Web, r: &axum::http::request::Parts, body: &[u8], secure: bool) -> Response {
    let path = r.uri.path();
    let method = r.method.as_str();
    if path.starts_with("/web/v1/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let host = r
        .headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if host != format!("localhost:{}", w.address.port())
        && host != format!("127.0.0.1:{}", w.address.port())
        && !(secure && !w.public_host.is_empty() && host == w.public_host)
    {
        return reject(403, "host");
    }
    let origin = format!("{}://{host}", if secure { "https" } else { "http" });
    let public = path == "/oauth/register" || path == "/oauth/token";
    let origins: Vec<_> = r.headers.get_all(header::ORIGIN).iter().collect();
    if (public && !origins.is_empty() || !public && method != "GET" && method != "HEAD")
        && (origins.len() != 1 || origins[0].to_str().ok() != Some(&origin))
    {
        return reject(403, "origin");
    }
    let ip = r
        .extensions
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map_or_else(
            || UNKNOWN_SOURCE.into(),
            |v| match v.0.ip().to_canonical() {
                std::net::IpAddr::V4(ip) => ip.to_string(),
                std::net::IpAddr::V6(ip) => {
                    let mut s = ip.segments();
                    s[4..].fill(0);
                    format!("{}/64", std::net::Ipv6Addr::from(s))
                }
            },
        );
    let cookie_name = if secure {
        "__Host-ab_session"
    } else {
        "ab_session_local"
    };
    let cookie = r
        .headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| {
            cookie::Cookie::split_parse(raw)
                .flatten()
                .find(|c| c.name() == cookie_name)
                .map(|c| c.value().to_owned())
        })
        .unwrap_or_default();
    let cookie = cookie.as_str();
    let protected =
        path.starts_with("/auth/") || path.starts_with("/approvals") || path == "/oauth/authorize";
    let anonymous = method == "GET" && path == "/oauth/authorize"
        || path.starts_with("/auth/login/")
        || path.starts_with("/auth/enroll/");
    let mut identity = None;
    if public && !w.allow(&format!("oauth:{ip}"), false, 50) {
        return reject(429, "rate_limited");
    }
    if protected {
        if anonymous {
            if path == "/oauth/authorize" {
                if !w.allow(&format!("connector-authorize:{ip}"), false, 10) {
                    return reject(429, "Too many requests. Wait a moment and try again.");
                }
                identity = w.authority.authenticate_device(cookie).ok();
            } else if !w.allow(&format!("auth:{ip}"), true, 50) && !w.owner_recovery(path, body) {
                return reject(429, "rate_limited");
            }
        } else {
            identity = w.authority.authenticate_device(cookie).ok();
            if identity.is_none() {
                if method == "GET" && path == "/approvals" && cookie.is_empty() {
                    return web_pages::sign_in_for_approvals();
                }
                return reject(401, "unauthorized");
            }
            if !identity
                .as_ref()
                .is_some_and(|(p, _)| w.allow(&p.id, false, 50))
            {
                return reject(429, "rate_limited");
            }
        }
    }
    let q = fields(r.uri.query().unwrap_or_default());
    let mut response = route(w, r, body, &origin, &ip, &q, identity.as_ref());
    if path.starts_with("/oauth/") || protected {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-store"),
        );
    }
    if let Some((_, s)) = &identity
        && s.kind == "trusted"
    {
        set_cookie(&mut response, cookie_name, cookie, s);
    }
    response
}
fn set_cookie(response: &mut Response, name: &str, token: &str, s: &Session) {
    let mut built = cookie::Cookie::build((name.to_owned(), token.to_owned()))
        .path("/")
        .http_only(true)
        .same_site(cookie::SameSite::Strict)
        .secure(name.starts_with("__Host-"));
    if s.kind == "trusted"
        && let Ok(expires) = s.expires_at.parse::<jiff::Timestamp>()
    {
        built = built.max_age(cookie::time::Duration::seconds(
            (expires.as_second() - jiff::Timestamp::now().as_second()).max(1),
        ));
        if let Ok(at) = cookie::time::OffsetDateTime::from_unix_timestamp(expires.as_second()) {
            built = built.expires(at);
        }
    }
    if let Ok(value) = built.build().to_string().parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}
#[allow(clippy::many_single_char_names)]
fn route(
    w: &Web,
    r: &axum::http::request::Parts,
    body: &[u8],
    origin: &str,
    ip: &str,
    q: &HashMap<String, Vec<String>>,
    identity: Option<&(Principal, Session)>,
) -> Response {
    let path = r.uri.path();
    let method = r.method.as_str();
    match (method, path) {
        ("GET", "/") => web_pages::home(),
        ("GET", "/enroll") => web_pages::enroll(),
        ("GET", "/webauthn.js") => script(include_str!("../assets/webauthn.js")),
        ("GET", "/assets/connector.js") => script(include_str!("../assets/connector.js")),
        ("GET", ab_pages::STYLESHEET_PATH) => {
            ([(header::CONTENT_TYPE, "text/css")], ab_pages::STYLESHEET).into_response()
        }
        (
            "GET",
            "/.well-known/oauth-protected-resource" | "/.well-known/oauth-protected-resource/mcp",
        ) => respond(200, &crate::web_oauth::protected_resource(origin)),
        ("GET", "/.well-known/oauth-authorization-server") => {
            respond(200, &crate::web_oauth::authorization_server(origin))
        }
        ("POST", "/oauth/register") => crate::web_oauth::register(w, body),
        ("POST", "/oauth/token") => crate::web_oauth::token(w, body, origin),
        ("POST", "/oauth/pending") => crate::web_oauth::pending_request(w, origin, ip, q),
        ("GET", _) if path.starts_with("/oauth/pending/") => {
            crate::web_oauth::pending_poll(w, path, origin, ip)
        }
        ("GET" | "POST", "/oauth/authorize") => {
            crate::web_oauth::authorize(w, r, body, origin, q, identity)
        }
        ("GET", "/auth/session") => {
            identity.map_or_else(|| reject(401, "unauthorized"), |(_, s)| respond(200, s))
        }
        ("POST", _) if path.starts_with("/auth/") => auth(w, path, body, origin, ip, identity),
        ("GET" | "POST", _) if path.starts_with("/approvals") => {
            approval(w, path, method, body, identity)
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
fn script(source: &'static str) -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        source,
    )
        .into_response()
}
#[allow(clippy::too_many_lines)]
fn auth(
    w: &Web,
    path: &str,
    body: &[u8],
    origin: &str,
    ip: &str,
    identity: Option<&(Principal, Session)>,
) -> Response {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 4 || !["enroll", "login", "passkeys"].contains(&parts[2]) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let flow = parts[2];
    let Ok(input) = serde_json::from_slice::<Value>(body) else {
        return reject(400, "invalid");
    };
    let Ok(url) = url::Url::parse(origin) else {
        return reject(400, "invalid");
    };
    let rp = url.host_str().unwrap_or_default();
    if parts[3] == "options" {
        let kind = input["kind"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("trusted");
        if !["trusted", "public"].contains(&kind) {
            return reject(400, "invalid");
        }
        let enroll = input["token"].as_str().unwrap_or_default();
        if flow == "enroll" && w.authority.check_web_enroll(enroll).is_err() {
            return w.failure(ip);
        }
        let Ok(keys) = w.authority.passkeys(rp) else {
            return reject(503, "unavailable");
        };
        let Ok((mut options, state)) = passkeys::begin(rp, origin, flow == "login", &keys) else {
            return reject(400, "invalid");
        };
        let Ok(mut m) = w.memory.lock() else {
            return reject(503, "unavailable");
        };
        m.ceremonies.retain(|_, c| c.expires > Instant::now());
        if m.ceremonies.len() >= 1024 && flow != "enroll" {
            return reject(429, "rate_limited");
        }
        let id = format!("ceremony_{}", oauth::token(24));
        m.ceremonies.insert(
            id.clone(),
            Ceremony {
                state,
                origin: origin.into(),
                flow: flow.into(),
                kind: kind.into(),
                enroll: enroll.into(),
                principal: identity.map(|(p, _)| p.clone()),
                expires: Instant::now() + Duration::from_secs(300),
            },
        );
        options["ceremony_id"] = id.into();
        return respond(200, &options);
    }
    if parts[3] != "finish" || input["credential"].is_null() {
        return reject(400, "invalid");
    }
    let ceremony = w.memory.lock().ok().and_then(|mut m| {
        m.ceremonies
            .remove(input["ceremony_id"].as_str().unwrap_or_default())
    });
    let Some(c) = ceremony else {
        return w.failure(ip);
    };
    if c.origin != origin
        || c.flow != flow
        || c.expires <= Instant::now()
        || flow == "passkeys" && c.principal.as_ref().map(|p| &p.id) != identity.map(|(p, _)| &p.id)
    {
        return w.failure(ip);
    }
    let mut id = String::new();
    let result = if flow == "login" {
        w.authority
            .finish_passkey_login(rp, origin, c.state, input["credential"].clone())
    } else {
        passkeys::finish_registration(rp, origin, c.state, &input["credential"]).and_then(|key| {
            id.clone_from(&key.id);
            w.authority
                .save_passkey(key, &c.enroll, c.principal.as_ref())
        })
    };
    if result.is_err() {
        return w.failure(ip);
    }
    if let Ok(mut m) = w.memory.lock()
        && let Some(r) = m.rates.get_mut(&format!("auth:{ip}"))
    {
        r.failures = 0;
        r.failure_at = None;
        r.retry_at = None;
    }
    if flow == "passkeys" {
        return respond(200, &json!({"id":id}));
    }
    let Ok((token, session)) = w.authority.create_device_session(&c.kind) else {
        return reject(503, "unavailable");
    };
    let mut response = respond(200, &session);
    set_cookie(
        &mut response,
        if origin.starts_with("https:") {
            "__Host-ab_session"
        } else {
            "ab_session_local"
        },
        &token,
        &session,
    );
    response
}
fn approval(
    w: &Web,
    path: &str,
    method: &str,
    body: &[u8],
    identity: Option<&(Principal, Session)>,
) -> Response {
    let Some(a) = &w.approvals else {
        return reject(503, "unavailable");
    };
    if path == "/approvals" && method == "GET" {
        let Ok(rows) = a.pending() else {
            return reject(503, "unavailable");
        };
        return web_pages::approvals(&rows);
    }
    let Some(id) = path.strip_prefix("/approvals/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some((id, index)) = id.split_once("/attachments/") {
        return match (method, index.parse::<usize>()) {
            ("GET", Ok(index)) => a.attachment(id, index).map_or_else(
                |_| StatusCode::NOT_FOUND.into_response(),
                |file| web_pages::attachment_file(index, file),
            ),
            _ => StatusCode::NOT_FOUND.into_response(),
        };
    }
    let record = if method == "GET" {
        a.command(id)
    } else {
        let f = fields(&String::from_utf8_lossy(body));
        let decision = field(&f, "decision");
        if !["approve", "reject"].contains(&decision) {
            return reject(400, "invalid");
        }
        let Some((p, _)) = identity else {
            return reject(401, "unauthorized");
        };
        a.decide(
            p,
            id,
            decision == "approve",
            &Reviewed {
                binding: field(&f, "attachments"),
                accepted_unpreviewable: field(&f, "accept_unpreviewable") == "1",
            },
        )
    };
    record.map_or_else(
        |_| {
            if method == "GET" {
                reject(404, "not_found")
            } else {
                reject(409, "conflict")
            }
        },
        |record| web_pages::command_status(&record),
    )
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn web() -> (tempfile::TempDir, Arc<Web>) {
        let home = tempfile::tempdir().unwrap();
        let authority = Arc::new(Authority::open(home.path()).unwrap());
        let connectors = crate::web_connector::Connectors::new(authority.clone());
        let address = "127.0.0.1:7780".parse().unwrap();
        (home, Web::new(authority, address, String::new(), None, connectors))
    }

    #[test]
    fn one_source_can_spend_its_daily_budget_without_touching_another() {
        let (_home, web) = web();
        let start = Instant::now();
        let mut refused_at = None;
        'minutes: for minute in 0..20 {
            for _ in 0..10 {
                let now = start + Duration::from_secs(minute * 61);
                if !web.allow_at("auth:203.0.113.1", true, 50, now) {
                    refused_at = Some(minute);
                    break 'minutes;
                }
            }
        }
        assert!(refused_at.is_some(), "the daily budget never ran out");
        assert!(web.allow_at(
            "auth:203.0.113.2",
            true,
            50,
            start + Duration::from_secs(1300)
        ));
    }

    #[test]
    fn clients_with_no_known_source_are_never_locked_out_for_the_day() {
        let (_home, web) = web();
        let start = Instant::now();
        for minute in 0..30 {
            let now = start + Duration::from_secs(minute * 61);
            for _ in 0..UNKNOWN_SOURCE_PER_MINUTE {
                assert!(
                    web.allow_at("auth:unknown", true, 50, now),
                    "an anonymous caller exhausted the shared bucket in minute {minute}"
                );
            }
            assert!(!web.allow_at("auth:unknown", true, 50, now));
        }
    }

    #[test]
    fn failures_from_an_unknown_source_do_not_lock_everyone_out() {
        let (_home, web) = web();
        for _ in 0..30 {
            let _ = web.failure(UNKNOWN_SOURCE);
        }
        assert!(web.allow("auth:unknown", true, 50));
    }
}
