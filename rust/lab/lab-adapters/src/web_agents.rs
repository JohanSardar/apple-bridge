#![allow(clippy::too_many_lines)]
use crate::{
    http::{Client, random_id},
    replicas,
    webauthn::Owner,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, time::SystemTime};

pub const SCENARIOS: [&str; 5] = [
    "WEB-AGENT-CREDENTIALS",
    "WEB-AGENT-LEVELS",
    "WEB-AGENT-ACTIVITY",
    "WEB-AGENT-OAUTH",
    "WEB-AGENT-OAUTH-CIMD",
];
const REDIRECT: &str = "https://connector.example/callback";

pub const SCOPE: &str = "messages:read,messages:write,system:read";

pub fn start(world: &World, count: usize) -> Result<()> {
    replicas::start_with(world, count, SCOPE, 0)
}

pub fn web(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "WEB-AGENT-CREDENTIALS" => credentials(world, evidence),
        "WEB-AGENT-LEVELS" => session_run(world, evidence, crate::web_agent_levels::levels),
        "WEB-AGENT-ACTIVITY" => crate::web_agent_activity::activity(world, evidence),
        "WEB-AGENT-OAUTH" => session_run(world, evidence, crate::web_agent_oauth::oauth),
        "WEB-AGENT-OAUTH-CIMD" => crate::web_agent_cimd::cimd(world, evidence),
        id => Err(Error::Invalid(format!(
            "web agent scenario not ported: {id}"
        ))),
    }
}

type SessionRun = fn(&mut World, &Owner, &mut Evidence) -> Result<()>;

fn session_run(world: &mut World, evidence: &mut Evidence, run: SessionRun) -> Result<()> {
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("authority", false, "agent list");
        return Ok(());
    }
    let Ok(owner) = owner_session(world, None) else {
        evidence.check("owner_session", false, "owner enrollment");
        return Ok(());
    };
    run(world, &owner, evidence)
}

pub(crate) fn address(world: &World) -> Result<String> {
    world.runtime["api_address"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid("runtime API address".into()))
}

pub(crate) fn post(world: &World, token: &str, path: &str, body: &Value) -> Result<(u16, Value)> {
    let client = Client::with_token(world, token)?;
    let reply = client.request(
        "POST",
        path,
        &[("Content-Type", "application/json")],
        Some(&serde_json::to_vec(body)?),
        15,
    )?;
    Ok((
        reply.status,
        serde_json::from_slice(&reply.body).unwrap_or(Value::Null),
    ))
}

pub(crate) fn anonymous(
    world: &World,
    path: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(u16, Value)> {
    let client = Client::with_token(world, "")?;
    let reply = client.request_with(
        false,
        "POST",
        path,
        &[("Content-Type", content_type)],
        Some(body),
        15,
    )?;
    Ok((
        reply.status,
        serde_json::from_slice(&reply.body).unwrap_or(Value::Null),
    ))
}

pub(crate) fn chats_status(world: &World, token: &str) -> u16 {
    post(world, token, "/v1/op/messages.chats", &json!({})).map_or(0, |(status, _)| status)
}

pub(crate) fn cli_json(world: &World, args: &[&str]) -> Option<Value> {
    serde_json::from_str(&world.gateway_cli(args).ok()?).ok()
}

pub(crate) fn create_agent(
    world: &World,
    name: &str,
    level: &str,
    scope: &str,
) -> Option<(String, String, Value)> {
    let created = cli_json(
        world,
        &[
            "agent", "create", "--name", name, "--level", level, "--scope", scope,
        ],
    )?;
    let id = created["agent"]["id"].as_str()?.to_owned();
    let token = created["token"].as_str()?.to_owned();
    if id.is_empty() || token.is_empty() {
        return None;
    }
    Some((id, token, created))
}

pub(crate) fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

pub(crate) fn edit_credentials(world: &World, edit: impl FnOnce(&mut Value)) -> Result<()> {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(world.gateway_home.join("credentials.lock"))?;
    lock.lock()?;
    let path = world.gateway_home.join("credentials.json");
    let mut data: Value = serde_json::from_slice(&fs::read(&path)?)?;
    edit(&mut data);
    let staged = world.gateway_home.join("credentials.json.agent-expiry");
    fs::write(&staged, serde_json::to_vec(&data)?)?;
    fs::rename(&staged, &path)?;
    Ok(())
}

pub(crate) fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

pub(crate) fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub(crate) fn owner_session(world: &World, origin: Option<&str>) -> Result<Owner> {
    let mut owner = Owner::at(&address(world)?)?;
    let output = world.gateway_cli(&["web", "enroll", "--host", &owner.origin])?;
    let token = output
        .split_whitespace()
        .next()
        .and_then(|link| link.split_once('#'))
        .map(|(_, fragment)| fragment.to_owned())
        .ok_or_else(|| Error::Invalid("enrollment link".into()))?;
    let reply = owner.sign_in("enroll", &token, "trusted")?;
    if reply.status != 200 || owner.cookie.is_none() {
        return Err(Error::Invalid(format!(
            "owner enrollment HTTP {}",
            reply.status
        )));
    }
    if let Some(origin) = origin {
        origin.clone_into(&mut owner.origin);
    }
    Ok(owner)
}

struct Grants<'a> {
    world: &'a World,
    owner: Owner,
}

