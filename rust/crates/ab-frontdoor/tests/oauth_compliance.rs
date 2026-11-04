#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
use ab_frontdoor::{web::Web, web_connector::Connectors};
use ab_trust::Authority;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;

const CLAUDE_CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";
const CHATGPT_CALLBACK: &str = "https://chatgpt.com/connector_platform_oauth_redirect";

struct Server {
    _home: tempfile::TempDir,
    router: Router,
    connectors: Arc<Connectors>,
    origin: String,
    host: String,
}
impl Server {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let authority = Arc::new(Authority::open(home.path()).unwrap());
        let address: SocketAddr = "127.0.0.1:7780".parse().unwrap();
        let connectors = Connectors::new(authority.clone());
        let web = Web::new(authority, address, String::new(), None, connectors.clone());
        Self {
            _home: home,
            router: web.router(),
            connectors,
            origin: "http://127.0.0.1:7780".into(),
            host: "127.0.0.1:7780".into(),
        }
    }
    async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router.clone().oneshot(request).await.unwrap()
    }
    fn request(&self, method: &str, path: &str) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, &self.host)
    }
    async fn get_json(&self, path: &str) -> Value {
        let response = self
            .send(self.request("GET", path).body(Body::empty()).unwrap())
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        body_json(response).await
    }
    async fn register(&self, metadata: &Value) -> (StatusCode, Value) {
        let response = self
            .send(
                self.request("POST", "/oauth/register")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(metadata.to_string()))
                    .unwrap(),
            )
            .await;
        let status = response.status();
        (status, body_json(response).await)
    }
    async fn token(&self, form: &[(&str, &str)]) -> (StatusCode, Value, Option<String>) {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let response = self
            .send(
                self.request("POST", "/oauth/token")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await;
        let status = response.status();
        let cache = response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_owned());
        (status, body_json(response).await, cache)
    }
    fn authorize_query(&self, client_id: &str, redirect: &str, challenge: &str) -> String {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("response_type", "code"),
                ("client_id", client_id),
                ("redirect_uri", redirect),
                ("scope", "calendar:read calendar:write offline_access"),
                ("state", "xyz"),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("resource", &format!("{}/mcp", self.origin)),
            ])
            .finish()
    }
    async fn approve(&self, query: &str) -> url::Url {
        let created = self
            .send(
                self.request("POST", &format!("/oauth/pending?{query}"))
                    .header(header::ORIGIN, &self.origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let summary = body_json(created).await;
        self.connectors
            .decide(
                summary["id"].as_str().unwrap(),
                summary["code"].as_str().unwrap(),
                true,
            )
            .unwrap();
        let polled = self
            .get_json(&format!(
                "/oauth/pending/{}",
                summary["id"].as_str().unwrap()
            ))
            .await;
        url::Url::parse(polled["redirect"].as_str().unwrap()).unwrap()
    }
}
async fn body_json(response: Response<Body>) -> Value {
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}
async fn body_text(response: Response<Body>) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}
fn pkce() -> (String, String) {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk-compliance".to_owned();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}
fn param(url: &url::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
async fn discovery_has_what_both_clients_read() {
    let s = Server::new();
    let origin = s.origin.clone();
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let prm = s.get_json(path).await;
        assert_eq!(prm["resource"], format!("{origin}/mcp"), "exact MCP URL");
        assert_eq!(prm["authorization_servers"][0], origin);
    }
    let meta = s.get_json("/.well-known/oauth-authorization-server").await;
    assert_eq!(meta["issuer"], origin, "RFC 8414 issuer equals PRM entry");
    let has = |field: &str, value: &str| {
        meta[field]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == value)
    };
    assert!(has("code_challenge_methods_supported", "S256"));
    assert!(has("token_endpoint_auth_methods_supported", "none"));
    assert!(
        has("scopes_supported", "offline_access"),
        "Claude asks for refresh tokens with it"
    );
    assert!(has("grant_types_supported", "refresh_token"));
    assert_eq!(meta["client_id_metadata_document_supported"], true);
    assert_eq!(meta["authorization_response_iss_parameter_supported"], true);
    assert!(meta["registration_endpoint"].is_string());
}

