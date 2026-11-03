#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value,
    clippy::ref_option,
    clippy::semicolon_if_nothing_returned,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::unnecessary_wraps
)]
use crate::{
    assertions::{
        Observed, Visibility, await_until, gateway_aliases, read_all, record_integrity,
        record_multiset, truth_records,
    },
    http::{Client, random_id},
    journey::observed as command_observed,
    replicas::{self, Endpoint},
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{cell::Cell, collections::BTreeSet, time::Instant};

pub const SCENARIOS: [&str; 16] = [
    "CON-CONFLICT",
    "CON-CRUD",
    "CON-DROP-EVERYTHING",
    "CON-EXTERNAL",
    "CON-GROUPS",
    "CON-HANDLE-LINK",
    "CON-IMAGE",
    "CON-PERMISSION-LOSS",
    "CON-READONLY-CONTAINER",
    "CON-RESOLVE-RESTART",
    "CON-SEARCH",
    "CON-TOKEN-CRASH",
    "CON-ME",
    "CON-NOTE",
    "CON-RELAY",
    "CON-RECIPIENT-LOOKUP",
];
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII=";
const COLLECTIONS: [&str; 3] = ["contacts.contact", "contacts.group", "messages.handle"];

#[must_use]
pub fn scope(id: &str) -> &'static str {
    if id == "CON-HANDLE-LINK" {
        "contacts:write,system:read,messages:read"
    } else {
        "contacts:write,system:read"
    }
}
fn base64_decode(text: &str) -> Vec<u8> {
    let mut out = vec![];
    let (mut buffer, mut bits) = (0u32, 0);
    for byte in text.bytes().filter(|b| *b != b'=') {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).unwrap_or(0));
        }
    }
    out
}
fn add(evidence: &mut Evidence, name: &str, ok: bool) {
    evidence.check(name, ok, "");
}
fn failed(reply: &Option<(u16, Value)>) -> bool {
    reply
        .as_ref()
        .is_none_or(|(status, value)| *status != 200 || !value["error"].is_null())
}
fn contact_records(client: &Client) -> Vec<Value> {
    read_all(client, false, false).map_or_else(
        |_| vec![],
        |read| {
            read.records
                .into_iter()
                .filter(|r| COLLECTIONS.contains(&r["collection"].as_str().unwrap_or_default()))
                .collect()
        },
    )
}
fn contact_records_checked(client: &Client) -> Option<Vec<Value>> {
    let read = read_all(client, false, false).ok()?;
    Some(
        read.records
            .into_iter()
            .filter(|r| COLLECTIONS.contains(&r["collection"].as_str().unwrap_or_default()))
            .collect(),
    )
}
struct Run {
    tag: String,
    name: String,
    synthetic: Value,
    clients: Vec<Client>,
    replica: Option<Endpoint>,
    scope: &'static str,
}
impl Run {
    fn find(&self, client: &Client) -> Option<Value> {
        contact_records(client).into_iter().find(|r| {
            r["collection"] == "contacts.contact"
                && r["data"]["given_name"] == self.name.as_str()
                && r["data"]["organization"] == "Bridge Test"
        })
    }
    fn fixture(&self, world: &World, action: &str, fields: &Value) -> bool {
        let action = if action == "modify" { "update" } else { action };
        let mut request = json!({"action": action, "key": self.tag, "data": fields});
        if action == "create" {
            request["data"] = self.synthetic.clone();
        }
        if action == "readonly" {
            request["action"] = json!("create");
            request["data"] = self.synthetic.clone();
            request["readonly"] = json!(true);
        }
        world.control("/contacts", &request).is_ok()
    }
    fn submit(
        &self,
        id: &str,
        protocol: &str,
        operation: &str,
        args: &Value,
        version: u64,
    ) -> Result<(Value, Option<(u16, Value)>)> {
        let client = if id == "CON-CRUD" {
            self.replica
                .as_ref()
                .map(Endpoint::client)
                .ok_or_else(|| Error::Invalid("no replica".into()))?
        } else {
            self.clients[0].clone()
        };
        let command = json!({
            "command_id": random_id("cmd")?, "op": operation, "args": args,
            "expected_version": version, "wait_ms": 15000
        });
        let reply = if protocol == "rest" {
            client.call_raw(operation, &command, 30)?
        } else {
            client.call_mcp_raw(operation, &command, 30)?
        };
        Ok((command, reply))
    }
    fn truth_converged(&self, world: &World) -> bool {
        let Ok(truth) = world.control("/inspect", &Value::Null) else {
            return false;
        };
        let Ok(aliases) = gateway_aliases(&self.clients[0]) else {
            return false;
        };
        let filtered: Vec<Value> = truth
            .as_array()
            .into_iter()
            .flatten()
            .filter(|o| {
                matches!(
                    o["collection"]
                        .as_str()
                        .and_then(|c| c.split('.').next())
                        .unwrap_or_default(),
                    "contacts" | "messages"
                )
            })
            .cloned()
            .collect();
        let Ok(expected) = truth_records(
            &Value::Array(filtered),
            &aliases,
            &Visibility::from_scope(self.scope),
        ) else {
            return false;
        };
        let mut actual = contact_records(&self.clients[0]);
        for record in &mut actual {
            if record["collection"] == "messages.handle"
                && let Some(data) = record["data"].as_object_mut()
            {
                data.remove("contact_id");
            }
        }
        record_multiset(&actual, false) == record_multiset(&expected, false)
    }
}
fn endpoints(world: &World, gateway: &Client) -> Vec<Client> {
    let _ = world;
    std::iter::once(gateway.clone())
        .chain(replicas::endpoints().iter().map(Endpoint::client))
        .collect()
}
fn scoped_token(world: &World, index: usize, scope: &str) -> Result<Client> {
    if index == 0 {
        Client::new(world, scope)
    } else {
        let endpoint = replicas::endpoints()
            .get(index - 1)
            .cloned()
            .ok_or_else(|| Error::Invalid("replica index".into()))?;
        let token = replicas::cli(&endpoint.home, &["token", "create", "--scope", scope])?;
        Ok(Client::at(&endpoint.address, &token))
    }
}
fn percentile95(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    if samples.is_empty() {
        return f64::MAX;
    }
    let rank = ((samples.len() as f64) * 0.95).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}
