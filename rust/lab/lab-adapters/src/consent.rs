#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    journey::command,
    rawhttp::{self, Response, form, query_value},
    webauthn::Owner,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jiff::{Timestamp, ToSpan};
use lab_runner::{Evidence, Scenario};
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub const SCENARIOS: [&str; 1] = ["CONSENT-CONNECTOR"];

pub fn inject(scenario: &mut Scenario) {
    if SCENARIOS.contains(&scenario.id.as_str())
        && scenario.when.is_empty()
        && scenario.then.is_empty()
    {
        scenario.when.push(json!({"consent_connector": {}}));
    }
}

pub(crate) struct Site {
    pub address: String,
    pub origin: String,
    pub host: String,
    pub home: PathBuf,
    pub directory: PathBuf,
}
impl Site {
    pub(crate) fn new(world: &World) -> Result<Self> {
        let address = world.runtime["api_address"]
            .as_str()
            .ok_or_else(|| Error::Invalid("runtime API address".into()))?
            .to_owned();
        let port = address
            .rsplit(':')
            .next()
            .ok_or_else(|| Error::Invalid("API port".into()))?;
        Ok(Self {
            origin: format!("http://localhost:{port}"),
            host: format!("localhost:{port}"),
            address,
            home: world.gateway_home.clone(),
            directory: world.directory.clone(),
        })
    }

    pub(crate) fn send(
        &self,
        method: &str,
        path: &str,
        token: &str,
        content_type: &str,
        body: Option<&[u8]>,
    ) -> Response {
        let bearer = format!("Bearer {token}");
        let mut headers = vec![
            ("Host", self.host.as_str()),
            ("Accept", "application/json, text/event-stream"),
            ("Content-Type", content_type),
        ];
        if !token.is_empty() {
            headers.push(("Authorization", &bearer));
        }
        rawhttp::request(&self.address, method, path, &headers, body, 10).unwrap_or(Response {
            status: 0,
            headers: vec![],
            body: vec![],
        })
    }
    pub(crate) fn json(&self, method: &str, path: &str, token: &str, input: &Value) -> Response {
        self.send(
            method,
            path,
            token,
            "application/json",
            Some(input.to_string().as_bytes()),
        )
    }
    pub(crate) fn bare(&self, method: &str, path: &str, token: &str) -> Response {
        self.send(method, path, token, "application/json", None)
    }
    pub(crate) fn form(&self, path: &str, pairs: &[(&str, &str)]) -> Response {
        self.send(
            "POST",
            path,
            "",
            "application/x-www-form-urlencoded",
            Some(form(pairs).as_bytes()),
        )
    }
    pub(crate) fn owner_session(&self, world: &World) -> Result<Owner> {
        let mut owner = Owner::at(&self.address)?;
        let output = world.gateway_cli(&["web", "enroll", "--host", &owner.origin])?;
        let token = output
            .split_whitespace()
            .next()
            .and_then(|link| link.split_once('#'))
            .map(|(_, fragment)| fragment.to_owned())
            .ok_or_else(|| Error::Invalid("binary enrollment token missing".into()))?;
        let reply = owner.sign_in("enroll", &token, "trusted")?;
        if reply.status != 200 || owner.cookie.is_none() {
            return Err(Error::Invalid(format!(
                "binary enrollment status {}",
                reply.status
            )));
        }
        Ok(owner)
    }
}

pub(crate) fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

pub(crate) fn edit_credentials(home: &Path, edit: impl FnOnce(&mut Value)) -> bool {
    let apply = || -> std::io::Result<()> {
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(home.join("credentials.lock"))?;
        lock.lock()?;
        let path = home.join("credentials.json");
        let mut data: Value = serde_json::from_slice(&fs::read(&path)?)?;
        edit(&mut data);
        let scratch = home.join("credentials.json.agent-expiry");
        fs::write(&scratch, serde_json::to_vec(&data)?)?;
        fs::set_permissions(&scratch, fs::Permissions::from_mode(0o600))?;
        fs::rename(&scratch, &path)
    };
    apply().is_ok()
}

pub(crate) fn past() -> String {
    (Timestamp::now() - 1.second()).to_string()
}

fn expire_all(data: &mut Value, key: &str) {
    for entry in data[key].as_array_mut().into_iter().flatten() {
        entry["expires_at"] = json!(past());
    }
}
fn expire_hash(data: &mut Value, key: &str, hash: &str) {
    for entry in data[key].as_array_mut().into_iter().flatten() {
        if entry["hash"] == hash {
            entry["expires_at"] = json!(past());
        }
    }
}
fn seconds_until(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|text| text.parse::<Timestamp>().ok())
        .map_or(i64::MIN, |at| at.as_second() - Timestamp::now().as_second())
}

