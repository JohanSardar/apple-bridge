use ab_trust::{
    Authority, Grant, Principal,
    devices::Session,
    oauth::{self, Client, Code, Exchange},
    passkeys,
};
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::fmt::Write;
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
    fn decide(&self, owner: &Principal, id: &str, approve: bool) -> Result<Value, ab_trust::Error>;
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
#[derive(Debug, Default)]
pub(crate) struct Memory {
    pub(crate) connectors: HashMap<String, crate::web_connector::Pending>,
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
    metadata: crate::web_metadata::Fetcher,
}
impl Web {
    pub fn new(
        authority: Arc<Authority>,
        address: std::net::SocketAddr,
        public_host: String,
        approvals: Option<Arc<dyn Approvals>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            authority,
            address,
            public_host,
            approvals,
            memory: Mutex::default(),
            metadata: crate::web_metadata::Fetcher::default(),
        })
    }
    pub fn router(self: Arc<Self>) -> Router {
        Router::new().fallback(any(handle)).with_state(self)
    }
    fn allow(&self, key: &str, auth: bool, limit: u32) -> bool {
        let Ok(mut m) = self.memory.lock() else {
            return false;
        };
        let now = Instant::now();
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
            r.minutes <= 10 && r.days <= 100 && r.retry_at.is_none_or(|t| now >= t)
        } else {
            r.seconds <= limit
        }
    }
    fn failure(&self, ip: &str) -> Response {
        if let Ok(mut m) = self.memory.lock() {
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
fn reject(status: u16, code: &str) -> Response {
    respond(status, &json!({"error":{"code":code,"message":code}}))
}
fn respond(status: u16, value: &impl serde::Serialize) -> Response {
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
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
        .replace('\'', "&#39;")
        .replace('+', "&#43;")
}
fn page(title: &str, body: &str) -> Response {
    ([(header::CONTENT_TYPE,"text/html; charset=utf-8"),(header::CACHE_CONTROL,"no-store")],format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Apple Bridge</title><link rel=\"stylesheet\" href=\"/consent.css\"><script src=\"/webauthn.js\" defer></script></head><body><main><nav><a href=\"/\">Apple Bridge</a> · <a href=\"/approvals\">Approvals</a></nav><h1>{}</h1>{body}</main></body></html>",escape(title))).into_response()
}
const SIGNIN: &str =
    "<button id=\"signin\">Sign in with passkey</button><p id=\"message\" role=\"status\"></p>";
pub(crate) fn fields(raw: &str) -> HashMap<String, Vec<String>> {
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(raw.as_bytes()) {
        m.entry(k.into()).or_default().push(v.into());
    }
    m
}
fn field<'a>(m: &'a HashMap<String, Vec<String>>, k: &str) -> &'a str {
    m.get(k).and_then(|v| v.first()).map_or("", String::as_str)
}
fn scopes(q: &HashMap<String, Vec<String>>) -> Vec<Grant> {
    let raw = field(q, "scope");
    if raw.trim().is_empty() {
        return Grant::parse("messages:read").into_iter().collect();
    }
    raw.split_whitespace()
        .filter_map(|s| Grant::parse(s).ok())
        .filter(|g| oauth::agent_grants("approve", std::slice::from_ref(g)).is_ok())
        .collect()
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
    for (k, v) in [
        ("content-security-policy", CSP),
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
            || "unknown".into(),
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
        .unwrap_or_default()
        .split(';')
        .filter_map(|v| v.trim().split_once('='))
        .find(|(k, _)| *k == cookie_name)
        .map_or("", |(_, v)| v);
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
            } else if !w.allow(&format!("auth:{ip}"), true, 50) {
                return reject(429, "rate_limited");
            }
        } else {
            identity = w.authority.authenticate_device(cookie).ok();
            if identity.is_none() {
                if method == "GET" && path == "/approvals" && cookie.is_empty() {
                    return page("Sign in for approvals", SIGNIN);
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
    let mut value = format!("{name}={token}; Path=/; HttpOnly; SameSite=Strict");
    if name.starts_with("__Host-") {
        value.push_str("; Secure");
    }
    if s.kind == "trusted" {
        let age = s.expires_at.parse::<jiff::Timestamp>().map_or(1, |t| {
            (t.as_second() - jiff::Timestamp::now().as_second()).max(1)
        });
        let _ = write!(value, "; Max-Age={age}");
        if let Ok(expires) = s.expires_at.parse::<jiff::Timestamp>()
            && let Ok(seconds) = u64::try_from(expires.as_second())
        {
            let _ = write!(
                value,
                "; Expires={}",
                httpdate::fmt_http_date(std::time::UNIX_EPOCH + Duration::from_secs(seconds))
            );
        }
    }
    if let Ok(value) = value.parse() {
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
    match (method,path) {
        ("GET","/")=>page("Apple Bridge",&format!("{SIGNIN}<p>Consent and pending agent approvals. Manage agents with <code>bridge agent create</code>, <code>bridge agent list</code> and <code>bridge agent revoke</code>.</p>")),
        ("GET","/enroll")=>page("Enroll first passkey",SIGNIN),
        ("GET","/webauthn.js")=>([(header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("../assets/webauthn.js")).into_response(),
        ("GET","/consent.css")=>([(header::CONTENT_TYPE,"text/css")],"body{font:18px system-ui;line-height:1.6;margin:2rem auto;padding:0 1rem;max-width:48rem;color:#222;background:#fafafa}label{display:block}button,select{font:inherit;margin:.5rem;padding:.4rem}article,fieldset{border:1px solid #aaa;padding:1rem;margin:1rem 0}pre{white-space:pre-wrap;overflow-wrap:anywhere}a{color:#174b91}").into_response(),
        ("GET","/.well-known/oauth-protected-resource"|"/.well-known/oauth-protected-resource/mcp")=>respond(200,&json!({"resource":format!("{origin}/mcp"),"authorization_servers":[origin],"scopes_supported":all_scopes(),"bearer_methods_supported":["header"]})),
        ("GET","/.well-known/oauth-authorization-server")=>respond(200,&json!({"issuer":origin,"authorization_endpoint":format!("{origin}/oauth/authorize"),"token_endpoint":format!("{origin}/oauth/token"),"registration_endpoint":format!("{origin}/oauth/register"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"],"scopes_supported":all_scopes(),"client_id_metadata_document_supported":true,"authorization_response_iss_parameter_supported":true})),
        ("POST","/oauth/register")=>serde_json::from_slice::<Client>(body).ok().and_then(|c|w.authority.register_oauth(c).ok()).map_or_else(||respond(400,&json!({"error":"invalid_client_metadata"})),|c|respond(201,&c)),
        ("POST","/oauth/token")=>{
            let f=fields(&String::from_utf8_lossy(body));if f.is_empty() || f.values().any(|v|v.len()!=1){return respond(400,&json!({"error":"invalid_request"}));}
            if !["authorization_code","refresh_token"].contains(&field(&f,"grant_type")){return respond(400,&json!({"error":"unsupported_grant_type"}));}
            if field(&f,"resource")!=format!("{origin}/mcp"){return respond(400,&json!({"error":"invalid_target"}));}
            let e=Exchange {grant_type:field(&f,"grant_type").into(),client_id:field(&f,"client_id").into(),code:field(&f,"code").into(),redirect_uri:field(&f,"redirect_uri").into(),code_verifier:field(&f,"code_verifier").into(),resource:field(&f,"resource").into(),refresh_token:field(&f,"refresh_token").into(),scope:field(&f,"scope").into()};
            w.authority.exchange_oauth(&e).map_or_else(|_|respond(400,&json!({"error":"invalid_grant"})),|t|respond(200,&t))
        }
        ("POST","/oauth/pending")=>pending_request(w,r,body,origin,ip,q),
        ("GET",_) if path.starts_with("/oauth/pending/")=>{
            if !w.allow(&format!("connector-poll:{ip}"),false,10){return reject(429,"Wait a moment and try again.");}
            w.poll_connector(path.trim_start_matches("/oauth/pending/"),origin).map_or_else(|_|reject(410,"This approval has expired or was already used. Start again from your connector."),|redirect|respond(200,&json!({"redirect":redirect})))
        },
        ("GET"|"POST","/oauth/authorize")=>authorize(w,r,body,origin,q,identity),
        ("GET","/auth/session")=>identity.map_or_else(||reject(401,"unauthorized"),|(_,s)|respond(200,s)),
        ("POST",_) if path.starts_with("/auth/")=>auth(w,path,body,origin,ip,identity),
        ("GET"|"POST",_) if path.starts_with("/approvals")=>approval(w,path,method,body,identity),
        _=>StatusCode::NOT_FOUND.into_response(),
    }
}
fn all_scopes() -> Vec<String> {
    ab_trust::Grant::agent_scopes()
}
#[allow(clippy::too_many_lines)]
fn authorize(
    w: &Web,
    r: &axum::http::request::Parts,
    body: &[u8],
    origin: &str,
    q: &HashMap<String, Vec<String>>,
    identity: Option<&(Principal, Session)>,
) -> Response {
    let invalid = || respond(400, &json!({"error":"invalid_request"}));
    if q.values().any(|v| v.len() != 1)
        || field(q, "response_type") != "code"
        || field(q, "resource") != format!("{origin}/mcp")
        || field(q, "state").len() > 2048
        || field(q, "code_challenge_method") != "S256"
        || !URL_SAFE_NO_PAD
            .decode(field(q, "code_challenge"))
            .is_ok_and(|b| b.len() == 32)
        || scopes(q).is_empty()
    {
        return invalid();
    }
    let client_result = if field(q, "client_id").starts_with("https://") {
        w.metadata
            .fetch(field(q, "client_id"), field(q, "redirect_uri"))
    } else {
        w.authority
            .oauth_client(field(q, "client_id"), field(q, "redirect_uri"))
    };
    let Ok(client) = client_result else {
        return invalid();
    };
    if r.method == "GET" {
        if field(q, "format") == "json" {
            let Ok(dest) = url::Url::parse(field(q, "redirect_uri")) else {
                return invalid();
            };
            return respond(
                200,
                &json!({"client_name":client.client_name,"redirect_host":dest.authority(),"scopes":scopes(q),"groups":groups()}),
            );
        }
        let mut body = format!(
            "<p>{} requests access to this bridge.</p>",
            escape(&client.client_name)
        );
        if identity.is_none() {
            body.push_str(SIGNIN);
        }
        body.push_str("<form method=\"post\"><label>Agent level <select name=\"level\"><option value=\"read\">Read</option><option value=\"approve\" selected>Approve every command</option><option value=\"full\">Full</option></select></label><fieldset><legend>Scopes</legend>");
        for g in scopes(q) {
            let _ = write!(
                body,
                "<label><input type=\"checkbox\" name=\"scopes\" value=\"{}\" checked>{}</label>",
                escape(g.as_str()),
                escape(g.as_str())
            );
        }
        body.push_str("</fieldset><button name=\"decision\" value=\"approve\">Allow connection</button><button name=\"decision\" value=\"reject\">Reject</button></form>");
        return page("Connect an agent", &body);
    }
    let form = r.headers.get(header::CONTENT_TYPE).is_some_and(|v| {
        v.as_bytes()
            .starts_with(b"application/x-www-form-urlencoded")
    });
    let input: Value = if form {
        let f = fields(&String::from_utf8_lossy(body));
        json!({"decision":field(&f,"decision"),"level":field(&f,"level"),"scopes":f.get("scopes").cloned().unwrap_or_default()})
    } else {
        let Ok(v) = serde_json::from_slice(body) else {
            return invalid();
        };
        v
    };
    let decision = input["decision"].as_str().unwrap_or_default();
    if !["approve", "reject"].contains(&decision) {
        return invalid();
    }
    let level = input["level"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or("approve");
    let grants = if decision == "approve" {
        let Ok(grants) = serde_json::from_value::<Vec<Grant>>(input["scopes"].clone()) else {
            return respond(400, &json!({"error":"invalid_scope"}));
        };
        match oauth::agent_grants(level, &grants) {
            Ok(grants) if ab_trust::subset(&grants, &scopes(q)) => grants,
            _ => return respond(400, &json!({"error":"invalid_scope"})),
        }
    } else {
        Vec::new()
    };
    let code = Code {
        client_id: client.client_id.clone(),
        redirect_uri: field(q, "redirect_uri").into(),
        challenge: field(q, "code_challenge").into(),
        resource: field(q, "resource").into(),
        ..Code::default()
    };
    let Ok(dest) = crate::web_connector::decision(
        w,
        &client,
        code,
        field(q, "state"),
        origin,
        decision == "approve",
        level,
        &grants,
    ) else {
        return reject(503, "unavailable");
    };
    if form {
        page(
            "Connection authorized",
            &format!(
                "<p><a id=\"connector-redirect\" href=\"{}\">Continue to connector</a></p>",
                escape(&dest)
            ),
        )
    } else {
        respond(200, &json!({"redirect":dest}))
    }
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
        if m.ceremonies.len() >= 1024 {
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
        let mut body = String::new();
        for row in &rows {
            let id = escape(row["command_id"].as_str().unwrap_or_default());
            let _ = write!(
                body,
                "<article><h2>{}</h2><p>{} · {id}</p><pre>{}</pre><details><summary>Owner arguments</summary><pre>{}</pre></details><form method=\"post\" action=\"/approvals/{id}\"><button name=\"decision\" value=\"approve\">Approve</button><button name=\"decision\" value=\"reject\">Reject</button></form></article>",
                escape(row["operation"].as_str().unwrap_or_default()),
                escape(row["principal"].as_str().unwrap_or_default()),
                escape(row["arguments"].as_str().unwrap_or_default()),
                escape(row["owner_arguments"].as_str().unwrap_or_default())
            );
        }
        if rows.is_empty() {
            body.push_str("<p>No pending approvals.</p>");
        }
        return page("Pending approvals", &body);
    }
    let Some(id) = path.strip_prefix("/approvals/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
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
        a.decide(p, id, decision == "approve")
    };
    record.map_or_else(
        |_| {
            reject(
                if method == "GET" { 404 } else { 409 },
                if method == "GET" {
                    "not_found"
                } else {
                    "conflict"
                },
            )
        },
        |record| {
            page(
                "Command status",
                &format!(
                    "<pre>{}</pre>",
                    escape(&serde_json::to_string_pretty(&record).unwrap_or_default())
                ),
            )
        },
    )
}

fn pending_request(
    web: &Web,
    request: &axum::http::request::Parts,
    _body: &[u8],
    origin: &str,
    ip: &str,
    query: &HashMap<String, Vec<String>>,
) -> Response {
    if !web.allow(&format!("connector-create:{ip}"), true, 50) {
        return reject(429, "Too many requests. Wait a minute and try again.");
    }
    let mut request = request.clone();
    request.method = axum::http::Method::GET;
    let validation = authorize(web, &request, &[], origin, query, None);
    if validation.status() != 200 {
        return reject(
            400,
            "This connection request is invalid. Start again from your connector.",
        );
    }
    let result = if field(query, "client_id").starts_with("https://") {
        web.metadata
            .fetch(field(query, "client_id"), field(query, "redirect_uri"))
    } else {
        web.authority
            .oauth_client(field(query, "client_id"), field(query, "redirect_uri"))
    };
    let Ok(client) = result else {
        return reject(
            400,
            "This connection request is invalid. Start again from your connector.",
        );
    };
    let code = Code {
        client_id: client.client_id.clone(),
        redirect_uri: field(query, "redirect_uri").into(),
        challenge: field(query, "code_challenge").into(),
        resource: field(query, "resource").into(),
        ..Code::default()
    };
    web.connector_begin(client, code, field(query, "state"), origin, scopes(query))
        .map_or_else(
            |_| reject(503, "Approval is unavailable. Try again in a few minutes."),
            |summary| respond(201, &summary),
        )
}

fn groups() -> Vec<Value> {
    let titles = [
        "Audio",
        "Calendar",
        "Contacts",
        "Files",
        "Focus",
        "Health",
        "iPhone",
        "Mac",
        "Mail",
        "Location",
        "Maps",
        "Messages",
        "Notes",
        "PDF",
        "Phone",
        "Capture",
        "Print",
        "Voice Memos",
        "Voicemail",
        "Reminders",
        "Safari",
        "Shortcuts",
        "Speech",
        "Usage",
        "Vision",
        #[cfg(feature = "lab-harness")]
        "Lab dummy",
    ];
    all_scopes().chunks(2).zip(titles).map(|(grants,title)|json!({"source":grants[0].trim_end_matches(":read"),"title":title,"grants":grants,"agent":true})).collect()
}