impl Grants<'_> {
    fn register(&self) -> Result<String> {
        let body = json!({
            "client_name": "Bridge Test connector",
            "redirect_uris": [REDIRECT],
            "grant_types": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_method": "none"
        });
        let (status, value) = anonymous(
            self.world,
            "/oauth/register",
            "application/json",
            &serde_json::to_vec(&body)?,
        )?;
        let id = value["client_id"].as_str().unwrap_or_default().to_owned();
        if status != 201 || id.is_empty() {
            return Err(Error::Invalid(format!("OAuth registration HTTP {status}")));
        }
        Ok(id)
    }

    fn issue(&self, client: &str, challenge: &str, resource: &str) -> Result<String> {
        let query = form(&[
            ("client_id", client),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT),
            ("scope", "messages:read"),
            ("resource", resource),
            ("state", "bridge-test-grant"),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
        ]);
        let body = json!({"decision": "approve", "level": "read", "scopes": ["messages:read"]});
        let reply = self.owner.request(
            "POST",
            &format!("/oauth/authorize?{query}"),
            "application/json",
            &serde_json::to_vec(&body)?,
        )?;
        if reply.status != 200 {
            return Err(Error::Invalid(format!(
                "OAuth consent HTTP {}",
                reply.status
            )));
        }
        reply.json()["redirect"]
            .as_str()
            .and_then(|link| link.split_once('?'))
            .and_then(|(_, query)| query.split('&').find_map(|pair| pair.strip_prefix("code=")))
            .filter(|code| !code.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| Error::Invalid("OAuth code missing".into()))
    }

    fn exchange(&self, fields: &[(&str, &str)]) -> Result<Value> {
        let (status, value) = anonymous(
            self.world,
            "/oauth/token",
            "application/x-www-form-urlencoded",
            form(fields).as_bytes(),
        )?;
        if status != 200 {
            return Err(Error::Invalid(format!("OAuth token HTTP {status}")));
        }
        Ok(value)
    }

    fn code_grant(
        &self,
        client: &str,
        code: &str,
        verifier: &str,
        resource: &str,
    ) -> Result<Value> {
        self.exchange(&[
            ("grant_type", "authorization_code"),
            ("client_id", client),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
            ("resource", resource),
            ("refresh_token", ""),
            ("scope", ""),
        ])
    }

    fn refresh(&self, client: &str, refresh: &str, resource: &str) -> Result<Value> {
        self.exchange(&[
            ("grant_type", "refresh_token"),
            ("client_id", client),
            ("code", ""),
            ("redirect_uri", ""),
            ("code_verifier", ""),
            ("resource", resource),
            ("refresh_token", refresh),
            ("scope", ""),
        ])
    }
}

