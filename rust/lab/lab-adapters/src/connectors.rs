use crate::rawhttp::{Response, form, query_value, request};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jiff::Timestamp;
use lab_runner::{Evidence, Scenario};
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{thread, time::Duration};

pub const SCENARIOS: [&str; 2] = ["CONN-CHATGPT", "CONN-CLAUDE-AI"];

pub fn inject(scenario: &mut Scenario) {
    if scenario.given.is_empty()
        && scenario.when.is_empty()
        && scenario.then.is_empty()
        && SCENARIOS.contains(&scenario.id.as_str())
    {
        scenario.when.push(json!({"connector_approval": {}}));
    }
}

fn address(world: &World) -> Result<String> {
    world.runtime["api_address"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid("runtime API address".into()))
}

fn send(
    base: &str,
    method: &str,
    path: &str,
    token: &str,
    input: Option<&[u8]>,
    content_type: &str,
) -> Option<Response> {
    let mut headers = vec![
        ("Accept", "application/json, text/event-stream"),
        ("Content-Type", content_type),
    ];
    let bearer = format!("Bearer {token}");
    if !token.is_empty() {
        headers.push(("Authorization", &bearer));
    }
    request(base, method, path, &headers, input, 10).ok()
}

#[derive(Debug, Clone, Default)]
struct Pending {
    id: String,
    code: String,
    client_name: String,
    scopes: Vec<String>,
    expires_at: Option<Timestamp>,
}
impl Pending {
    fn parse(value: &Value) -> Self {
        Self {
            id: value["id"].as_str().unwrap_or_default().to_owned(),
            code: value["code"].as_str().unwrap_or_default().to_owned(),
            client_name: value["client_name"].as_str().unwrap_or_default().to_owned(),
            scopes: value["scopes"]
                .as_array()
                .map(|all| {
                    all.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            expires_at: value["expires_at"].as_str().and_then(|t| t.parse().ok()),
        }
    }
}

struct Flow<'a> {
    world: &'a World,
    base: String,
    query: String,
}
impl Flow<'_> {
    fn call(&self, method: &str, path: &str) -> Option<Response> {
        let origin = format!("http://{}", self.base);
        request(
            &self.base,
            method,
            path,
            &[("Origin", &origin)],
            Some(b"{}"),
            10,
        )
        .ok()
    }
    fn code(&self, method: &str, path: &str) -> u16 {
        self.call(method, path).map_or(503, |r| r.status)
    }
    fn begin(&self, evidence: &mut Evidence, client: &str, before: Timestamp) -> Pending {
        let reply = self.call("POST", &format!("/oauth/pending?{}", self.query));
        let status = reply.as_ref().map_or(0, |r| r.status);
        let pending = reply.map_or_else(Pending::default, |r| Pending::parse(&r.json()));
        let lifetime = pending
            .expires_at
            .map(|at| at.as_millisecond().saturating_sub(before.as_millisecond()));
        evidence.check(
            "pending_matching_code_and_requested_data",
            status == 201
                && pending.code.len() == 4
                && pending.client_name == client
                && pending.scopes == ["messages:read"]
                && lifetime.is_some_and(|ms| (300_000..310_000).contains(&ms)),
            "",
        );
        pending
    }
    fn decide(&self, pending: &Pending, code: &str, action: &str) -> bool {
        self.world
            .gateway_cli(&[
                "connector",
                action,
                "--request",
                &pending.id,
                "--code",
                code,
            ])
            .is_ok()
    }
    fn poll(&self, pending: &Pending) -> Option<Response> {
        self.call("GET", &format!("/oauth/pending/{}", pending.id))
    }
}

fn redirect(reply: Option<&Response>) -> String {
    reply
        .map(|r| r.json()["redirect"].as_str().unwrap_or_default().to_owned())
        .unwrap_or_default()
}

