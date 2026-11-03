use crate::{http::Client, webauthn::Owner};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::process::Command;

const DUMMY_PATHS: [&str; 3] = [
    "/.well-known/oauth-protected-resource",
    "/.well-known/oauth-protected-resource/mcp",
    "/.well-known/oauth-authorization-server",
];
const CALLBACK: &str = "https://connector.example/callback";

fn fetch(
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Option<&[u8]>,
) -> Result<(u16, Vec<u8>)> {
    let mut command = Command::new("curl");
    command
        .args(["--silent", "--max-time", "10", "-X", method])
        .args(["-w", "%{http_code}"]);
    for (name, value) in headers {
        command.args(["-H", &format!("{name}: {value}")]);
    }
    if let Some(body) = body {
        command.args(["--data-binary", &String::from_utf8_lossy(body)]);
    }
    let output = command.arg(url).output()?;
    let split = output.stdout.len().saturating_sub(3);
    let status = std::str::from_utf8(&output.stdout[split..])
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    Ok((status, output.stdout[..split].to_vec()))
}
fn agent_token(world: &World, scope: &str) -> Option<String> {
    let raw = world
        .gateway_cli(&[
            "agent",
            "create",
            "--name",
            "Bridge Test grants",
            "--level",
            "approve",
            "--scope",
            scope,
        ])
        .ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    value["token"].as_str().map(str::to_owned)
}
fn consent_group(world: &World, base: &str, evidence: &mut Evidence) {
    let origin = base.replace("127.0.0.1", "localhost");
    let port = base.rsplit(':').next().unwrap_or_default();
    let enrolled = (|| -> Result<Owner> {
        let mut owner = Owner {
            authenticator: crate::webauthn::Authenticator::new()?,
            origin: format!("http://localhost:{port}"),
            cookie: None,
        };
        let output = world.gateway_cli(&["web", "enroll", "--host", &owner.origin])?;
        let link = output.split_whitespace().next().unwrap_or_default();
        let token = link
            .split_once('#')
            .map(|(_, fragment)| fragment.to_owned())
            .unwrap_or_default();
        let reply = owner.sign_in("enroll", &token, "trusted")?;
        if reply.status != 200 || owner.cookie.is_none() {
            return Err(lab_world::Error::Invalid("owner enrollment".into()));
        }
        Ok(owner)
    })();
    let Ok(owner) = enrolled else {
        evidence.check("consent_dummy_group", false, "owner enrollment failed");
        return;
    };
    let registration = serde_json::to_vec(&json!({
        "client_name": "Bridge Test grants",
        "redirect_uris": [CALLBACK],
        "token_endpoint_auth_method": "none"
    }))
    .unwrap_or_default();
    let client_id = fetch(
        "POST",
        &format!("{origin}/oauth/register"),
        &[
            ("Content-Type", "application/json".into()),
            ("Accept", "application/json".into()),
        ],
        Some(&registration),
    )
    .ok()
    .and_then(|(_, raw)| serde_json::from_slice::<Value>(&raw).ok())
    .and_then(|value| value["client_id"].as_str().map(str::to_owned))
    .unwrap_or_default();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest("a".repeat(43)));
    let query = [
        ("client_id", client_id.as_str()),
        ("redirect_uri", CALLBACK),
        ("response_type", "code"),
        ("resource", &format!("{}/mcp", owner.origin)),
        ("code_challenge_method", "S256"),
        ("code_challenge", &challenge),
        ("scope", "labdummy:read labdummy:write"),
    ]
    .iter()
    .map(|(key, value)| format!("{key}={}", encode(value)))
    .collect::<Vec<_>>()
    .join("&");
    let cookie = format!(
        "{}={}",
        crate::webauthn::COOKIE,
        owner.cookie.clone().unwrap_or_default()
    );
    let reply = fetch(
        "GET",
        &format!("{}/oauth/authorize?{query}&format=json", owner.origin),
        &[("Cookie", cookie)],
        None,
    );
    let found = reply.as_ref().is_ok_and(|(status, raw)| {
        *status == 200
            && serde_json::from_slice::<Value>(raw).is_ok_and(|page| {
                page["groups"].as_array().is_some_and(|groups| {
                    groups.iter().any(|group| {
                        group["source"] == "labdummy"
                            && group["grants"].as_array().is_some_and(|g| g.len() == 2)
                    })
                })
            })
    });
    evidence.check(
        "consent_dummy_group",
        found,
        "the OAuth consent page lists the dummy source with two grants",
    );
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
fn visibility(world: &World, base: &str, evidence: &mut Evidence) {
    for (scope, expected) in [
        ("messages:read", 0),
        ("labdummy:read", 1),
        ("labdummy:write", 1),
    ] {
        let token = agent_token(world, scope);
        let count = token.as_deref().and_then(|token| {
            Client::at(base.trim_start_matches("http://"), token)
                .call_raw("records.query", &json!({"collection": "labdummy.item"}), 10)
                .ok()
                .flatten()
        });
        evidence.check(
            &format!("dummy_visibility_{scope}"),
            count.is_some_and(|(status, page)| {
                status == 200 && page["records"].as_array().map_or(0, Vec::len) == expected
            }),
            "an agent sees dummy records only with a dummy grant",
        );
    }
}
fn cli_grants(world: &World, evidence: &mut Evidence) {
    for scope in ["labdummy:read", "labdummy:write", "labdummy:unknown"] {
        let created = world
            .gateway_cli(&[
                "agent",
                "create",
                "--name",
                "Bridge Test CLI grants",
                "--scope",
                scope,
            ])
            .is_ok();
        evidence.check(
            &format!("cli_grant_{scope}"),
            created == (scope != "labdummy:unknown"),
            "the CLI accepts known dummy grants and rejects unknown ones",
        );
    }
}
pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let address = world.runtime["api_address"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let base = format!("http://{address}");
    for path in DUMMY_PATHS {
        let reply = fetch("GET", &format!("{base}{path}"), &[], None)?;
        let text = String::from_utf8_lossy(&reply.1);
        evidence.check(
            &format!("dummy_grants_{path}"),
            reply.0 == 200 && text.contains("labdummy:read") && text.contains("labdummy:write"),
            "discovery metadata lists the dummy grants",
        );
    }
    let authority = world.gateway_cli(&["agent", "list"]).is_ok();
    evidence.check("grants_authority", authority, "the agent registry answers");
    if !authority {
        return Ok(());
    }
    consent_group(world, &base, evidence);
    let stored = crate::journey::records(&Client::at(
        &address,
        world
            .gateway_cli(&["token", "create", "--scope", "admin"])?
            .trim(),
    ))
    .is_ok();
    evidence.check("dummy_store", stored, "the gateway store is readable");
    if !stored {
        return Ok(());
    }
    let now = jiff::Timestamp::now().to_string();
    let captured = world
        .control(
            "/capture",
            &json!({
                "source": "labdummy",
                "generation": "lab-grants",
                "checkpoint": "1",
                "observations": [
                    {"kind": "upsert", "collection": "labdummy.item", "native_key": "fixture",
                     "observed_at": now, "data": {"title": "Bridge Test grants"}},
                    {"kind": "upsert", "collection": "system.source", "native_key": "labdummy",
                     "observed_at": now, "data": {"source": "labdummy", "state": "ready"}}
                ]
            }),
        )
        .is_ok();
    evidence.check(
        "dummy_record_captured",
        captured,
        "the dummy record was captured",
    );
    visibility(world, &base, evidence);
    let owner = world
        .gateway_cli(&["token", "create", "--scope", "admin"])
        .ok();
    evidence.check(
        "source_status_owner_token",
        owner.is_some(),
        "an owner token was issued",
    );
    let statuses = owner.as_deref().and_then(|token| {
        Client::at(&address, token.trim())
            .call_raw("sources.status", &json!({}), 10)
            .ok()
            .flatten()
    });
    let found = statuses.as_ref().is_some_and(|(_, page)| {
        page["records"].as_array().is_some_and(|records| {
            records.iter().any(|record| {
                record["source"] == "labdummy"
                    && record["data"]["grants"]
                        .as_array()
                        .is_some_and(|g| g.len() == 2)
            })
        })
    });
    evidence.check(
        "source_status_dummy_grants",
        statuses.is_some_and(|(status, _)| status == 200) && found,
        "source status reports the two dummy grants",
    );
    cli_grants(world, evidence);
    Ok(())
}