pub(crate) fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

pub(crate) fn seconds_until(value: &Value) -> i64 {
    value["expires_at"]
        .as_str()
        .and_then(|stamp| stamp.parse::<jiff::Timestamp>().ok())
        .map_or(i64::MIN, |stamp| {
            stamp.as_second() - jiff::Timestamp::now().as_second()
        })
}

fn oauth_grants(world: &World, evidence: &mut Evidence) -> Result<()> {
    let origin = format!("http://{}", address(world)?);
    let Ok(owner) = owner_session(world, Some(&origin)) else {
        evidence.check("oauth_grant_client_registration", false, "owner session");
        return Ok(());
    };
    let grants = Grants { world, owner };
    let client = grants.register();
    evidence.check(
        "oauth_grant_client_registration",
        client.is_ok(),
        "dynamic client registered",
    );
    let Ok(client) = client else {
        return Ok(());
    };
    let verifier = "a".repeat(43);
    let wrong = "b".repeat(43);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let resource = format!("{origin}/mcp");
    let issue = |evidence: &mut Evidence| -> Result<(Value, String)> {
        let code = grants.issue(&client, &challenge, &resource)?;
        let bad = grants.code_grant(&client, &code, &wrong, &resource);
        evidence.check(
            "oauth_grant_wrong_pkce_rejected",
            bad.is_err(),
            "wrong verifier refused",
        );
        let tokens = grants.code_grant(&client, &code, &verifier, &resource)?;
        Ok((tokens, code))
    };
    let first = issue(evidence);
    let tokens = first.as_ref().map(|(tokens, _)| tokens.clone());
    evidence.check(
        "oauth_grant_exchanged_and_readable",
        tokens.as_ref().is_ok_and(|t| {
            t["expires_in"] == 3600 && chats_status(world, text(t, "access_token")) == 200
        }),
        "code exchanged and token reads",
    );
    let Ok(tokens) = tokens else {
        return Ok(());
    };
    let access = text(&tokens, "access_token").to_owned();
    let refresh_token = text(&tokens, "refresh_token").to_owned();
    let stored = fs::read_to_string(world.gateway_home.join("credentials.json"))?;
    let data: Value = serde_json::from_str(&stored).unwrap_or(Value::Null);
    let find = |list: &str, token: &str| {
        data[list]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| text(entry, "hash") == token_hash(token))
            .cloned()
    };
    let access_ttl = find("oauth_access", &access).is_some_and(|v| {
        let left = seconds_until(&v);
        left > 59 * 60 && left <= 3600
    });
    let refresh_ttl = find("oauth_refresh", &refresh_token).is_some_and(|v| {
        let left = seconds_until(&v);
        left > 89 * 86400 && left <= 90 * 86400
    });
    evidence.check(
        "oauth_grant_lifetimes",
        access_ttl && refresh_ttl,
        "access one hour, refresh ninety days",
    );
    evidence.check(
        "oauth_grant_secrets_hashed",
        !stored.contains(&access) && !stored.contains(&refresh_token),
        "no raw OAuth secret on disk",
    );
    let rotated = grants.refresh(&client, &refresh_token, &resource);
    evidence.check(
        "oauth_grant_refresh_rotates",
        rotated.as_ref().is_ok_and(|r| {
            text(r, "refresh_token") != refresh_token
                && text(r, "access_token") != access
                && chats_status(world, text(r, "access_token")) == 200
        }),
        "refresh rotates both tokens",
    );
    let rotated_access = rotated
        .as_ref()
        .map(|r| text(r, "access_token").to_owned())
        .unwrap_or_default();
    let retry = grants.refresh(&client, &refresh_token, &resource);
    evidence.check(
        "oauth_grant_refresh_retry_returns_saved_successor",
        rotated.as_ref().is_ok_and(|r| {
            retry.as_ref().is_ok_and(|again| {
                text(again, "refresh_token") == text(r, "refresh_token")
                    && text(again, "access_token") == rotated_access
            })
        }) && chats_status(world, &rotated_access) == 200,
        "a retry inside the window returns the one saved successor",
    );
    let aged = edit_credentials(world, |data| {
        for row in data["oauth_refresh"].as_array_mut().into_iter().flatten() {
            if row["used"] == true {
                row["rotated_at"] = json!(1);
            }
        }
    });
    let reuse = aged.and_then(|()| grants.refresh(&client, &refresh_token, &resource));
    evidence.check(
        "oauth_grant_refresh_reuse_revokes",
        reuse.is_err() && chats_status(world, &rotated_access) == 401,
        "reused refresh revokes the family",
    );
    let rotated_refresh = rotated
        .as_ref()
        .map(|r| text(r, "refresh_token").to_owned())
        .unwrap_or_default();
    evidence.check(
        "oauth_grant_refresh_family_revoked",
        grants
            .refresh(&client, &rotated_refresh, &resource)
            .is_err(),
        "rotated refresh refused",
    );
    let second = issue(evidence);
    let replay = second
        .as_ref()
        .map(|(_, code)| grants.code_grant(&client, code, &verifier, &resource));
    evidence.check(
        "oauth_grant_code_reuse_revokes",
        second.as_ref().is_ok_and(|(tokens, _)| {
            replay.as_ref().is_ok_and(Result::is_err)
                && chats_status(world, text(tokens, "access_token")) == 401
        }),
        "reused code revokes its tokens",
    );
    let third = issue(evidence);
    let revoked = third.as_ref().is_ok_and(|(tokens, _)| {
        let access = text(tokens, "access_token");
        let principal = (chats_status(world, access) == 200)
            .then(|| {
                let stored = fs::read(world.gateway_home.join("credentials.json")).ok()?;
                let data: Value = serde_json::from_slice(&stored).ok()?;
                data["oauth_access"].as_array()?.iter().find_map(|entry| {
                    (text(entry, "hash") == token_hash(access)
                        && !text(entry, "principal").is_empty())
                    .then(|| text(entry, "principal").to_owned())
                })
            })
            .flatten();
        principal.is_some_and(|principal| {
            world.gateway_cli(&["agent", "revoke", &principal]).is_ok()
                && chats_status(world, access) == 401
        })
    });
    evidence.check(
        "oauth_grant_owner_revocation",
        revoked,
        "owner revocation stops the OAuth token",
    );
    Ok(())
}