#[derive(Default, Clone)]
struct Tokens {
    access: String,
    refresh: String,
    expires_in: i64,
}
fn tokens(reply: &Response) -> Tokens {
    let value = reply.json();
    Tokens {
        access: value["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        refresh: value["refresh_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        expires_in: value["expires_in"].as_i64().unwrap_or_default(),
    }
}

pub(crate) fn agent_oauth(world: &World, site: &Site, owner: &Owner, evidence: &mut Evidence) {
    let origin = &site.origin;
    let resource = format!("{origin}/mcp");
    for path in ["/mcp", "/v1/op/messages.chats"] {
        let reply = site.json("POST", path, "", &json!({}));
        evidence.check(
            &format!("oauth_discovery_challenge_{path}"),
            reply.status == 401
                && reply.header("WWW-Authenticate").is_some_and(|h| {
                    h.contains(&format!("{origin}/.well-known/oauth-protected-resource"))
                }),
            "",
        );
    }
    let reply = site.bare("GET", "/.well-known/oauth-protected-resource", "");
    evidence.check(
        "resource_metadata",
        reply.status == 200 && reply.text().contains(&resource),
        "",
    );
    let reply = site.bare("GET", "/.well-known/oauth-authorization-server", "");
    evidence.check(
        "authorization_metadata_pkce",
        reply.status == 200 && reply.text().contains("S256"),
        "",
    );
    let redirect = "https://connector.example/callback";
    let registered = site.json(
        "POST",
        "/oauth/register",
        "",
        &json!({
            "client_name": "Bridge Test connector",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_method": "none"
        }),
    );
    let client = registered.json()["client_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    evidence.check(
        "dynamic_registration",
        registered.status == 201 && !client.is_empty(),
        "",
    );
    if client.is_empty() {
        return;
    }
    let verifier = "a".repeat(43);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let query = |overrides: &[(&str, &str)]| {
        let mut pairs: Vec<(&str, &str)> = vec![
            ("client_id", &client),
            ("response_type", "code"),
            ("redirect_uri", redirect),
            ("scope", "messages:read"),
            ("resource", &resource),
            ("state", "bridge-test"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ];
        for (key, value) in overrides {
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            }
        }
        form(&pairs)
    };
    let q = query(&[]);
    let owner_call = |method: &str, q: &str, input: Option<&Value>| {
        owner
            .request(
                method,
                &format!("/oauth/authorize?{q}"),
                "application/json",
                input.map(Value::to_string).unwrap_or_default().as_bytes(),
            )
            .map_or((0, Value::Null), |reply| (reply.status, reply.json()))
    };
    evidence.check(
        "authorize_offers_local_approval",
        site.bare("GET", &format!("/oauth/authorize?{q}"), "")
            .status
            == 200,
        "",
    );
    for bad in ["plain", ""] {
        let (status, _) = owner_call("GET", &query(&[("code_challenge_method", bad)]), None);
        evidence.check(&format!("reject_non_s256_{bad}"), status == 303, "");
    }
    let (status, _) = owner_call(
        "GET",
        &query(&[("redirect_uri", "https://attacker.example/callback")]),
        None,
    );
    evidence.check("redirect_exact_match", status == 400, "");
    let (status, _) = owner_call(
        "POST",
        &q,
        Some(&json!({"decision": "approve", "level": "full", "scopes": ["contacts:write"]})),
    );
    evidence.check("consent_cannot_exceed_requested_scopes", status == 400, "");
    let issue = || -> (u16, String, Option<String>, Option<String>) {
        let (status, value) = owner_call(
            "POST",
            &q,
            Some(&json!({"decision": "approve", "level": "read", "scopes": ["messages:read"]})),
        );
        let destination = value["redirect"].as_str().unwrap_or_default();
        (
            status,
            query_value(destination, "code").unwrap_or_default(),
            query_value(destination, "state"),
            query_value(destination, "iss"),
        )
    };
    let (status, code, state, issuer) = issue();
    evidence.check(
        "owner_consent_binds_state_issuer",
        status == 200
            && state.as_deref() == Some("bridge-test")
            && issuer.as_deref() == Some(origin.as_str())
            && !code.is_empty(),
        "",
    );
    let exchange = |code: &str, overrides: &[(&str, &str)]| {
        let mut pairs: Vec<(&str, &str)> = vec![
            ("grant_type", "authorization_code"),
            ("client_id", &client),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", &verifier),
            ("resource", &resource),
        ];
        for (key, value) in overrides {
            if let Some(slot) = pairs.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            }
        }
        site.form("/oauth/token", &pairs)
    };
    let wrong_verifier = "b".repeat(43);
    evidence.check(
        "pkce_wrong_verifier_rejected",
        exchange(&code, &[("code_verifier", &wrong_verifier)]).status == 400,
        "",
    );
    evidence.check(
        "resource_binding",
        exchange(&code, &[("resource", "https://attacker.example/mcp")]).status == 400,
        "",
    );
    let reply = exchange(&code, &[]);
    let first = tokens(&reply);
    evidence.check(
        "code_exchange_one_hour_access",
        reply.status == 200
            && !first.access.is_empty()
            && first.expires_in == 3600
            && !first.refresh.is_empty(),
        "",
    );
    let mcp = |token: &str, method: &str, params: &Value| {
        let reply = site.json(
            "POST",
            "/mcp",
            token,
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        );
        let text = reply.text();
        reply.status == 200 && text.contains("\"result\"") && !text.contains("\"isError\":true")
    };
    evidence.check(
        "oauth_mcp_tools_list",
        mcp(&first.access, "tools/list", &json!({})),
        "",
    );
    evidence.check(
        "oauth_mcp_read",
        mcp(
            &first.access,
            "tools/call",
            &json!({"name": "messages_chats", "arguments": {}}),
        ),
        "",
    );
    let stored = fs::read_to_string(site.home.join("credentials.json")).ok();
    evidence.check(
        "oauth_secrets_stored_hashed",
        stored.as_ref().is_some_and(|text| {
            !text.contains(&first.access) && !text.contains(&first.refresh) && !text.contains(&code)
        }),
        "",
    );
    let credentials: Value = stored
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let access_left = seconds_until(&credentials["oauth_access"][0]["expires_at"]);
    let refresh_left = seconds_until(&credentials["oauth_refresh"][0]["expires_at"]);
    evidence.check(
        "oauth_one_hour_and_90_day_expiry",
        credentials["oauth_access"][0].is_object()
            && credentials["oauth_refresh"][0].is_object()
            && access_left > 59 * 60
            && access_left <= 3600
            && refresh_left > 89 * 86400
            && refresh_left <= 90 * 86400,
        "",
    );
    let expired = edit_credentials(&site.home, |data| expire_all(data, "oauth_access"));
    let chats = |token: &str| site.json("POST", "/v1/op/messages.chats", token, &json!({}));
    evidence.check(
        "expired_access_rejected",
        expired && chats(&first.access).status == 401,
        "",
    );
    let refresh = |token: &str| {
        site.form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &client),
                ("refresh_token", token),
                ("resource", &resource),
            ],
        )
    };
    let reply = refresh(&first.refresh);
    let rotated = tokens(&reply);
    evidence.check(
        "refresh_rotates",
        reply.status == 200
            && !rotated.refresh.is_empty()
            && rotated.refresh != first.refresh
            && rotated.access != first.access,
        "",
    );
    evidence.check(
        "refresh_retry_tolerated",
        refresh(&first.refresh).status == 200,
        "",
    );
    let aged = edit_credentials(&site.home, |data| {
        if let Some(rows) = data["oauth_refresh"].as_array_mut() {
            for row in rows {
                if row["used"] == true {
                    row["rotated_at"] = json!(1);
                }
            }
        }
    });
    evidence.check(
        "refresh_reuse_detected",
        aged && refresh(&first.refresh).status == 400,
        "",
    );
    evidence.check(
        "reuse_revokes_access_family",
        chats(&rotated.access).status == 401,
        "",
    );
    evidence.check(
        "reuse_revokes_refresh_family",
        refresh(&rotated.refresh).status == 400,
        "",
    );
    let (_, second, _, _) = issue();
    let reply = exchange(&second, &[]);
    let family = tokens(&reply);
    let (_, third, _, _) = issue();
    let extra_reply = exchange(&third, &[]);
    let extra = tokens(&extra_reply);
    let hash = sha256_hex(&extra.refresh);
    let edited = edit_credentials(&site.home, |data| expire_hash(data, "oauth_refresh", &hash));
    evidence.check(
        "expired_refresh_rejected",
        extra_reply.status == 200 && edited && refresh(&extra.refresh).status == 400,
        "",
    );
    evidence.check(
        "authorization_code_single_use",
        exchange(&second, &[]).status == 400,
        "",
    );
    evidence.check(
        "code_reuse_revokes_family",
        chats(&family.access).status == 401,
        "",
    );
    let (_, fourth, _, _) = issue();
    let live = tokens(&exchange(&fourth, &[]));
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("owner_revoke_authority", false, "");
        return;
    }
    let principal = principal_of(site, &live.access, &chats(&live.access));
    let Some(principal) = principal else {
        evidence.check("owner_revoke_token", false, "");
        return;
    };
    let revoked = world.gateway_cli(&["agent", "revoke", &principal]);
    evidence.check(
        "owner_revoke_oauth",
        revoked.is_ok() && chats(&live.access).status == 401,
        "",
    );
}

