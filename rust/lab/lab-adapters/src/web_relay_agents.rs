#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    http::{Client, random_id},
    journey::{command, effect_count, gateway_client},
    replicas::{self, Endpoint},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lab_relay::{Identity, peer_post};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

type Mutation = fn(&mut Value);
const SCOPE: &str = "reminders:write,system:read";

struct Owner {
    client: Client,
    origin: String,
    cookie: String,
}
impl Owner {
    fn request(&self, method: &str, path: &str, form: Option<&str>) -> Option<(u16, String)> {
        let cookie = format!("ab_session_local={}", self.cookie);
        let headers = [
            ("Origin", self.origin.as_str()),
            ("Cookie", cookie.as_str()),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ];
        let reply = self
            .client
            .request_with(false, method, path, &headers, form.map(str::as_bytes), 15)
            .ok()?;
        Some((
            reply.status,
            String::from_utf8_lossy(&reply.body).into_owned(),
        ))
    }
    fn decide(&self, id: &str, decision: &str) -> Option<u16> {
        self.request(
            "POST",
            &format!("/approvals/{id}"),
            Some(&format!("decision={decision}")),
        )
        .map(|(status, _)| status)
    }
    fn pending(&self, id: &str) -> bool {
        self.request("GET", "/approvals", None)
            .is_some_and(|(status, body)| {
                status == 200 && body.contains(&format!("/approvals/{id}"))
            })
    }
}
struct Agent {
    id: String,
    token: String,
}
fn credentials(home: &Path) -> Value {
    fs::read(home.join("credentials.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or(Value::Null)
}
fn create_agent(home: &Path, level: &str) -> Option<Agent> {
    let out = replicas::cli(
        home,
        &[
            "agent",
            "create",
            "--name",
            &format!("Bridge Test replica {level}"),
            "--level",
            level,
            "--scope",
            "reminders:write",
        ],
    )
    .ok()?;
    let value: Value = serde_json::from_str(&out).ok()?;
    Some(Agent {
        id: value["agent"]["id"].as_str()?.to_owned(),
        token: value["token"].as_str()?.to_owned(),
    })
}
fn call(endpoint: &Endpoint, token: &str, c: &Value) -> (u16, Value) {
    Client::at(&endpoint.address, token)
        .call_raw("reminders.create", c, 40)
        .ok()
        .flatten()
        .unwrap_or((0, Value::Null))
}
fn command_state(endpoint: &Endpoint, token: &str, id: &str) -> String {
    Client::at(&endpoint.address, token)
        .call_raw("commands.status", &json!({"command_id": id}), 10)
        .ok()
        .flatten()
        .and_then(|(_, v)| v["state"].as_str().map(str::to_owned))
        .unwrap_or_default()
}
fn stored_state(world: &World, id: &str) -> String {
    rusqlite::Connection::open_with_flags(
        world.gateway_home.join("ledger.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()
    .and_then(|db| {
        db.query_row("SELECT state FROM commands WHERE id=?", [id], |row| {
            row.get(0)
        })
        .ok()
    })
    .unwrap_or_default()
}
fn effects(world: &World, id: &str) -> u64 {
    effect_count(world, id).unwrap_or(u64::MAX)
}
fn command_hash(c: &Value) -> String {
    let args = serde_json::to_string(&c["args"]).unwrap_or_default();
    let op = serde_json::to_string(&c["op"]).unwrap_or_default();
    let text = format!(
        "{{\"Op\":{op},\"Args\":{args},\"Version\":0,\"Expires\":\"0001-01-01T00:00:00Z\",\"Era\":\"\"}}"
    );
    hex::encode(Sha256::digest(text.as_bytes()))
}
fn replica_connected(endpoint: &Endpoint, revision: Option<u64>) -> bool {
    endpoint
        .client()
        .call_raw("replica.status", &json!({}), 5)
        .ok()
        .flatten()
        .is_some_and(|(_, v)| {
            v["replica"]["state"] == "connected"
                && revision.is_none_or(|r| v["replica"]["cursor"]["subscription_revision"] == r)
        })
}
fn principal_revision(world: &World, id: &str) -> Option<u64> {
    let text = replicas::cli(&world.gateway_home, &["lab-principal", "--id", id]).ok()?;
    serde_json::from_str::<Value>(&text).ok()?["revision"].as_u64()
}

pub fn agents(world: &mut World, e: &mut Evidence) -> Result<()> {
    let seeded = world.control("/seed", &json!({"count": 3})).is_ok()
        && gateway_client(world).is_ok_and(|client| {
            await_until(10, || {
                client
                    .call(
                        "records.query",
                        &json!({"collection": "reminders.list"}),
                        10,
                    )
                    .ok()
                    .flatten()
                    .is_some_and(|v| v["records"].as_array().is_some_and(|r| !r.is_empty()))
            })
        });
    let started = if seeded {
        replicas::start_with(world, 1, SCOPE, 0)
    } else {
        Err(lab_world::Error::Invalid("seed".into()))
    };
    e.check("paired_replica_world", started.is_ok(), "");
    if started.is_err() {
        return Ok(());
    }
    let gateway_identity = Identity::load(&world.gateway_home);
    if gateway_identity.is_err() {
        e.check("gateway_authority", false, "");
    }
    let Ok(gateway_identity) = gateway_identity else {
        return Ok(());
    };
    let endpoint = replicas::endpoints().remove(0);
    let replica = Identity::load(&endpoint.home);
    if replica.is_err() {
        e.check("replica_authority", false, "");
    }
    let Ok(replica) = replica else {
        return Ok(());
    };
    let session = replicas::cli(&world.gateway_home, &["lab-session"])
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v["value"].as_str().map(str::to_owned));
    if session.is_none() {
        e.check("owner_session", false, "");
    }
    let Some(session) = session else {
        return Ok(());
    };
    let api = world.runtime["api_address"].as_str().unwrap_or_default();
    let port = api.rsplit(':').next().unwrap_or_default();
    let owner = Owner {
        client: Client::at(&format!("localhost:{port}"), ""),
        origin: format!("http://localhost:{port}"),
        cookie: session,
    };
    let mut list = None;
    await_until(20, || {
        list = world_lists(&endpoint);
        list.is_some()
    });
    if list.is_none() {
        e.check("bridge_test_list", false, "");
    }
    let Some(list) = list else {
        return Ok(());
    };
    let run = random_id("relay-agents")?;
    let new_command = || {
        command(
            "reminders.create",
            &json!({"list_id": list, "title": format!("[ab-test {run}] delegated agent")}),
        )
    };
    let create = |level: &str, e: &mut Evidence| {
        let agent = create_agent(&endpoint.home, level);
        e.check(&format!("create_replica_{level}"), agent.is_some(), "");
        agent.unwrap_or(Agent {
            id: String::new(),
            token: String::new(),
        })
    };
    let read = create("read", e);
    let denied = new_command()?;
    let (status, _) = call(&endpoint, &read.token, &denied);
    let id = denied["command_id"].as_str().unwrap_or_default();
    e.check(
        "replica_read_agent_denied",
        status == 401 && effects(world, id) == 0,
        "",
    );
    let full = create("full", e);
    delegation(
        world,
        e,
        &endpoint,
        &replica,
        &gateway_identity.fingerprint,
        &full.id,
    );
    let immediate = new_command()?;
    let (status, record) = call(&endpoint, &full.token, &immediate);
    e.check(
        "replica_full_agent_forwarded",
        status == 200 && record["state"] == "observed",
        "",
    );
    let approve = create("approve", e);
    let pending = new_command()?;
    let pending_id = pending["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let (status, record) = call(&endpoint, &approve.token, &pending);
    let queued = status == 200 && record["state"] == "awaiting_approval";
    e.check("replica_approve_agent_queued_on_gateway", queued, "");
    if !queued {
        return Ok(());
    }
    e.check(
        "delegated_agent_pending_approval_page",
        owner.pending(&pending_id),
        "",
    );
    e.check(
        "no_effect_before_gateway_owner_consent",
        effects(world, &pending_id) == 0,
        "",
    );
    e.check(
        "gateway_owner_approves_replica_agent",
        owner.decide(&pending_id, "approve") == Some(200),
        "",
    );
    let executed = await_until(5, || {
        command_state(&endpoint, &approve.token, &pending_id) == "observed"
    });
    e.check(
        "gateway_approval_executes_once",
        executed && effects(world, &pending_id) == 1,
        "",
    );
    approvals(world, e, &owner, &endpoint, &new_command);
    Ok(())
}

fn delegation(
    world: &World,
    e: &mut Evidence,
    endpoint: &Endpoint,
    replica: &Identity,
    pin: &str,
    agent_id: &str,
) {
    let creds = credentials(&endpoint.home);
    let agent = creds["principals"]
        .as_array()
        .and_then(|list| list.iter().find(|p| p["id"] == agent_id));
    let replica_id = creds["enrollment"]["replica_id"].as_str();
    let (Some(agent), Some(replica_id)) = (agent, replica_id) else {
        e.check("delegation_boundary_fixture", false, "");
        return;
    };
    let peer = world.runtime["peer_address"].as_str().unwrap_or_default();
    let cases: [(&str, Mutation, bool); 6] = [
        (
            "tampered_level",
            |d| d["local_level"] = json!("approve"),
            false,
        ),
        ("stale_signature", |d| d["iat"] = json!(now() - 61), true),
        ("future_signature", |d| d["iat"] = json!(now() + 3600), true),
        ("invalid_level", |d| d["local_level"] = json!("owner"), true),
        (
            "read_level_write_scope",
            |d| d["local_level"] = json!("read"),
            true,
        ),
        (
            "scope_escalation",
            |d| d["local_grants"] = json!(["admin"]),
            true,
        ),
    ];
    let list_id = world_lists(endpoint).unwrap_or_default();
    for (name, change, resign) in cases {
        let c = command(
            "reminders.create",
            &json!({"list_id": list_id, "title": format!("[ab-test] delegation {name}")}),
        );
        let Ok(c) = c else {
            e.check(name, false, "");
            continue;
        };
        let id = c["command_id"].as_str().unwrap_or_default().to_owned();
        let mut d = json!({
            "local_level": agent["level"],
            "replica_id": replica_id,
            "local_principal": agent_id,
            "local_grants": agent["grants"],
            "command_id": id,
            "request_hash": command_hash(&c),
            "iat": now(),
        });
        let original = serde_json::to_vec(&d).unwrap_or_default();
        let signature = replica.sign(&original);
        change(&mut d);
        let raw = serde_json::to_vec(&d).unwrap_or_default();
        let signature = if resign {
            replica.sign(&raw)
        } else {
            signature
        };
        let header = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&raw),
            URL_SAFE_NO_PAD.encode(signature)
        );
        let body =
            serde_json::to_vec(&json!({"method": "submit", "command": c})).unwrap_or_default();
        let reply = peer_post(
            peer,
            replica,
            pin,
            "/v1/commands",
            &[("X-Apple-Bridge-Delegation", &header)],
            &body,
        );
        let code = reply
            .ok()
            .and_then(|r| serde_json::from_slice::<Value>(&r.body).ok())
            .and_then(|v| v["error"]["code"].as_str().map(str::to_owned));
        e.check(
            &format!("delegated_agent_{name}_rejected"),
            code.as_deref() == Some("unauthorized") && effects(world, &id) == 0,
            "",
        );
    }
}
fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}
fn world_lists(endpoint: &Endpoint) -> Option<String> {
    let value = endpoint
        .client()
        .call(
            "records.query",
            &json!({"collection": "reminders.list"}),
            10,
        )
        .ok()
        .flatten()?;
    let records = value["records"].as_array().filter(|r| r.len() == 1)?;
    records[0]["id"].as_str().map(str::to_owned)
}

fn approvals(
    world: &World,
    e: &mut Evidence,
    owner: &Owner,
    endpoint: &Endpoint,
    new_command: &dyn Fn() -> Result<Value>,
) {
    let home = &endpoint.home;
    let Some(revoked) = create_agent(home, "approve") else {
        e.check("create_replica_approve", false, "");
        return;
    };
    e.check("create_replica_approve", true, "");
    let Ok(c) = new_command() else {
        return;
    };
    let id = c["command_id"].as_str().unwrap_or_default().to_owned();
    let (status, row) = call(endpoint, &revoked.token, &c);
    e.check(
        "revocation_fixture_pending",
        status == 200 && row["state"] == "awaiting_approval",
        "",
    );
    let revoked_locally = replicas::cli(home, &["agent", "revoke", &revoked.id]).is_ok();
    e.check("replica_agent_revoked_locally", revoked_locally, "");
    let status = owner.decide(&id, "approve");
    let state = stored_state(world, &id);
    e.check(
        "fresh_replica_revocation_rejects_pending_approval",
        status == Some(200) && state == "rejected" && effects(world, &id) == 0,
        &format!("{status:?} {state} {}", effects(world, &id)),
    );
    let Some(agent) = create_agent(home, "approve") else {
        e.check("create_replica_approve", false, "");
        return;
    };
    e.check("create_replica_approve", true, "");
    let Ok(offline) = new_command() else {
        return;
    };
    let offline_id = offline["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let (status, row) = call(endpoint, &agent.token, &offline);
    e.check(
        "offline_fixture_pending",
        status == 200 && row["state"] == "awaiting_approval",
        "",
    );
    let _ = replicas::crash(0);
    let status = owner.decide(&offline_id, "approve");
    e.check(
        "offline_replica_keeps_approval_pending_without_effect",
        status == Some(409) && owner.pending(&offline_id) && effects(world, &offline_id) == 0,
        "",
    );
    let restarted = replicas::restart(0);
    e.check(
        "replica_reconnects_for_fresh_approval",
        restarted.is_ok(),
        "",
    );
    if restarted.is_err() {
        return;
    }
    e.check(
        "replica_feed_reconnected",
        await_until(5, || replica_connected(endpoint, None)),
        "",
    );
    e.check(
        "owner_approves_after_replica_reconnect",
        owner.decide(&offline_id, "approve") == Some(200),
        "",
    );
    let executed = await_until(5, || {
        command_state(endpoint, &agent.token, &offline_id) == "observed"
    });
    e.check(
        "reconnected_approval_executes_once",
        executed && effects(world, &offline_id) == 1,
        "",
    );
    parent(world, e, owner, endpoint, &agent, new_command);
}

fn parent(
    world: &World,
    e: &mut Evidence,
    owner: &Owner,
    endpoint: &Endpoint,
    agent: &Agent,
    new_command: &dyn Fn() -> Result<Value>,
) {
    let creds = credentials(&endpoint.home);
    let Some(replica_id) = creds["enrollment"]["replica_id"].as_str() else {
        e.check("parent_scope_fixture", false, "");
        return;
    };
    let (Ok(scoped), Ok(revoked)) = (new_command(), new_command()) else {
        return;
    };
    let scoped_id = scoped["command_id"].as_str().unwrap_or_default().to_owned();
    let revoked_id = revoked["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let (status, row) = call(endpoint, &agent.token, &scoped);
    e.check(
        "scope_fixture_pending",
        status == 200 && row["state"] == "awaiting_approval",
        "",
    );
    let (status, row) = call(endpoint, &agent.token, &revoked);
    e.check(
        "parent_revocation_fixture_pending",
        status == 200 && row["state"] == "awaiting_approval",
        "",
    );
    let narrowed = replicas::cli(
        &world.gateway_home,
        &["scope", replica_id, "--scope", "reminders:read,system:read"],
    )
    .is_ok();
    e.check("parent_scope_narrowed", narrowed, "");
    let Some(revision) = principal_revision(world, replica_id) else {
        e.check("updated_parent_principal", false, "");
        return;
    };
    e.check(
        "narrowed_subscription_connected",
        await_until(5, || replica_connected(endpoint, Some(revision))),
        "",
    );
    let status = owner.decide(&scoped_id, "approve");
    e.check(
        "fresh_parent_scope_rejects_pending_write",
        status == Some(200)
            && stored_state(world, &scoped_id) == "rejected"
            && effects(world, &scoped_id) == 0,
        "",
    );
    let revoked_parent = replicas::cli(&world.gateway_home, &["revoke", replica_id]).is_ok();
    e.check("parent_replica_revoked", revoked_parent, "");
    let status = owner.decide(&revoked_id, "approve");
    e.check(
        "revoked_parent_rejects_pending_agent_approval",
        status == Some(200)
            && stored_state(world, &revoked_id) == "rejected"
            && effects(world, &revoked_id) == 0,
        "",
    );
}