pub fn run(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    match id.as_str() {
        "CON-IMPORT" => crate::imports::contacts(world, input, evidence),
        "CON-RECIPIENT-LOOKUP" => recipient_lookup(world, evidence),
        "CON-ME" | "CON-NOTE" | "CON-RELAY" => Err(Error::Invalid(format!(
            "{id} needs a real or relay world, not ported to the sim-only Rust runner"
        ))),
        "CON-GROUPS" => groups(world, evidence),
        _ if SCENARIOS.contains(&id.as_str()) => scenario(world, &id, evidence),
        _ => Err(Error::Invalid(format!("contacts not ported for {id}"))),
    }
}
fn scenario(world: &mut World, id: &str, evidence: &mut Evidence) -> Result<()> {
    let scope = scope(id);
    let gateway = Client::new(world, scope)?;
    let clients = endpoints(world, &gateway);
    let tag = format!("[ab-test {}]", random_id("contacts")?);
    let run = Run {
        name: format!("{tag} Testperson"),
        synthetic: json!({
            "given_name": format!("{tag} Testperson"), "family_name": "Testfamily",
            "organization": "Bridge Test", "job_title": "Testjob",
            "phones": [{"label": "mobile", "value": "+15555550123"}],
            "emails": [{"label": "home", "value": "bridge-test@example.com"}]
        }),
        tag,
        clients,
        replica: replicas::endpoints().first().cloned(),
        scope,
    };
    if !run.fixture(world, "create", &json!({})) {
        add(evidence, "fixture_create", false);
        return Ok(());
    }
    if !await_until(15, || run.find(&run.clients[0]).is_some()) {
        add(evidence, "unified_contacts_imported", false);
        return Ok(());
    }
    add(evidence, "unified_contacts_imported", true);
    let watchers = run
        .clients
        .iter()
        .map(|client| Watcher::start(client, false))
        .collect::<Result<Vec<_>>>()?;
    if !body(world, id, &run, &watchers, evidence)? {
        return Ok(());
    }
    add(
        evidence,
        "replica_convergence",
        await_until(10, || {
            let Some(gateway) = contact_records_checked(&run.clients[0]) else {
                return false;
            };
            run.clients[1..].iter().all(|client| {
                contact_records_checked(client).is_some_and(|replica| {
                    record_multiset(&gateway, true) == record_multiset(&replica, true)
                })
            })
        }),
    );
    if id != "CON-PERMISSION-LOSS" {
        add(
            evidence,
            "independent_sim_truth",
            await_until(5, || run.truth_converged(world)),
        );
    }
    let observed = Observed::new(run.clients[0].clone(), scope);
    for (index, watcher) in watchers.iter().enumerate() {
        add(
            evidence,
            &format!("sse_contiguous_{index}"),
            watcher.valid(),
        );
        add(
            evidence,
            &format!("no_false_deletion_{index}"),
            crate::assertions::false_deletion_free(world, &observed, watcher),
        );
    }
    let limited = Client::new(world, "system:read")?;
    let hidden = limited.call("contacts.lookup", &json!({}), 20)?;
    add(
        evidence,
        "contacts_scope_enforced",
        hidden.is_some_and(|p| p["records"].as_array().is_none_or(Vec::is_empty)),
    );
    record_integrity(evidence, &observed, None);
    Ok(())
}
fn body(
    world: &mut World,
    id: &str,
    run: &Run,
    watchers: &[Watcher],
    evidence: &mut Evidence,
) -> Result<bool> {
    let gateway = &run.clients[0];
    match id {
        "CON-SEARCH" => {
            for client in &run.clients {
                let query = json!({"collection":"contacts.contact","text":format!("{} Bridge Test", run.tag)});
                add(
                    evidence,
                    "contact_names_and_organization_searchable",
                    await_until(5, || {
                        client
                            .call("search", &query, 20)
                            .ok()
                            .flatten()
                            .is_some_and(|p| p["records"].as_array().is_some_and(|r| r.len() == 1))
                    }),
                );
            }
        }
        "CON-EXTERNAL" => external(world, run, watchers, evidence)?,
        "CON-CRUD" => return crud(world, run, evidence),
        "CON-RESOLVE-RESTART" => {
            let before = run.find(gateway).unwrap_or(Value::Null);
            if world
                .control("/fault", &json!({"name": "ambiguous-outcome"}))
                .is_err()
            {
                add(evidence, "ambiguous_contact_fault", false);
                return Ok(false);
            }
            let (command, reply) = run.submit(
                id,
                "rest",
                "contacts.update",
                &json!({"id": before["id"], "job_title": "Resolved Testjob"}),
                before["version"].as_u64().unwrap_or(0),
            )?;
            if reply
                .as_ref()
                .is_none_or(|(_, v)| !v["error"].is_null() || v["disposition"] != "indeterminate")
            {
                add(evidence, "contact_outcome_indeterminate", false);
                return Ok(false);
            }
            if world.restart_gateway().is_err() {
                add(evidence, "gateway_restart", false);
                return Ok(false);
            }
            let command_id = command["command_id"].clone();
            let resolved = await_until(5, || {
                gateway
                    .call(
                        "commands.status",
                        &json!({"command_id": command_id, "resolve": true}),
                        20,
                    )
                    .ok()
                    .flatten()
                    .is_some_and(|v| {
                        v["disposition"] == "locally_observed"
                            && v["commit_seq"].as_u64().unwrap_or(0) > 0
                    })
            });
            if !resolved {
                add(evidence, "same_history_token_resolves_after_restart", false);
                return Ok(false);
            }
            let current = run.find(gateway);
            add(
                evidence,
                "reobserved_token_does_not_duplicate_version",
                current.is_some_and(|c| {
                    c["version"].as_u64() == before["version"].as_u64().map(|v| v + 1)
                }),
            );
            let effects = world.control("/effects", &Value::Null);
            add(
                evidence,
                "resolve_never_redispatches",
                effects.is_ok_and(|e| e[command_id.as_str().unwrap_or_default()] == 1),
            );
        }
        "CON-PERMISSION-LOSS" => {
            let before = run.find(gateway);
            if world
                .control(
                    "/source-state",
                    &json!({"source":"contacts","state":"permission_missing"}),
                )
                .is_err()
            {
                add(evidence, "permission_loss_fault", false);
                return Ok(false);
            }
            if !run.fixture(world, "delete", &json!({})) {
                add(evidence, "permission_denied_external_effect", false);
                return Ok(false);
            }
            add(
                evidence,
                "permission_missing_reported",
                await_until(3, || {
                    gateway
                        .call("sources.status", &json!({}), 20)
                        .ok()
                        .flatten()
                        .is_some_and(|status| {
                            status["records"].as_array().is_some_and(|rows| {
                                rows.iter().any(|r| {
                                    r["data"]["source"] == "contacts"
                                        && r["data"]["state"] == "permission_missing"
                                })
                            })
                        })
                }),
            );
            let after = run.find(gateway);
            add(
                evidence,
                "permission_loss_preserves_record",
                before.is_some() && before == after,
            );
        }
        "CON-CONFLICT" => conflict(world, run, evidence)?,
        "CON-DROP-EVERYTHING" => {
            let before = run.find(gateway).unwrap_or(Value::Null);
            let generation = |reply: Option<Value>| -> Vec<String> {
                reply
                    .and_then(|v| v["aliases"].as_array().cloned())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|a| a["native_key"] == run.tag.as_str())
                    .filter_map(|a| a["generation"].as_str().map(str::to_owned))
                    .collect()
            };
            let query =
                json!({"collection":"contacts.contact","id":before["id"],"include_aliases":true});
            let old = generation(gateway.call("records.query", &query, 20)?);
            if old.is_empty() {
                add(evidence, "history_generation_before_reset", false);
                return Ok(false);
            }
            if !run.fixture(world, "drop-everything", &json!({})) {
                add(evidence, "history_reset", false);
                return Ok(false);
            }
            let after = run.find(gateway).unwrap_or(Value::Null);
            add(
                evidence,
                "generation_reset_preserves_identity_and_version",
                before["id"] == after["id"] && before["version"] == after["version"],
            );
            let fresh: BTreeSet<String> = generation(gateway.call("records.query", &query, 20)?)
                .into_iter()
                .collect();
            add(
                evidence,
                "history_reset_changes_generation",
                fresh.len() == 1 && !fresh.contains(&old[0]),
            );
        }
        "CON-TOKEN-CRASH" => {
            let before = run.find(gateway).unwrap_or(Value::Null);
            let _ = run.fixture(world, "token-crash", &json!({"job_title": "Crash Testjob"}));
            if world.restart_peer().is_err() {
                add(evidence, "restart_after_history_fetch", false);
                return Ok(false);
            }
            if !await_until(5, || {
                run.find(gateway)
                    .is_some_and(|r| r["data"]["job_title"] == "Crash Testjob")
            }) {
                add(evidence, "history_change_replayed", false);
                return Ok(false);
            }
            let after = run.find(gateway).unwrap_or(Value::Null);
            add(
                evidence,
                "exactly_one_record_version",
                after["id"] == before["id"]
                    && after["version"].as_u64() == before["version"].as_u64().map(|v| v + 1),
            );
        }
        "CON-READONLY-CONTAINER" => {
            if !run.fixture(world, "readonly", &json!({})) {
                add(evidence, "readonly_fixture", false);
                return Ok(false);
            }
            let record = run.find(gateway).unwrap_or(Value::Null);
            let (_, reply) = run.submit(
                id,
                "rest",
                "contacts.update",
                &json!({"id": record["id"], "job_title": "Forbidden Testjob"}),
                record["version"].as_u64().unwrap_or(0),
            )?;
            add(
                evidence,
                "readonly_rejected_before_effect",
                failed(&reply)
                    || reply.is_some_and(|(_, v)| v["disposition"] != "locally_observed"),
            );
            let effects = world.control("/effects", &Value::Null);
            add(
                evidence,
                "readonly_zero_effects",
                effects.is_ok_and(|e| e.as_object().is_some_and(serde_json::Map::is_empty)),
            );
        }
        "CON-IMAGE" => image(world, run, evidence),
        "CON-HANDLE-LINK" => return handle_link(world, run, watchers, evidence),
        _ => return Err(Error::Invalid(format!("contacts not ported for {id}"))),
    }
    Ok(true)
}
fn external(world: &World, run: &Run, watchers: &[Watcher], evidence: &mut Evidence) -> Result<()> {
    let mut samples: Vec<Vec<f64>> = vec![vec![]; watchers.len()];
    for n in 0..20 {
        let value = format!("Testjob-{n}");
        let started = Instant::now();
        if !run.fixture(world, "modify", &json!({"job_title": value})) {
            add(evidence, "external_modify", false);
            return Ok(());
        }
        let effect = Instant::now();
        for (index, watcher) in watchers.iter().enumerate() {
            let matches = |record: &Value| {
                record["collection"] == "contacts.contact"
                    && record["data"]["given_name"] == run.name.as_str()
                    && record["data"]["job_title"] == value.as_str()
            };
            if !await_until(3, || watcher.first_seen_record(matches).is_some()) {
                add(evidence, "external_visible", false);
                return Ok(());
            }
            let seen = watcher.first_seen_record(matches).unwrap_or(effect);
            samples[index].push(
                seen.saturating_duration_since(started).as_secs_f64() * 1000.0
                    - effect.saturating_duration_since(started).as_secs_f64() * 1000.0,
            )
        }
        add(
            evidence,
            "bounded_fixture_execution",
            started.elapsed().as_secs() < 10,
        );
    }
    for (index, mut values) in samples.into_iter().enumerate() {
        let name = if index == 0 {
            "effect_done→gateway_visible".to_owned()
        } else {
            format!("effect_done→replica_visible:replica{}", index - 1)
        };
        for value in &mut values {
            *value = value.max(0.0);
        }
        add(
            evidence,
            &format!("latency_{name}"),
            percentile95(&mut values) < 1000.0,
        );
    }
    Ok(())
}
fn crud(world: &World, run: &Run, evidence: &mut Evidence) -> Result<bool> {
    let id = "CON-CRUD";
    let gateway = &run.clients[0];
    if !run.fixture(world, "delete", &json!({})) {
        add(evidence, "remove_external_fixture", false);
        return Ok(false);
    }
    if !await_until(3, || run.find(gateway).is_none()) {
        add(evidence, "fixture_absence", false);
        return Ok(false);
    }
    for protocol in ["rest", "mcp"] {
        let (_, reply) = run.submit(id, protocol, "contacts.create", &run.synthetic, 0)?;
        if !reply.is_some_and(|(_, v)| command_observed(&v)) {
            add(evidence, &format!("{protocol}_create_evidence"), false);
            return Ok(false);
        }
        add(evidence, &format!("{protocol}_create_evidence"), true);
        for operation in ["contacts.update", "contacts.delete"] {
            let mut record = Value::Null;
            if !await_until(5, || {
                run.find(gateway).is_some_and(|r| {
                    record = r;
                    true
                })
            }) {
                add(evidence, "command_target", false);
                return Ok(false);
            }
            let mut args = json!({"id": record["id"]});
            if operation == "contacts.update" {
                args["job_title"] = json!("Updated Testjob");
            }
            let (_, reply) = run.submit(
                id,
                protocol,
                operation,
                &args,
                record["version"].as_u64().unwrap_or(0),
            )?;
            if !reply.is_some_and(|(_, v)| command_observed(&v)) {
                add(evidence, &format!("{protocol}_{operation}"), false);
                return Ok(false);
            }
            add(evidence, &format!("{protocol}_{operation}"), true);
            let visible = await_until(5, || {
                run.clients.iter().all(|client| {
                    let current = run.find(client);
                    if operation == "contacts.delete" {
                        current.is_none()
                    } else {
                        current.is_some_and(|c| c["data"]["job_title"] == "Updated Testjob")
                    }
                })
            });
            if !visible {
                add(evidence, "read_your_write", false);
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn conflict(world: &World, run: &Run, evidence: &mut Evidence) -> Result<()> {
    let id = "CON-CONFLICT";
    let gateway = &run.clients[0];
    let no_effects = || {
        world
            .control("/effects", &Value::Null)
            .is_ok_and(|e| e.as_object().is_some_and(serde_json::Map::is_empty))
    };
    let before = run.find(gateway).unwrap_or(Value::Null);
    if !run.fixture(world, "modify", &json!({"job_title": "External Testjob"})) {
        add(evidence, "conflict_external_fixture", false);
        return Ok(());
    }
    let update = |record: &Value| {
        run.submit(
            id,
            "rest",
            "contacts.update",
            &json!({"id": record["id"], "job_title": "Forbidden Testjob"}),
            record["version"].as_u64().unwrap_or(0),
        )
        .map(|(_, reply)| reply)
    };
    add(
        evidence,
        "stale_expected_version_conflicts",
        failed(&update(&before)?),
    );
    add(evidence, "stale_version_zero_effects", no_effects());
    let mut fresh = run.find(gateway).unwrap_or(Value::Null);
    let mut hidden = fresh["data"].clone();
    hidden["job_title"] = json!("Hidden External Testjob");
    if world
        .control(
            "/external-change",
            &json!({"key": run.tag, "collection": "contacts.contact", "data": hidden, "omit_capture": true}),
        )
        .is_err()
    {
        add(evidence, "unreported_native_edit", false);
        return Ok(());
    }
    let reply = update(&fresh)?;
    add(
        evidence,
        "fresh_native_refetch_conflict_with_current",
        reply.is_some_and(|(_, v)| {
            v["error"]["code"] == "conflict"
                && v["error"]["current"]["id"] == fresh["id"]
                && v["error"]["current"]["version"].as_u64() > fresh["version"].as_u64()
        }),
    );
    fresh = run.find(gateway).unwrap_or(Value::Null);
    if world
        .control(
            "/external-change",
            &json!({"key": run.tag, "collection": "contacts.contact", "delete": true, "omit_capture": true}),
        )
        .is_err()
    {
        add(evidence, "unreported_native_delete", false);
        return Ok(());
    }
    let reply = update(&fresh)?;
    add(
        evidence,
        "fresh_native_absence_conflict_with_current",
        reply.is_some_and(|(_, v)| {
            v["error"]["code"] == "conflict" && v["error"]["current"]["deleted"] == true
        }),
    );
    add(evidence, "native_races_zero_effects", no_effects());
    Ok(())
}
fn image(world: &World, run: &Run, evidence: &mut Evidence) {
    if !run.fixture(world, "image", &json!({"base64": PNG})) {
        add(evidence, "image_fixture", false);
        return;
    }
    let expected = hex::encode(Sha256::digest(base64_decode(PNG)));
    for client in &run.clients {
        let verified = await_until(30, || {
            let Some(record) = run.find(client) else {
                return false;
            };
            let digest = record["data"]["image"]["digest"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if digest != expected {
                return false;
            }
            let path = format!(
                "/v1/blobs/{digest}?record={}",
                record["id"].as_str().unwrap_or_default()
            );
            client
                .request("GET", &path, &[], None, 20)
                .is_ok_and(|reply| {
                    reply.status == 200 && hex::encode(Sha256::digest(&reply.body)) == digest
                })
        });
        add(
            evidence,
            "image_blob_authorized_and_digest_verified",
            verified,
        );
    }
}
fn linked(client: &Client) -> bool {
    contact_records(client)
        .iter()
        .any(|r| r["collection"] == "messages.handle" && !r["data"]["contact_id"].is_null())
}
fn handle_link(
    world: &World,
    run: &Run,
    watchers: &[Watcher],
    evidence: &mut Evidence,
) -> Result<bool> {
    let gateway = &run.clients[0];
    let lookup = gateway.call(
        "contacts.lookup",
        &json!({"handle": "+1 (555) 555-0123"}),
        20,
    )?;
    add(
        evidence,
        "normalized_handle_lookup",
        lookup.is_some_and(|p| p["records"].as_array().is_some_and(|r| !r.is_empty())),
    );
    if world
        .control(
            "/external-change",
            &json!({"key":"test-handle","collection":"messages.handle",
            "data":{"address":"+1 (555) 555-0123","service":"iMessage"}}),
        )
        .is_err()
    {
        add(evidence, "handle_fixture", false);
        return Ok(false);
    }
    add(
        evidence,
        "message_handle_contact_link",
        await_until(5, || linked(gateway)),
    );
    let journaled = await_until(3, || {
        watchers[0]
            .first_seen_record(|r| {
                r["collection"] == "messages.handle" && !r["data"]["contact_id"].is_null()
            })
            .is_some()
    });
    add(evidence, "handle_link_journaled", journaled);
    let handle_versions = || -> Option<std::collections::BTreeMap<String, u64>> {
        let records = contact_records_checked(gateway)?;
        Some(
            records
                .iter()
                .filter(|r| r["collection"] == "messages.handle")
                .map(|r| {
                    (
                        r["id"].as_str().unwrap_or_default().to_owned(),
                        r["version"].as_u64().unwrap_or(0),
                    )
                })
                .collect(),
        )
    };
    let before = handle_versions().unwrap_or_default();
    if before.is_empty() || !run.fixture(world, "modify", &json!({"job_title": "Link unchanged"})) {
        add(evidence, "non_handle_contact_edit_fixture", false);
        return Ok(false);
    }
    add(
        evidence,
        "non_handle_edit_preserves_links_and_versions",
        await_until(5, || {
            run.find(gateway)
                .is_some_and(|c| c["data"]["job_title"] == "Link unchanged")
                && linked(gateway)
                && handle_versions().is_some_and(|now| {
                    before
                        .iter()
                        .all(|(id, version)| now.get(id) == Some(version))
                })
        }),
    );
    if !run.fixture(world, "modify", &json!({"phones": [], "emails": []})) {
        add(evidence, "contact_handles_removed", false);
        return Ok(false);
    }
    add(
        evidence,
        "contact_edit_clears_link",
        await_until(3, || !linked(gateway)),
    );
    if !run.fixture(
        world,
        "modify",
        &json!({"phones": run.synthetic["phones"], "emails": run.synthetic["emails"]}),
    ) {
        add(evidence, "contact_handles_restored", false);
        return Ok(false);
    }
    add(
        evidence,
        "contact_edit_restores_link",
        await_until(3, || linked(gateway)),
    );
    let duplicate = format!("{}-duplicate", run.tag);
    if world
        .control(
            "/contacts",
            &json!({"action":"create","key":duplicate,"data":run.synthetic}),
        )
        .is_err()
    {
        add(evidence, "ambiguous_contact_fixture", false);
        return Ok(false);
    }
    add(
        evidence,
        "ambiguous_handles_unlinked",
        await_until(3, || !linked(gateway)),
    );
    if world
        .control("/contacts", &json!({"action":"delete","key":duplicate}))
        .is_err()
    {
        add(evidence, "ambiguous_contact_removed", false);
        return Ok(false);
    }
    add(
        evidence,
        "unambiguous_link_restored",
        await_until(3, || linked(gateway)),
    );
    for index in 0..run.clients.len() {
        let scoped = scoped_token(world, index, "messages:read")?;
        let hidden = scoped.call(
            "records.query",
            &json!({"collection":"messages.handle"}),
            20,
        )?;
        let masked = hidden.is_some_and(|p| {
            let rows = p["records"].as_array().cloned().unwrap_or_default();
            !rows.is_empty() && rows.iter().all(|r| r["data"]["contact_id"].is_null())
        });
        add(
            evidence,
            "contact_link_hidden_without_contacts_scope",
            masked,
        );
        let observer = Watcher::start(&scoped, false)?;
        if world
            .control(
                "/external-change",
                &json!({"key":"test-handle","collection":"messages.handle",
                "data":{"address":"+1 (555) 555-0123","service":random_id("test")?}}),
            )
            .is_err()
        {
            add(evidence, "scoped_handle_change", false);
            return Ok(false);
        }
        let verdict = Cell::new(None);
        add(
            evidence,
            "contact_link_hidden_in_sse",
            await_until(3, || {
                let _ = observer.first_seen_record(|r| {
                    if r["collection"] == "messages.handle" {
                        verdict.set(Some(r["data"]["contact_id"].is_null()));
                        true
                    } else {
                        false
                    }
                });
                verdict.get() == Some(true)
            }),
        );
    }
    Ok(true)
}
fn groups(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    add(evidence, "world_start", true);
    let gateway = Client::new(world, "contacts:read,contacts:write,system:read")?;
    for protocol in ["rest", "mcp"] {
        let submit = |evidence: &mut Evidence,
                      operation: &str,
                      args: Value,
                      version: u64|
         -> Result<(Option<String>, Option<(u16, Value)>)> {
            let command = json!({
                "command_id": random_id("cmd")?, "op": operation, "args": args,
                "expected_version": version, "wait_ms": 15000
            });
            let reply = if protocol == "rest" {
                gateway.call_raw(operation, &command, 30)?
            } else {
                gateway.call_mcp_raw(operation, &command, 30)?
            };
            let ids = reply
                .as_ref()
                .filter(|(status, v)| {
                    *status == 200 && v["error"].is_null() && v["state"] == "observed"
                })
                .and_then(|(_, v)| v["observed_record_ids"].as_array())
                .filter(|ids| ids.len() == 1)
                .and_then(|ids| ids[0].as_str().map(str::to_owned));
            add(evidence, &format!("{protocol}_{operation}"), ids.is_some());
            Ok((ids, reply))
        };
        let call = |operation: &str, input: &Value| -> Result<Option<Value>> {
            if protocol == "rest" {
                gateway.call(operation, input, 20)
            } else {
                gateway.call_mcp(operation, input, 20)
            }
        };
        let read = |evidence: &mut Evidence, collection: &str, id: &str| -> Value {
            let page = call(
                "records.query",
                &json!({"collection": collection, "id": id}),
            )
            .ok()
            .flatten();
            match page.and_then(|p| p["records"].as_array().cloned()) {
                Some(rows) if rows.len() == 1 => rows[0].clone(),
                _ => {
                    add(evidence, &format!("{protocol}_read_{collection}"), false);
                    Value::Null
                }
            }
        };
        let Some(contact_id) = submit(
            evidence,
            "contacts.create",
            json!({"given_name":"[ab-test groups]","organization":"Bridge Test"}),
            0,
        )?
        .0
        else {
            continue;
        };
        let Some(group_id) = submit(
            evidence,
            "contacts.create_group",
            json!({"name":"Bridge Test [ab-test groups]"}),
            0,
        )?
        .0
        else {
            continue;
        };
        let version = |record: &Value| record["version"].as_u64().unwrap_or(0);
        let group = read(evidence, "contacts.group", &group_id);
        submit(
            evidence,
            "contacts.rename_group",
            json!({"id": group_id, "name": "Bridge Test [ab-test groups] renamed"}),
            version(&group),
        )?;
        let stale_command = json!({
            "command_id": random_id("cmd")?, "op": "contacts.delete_group",
            "args": {"id": group_id}, "expected_version": version(&group), "wait_ms": 15000
        });
        let stale = if protocol == "rest" {
            gateway.call_raw("contacts.delete_group", &stale_command, 30)?
        } else {
            gateway.call_mcp_raw("contacts.delete_group", &stale_command, 30)?
        };
        add(
            evidence,
            &format!("{protocol}_stale_group_rejected"),
            failed(&stale) || stale.is_some_and(|(_, v)| v["state"] == "rejected"),
        );
        let group = read(evidence, "contacts.group", &group_id);
        submit(
            evidence,
            "contacts.add_member",
            json!({"id": group_id, "contact_id": contact_id}),
            version(&group),
        )?;
        let listing = call("contacts.list", &json!({"group": group_id}))?;
        add(
            evidence,
            &format!("{protocol}_membership_read_model"),
            listing.is_some_and(|l| {
                l["count"] == 1
                    && l["sections"].as_array().is_some_and(|s| s.len() == 1)
                    && l["sections"][0]["contacts"][0]["contact"]["id"] == contact_id.as_str()
            }),
        );
        let group = read(evidence, "contacts.group", &group_id);
        submit(
            evidence,
            "contacts.remove_member",
            json!({"id": group_id, "contact_id": contact_id}),
            version(&group),
        )?;
        let listing = call("contacts.list", &json!({"group": group_id}))?;
        add(
            evidence,
            &format!("{protocol}_membership_removed"),
            listing.is_some_and(|l| l["count"] == 0),
        );
        let group = read(evidence, "contacts.group", &group_id);
        submit(
            evidence,
            "contacts.add_member",
            json!({"id": group_id, "contact_id": contact_id}),
            version(&group),
        )?;
        let group = read(evidence, "contacts.group", &group_id);
        submit(
            evidence,
            "contacts.delete_group",
            json!({"id": group_id}),
            version(&group),
        )?;
        let contact = read(evidence, "contacts.contact", &contact_id);
        add(
            evidence,
            &format!("{protocol}_group_delete_preserves_contact"),
            contact["id"] == contact_id.as_str(),
        );
        submit(
            evidence,
            "contacts.delete",
            json!({"id": contact_id}),
            version(&contact),
        )?;
        let missing = if protocol == "rest" {
            gateway
                .call("contacts.list", &json!({"group": group_id}), 20)
                .is_ok_and(|v| v.is_none())
        } else {
            gateway
                .call_mcp("contacts.list", &json!({"group": group_id}), 20)
                .is_ok_and(|v| v.is_none())
        };
        add(
            evidence,
            &format!("{protocol}_deleted_group_not_found"),
            missing,
        );
    }
    Ok(())
}
fn recipient_lookup(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let gateway = Client::new(world, "admin")?;
    let names = [
        "Aster", "Beryl", "Cobalt", "Dune", "Ember", "Fern", "Garnet", "Halo", "Indigo", "Juniper",
        "Kite", "Lumen",
    ];
    let records: Vec<Value> = (0..60)
        .map(|i| {
            json!({
                "kind": "upsert", "collection": "contacts.contact",
                "native_key": format!("demo-contact-{i:02}"),
                "data": {
                    "given_name": format!("Demo {}{i:02}", names[i % names.len()]),
                    "family_name": "Example", "organization": "Synthetic Studio",
                    "phones": [{"label":"mobile","value":format!("+1 555 01{i:02}")}],
                    "emails": [{"label":"home","value":format!("demo.person{i:02}@example.com")}],
                    "container_id": "demo-synthetic", "writable": true
                }
            })
        })
        .collect();
    if world
        .control("/dataset", &json!({"records": records, "blobs": {}}))
        .is_err()
    {
        add(evidence, "demo_dataset", false);
        return Ok(());
    }
    if world.control("/flush", &json!({})).is_err() {
        add(evidence, "demo_capture", false);
        return Ok(());
    }
    await_until(20, || {
        gateway
            .call("contacts.lookup", &json!({"q":"Demo","limit":500}), 20)
            .ok()
            .flatten()
            .is_some_and(|p| p["records"].as_array().is_some_and(|r| r.len() == 25))
    });
    let queries: [(&str, &str, i64, usize); 7] = [
        ("name", "Aster00", 8, 1),
        ("name_with_digits", "Beryl01", 8, 1),
        ("partial_email", "person01@", 8, 1),
        ("normalized_phone", "5550101", 8, 1),
        ("bounded", "Demo", 3, 3),
        ("capped", "Demo", 500, 25),
        ("default", "Demo", 0, 25),
    ];
    for (name, q, limit, count) in queries {
        let mut prior: Vec<Value> = vec![];
        for protocol in ["rest", "mcp"] {
            let input = json!({"q": q, "limit": limit});
            let reply = if protocol == "rest" {
                gateway.call("contacts.lookup", &input, 20)?
            } else {
                gateway.call_mcp("contacts.lookup", &input, 20)?
            };
            let rows = reply
                .as_ref()
                .and_then(|v| v["records"].as_array().cloned())
                .unwrap_or_default();
            add(
                evidence,
                &format!("{name}_{protocol}_count"),
                reply.is_some() && rows.len() == count,
            );
            let ids: Vec<Value> = rows.iter().map(|r| r["id"].clone()).collect();
            if name == "bounded" {
                let order: Vec<&str> = rows
                    .iter()
                    .filter_map(|r| r["data"]["given_name"].as_str())
                    .collect();
                add(
                    evidence,
                    &format!("{protocol}_deterministic_name_order"),
                    order == ["Demo Aster00", "Demo Aster12", "Demo Aster24"],
                );
            }
            if protocol == "mcp" {
                add(
                    evidence,
                    &format!("{name}_rest_mcp_parity"),
                    reply.is_some() && ids == prior,
                );
            }
            prior = ids;
        }
    }
    let legacy = gateway.call("contacts.lookup", &json!({"name":"Aster00"}), 20)?;
    add(
        evidence,
        "legacy_name_query",
        legacy.is_some_and(|p| p["records"].as_array().is_some_and(|r| r.len() == 1)),
    );
    let legacy = gateway.call("contacts.lookup", &json!({"name":"Demo"}), 20)?;
    add(
        evidence,
        "legacy_name_cardinality",
        legacy.is_some_and(|p| p["records"].as_array().is_some_and(|r| r.len() == 60)),
    );
    let long = "x".repeat(65);
    for protocol in ["rest", "mcp"] {
        let inputs = [
            json!({"q": long}),
            json!({"q":"Demo","limit":-1}),
            json!({"q":""}),
            json!({"q":"   "}),
        ];
        for (index, input) in inputs.iter().enumerate() {
            let reply = if protocol == "rest" {
                gateway.call("contacts.lookup", input, 20)?
            } else {
                gateway.call_mcp("contacts.lookup", input, 20)?
            };
            add(
                evidence,
                &format!("invalid_lookup_{index}_{protocol}_rejected"),
                reply.is_none(),
            );
        }
    }
    Ok(())
}