#[tokio::test]
async fn registration_takes_what_real_clients_send() {
    let s = Server::new();
    for metadata in [
        json!({"redirect_uris": [CLAUDE_CALLBACK], "client_name": "Claude", "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"], "token_endpoint_auth_method": "none"}),
        json!({"redirect_uris": [CHATGPT_CALLBACK]}),
        json!({"redirect_uris": [CHATGPT_CALLBACK], "client_name": "ChatGPT", "token_endpoint_auth_method": "client_secret_basic", "grant_types": ["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:jwt-bearer"], "scope": "calendar:read"}),
    ] {
        let (status, client) = s.register(&metadata).await;
        assert_eq!(status, StatusCode::CREATED, "{metadata}");
        assert_eq!(client["token_endpoint_auth_method"], "none");
        assert!(client["client_id"].as_str().unwrap().starts_with("client_"));
        assert!(client["client_id_issued_at"].as_i64().unwrap() > 0);
        assert_ne!(client["client_name"], "");
        assert!(client.get("used").is_none());
        assert!(client.get("client_secret").is_none());
    }
    for bad in [
        json!({"client_name": "no redirect"}),
        json!({"redirect_uris": []}),
        json!({"redirect_uris": ["http://evil.example/cb"]}),
        json!({"redirect_uris": ["https://user:pw@claude.ai/cb"]}),
        json!({"redirect_uris": ["https://claude.ai/cb#fragment"]}),
    ] {
        let (status, error) = s.register(&bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error["error"], "invalid_client_metadata");
    }
}

