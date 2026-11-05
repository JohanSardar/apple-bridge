use crate::{
    hello::valid_host,
    server::Server,
    store::{Enrollment, Limits, Proof, digest},
};
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, State, WebSocketUpgrade, ws::WebSocket},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

pub trait DnsProvider: std::fmt::Debug + Send + Sync {
    fn txt<'a>(
        &'a self,
        name: &'a str,
        value: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + 'a>>;
}
pub trait NameResolver: std::fmt::Debug + Send + Sync {
    fn cname<'a>(
        &'a self,
        name: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<String>> + Send + 'a>>;
}
#[derive(Debug)]
pub struct SystemResolver(hickory_resolver::TokioResolver);
impl NameResolver for SystemResolver {
    fn cname<'a>(
        &'a self,
        name: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<String>> + Send + 'a>>
    {
        Box::pin(async move {
            let answer = self
                .0
                .lookup(name, hickory_resolver::proto::rr::RecordType::CNAME)
                .await
                .map_err(|_| crate::Error::Denied)?;
            answer
                .answers()
                .iter()
                .find_map(|r| match &r.data {
                    hickory_resolver::proto::rr::RData::CNAME(target) => Some(target.to_string()),
                    _ => None,
                })
                .ok_or(crate::Error::Denied)
        })
    }
}
const SOURCE_BUDGETS: usize = 4096;
#[derive(Debug)]
struct Budget {
    until: tokio::time::Instant,
    challenges: usize,
    enrollments: usize,
    proofs: usize,
}
#[derive(Debug, Clone, Copy)]
enum Spend {
    Challenge,
    Enrollment,
    Proof,
}
impl Spend {
    fn of(path: &str) -> Option<Self> {
        match path {
            "/challenge" => Some(Self::Challenge),
            "/enroll" | "/rotate" => Some(Self::Enrollment),
            "/dns" | "/domain" | "/certificate" | "/tunnel" | "/ct" | "/revoke" | "/info" => {
                Some(Self::Proof)
            }
            _ => None,
        }
    }
}
#[derive(Debug)]
pub struct Api {
    pub server: Arc<Server>,
    pub admin: String,
    pub push: crate::push::Push,
    sources: Mutex<HashMap<IpAddr, Budget>>,
    pub dns: Option<Arc<dyn DnsProvider>>,
    pub resolver: Arc<dyn NameResolver>,
    pub ct_origin: String,
    pub warnings: Arc<Mutex<HashMap<String, String>>>,
    pub accounts_url: String,
}
impl Api {
    pub fn new(server: Arc<Server>, admin: String) -> crate::Result<Self> {
        Ok(Self {
            server,
            admin,
            push: crate::push::Push::default(),
            sources: Mutex::new(HashMap::new()),
            dns: None,
            resolver: Arc::new(SystemResolver(
                hickory_resolver::TokioResolver::builder_tokio()
                    .map_err(|_| crate::Error::Configuration)?
                    .build()
                    .map_err(|_| crate::Error::Configuration)?,
            )),
            ct_origin: "https://crt.sh/".into(),
            warnings: Arc::new(Mutex::new(HashMap::new())),
            accounts_url: String::new(),
        })
    }
    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .fallback(any(handle))
            .with_state(self)
            .layer(axum::extract::DefaultBodyLimit::max(4096))
    }
    async fn allowed(&self, ip: IpAddr, spend: Spend) -> bool {
        let now = tokio::time::Instant::now();
        let key = crate::server::source_key(ip);
        let mut sources = self.sources.lock().await;
        sources.retain(|_, b| now < b.until);
        if !sources.contains_key(&key)
            && sources.len() >= SOURCE_BUDGETS
            && let Some(oldest) = sources
                .iter()
                .min_by_key(|(_, budget)| budget.until)
                .map(|(source, _)| *source)
        {
            sources.remove(&oldest);
        }
        let b = sources.entry(key).or_insert(Budget {
            until: now + Duration::from_secs(60),
            challenges: 0,
            enrollments: 0,
            proofs: 0,
        });
        let (counter, max) = match spend {
            Spend::Challenge => (&mut b.challenges, 30),
            Spend::Enrollment => (&mut b.enrollments, 20),
            Spend::Proof => (&mut b.proofs, 60),
        };
        if *counter >= max {
            return false;
        }
        *counter += 1;
        true
    }
    fn nonce_is_live(&self, proof: &Proof) -> bool {
        self.server.nonces.contains(&proof.nonce)
    }
    async fn upgrade(self: Arc<Self>, ws: WebSocket, proof: Proof, features: crate::mux::Features) {
        let (tx, rx) = mpsc::channel(32);
        let cancel = CancellationToken::new();
        let Ok(session) = self
            .server
            .attach(&proof.hostname, &proof.credential, tx, cancel.clone())
            .await
        else {
            return;
        };
        crate::mux::run(ws, rx, cancel, features).await;
        let mut sessions = self.server.sessions.lock().await;
        if sessions
            .get(&proof.hostname)
            .is_some_and(|s| Arc::ptr_eq(s, &session))
        {
            sessions.remove(&proof.hostname);
        }
    }
}
pub(crate) fn error(status: u16, message: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [
            ("content-type", "text/plain; charset=utf-8"),
            ("x-content-type-options", "nosniff"),
        ],
        format!("{message}\n"),
    )
        .into_response()
}
fn reply<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_string(value) {
        Ok(data) => (
            [
                ("content-type", "application/json"),
                ("cache-control", "no-store"),
            ],
            format!("{data}\n"),
        )
            .into_response(),
        Err(_) => error(500, "encoding unavailable"),
    }
}
fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> Option<T> {
    if body.len() > 4096 {
        return None;
    }
    let mut decoder = serde_json::Deserializer::from_slice(body);
    T::deserialize(&mut decoder).ok()
}
fn header_proof(headers: &HeaderMap) -> Option<Proof> {
    let raw = B64
        .decode(headers.get("X-Bridge-Proof")?.to_str().ok()?)
        .ok()?;
    if raw.len() > 4096 {
        return None;
    }
    serde_json::from_slice(&raw).ok()
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Policy {
    hostname: String,
    limits: Limits,
    disabled: bool,
    revoked: bool,
    version: i64,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Host {
    hostname: String,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Dns {
    proof: Proof,
    value: String,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Domain {
    proof: Proof,
    domain: String,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Certificate {
    proof: Proof,
    fingerprint: String,
}
async fn handle(
    State(api): State<Arc<Api>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    method: axum::http::Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
    ws: std::result::Result<
        WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
    body: Bytes,
) -> Response {
    let path = uri.path();
    if let Some(spend) = Spend::of(path)
        && !api.allowed(peer.ip(), spend).await
    {
        let mut response = error(429, "busy");
        response
            .headers_mut()
            .insert("retry-after", axum::http::HeaderValue::from_static("60"));
        return response;
    }
    let token = headers
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = token.strip_prefix("Bearer ").unwrap_or(token);
    match (method.as_str(), path) {
        ("GET" | "HEAD", "/challenge") => match api.server.nonces.issue() {
            Ok(nonce) => reply(&json!({"nonce":nonce})),
            Err(_) => error(429, "busy"),
        },
        ("GET" | "HEAD", "/info") => {
            let zone = api.server.store.lock().await.zone.clone();
            reply(&json!({"zone": zone, "accounts": api.accounts_url}))
        }
        ("POST", "/admit") => admit(&api, &body, token).await,
        ("POST", "/enroll" | "/rotate") => enrollment(&api, &body, path).await,
        ("POST", "/iphone/push") => {
            let Some(request) = decode::<crate::push::Request>(&body) else {
                return error(400, "invalid request");
            };
            crate::push::handle(&api, request, token).await
        }
        ("POST", "/revoke") => revoke(&api, &body, token).await,
        ("POST", "/policy") => policy(&api, &body, token).await,
        ("GET", "/tunnel") => {
            let Some(proof) = header_proof(&headers) else {
                return error(403, "denied");
            };
            if !api.nonce_is_live(&proof) || !api.server.store.lock().await.verify(&proof, "tunnel")
            {
                return error(403, "denied");
            }
            let Ok(ws) = ws else {
                return error(400, "WebSocket protocol violation");
            };
            let features = crate::mux::Features {
                source: headers
                    .get(ab_wire::framing::TUNNEL_FEATURES_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| {
                        value.split(',').any(|feature| {
                            feature.trim() == ab_wire::framing::TUNNEL_FEATURE_SOURCE
                        })
                    }),
            };
            ws.max_message_size(4 << 20)
                .on_upgrade(move |socket| api.upgrade(socket, proof, features))
        }
        ("POST", "/dns") => dns(&api, &body).await,
        ("POST", "/domain") => domain(&api, &body).await,
        ("POST", "/certificate") => certificate(&api, &body).await,
        ("GET" | "HEAD", "/ct") => {
            let Some(proof) = header_proof(&headers) else {
                return error(403, "denied");
            };
            if !api.nonce_is_live(&proof) || !api.server.store.lock().await.verify(&proof, "ct") {
                return error(403, "denied");
            }
            reply(
                &json!({"warning":api.warnings.lock().await.get(&proof.hostname).cloned().unwrap_or_default()}),
            )
        }
        _ => error(404, "404 page not found"),
    }
}

async fn enrollment(api: &Api, body: &[u8], path: &str) -> Response {
    let Some(input) = decode::<Enrollment>(body) else {
        return error(400, "invalid request");
    };
    if !api.server.nonces.contains(&input.nonce) {
        return error(403, "enrollment refused");
    }
    let result = api.server.store.lock().await.enroll(input);
    match result {
        Ok(c) => {
            api.server.disconnect(&c.hostname).await;
            reply(&c)
        }
        Err(_) => error(
            403,
            if path == "/rotate" {
                "rotation refused"
            } else {
                "enrollment refused"
            },
        ),
    }
}

async fn revoke(api: &Api, body: &[u8], token: &str) -> Response {
    let Some(input) = decode::<Host>(body) else {
        return error(400, "invalid request");
    };
    {
        let mut store = api.server.store.lock().await;
        if !store.authenticated(&input.hostname, token) {
            return error(403, "denied");
        }
        let Some(d) = store.devices.get(&input.hostname).cloned() else {
            return error(403, "denied");
        };
        if store
            .policy(&input.hostname, d.limits, d.disabled, true)
            .is_err()
        {
            return error(503, "storage unavailable");
        }
    }
    api.server.disconnect(&input.hostname).await;
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Admit {
    public_key: String,
}
async fn admit(api: &Api, body: &[u8], token: &str) -> Response {
    if api.admin.is_empty() || digest(token) != digest(&api.admin) {
        return error(403, "denied");
    }
    let Some(input) = decode::<Admit>(body) else {
        return error(400, "invalid request");
    };
    let result = api.server.store.lock().await.admit(&input.public_key);
    match result {
        Ok(credential) => {
            api.server.disconnect(&credential.hostname).await;
            reply(&credential)
        }
        Err(_) => error(403, "admission refused"),
    }
}

async fn policy(api: &Api, body: &[u8], token: &str) -> Response {
    if api.admin.is_empty() || digest(token) != digest(&api.admin) {
        return error(403, "denied");
    }
    let Some(input) = decode::<Policy>(body) else {
        return error(400, "invalid request");
    };
    let applied = api.server.store.lock().await.versioned_policy(
        &input.hostname,
        input.limits,
        input.disabled,
        input.revoked,
        input.version,
    );
    match applied {
        Ok(true) => api.server.disconnect(&input.hostname).await,
        Ok(false) => {}
        Err(_) => return error(403, "policy refused"),
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn dns(api: &Api, body: &[u8]) -> Response {
    let Some(input) = decode::<Dns>(body) else {
        return error(400, "invalid request");
    };
    if input.value.len() > 128
        || !input
            .value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        || !api.nonce_is_live(&input.proof)
        || !api
            .server
            .store
            .lock()
            .await
            .verify(&input.proof, &format!("dns:{}", input.value))
    {
        return error(403, "denied");
    }
    let Some(dns) = &api.dns else {
        return error(503, "DNS unavailable");
    };
    match dns
        .txt(
            &format!("_acme-challenge.{}", input.proof.hostname),
            &input.value,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => error(503, "DNS unavailable"),
    }
}

async fn domain(api: &Api, body: &[u8]) -> Response {
    let Some(input) = decode::<Domain>(body) else {
        return error(400, "invalid request");
    };
    if !valid_host(&input.domain)
        || !api.nonce_is_live(&input.proof)
        || !api
            .server
            .store
            .lock()
            .await
            .verify(&input.proof, &format!("domain:{}", input.domain))
    {
        return error(403, "denied");
    }
    let mut missing = Vec::new();
    for (name, target) in [
        (input.domain.clone(), input.proof.hostname.clone()),
        (
            format!("_acme-challenge.{}", input.domain),
            format!("_acme-challenge.{}", input.proof.hostname),
        ),
    ] {
        let found = api.resolver.cname(&name).await.is_ok_and(|target_name| {
            target_name
                .trim_end_matches('.')
                .eq_ignore_ascii_case(&target)
        });
        if !found {
            missing.push(format!("{name} CNAME {target}."));
        }
    }
    if !missing.is_empty() {
        return reply(&json!({"missing":missing}));
    }
    if api
        .server
        .store
        .lock()
        .await
        .set_domain(&input.proof.hostname, &input.domain)
        .is_err()
    {
        return error(403, "domain refused");
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn certificate(api: &Api, body: &[u8]) -> Response {
    let Some(input) = decode::<Certificate>(body) else {
        return error(400, "invalid request");
    };
    if input.fingerprint.len() != 64
        || !input
            .fingerprint
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        || !api.nonce_is_live(&input.proof)
    {
        return error(403, "denied");
    }
    let mut store = api.server.store.lock().await;
    if !store.verify(&input.proof, &format!("certificate:{}", input.fingerprint)) {
        return error(403, "denied");
    }
    match store.certificate(&input.proof.hostname, &input.fingerprint) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(409, "certificate history full"),
        Err(_) => error(503, "storage unavailable"),
    }
}