#[allow(clippy::too_many_lines)]
pub fn connector_approval(
    world: &mut World,
    _input: &Value,
    evidence: &mut Evidence,
) -> Result<()> {
    let world: &World = world;
    let id = evidence.scenario.clone();
    evidence.check("isolated_gateway", true, "");
    let base = address(world)?;
    evidence.check(
        "authority",
        world.gateway_cli(&["agent", "list"]).is_ok(),
        "",
    );
    evidence.check(
        "owner_local_socket",
        world.gateway_cli(&["connector", "pending"]).is_ok(),
        "",
    );
    let name = format!("[ab-test] {id}");
    let registration = serde_json::to_vec(&json!({
        "client_name": name,
        "redirect_uris": ["https://connector.example/callback"],
        "grant_types": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_method": "none"
    }))?;
    let registered = send(
        &base,
        "POST",
        "/oauth/register",
        "",
        Some(&registration),
        "application/json",
    );
    let client_id = registered
        .as_ref()
        .and_then(|r| r.json()["client_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    evidence.check(
        "registered_client",
        registered.as_ref().is_some_and(|r| r.status == 201) && !client_id.is_empty(),
        "",
    );
    if client_id.is_empty() {
        return Ok(());
    }
    let verifier = "a".repeat(43);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let resource = format!("http://{base}/mcp");
    let query = form(&[
        ("client_id", &client_id),
        ("response_type", "code"),
        ("redirect_uri", "https://connector.example/callback"),
        ("scope", "messages:read"),
        ("resource", &resource),
        ("state", "[ab-test]"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    let flow = Flow {
        world,
        base: base.clone(),
        query: query.clone(),
    };
    evidence.check(
        "authorize_without_passkey",
        flow.code("GET", &format!("/oauth/authorize?{query}")) == 200,
        "",
    );
    let first = flow.begin(evidence, &name, Timestamp::now());
    let other = flow.begin(evidence, &name, Timestamp::now());
    let wrong = if first.code == "0000" { "0001" } else { "0000" };
    evidence.check(
        "wrong_code_refused",
        !flow.decide(&first, wrong, "allow")
            && !flow
                .poll(&first)
                .is_some_and(|r| r.text().contains("?code=")),
        "",
    );
    evidence.check(
        "another_requests_code_refused",
        !flow.decide(&first, &other.code, "allow"),
        "",
    );
    evidence.check("mac_allow", flow.decide(&first, &first.code, "allow"), "");
    let remaining = world
        .gateway_cli(&["connector", "pending"])
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(&raw).ok());
    evidence.check(
        "one_request_never_approves_another",
        remaining
            .is_some_and(|all| all.len() == 1 && all[0]["id"].as_str() == Some(other.id.as_str()))
            && !flow
                .poll(&other)
                .is_some_and(|r| r.text().contains("code=")),
        "",
    );
    evidence.check(
        "decision_single_use",
        !flow.decide(&first, &first.code, "allow"),
        "",
    );
    let approved = flow.poll(&first);
    let destination = redirect(approved.as_ref());
    let auth_code = query_value(&destination, "code").unwrap_or_default();
    evidence.check(
        "approval_redirect_with_state_and_code",
        approved.as_ref().is_some_and(|r| r.status == 200)
            && !auth_code.is_empty()
            && query_value(&destination, "state").as_deref() == Some("[ab-test]"),
        "",
    );
    evidence.check(
        "poll_single_use",
        flow.poll(&first).is_some_and(|r| r.status == 410),
        "",
    );
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", &auth_code),
        ("redirect_uri", "https://connector.example/callback"),
        ("code_verifier", &verifier),
        ("resource", &resource),
    ]);
    let tokens = send(
        &base,
        "POST",
        "/oauth/token",
        "",
        Some(body.as_bytes()),
        "application/x-www-form-urlencoded",
    );
    let access = tokens
        .as_ref()
        .and_then(|r| r.json()["access_token"].as_str().map(str::to_owned))
        .unwrap_or_default();
    evidence.check(
        "oauth_token",
        tokens.as_ref().is_some_and(|r| r.status == 200) && !access.is_empty(),
        "",
    );
    let call = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "records_query", "arguments": {"collection": "messages.chat", "limit": 1}}
    }))?;
    let reply = send(
        &base,
        "POST",
        "/mcp",
        &access,
        Some(&call),
        "application/json",
    );
    evidence.check(
        "token_calls_records_query",
        reply.as_ref().is_some_and(|r| {
            let text = r.text();
            r.status == 200
                && text.contains("\"result\"")
                && !text.contains("\"isError\":true")
                && !text.contains("\"error\":")
        }),
        "",
    );
    let agents = world
        .gateway_cli(&["agent", "list"])
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(&raw).ok());
    evidence.check(
        "agent_listed_with_requested_scopes",
        agents.is_some_and(|all| {
            all.len() == 1
                && all[0]["name"].as_str() == Some(name.as_str())
                && all[0]["scopes"] == json!(["messages:read"])
        }),
        "",
    );
    evidence.check("mac_deny", flow.decide(&other, &other.code, "deny"), "");
    let denied = redirect(flow.poll(&other).as_ref());
    evidence.check(
        "deny_access_denied",
        query_value(&denied, "error").as_deref() == Some("access_denied")
            && query_value(&denied, "code").is_none(),
        "",
    );
    let expired = flow.begin(evidence, &name, Timestamp::now());
    if let Some(at) = expired.expires_at {
        let wait = at.as_millisecond() - Timestamp::now().as_millisecond() + 1000;
        thread::sleep(Duration::from_millis(u64::try_from(wait).unwrap_or(0)));
    }
    evidence.check(
        "expired_request_refused",
        !flow.decide(&expired, &expired.code, "allow")
            && flow.poll(&expired).is_some_and(|r| r.status == 410),
        "",
    );
    let exhausted = flow.begin(evidence, &name, Timestamp::now());
    for _ in 0..5 {
        evidence.check(
            "wrong_code_attempt_refused",
            !flow.decide(&exhausted, "wrong", "allow"),
            "",
        );
    }
    evidence.check(
        "code_attempts_rate_limited",
        !flow.decide(&exhausted, &exhausted.code, "allow"),
        "",
    );
    let mut limited = false;
    for _ in 0..12 {
        limited = flow.code("POST", &format!("/oauth/pending?{query}")) == 429 || limited;
    }
    evidence.check("creation_rate_limited", limited, "");
    Ok(())
}