fn oauth_credential_access(world: &World, evidence: &mut Evidence) -> Result<()> {
    let Some((agent, token, _)) = create_agent(
        world,
        "Bridge Test OAuth credential",
        "read",
        "messages:read",
    ) else {
        evidence.check("oauth_credential_agent", false, "agent create");
        return Ok(());
    };
    let resource = format!("http://{}/mcp", address(world)?);
    let expiry = |offset: i64| {
        (jiff::Timestamp::now() + jiff::SignedDuration::from_secs(offset)).to_string()
    };
    let edited = edit_credentials(world, |data| {
        if let Some(list) = data["principals"].as_array_mut() {
            for principal in list.iter_mut().filter(|p| p["id"] == agent.as_str()) {
                principal["token_hash"] = json!("");
            }
        }
        data["oauth_access"] = json!([{
            "hash": token_hash(&token),
            "principal": agent,
            "resource": resource,
            "expires_at": expiry(3600)
        }]);
    });
    let status = chats_status(world, &token);
    evidence.check(
        "oauth_opaque_access_authenticates",
        edited.is_ok() && status == 200,
        "opaque access token authenticates",
    );
    if status != 200 {
        return Ok(());
    }
    let change = |resource: &str, offset: i64| {
        edit_credentials(world, |data| {
            data["oauth_access"][0]["resource"] = json!(resource);
            data["oauth_access"][0]["expires_at"] = json!(expiry(offset));
        })
    };
    let edited = change("https://attacker.example/mcp", 3600);
    evidence.check(
        "oauth_opaque_resource_bound",
        edited.is_ok() && chats_status(world, &token) == 401,
        "foreign resource refused",
    );
    let edited = change(&resource, -1);
    evidence.check(
        "oauth_opaque_access_expires",
        edited.is_ok() && chats_status(world, &token) == 401,
        "expired access refused",
    );
    oauth_grants(world, evidence)
}

