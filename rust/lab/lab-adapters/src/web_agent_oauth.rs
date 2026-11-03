#![allow(clippy::too_many_lines)]
use crate::{
    web_agents::{edit_credentials, form, seconds_until, text, token_hash},
    webauthn::{Owner, Reply, fetch},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;

pub(crate) type Query = Vec<(String, String)>;

pub(crate) fn agent(
    origin: &str,
    token: &str,
    method: &str,
    path: &str,
    json_body: Option<&Value>,
    form_body: Option<&Query>,
) -> Reply {
    let mut headers = vec![("Accept", "application/json, text/event-stream")];
    let bearer = format!("Bearer {token}");
    if !token.is_empty() {
        headers.push(("Authorization", &bearer));
    }
    let (content_type, body) = if let Some(fields) = form_body {
        (
            "application/x-www-form-urlencoded",
            Some(query_string(fields).into_bytes()),
        )
    } else {
        (
            "application/json",
            json_body.and_then(|value| serde_json::to_vec(value).ok()),
        )
    };
    headers.push(("Content-Type", content_type));
    fetch(origin, method, path, &headers, body.as_deref()).unwrap_or(Reply {
        status: 0,
        headers: vec![],
        body: vec![],
    })
}

pub(crate) fn header(reply: &Reply, name: &str) -> String {
    reply
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

pub(crate) fn contains(reply: &Reply, needle: &str) -> bool {
    String::from_utf8_lossy(&reply.body).contains(needle)
}

pub(crate) fn query_string(fields: &[(String, String)]) -> String {
    let pairs: Vec<(&str, &str)> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    form(&pairs)
}

pub(crate) fn pairs(fields: &[(&str, &str)]) -> Query {
    fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn set(fields: &mut Query, key: &str, value: &str) {
    if let Some(entry) = fields.iter_mut().find(|(name, _)| name == key) {
        value.clone_into(&mut entry.1);
    } else {
        fields.push((key.to_owned(), value.to_owned()));
    }
}

pub(crate) fn get(fields: &[(String, String)], key: &str) -> String {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' => {
                if let Some(value) = text
                    .get(index + 1..index + 3)
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                {
                    out.push(value);
                    index += 2;
                } else {
                    out.push(b'%');
                }
            }
            other => out.push(other),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn redirect_query(reply: &Reply) -> Query {
    reply.json()["redirect"]
        .as_str()
        .and_then(|link| link.split_once('?'))
        .map(|(_, query)| {
            query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(key, value)| (decode(key), decode(value)))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn authorize(
    owner: &Owner,
    method: &str,
    fields: &Query,
    body: Option<&Value>,
) -> Reply {
    let payload = body.map_or_else(Vec::new, |value| {
        serde_json::to_vec(value).unwrap_or_default()
    });
    owner
        .request(
            method,
            &format!("/oauth/authorize?{}", query_string(fields)),
            "application/json",
            &payload,
        )
        .unwrap_or(Reply {
            status: 0,
            headers: vec![],
            body: vec![],
        })
}

fn mcp(origin: &str, token: &str, method: &str, params: &Value) -> bool {
    let reply = agent(
        origin,
        token,
        "POST",
        "/mcp",
        Some(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})),
        None,
    );
    reply.status == 200 && contains(&reply, "\"result\"") && !contains(&reply, "\"isError\":true")
}

fn chats(origin: &str, token: &str) -> u16 {
    agent(
        origin,
        token,
        "POST",
        "/v1/op/messages.chats",
        Some(&json!({})),
        None,
    )
    .status
}

fn token_reply(reply: &Reply) -> Value {
    reply.json()
}

fn expiry(offset: i64) -> String {
    (jiff::Timestamp::now() + jiff::SignedDuration::from_secs(offset)).to_string()
}

pub fn oauth(world: &mut World, owner: &Owner, evidence: &mut Evidence) -> Result<()> {
    let origin = owner.origin.clone();
    let resource = format!("{origin}/mcp");
    let redirect = "https://connector.example/callback";
    for path in ["/mcp", "/v1/op/messages.chats"] {
        let reply = agent(&origin, "", "POST", path, Some(&json!({})), None);
        evidence.check(
            &format!("oauth_discovery_challenge_{path}"),
            reply.status == 401
                && header(&reply, "www-authenticate")
                    .contains(&format!("{origin}/.well-known/oauth-protected-resource")),
            "",
        );
    }
    let reply = agent(
        &origin,
        "",
        "GET",
        "/.well-known/oauth-protected-resource",
        None,
        None,
    );
    evidence.check(
        "resource_metadata",
        reply.status == 200 && contains(&reply, &resource),
        "",
    );
    let reply = agent(
        &origin,
        "",
        "GET",
        "/.well-known/oauth-authorization-server",
        None,
        None,
    );
    evidence.check(
        "authorization_metadata_pkce",
        reply.status == 200 && contains(&reply, "S256"),
        "",
    );
    let reply = agent(
        &origin,
        "",
        "POST",
        "/oauth/register",
        Some(&json!({
            "client_name": "Bridge Test connector",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_method": "none"
        })),
        None,
    );
    let client = reply.json()["client_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    evidence.check(
        "dynamic_registration",
        reply.status == 201 && !client.is_empty(),
        "",
    );
    if client.is_empty() {
        return Ok(());
    }
    let verifier = "a".repeat(43);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let query = pairs(&[
        ("client_id", &client),
        ("response_type", "code"),
        ("redirect_uri", redirect),
        ("scope", "messages:read"),
        ("resource", &resource),
        ("state", "bridge-test"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    let reply = agent(
        &origin,
        "",
        "GET",
        &format!("/oauth/authorize?{}", query_string(&query)),
        None,
        None,
    );
    evidence.check("authorize_offers_local_approval", reply.status == 200, "");
    for bad in ["plain", ""] {
        let mut variant = query.clone();
        set(&mut variant, "code_challenge_method", bad);
        evidence.check(
            &format!("reject_non_s256_{bad}"),
            authorize(owner, "GET", &variant, None).status == 400,
            "",
        );
    }
    let mut variant = query.clone();
    set(
        &mut variant,
        "redirect_uri",
        "https://attacker.example/callback",
    );
    evidence.check(
        "redirect_exact_match",
        authorize(owner, "GET", &variant, None).status == 400,
        "",
    );
    let reply = authorize(
        owner,
        "POST",
        &query,
        Some(&json!({"decision": "approve", "level": "full", "scopes": ["contacts:write"]})),
    );
    evidence.check(
        "consent_cannot_exceed_requested_scopes",
        reply.status == 400,
        "",
    );
    let issue = || {
        let reply = authorize(
            owner,
            "POST",
            &query,
            Some(&json!({"decision": "approve", "level": "read", "scopes": ["messages:read"]})),
        );
        (reply.status, redirect_query(&reply))
    };
    let (status, response) = issue();
    evidence.check(
        "owner_consent_binds_state_issuer",
        status == 200
            && get(&response, "state") == "bridge-test"
            && get(&response, "iss") == origin
            && !get(&response, "code").is_empty(),
        "",
    );
    let code_form = |code: &str, verifier: &str, resource: &str| {
        pairs(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
            ("resource", resource),
        ])
    };
    let token = |fields: &Query| agent(&origin, "", "POST", "/oauth/token", None, Some(fields));
    let first_code = get(&response, "code");
    let wrong = "b".repeat(43);
    evidence.check(
        "pkce_wrong_verifier_rejected",
        token(&code_form(&first_code, &wrong, &resource)).status == 400,
        "",
    );
    evidence.check(
        "resource_binding",
        token(&code_form(
            &first_code,
            &verifier,
            "https://attacker.example/mcp",
        ))
        .status
            == 400,
        "",
    );
    let reply = token(&code_form(&first_code, &verifier, &resource));
    let tokens = token_reply(&reply);
    let access = text(&tokens, "access_token").to_owned();
    let refresh_token = text(&tokens, "refresh_token").to_owned();
    evidence.check(
        "code_exchange_one_hour_access",
        reply.status == 200
            && !access.is_empty()
            && tokens["expires_in"] == 3600
            && !refresh_token.is_empty(),
        "",
    );
    evidence.check(
        "oauth_mcp_tools_list",
        mcp(&origin, &access, "tools/list", &json!({})),
        "",
    );
    evidence.check(
        "oauth_mcp_read",
        mcp(
            &origin,
            &access,
            "tools/call",
            &json!({"name": "messages_chats", "arguments": {}}),
        ),
        "",
    );
    let stored = fs::read_to_string(world.gateway_home.join("credentials.json"));
    let stored_text = stored.as_deref().unwrap_or_default();
    evidence.check(
        "oauth_secrets_stored_hashed",
        stored.is_ok()
            && !stored_text.contains(&access)
            && !stored_text.contains(&refresh_token)
            && !stored_text.contains(&first_code),
        "",
    );
    let data: Value = serde_json::from_str(stored_text).unwrap_or(Value::Null);
    let first = |list: &str| data[list].as_array().and_then(|rows| rows.first()).cloned();
    let lifetimes = match (first("oauth_access"), first("oauth_refresh")) {
        (Some(access), Some(refresh)) => {
            let access = seconds_until(&access);
            let refresh = seconds_until(&refresh);
            access > 59 * 60 && access <= 3600 && refresh > 89 * 86400 && refresh <= 90 * 86400
        }
        _ => false,
    };
    evidence.check("oauth_one_hour_and_90_day_expiry", lifetimes, "");
    let expired = edit_credentials(world, |data| {
        if let Some(rows) = data["oauth_access"].as_array_mut() {
            for row in rows {
                row["expires_at"] = json!(expiry(-1));
            }
        }
    });
    evidence.check(
        "expired_access_rejected",
        expired.is_ok() && chats(&origin, &access) == 401,
        "",
    );
    let refresh_form = |refresh: &str| {
        pairs(&[
            ("grant_type", "refresh_token"),
            ("client_id", &client),
            ("refresh_token", refresh),
            ("resource", &resource),
        ])
    };
    let reply = token(&refresh_form(&refresh_token));
    let rotated = token_reply(&reply);
    evidence.check(
        "refresh_rotates",
        reply.status == 200
            && !text(&rotated, "refresh_token").is_empty()
            && text(&rotated, "refresh_token") != refresh_token
            && text(&rotated, "access_token") != access,
        "",
    );
    evidence.check(
        "refresh_reuse_detected",
        token(&refresh_form(&refresh_token)).status == 400,
        "",
    );
    evidence.check(
        "reuse_revokes_access_family",
        chats(&origin, text(&rotated, "access_token")) == 401,
        "",
    );
    evidence.check(
        "reuse_revokes_refresh_family",
        token(&refresh_form(text(&rotated, "refresh_token"))).status == 400,
        "",
    );
    let (_, response) = issue();
    let code = get(&response, "code");
    let form_code = code_form(&code, &verifier, &resource);
    let tokens = token_reply(&token(&form_code));
    let (_, extra_response) = issue();
    let extra_form = code_form(&get(&extra_response, "code"), &verifier, &resource);
    let extra = token(&extra_form);
    let extra_tokens = token_reply(&extra);
    let extra_refresh = text(&extra_tokens, "refresh_token").to_owned();
    let edited = edit_credentials(world, |data| {
        if let Some(rows) = data["oauth_refresh"].as_array_mut() {
            for row in rows {
                if row["hash"] == token_hash(&extra_refresh).as_str() {
                    row["expires_at"] = json!(expiry(-1));
                }
            }
        }
    });
    evidence.check(
        "expired_refresh_rejected",
        extra.status == 200 && edited.is_ok() && token(&refresh_form(&extra_refresh)).status == 400,
        "",
    );
    evidence.check(
        "authorization_code_single_use",
        token(&form_code).status == 400,
        "",
    );
    evidence.check(
        "code_reuse_revokes_family",
        chats(&origin, text(&tokens, "access_token")) == 401,
        "",
    );
    let (_, response) = issue();
    let tokens = token_reply(&token(&code_form(
        &get(&response, "code"),
        &verifier,
        &resource,
    )));
    let access = text(&tokens, "access_token").to_owned();
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("owner_revoke_authority", false, "");
        return Ok(());
    }
    let principal = (chats(&origin, &access) == 200)
        .then(|| {
            let stored = fs::read(world.gateway_home.join("credentials.json")).ok()?;
            let data: Value = serde_json::from_slice(&stored).ok()?;
            data["oauth_access"].as_array()?.iter().find_map(|row| {
                (text(row, "hash") == token_hash(&access) && !text(row, "principal").is_empty())
                    .then(|| text(row, "principal").to_owned())
            })
        })
        .flatten();
    let Some(principal) = principal else {
        evidence.check("owner_revoke_token", false, "");
        return Ok(());
    };
    let revoked = world.gateway_cli(&["agent", "revoke", &principal]);
    evidence.check(
        "owner_revoke_oauth",
        revoked.is_ok() && chats(&origin, &access) == 401,
        "",
    );
    Ok(())
}