fn principal_of(site: &Site, token: &str, probe: &Response) -> Option<String> {
    if probe.status != 200 {
        return None;
    }
    let credentials: Value =
        serde_json::from_slice(&fs::read(site.home.join("credentials.json")).ok()?).ok()?;
    let hash = sha256_hex(token);
    credentials["oauth_access"]
        .as_array()?
        .iter()
        .find(|entry| entry["hash"] == hash.as_str() && entry["principal"].as_str().is_some())
        .and_then(|entry| entry["principal"].as_str().map(str::to_owned))
}

pub fn run(world: &mut World, _input: &Value, evidence: &mut Evidence) -> Result<()> {
    let world: &World = world;
    let site = Site::new(world)?;
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("authority", false, "");
        return Ok(());
    }
    let owner = match site.owner_session(world) {
        Ok(owner) => owner,
        Err(error) => {
            evidence.check("owner_session", false, &error.to_string());
            return Ok(());
        }
    };
    agent_oauth(world, &site, &owner, evidence);
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("authority", false, "");
        return Ok(());
    }
    let created = world
        .gateway_cli(&[
            "agent",
            "create",
            "--name",
            "Bridge Test consent",
            "--level",
            "approve",
            "--scope",
            "messages:write",
        ])
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    let token = created
        .as_ref()
        .and_then(|value| value["token"].as_str())
        .unwrap_or_default()
        .to_owned();
    if token.is_empty() {
        evidence.check("agent", false, "");
        return Ok(());
    }
    evidence.check(
        "synthetic_chat",
        world
            .control("/messages", &json!({"action": "seed", "count": 0}))
            .is_ok(),
        "",
    );
    let mut chat = String::new();
    let found = await_until(5, || {
        world
            .query("records.query", &json!({"collection": "messages.chat"}))
            .ok()
            .and_then(|value| value["records"][0]["id"].as_str().map(str::to_owned))
            .is_some_and(|id| {
                chat = id;
                true
            })
    });
    if !found {
        evidence.check("chat", false, "");
        return Ok(());
    }
    let mut send = command(
        "messages.send",
        &json!({"chat_id": chat, "text": "[ab-test consent] synthetic approval"}),
    )?;
    send["wait_ms"] = json!(1);
    let id = send["command_id"].as_str().unwrap_or_default().to_owned();
    let reply = site.json("POST", "/v1/op/messages.send", &token, &send);
    evidence.check(
        "awaiting_approval",
        reply.status == 200 && reply.json()["state"] == "awaiting_approval",
        "",
    );
    let page = owner.request("GET", "/approvals", "application/json", b"");
    let body = page
        .as_ref()
        .map(|reply| String::from_utf8_lossy(&reply.body).into_owned())
        .unwrap_or_default();
    if page.is_err() {
        evidence.check("approvals_page", false, "");
        return Ok(());
    }
    evidence.check(
        "owner_arguments",
        page.as_ref().is_ok_and(|reply| reply.status == 200)
            && body.contains("synthetic approval")
            && body.contains("argument_hash"),
        "",
    );
    let decided = owner.request(
        "POST",
        &format!("/approvals/{id}"),
        "application/x-www-form-urlencoded",
        b"decision=approve",
    );
    let Ok(decided) = decided else {
        evidence.check("approve_form", false, "");
        return Ok(());
    };
    evidence.check("approve_form", decided.status == 200, "");
    let observed = await_until(10, || {
        site.json(
            "POST",
            "/v1/op/commands.status",
            &token,
            &json!({"command_id": id}),
        )
        .json()["state"]
            == "observed"
    });
    evidence.check("approved_observed", observed, "");
    for path in [
        "/web/v1/session",
        "/web/v1/live",
        "/web/v1/handles",
        "/sw.js",
        "/manifest.webmanifest",
    ] {
        evidence.check(
            &format!("removed_{path}"),
            site.bare("GET", path, "").status == 404,
            "",
        );
    }
    let landing = site.bare("GET", "/", "");
    let text = landing.text();
    evidence.check(
        "plain_landing",
        landing.status == 200 && !text.contains("Solid") && text.contains("Apple Bridge"),
        "",
    );
    Ok(())
}