fn credentials(world: &World, evidence: &mut Evidence) -> Result<()> {
    let path = world.gateway_home.join("credentials.json");
    let before = modified(&path);
    let read = world.query("messages.chats", &json!({}));
    let after = modified(&path);
    evidence.check(
        "legacy_token_auth_stays_read_only",
        read.is_ok() && before.is_some() && before == after,
        "legacy token read leaves credentials untouched",
    );
    for level in ["read", "", "full"] {
        let mut args = vec![
            "agent",
            "create",
            "--name",
            "Bridge Test agent",
            "--scope",
            "messages:write",
        ];
        if !level.is_empty() {
            args.extend(["--level", level]);
        }
        let expected = if level.is_empty() { "approve" } else { level };
        let created = cli_json(world, &args).unwrap_or(Value::Null);
        let id = created["agent"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let token = text(&created, "token").to_owned();
        evidence.check(
            &format!("create_{expected}"),
            !id.is_empty() && !token.is_empty() && created["agent"]["level"] == expected,
            "agent created at the requested level",
        );
        if token.is_empty() {
            return Ok(());
        }
        for _ in 0..3 {
            evidence.check(
                "scoped_read",
                chats_status(world, &token) == 200,
                "agent token reads",
            );
        }
        let stored = fs::read_to_string(&path).unwrap_or_default();
        evidence.check(
            "token_stored_hashed",
            !stored.contains(&token),
            "token not stored raw",
        );
        if level == "read" {
            let scopes = created["agent"]["scopes"].as_array();
            evidence.check(
                "read_scope_normalized",
                scopes.is_some_and(|s| s.len() == 1 && s[0] == "messages:read"),
                "read level narrows scope",
            );
        }
        if expected != "full" {
            let command = random_id("cmd")?;
            let sent = post(
                world,
                &token,
                "/v1/op/messages.send",
                &json!({"command_id": command, "args": {"chat_id": "BridgeTest", "text": "[ab-test credentials] approval required"}}),
            );
            let effects = world.control("/effects", &Value::Null);
            evidence.check(
                "no_unapproved_effect",
                effects.is_ok_and(|e| e[&command].as_u64().unwrap_or(0) == 0)
                    && sent.is_ok_and(|(status, body)| {
                        status == 401 || status == 200 && body["state"] == "awaiting_approval"
                    }),
                "no native effect before approval",
            );
        }
        let listed = world.gateway_cli(&["agent", "list"]);
        evidence.check(
            "list_is_redacted",
            listed.is_ok_and(|l| {
                l.contains(&id)
                    && !l.contains(&token)
                    && !l.contains("token_hash")
                    && l.contains("last_used")
            }),
            "listing hides secrets",
        );
        let _ = world.gateway_cli(&["agent", "revoke", &id]);
        evidence.check(
            "revoke_stops_access",
            chats_status(world, &token) == 401,
            "revoked token refused",
        );
        let listed = world.gateway_cli(&["agent", "list"]);
        evidence.check(
            "revoke_time_retained",
            listed.is_ok_and(|l| l.contains("revoked_at")),
            "revocation time kept",
        );
    }
    for args in [
        vec!["--level", "invalid", "--scope", "messages:read"],
        vec!["--scope", "admin"],
    ] {
        let mut command = vec!["agent", "create", "--name", "Bridge Test invalid"];
        command.extend(args);
        evidence.check(
            "invalid_agent_refused",
            world.gateway_cli(&command).is_err(),
            "invalid agent refused",
        );
    }
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("owner_authority", false, "agent list");
        return Ok(());
    }
    oauth_credential_access(world, evidence)
}