#[tokio::test]
async fn the_full_flow_rotates_refresh_tokens_and_binds_the_resource() {
    let s = Server::new();
    let (_, client) = s
        .register(&json!({"redirect_uris": [CLAUDE_CALLBACK], "client_name": "Claude"}))
        .await;
    let id = client["client_id"].as_str().unwrap();
    let (verifier, challenge) = pkce();
    let query = s.authorize_query(id, CLAUDE_CALLBACK, &challenge);

    let page = s
        .send(
            s.request("GET", &format!("/oauth/authorize?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(page.status(), StatusCode::OK);
    let html = body_text(page).await;
    assert!(html.contains("Claude wants to connect to your Mac"));
    assert!(
        html.contains("claude.ai"),
        "consent shows the redirect host"
    );

    let back = s.approve(&query).await;
    assert_eq!(param(&back, "state").unwrap(), "xyz");
    assert_eq!(param(&back, "iss").unwrap(), s.origin, "RFC 9207");
    let code = param(&back, "code").unwrap();
    let resource = format!("{}/mcp", s.origin);

    let wrong = "wrong-verifier-wrong-verifier-wrong-verifier-wrong";
    let (status, error, _) = s
        .token(&[
            ("grant_type", "authorization_code"),
            ("client_id", id),
            ("code", &code),
            ("redirect_uri", CLAUDE_CALLBACK),
            ("code_verifier", wrong),
            ("resource", &resource),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_grant");

    let (status, tokens, cache) = s
        .token(&[
            ("grant_type", "authorization_code"),
            ("client_id", id),
            ("code", &code),
            ("redirect_uri", CLAUDE_CALLBACK),
            ("code_verifier", &verifier),
            ("resource", &resource),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(cache.as_deref(), Some("no-store"));
    assert!(tokens["expires_in"].as_u64().unwrap() > 0);
    assert_eq!(tokens["scope"], "calendar:read calendar:write");
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();

    let (status, next, _) = s
        .token(&[
            ("grant_type", "refresh_token"),
            ("client_id", id),
            ("refresh_token", &refresh),
            ("resource", &resource),
            ("scope", "calendar:read offline_access"),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "{next}");
    assert_ne!(next["refresh_token"], tokens["refresh_token"], "rotates");

    let (status, retried, _) = s
        .token(&[
            ("grant_type", "refresh_token"),
            ("client_id", id),
            ("refresh_token", &refresh),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "{retried}");

    let (status, error, _) = s
        .token(&[
            ("grant_type", "refresh_token"),
            ("client_id", "other"),
            ("refresh_token", next["refresh_token"].as_str().unwrap()),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        error["error"], "invalid_grant",
        "Claude needs invalid_grant here"
    );

    let (status, error, _) = s
        .token(&[
            ("grant_type", "refresh_token"),
            ("client_id", id),
            ("refresh_token", "not-a-token"),
        ])
        .await;
    assert_eq!(
        (status, error["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
    let (status, error, _) = s
        .token(&[
            ("grant_type", "refresh_token"),
            ("client_id", id),
            ("refresh_token", &refresh),
            ("resource", "https://other.example/mcp"),
        ])
        .await;
    assert_eq!(
        (status, error["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_target"))
    );
    let (status, error, _) = s.token(&[("grant_type", "client_credentials")]).await;
    assert_eq!(
        (status, error["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("unsupported_grant_type"))
    );

    let (status, _, _) = s
        .token(&[
            ("grant_type", "authorization_code"),
            ("client_id", id),
            ("code", &code),
            ("redirect_uri", CLAUDE_CALLBACK),
            ("code_verifier", &verifier),
            ("resource", &resource),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a code works once");
    let (status, error, _) = s
        .token(&[
            ("grant_type", "refresh_token"),
            ("client_id", id),
            ("refresh_token", retried["refresh_token"].as_str().unwrap()),
        ])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error"], "invalid_grant");
}

#[tokio::test]
async fn errors_redirect_only_once_the_redirect_is_verified() {
    let s = Server::new();
    let (_, client) = s
        .register(&json!({"redirect_uris": [CHATGPT_CALLBACK], "client_name": "ChatGPT"}))
        .await;
    let id = client["client_id"].as_str().unwrap();
    let (_, challenge) = pkce();

    for query in [
        s.authorize_query("client_unknown", CHATGPT_CALLBACK, &challenge),
        s.authorize_query(id, "https://evil.example/cb", &challenge),
    ] {
        let response = s
            .send(
                s.request("GET", &format!("/oauth/authorize?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            response.headers().get(header::LOCATION).is_none(),
            "never redirect to an unverified URI"
        );
    }
    for (name, value, expected) in [
        ("response_type", "token", "unsupported_response_type"),
        ("code_challenge_method", "plain", "invalid_request"),
        ("resource", "https://other.example/mcp", "invalid_target"),
        ("scope", "not-a-scope", "invalid_scope"),
    ] {
        let query: Vec<(String, String)> = url::form_urlencoded::parse(
            s.authorize_query(id, CHATGPT_CALLBACK, &challenge)
                .as_bytes(),
        )
        .map(|(k, v)| {
            let replaced = if k == name {
                value.to_owned()
            } else {
                v.into_owned()
            };
            (k.into_owned(), replaced)
        })
        .collect();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(query)
            .finish();
        let response = s
            .send(
                s.request("GET", &format!("/oauth/authorize?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{name}");
        let to = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        assert!(to.as_str().starts_with(CHATGPT_CALLBACK));
        assert_eq!(param(&to, "error").unwrap(), expected);
        assert_eq!(param(&to, "state").unwrap(), "xyz");
        assert_eq!(
            param(&to, "iss").unwrap(),
            s.origin,
            "ChatGPT: iss on every error too"
        );
    }
}

#[tokio::test]
async fn a_request_without_resource_still_binds_to_the_one_resource() {
    let s = Server::new();
    let (_, client) = s
        .register(&json!({"redirect_uris": [CLAUDE_CALLBACK]}))
        .await;
    let id = client["client_id"].as_str().unwrap();
    let (verifier, challenge) = pkce();
    let query: Vec<(String, String)> = url::form_urlencoded::parse(
        s.authorize_query(id, CLAUDE_CALLBACK, &challenge)
            .as_bytes(),
    )
    .filter(|(k, _)| k != "resource")
    .map(|(k, v)| (k.into_owned(), v.into_owned()))
    .collect();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(query)
        .finish();
    let back = s.approve(&query).await;
    let code = param(&back, "code").unwrap();
    let (status, tokens, _) = s
        .token(&[
            ("grant_type", "authorization_code"),
            ("client_id", id),
            ("code", &code),
            ("redirect_uri", CLAUDE_CALLBACK),
            ("code_verifier", &verifier),
        ])
        .await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
}

#[tokio::test]
async fn cross_origin_browser_writes_are_refused() {
    let s = Server::new();
    let (_, challenge) = pkce();
    let query = s.authorize_query("client_x", CLAUDE_CALLBACK, &challenge);
    for origin in ["https://evil.example", "null"] {
        let response = s
            .send(
                s.request("POST", &format!("/oauth/pending?{query}"))
                    .header(header::ORIGIN, origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let anonymous_decision = s
        .send(
            s.request("POST", &format!("/oauth/authorize?{query}"))
                .header(header::ORIGIN, &s.origin)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"decision":"approve","scopes":["calendar:read"]}"#,
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(
        anonymous_decision.status(),
        StatusCode::UNAUTHORIZED,
        "no approval without the owner"
    );
}

#[tokio::test]
async fn pages_never_echo_client_markup() {
    let s = Server::new();
    let (_, client) = s
        .register(&json!({"redirect_uris": [CLAUDE_CALLBACK], "client_name": "<script>alert(1)</script>"}))
        .await;
    let (_, challenge) = pkce();
    let query = s.authorize_query(
        client["client_id"].as_str().unwrap(),
        CLAUDE_CALLBACK,
        &challenge,
    );
    let page = s
        .send(
            s.request("GET", &format!("/oauth/authorize?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let csp = page.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.contains("script-src 'self'") && !csp.contains("unsafe-inline"));
    let html = body_text(page).await;
    assert!(!html.contains("<script>alert"));
    assert!(html.contains("&lt;script&gt;alert"));
}
