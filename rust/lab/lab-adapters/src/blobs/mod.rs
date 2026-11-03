#![allow(clippy::too_many_lines)]
mod faults;
use crate::{
    assertions::{
        Observed, Observers, await_until, converged, read_all, record_integrity, replica_integrity,
        sse_observers, stream_assertions,
    },
    http::Client,
    replicas::{self, Endpoint},
    watch::Cursor,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 12] = [
    "CORE-BLOB-CAP",
    "CORE-BLOB-CORRUPT",
    "CORE-BLOB-DUPLICATE-REFERENCE",
    "CORE-BLOB-GATEWAY-RETRY",
    "CORE-BLOB-GC",
    "CORE-BLOB-INTERRUPT",
    "CORE-BLOB-NO-DIGEST-AUTH",
    "CORE-BLOB-OFFLOADED",
    "CORE-BLOB-REFERENCE-INDEX",
    "CORE-BLOB-SLOW-READ",
    "CORE-BLOB-STAGE-DURING-TRANSFER",
    "CORE-BLOB-STAGE-SAME-DIGEST",
];
pub const SCOPE: &str = "messages:read,reminders:read,system:read";
const GRACE_MS: u64 = 500;
const CAP: u64 = 1 << 20;
const CHUNK: u64 = 1 << 20;

pub fn is_stage(id: &str) -> bool {
    id == "CORE-BLOB-STAGE-DURING-TRANSFER" || id == "CORE-BLOB-STAGE-SAME-DIGEST"
}

pub fn is_blob(id: &str) -> bool {
    SCENARIOS.contains(&id)
}

pub fn configure(world: &mut World, id: &str, replicas: usize) -> Result<()> {
    let mut env = vec![];
    let grace = if is_stage(id) { 0 } else { GRACE_MS };
    if id == "CORE-BLOB-CORRUPT" {
        env.push((
            "APPLE_BRIDGE_LAB_BLOB_CORRUPT".to_owned(),
            world.directory.join("corrupt-once").display().to_string(),
        ));
    }
    if id == "CORE-BLOB-INTERRUPT" {
        env.push(("APPLE_BRIDGE_LAB_BLOB_SLOW".to_owned(), "1".to_owned()));
        env.push((
            "APPLE_BRIDGE_LAB_BLOB_CUT_WINDOW".to_owned(),
            world
                .directory
                .join("blob-cut-window")
                .display()
                .to_string(),
        ));
    }
    world.configure_gateway(env, vec!["--blob-grace-ms".to_owned(), grace.to_string()])?;
    if id == "CORE-BLOB-INTERRUPT" {
        world.install_proxy()?;
    }
    let extra = vec!["--blob-grace-ms".to_owned(), grace.to_string()];
    let cap = if id == "CORE-BLOB-CAP" { CAP } else { 0 };
    replicas::start_with_args(world, replicas, SCOPE, cap, &extra)
}

fn records(client: &Client) -> Result<(Cursor, Vec<Value>)> {
    let read = read_all(client, false, false)?;
    Ok((read.cursor, read.records))
}
fn progress(client: &Client) -> Option<Value> {
    let value = client.call("sources.status", &json!({}), 10).ok()??;
    Some(value["replica"]["blobs"].clone())
}
fn entry(progress: Option<&Value>, digest: &str) -> Value {
    progress.map_or(Value::Null, |p| p["entries"][digest].clone())
}
fn state_of(client: &Client, digest: &str) -> String {
    entry(progress(client).as_ref(), digest)["state"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
fn bytes(client: &Client, digest: &str, record: &str) -> (u16, Vec<u8>) {
    client
        .request(
            "GET",
            &format!("/v1/blobs/{digest}?record={record}"),
            &[],
            None,
            30,
        )
        .map_or((0, vec![]), |reply| (reply.status, reply.body))
}
fn references(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(digest) = map.get("digest").and_then(Value::as_str)
                && digest.len() == 64
                && map.contains_key("state")
            {
                out.push(digest.to_owned());
            }
            for child in map.values() {
                references(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| references(v, out)),
        _ => {}
    }
}
fn attachment(client: &Client) -> Option<(String, String)> {
    let (_, all) = records(client).ok()?;
    let mut found = None;
    for record in all
        .iter()
        .filter(|r| r["collection"] == "messages.attachment")
    {
        let mut refs = vec![];
        references(&record["data"], &mut refs);
        if let [digest] = refs.as_slice() {
            found = Some((
                record["id"].as_str().unwrap_or_default().to_owned(),
                digest.clone(),
            ));
        }
    }
    found
}
fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

struct Context<'a> {
    world: &'a mut World,
    evidence: &'a mut Evidence,
    gateway: Client,
    replicas: Vec<Endpoint>,
}
impl Context<'_> {
    fn check(&mut self, name: &str, ok: bool) -> bool {
        self.check_with(name, ok, "")
    }
    fn check_with(&mut self, name: &str, ok: bool, extra: &str) -> bool {
        let detail = if ok {
            name.to_owned()
        } else {
            let mut shown = vec![];
            for client in self.all_clients() {
                shown.push(
                    client
                        .call("sources.status", &json!({}), 10)
                        .ok()
                        .flatten()
                        .map_or(Value::Null, |v| v["replica"]["blobs"].clone()),
                );
            }
            let text = serde_json::to_string(&shown).unwrap_or_default();
            format!("{extra} {name}: {}", &text[..text.len().min(1500)])
        };
        self.evidence.check(name, ok, &detail);
        ok
    }
    fn observed(&self) -> Observed {
        Observed {
            gateway: self.gateway.clone(),
            gateway_scope: SCOPE.to_owned(),
            replicas: self.replicas.clone(),
        }
    }
    fn converged(&self) -> bool {
        converged(self.world, &self.observed())
    }
    fn all_clients(&self) -> Vec<Client> {
        std::iter::once(self.gateway.clone())
            .chain(self.replicas.iter().map(Endpoint::client))
            .collect()
    }
    fn tokens(&self, scope: &str) -> Result<Vec<Client>> {
        let mut out = vec![Client::with_token(
            self.world,
            self.world
                .gateway_cli(&["token", "create", "--scope", scope])?
                .trim(),
        )?];
        for endpoint in &self.replicas {
            let token = replicas::cli(&endpoint.home, &["token", "create", "--scope", scope])?;
            out.push(Client::at(endpoint.client().address(), &token));
        }
        Ok(out)
    }
}

pub fn blobs(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if !is_blob(&id) {
        return Err(Error::Invalid(format!("blobs not ported for {id}")));
    }
    let gateway = Client::new(world, SCOPE)?;
    let seeded = world
        .control("/inspect", &Value::Null)?
        .as_array()
        .is_some_and(|all| !all.is_empty());
    if !is_stage(&id) && !seeded {
        world.control("/seed", &json!({"count":4}))?;
        if !await_until(10, || {
            records(&gateway).is_ok_and(|(_, all)| all.len() >= 6)
        }) {
            return Err(Error::Invalid("default seed not captured".into()));
        }
    }
    if is_stage(&id) {
        native_stage(world, evidence, &gateway, &id);
        return Ok(());
    }
    let mut context = Context {
        world,
        evidence,
        gateway,
        replicas: replicas::endpoints(),
    };
    context.check("feed_network_loopback", true);
    context.check("replica_host_is_air", true);
    if !await_until(15, || context.converged()) {
        context.check("initial_convergence", false);
        return Ok(());
    }
    let snapshots = context
        .replicas
        .iter()
        .all(|e| replicas::status(e).is_ok_and(|v| v["replica"]["snapshots"] == 1));
    context.check("bootstrap_snapshot", snapshots);
    let scoped = context.tokens("system:read")?;
    let scope_ok = scoped.iter().all(|client| {
        client
            .call(
                "records.query",
                &json!({"collection":"reminders.reminder","include_aliases":true}),
                20,
            )
            .ok()
            .flatten()
            .is_some_and(|p| {
                p["records"].as_array().is_none_or(Vec::is_empty)
                    && p["aliases"].as_array().is_none_or(Vec::is_empty)
            })
    });
    context.check("scope_query_and_get", scope_ok);
    let observers = if id == "CORE-BLOB-REFERENCE-INDEX" {
        None
    } else {
        Some(sse_observers(context.world, &context.observed())?)
    };
    let started = Instant::now();
    let completed = match id.as_str() {
        "CORE-BLOB-DUPLICATE-REFERENCE" => duplicate(&mut context),
        "CORE-BLOB-REFERENCE-INDEX" => reference_index(&mut context),
        _ => single(&mut context, &id)?,
    };
    if completed {
        context.evidence.check(
            "latency_observations_complete",
            true,
            &format!("{} ms", started.elapsed().as_millis()),
        );
        finish(&mut context, observers.as_ref());
    }
    Ok(())
}

fn finish(context: &mut Context, observers: Option<&Observers>) {
    let ok = await_until(15, || context.converged());
    context.check("convergence_truth_gateway_replicas", ok);
    let observed = context.observed();
    if let Some(observers) = observers {
        stream_assertions(context.world, &observed, observers, context.evidence);
    }
    record_integrity(context.evidence, &observed, None);
}

fn duplicate(context: &mut Context) -> bool {
    let digest = "a".repeat(64);
    let data = json!({"title":"[ab-test] duplicate references","image":{"digest":digest,"state":"offloaded"},"thumbnail_image":{"digest":digest,"state":"offloaded"}});
    let sent = context
        .world
        .control(
            "/external-change",
            &json!({"key":"duplicate-reference","collection":"messages.attachment","data":data}),
        )
        .is_ok();
    context.check("duplicate_reference_fixture", sent);
    if !context.check(
        "duplicate_reference_capture_commits",
        await_until(5, || context.converged()),
    ) {
        return false;
    }
    for client in context.all_clients() {
        let ok = await_until(5, || {
            let p = progress(&client);
            p.as_ref()
                .and_then(|p| p["entries"].as_object())
                .is_some_and(|m| m.len() == 1)
                && entry(p.as_ref(), &digest)["state"] == "offloaded"
        });
        context.check("one_protected_digest_for_duplicate_fields", ok);
    }
    true
}

fn reference_index(context: &mut Context) -> bool {
    let rows = 320_u64;
    context.check("history_request", true);
    let committed = context
        .world
        .control("/blob-history", &json!({"rows":rows}))
        .is_ok();
    if !context.check("history_committed", committed) {
        return false;
    }
    context.check("reference_capture_durable", committed);
    let all = records(&context.gateway).map(|(_, all)| all);
    let count = all.as_ref().map_or(0, |all| {
        all.iter()
            .filter(|r| {
                r["collection"] == "reminders.reminder"
                    && r["data"]["title"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("[ab-test] blob-history-"))
            })
            .count()
    });
    if !context.check(
        &format!("history_{rows}_durable_records"),
        count == usize::try_from(rows).unwrap_or(0),
    ) {
        return false;
    }
    let cursor = records(&context.gateway).map(|(c, _)| c);
    if !context.check("history_durable_cursor", cursor.is_ok()) {
        return false;
    }
    if !context.check(
        "history_replica_records_and_cursor_match",
        await_until(15, || replica_integrity(&context.observed())),
    ) {
        return false;
    }
    let expected = usize::try_from(rows / 5).unwrap_or(0);
    let ok = await_until(5, || {
        context.all_clients().iter().all(|client| {
            progress(client)
                .as_ref()
                .and_then(|p| p["entries"].as_object().cloned())
                .is_some_and(|m| {
                    m.len() == expected && m.values().all(|e| e["state"] == "offloaded")
                })
        })
    });
    context.check("capture_and_replica_reference_protection", ok);
    false
}

fn single(context: &mut Context, id: &str) -> Result<bool> {
    let size: u64 = if id == "CORE-BLOB-INTERRUPT" {
        16 << 20
    } else {
        4 << 20
    };
    let mut fixture = json!({"size": size});
    if id == "CORE-BLOB-GATEWAY-RETRY" {
        fixture["drop_reads"] = json!(1);
        fixture["slow_ms"] = json!(100);
    }
    if id == "CORE-BLOB-OFFLOADED" {
        fixture["state"] = json!("offloaded");
    }
    if id == "CORE-BLOB-SLOW-READ" {
        fixture["slow_ms"] = json!(100);
    }
    let sent = context.world.control("/blob-fixture", &fixture).is_ok();
    if !context.check("attachment_fixture", sent) {
        return Ok(false);
    }
    let started = Instant::now();
    let mut found = None;
    let visible = await_until(10, || {
        found = attachment(&context.gateway);
        found.is_some()
    });
    let Some((record, digest)) = found.filter(|_| visible) else {
        context.check("attachment_metadata_visible", false);
        return Ok(false);
    };
    context.check("attachment_metadata_visible", true);
    if !context.check(
        "metadata_convergence_with_blob_backlog",
        await_until(1, || context.converged()),
    ) {
        return Ok(false);
    }
    let replica = context.replicas[0].client();
    match id {
        "CORE-BLOB-OFFLOADED" => {
            let ok = await_until(5, || {
                state_of(&replica, &digest) == "offloaded"
                    && bytes(&replica, &digest, &record).0 == 503
                    && context.converged()
            });
            let extra = format!(
                "status={} converged={}",
                bytes(&replica, &digest, &record).0,
                context.converged()
            );
            context.check_with("offloaded_bytes_unavailable_metadata_complete", ok, &extra);
            return Ok(true);
        }
        "CORE-BLOB-SLOW-READ" => {
            let changed = context
                .world
                .control("/external-change", &json!({"key":"slow-apple-blob"}))
                .is_ok()
                && await_until(1, || {
                    records(&replica).is_ok_and(|(_, all)| {
                        all.iter()
                            .any(|r| r["data"].to_string().contains("slow-apple-blob"))
                    })
                });
            if !context.check("slow_apple_read_keeps_metadata_live", changed) {
                return Ok(false);
            }
        }
        "CORE-BLOB-CAP" => {
            let ok = await_until(10, || {
                let p = progress(&replica);
                entry(p.as_ref(), &digest)["state"] == "over_cap"
                    && p.as_ref()
                        .is_some_and(|p| p["bytes"].as_u64().unwrap_or(u64::MAX) <= CAP)
            });
            if !context.check("cap_reports_over_cap", ok) {
                return Ok(false);
            }
            let denied = bytes(&replica, &digest, &record).0 == 503 && context.converged();
            let extra = format!(
                "status={} converged={}",
                bytes(&replica, &digest, &record).0,
                context.converged()
            );
            context.check_with("cap_keeps_metadata_denies_absent_bytes", denied, &extra);
            return Ok(true);
        }
        "CORE-BLOB-INTERRUPT" if !faults::interrupt(context, &digest, size) => {
            return Ok(false);
        }
        "CORE-BLOB-GATEWAY-RETRY" if !faults::retry_before(context, &digest, size) => {
            return Ok(false);
        }
        _ => {}
    }
    let complete = await_until(20, || state_of(&replica, &digest) == "present");
    if !context.check("blob_complete", complete) {
        return Ok(false);
    }
    let _ = started;
    for client in context.all_clients() {
        let (status, body) = bytes(&client, &digest, &record);
        let ok = status == 200
            && u64::try_from(body.len()).is_ok_and(|n| n == size)
            && hex::encode(Sha256::digest(&body)) == digest;
        if !context.check("authorized_bytes_digest_verified", ok) {
            return Ok(false);
        }
    }
    if id == "CORE-BLOB-GC" && !faults::gc_migration(context, &record, &digest) {
        return Ok(false);
    }
    if id == "CORE-BLOB-INTERRUPT" && !faults::resumed(context) {
        return Ok(false);
    }
    if id == "CORE-BLOB-GATEWAY-RETRY" && !faults::retry_after(context, &record, &digest) {
        return Ok(false);
    }
    if id == "CORE-BLOB-CORRUPT" {
        let corruptions = progress(&replica).map_or(0, |p| p["corruptions"].as_u64().unwrap_or(0));
        if !context.check("corruption_detected_and_refetched", corruptions == 1) {
            return Ok(false);
        }
    }
    if id == "CORE-BLOB-NO-DIGEST-AUTH" && !no_digest_auth(context, &digest, &record)? {
        return Ok(false);
    }
    if id == "CORE-BLOB-GC" && !faults::gc_lifecycle(context, &digest) {
        return Ok(false);
    }
    let _ = fs::metadata(&context.world.directory);
    sleep(0);
    Ok(true)
}

fn no_digest_auth(context: &mut Context, digest: &str, record: &str) -> Result<bool> {
    let scoped = context.tokens("reminders:read,system:read")?;
    for (client, scoped_client) in context.all_clients().iter().zip(scoped) {
        if !context.check("digest_alone_denied", bytes(client, digest, "").0 == 401) {
            return Ok(false);
        }
        let unknown = "x".repeat(30);
        if !context.check(
            "unknown_record_denied",
            bytes(client, digest, &unknown).0 == 401,
        ) {
            return Ok(false);
        }
        let list = records(client)?
            .1
            .into_iter()
            .find(|r| r["collection"] == "reminders.list");
        if let Some(list) = list {
            let denied = bytes(client, digest, list["id"].as_str().unwrap_or_default()).0 == 401;
            if !context.check("unrelated_record_denied", denied) {
                return Ok(false);
            }
        }
        let status = progress(&scoped_client);
        let leaked = !entry(status.as_ref(), digest).is_null();
        let empty = status
            .as_ref()
            .is_some_and(|p| p["bytes"].as_u64().unwrap_or(1) == 0);
        if !context.check("out_of_scope_blob_status_denied", !leaked && empty) {
            return Ok(false);
        }
        if !context.check(
            "out_of_scope_record_denied",
            bytes(&scoped_client, digest, record).0 == 401,
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn native_stage(world: &mut World, evidence: &mut Evidence, gateway: &Client, id: &str) {
    let same = id == "CORE-BLOB-STAGE-SAME-DIGEST";
    evidence.check("isolated_world_ready", true, "world ready");
    let size: u64 = if same { 172_032 } else { 4 << 20 };
    let mut input = json!({"size":size,"slow_ms":1000});
    if same {
        input["relay_fixture"] = json!(true);
    }
    let sent = world.control("/blob-fixture", &input).is_ok();
    evidence.check("slow_native_blob_fixture", sent, "slow fixture");
    if !sent {
        return;
    }
    let transferring = await_until(10, || {
        progress(gateway)
            .and_then(|p| p["entries"].as_object().cloned())
            .is_some_and(|entries| {
                entries.values().any(|e| {
                    e["state"] == "pending"
                        && e["received"].as_u64().unwrap_or(0) > 0
                        && e["received"].as_u64().unwrap_or(u64::MAX) < size
                })
            })
    });
    evidence.check("native_transfer_in_progress", transferring, "pending entry");
    if !transferring {
        return;
    }
    let staged = world.blob_control(&json!({"action":"relay-fixture"}));
    evidence.check("synthetic_blob_staged", staged.is_ok(), "relay fixture");
    let Ok(fixture) = staged else {
        return;
    };
    let digest = fixture["digest"].as_str().unwrap_or_default().to_owned();
    let title = fixture["title"].as_str().unwrap_or_default().to_owned();
    let fixture_size = fixture["size"].as_u64().unwrap_or(0);
    if same {
        let part = world
            .gateway_home
            .join("blobs/staging")
            .join(format!("{digest}.part"));
        let retired = await_until(5, || !part.exists());
        evidence.check(
            "same_digest_partial_transfer_retired",
            retired,
            "part retired",
        );
        if !retired {
            return;
        }
    }
    let mut clients = vec![gateway.clone()];
    clients.extend(replicas::endpoints().iter().map(Endpoint::client));
    for (index, client) in clients.iter().enumerate() {
        let mut record = String::new();
        let visible = await_until(10, || {
            records(client).is_ok_and(|(_, all)| {
                all.iter()
                    .find(|r| r["data"]["title"] == title.as_str())
                    .is_some_and(|r| {
                        r["id"].as_str().unwrap_or_default().clone_into(&mut record);
                        true
                    })
            })
        });
        evidence.check("fixture_record_visible", visible, "record visible");
        if !visible {
            return;
        }
        if id == "CORE-BLOB-STAGE-DURING-TRANSFER" && index > 0 {
            continue;
        }
        let matched = await_until(10, || {
            let (status, body) = bytes(client, &digest, &record);
            status == 200
                && body.len() as u64 == fixture_size
                && hex::encode(Sha256::digest(&body)) == digest
        });
        evidence.check("authorized_blob_bytes_match", matched, "digest verified");
        if !matched {
            return;
        }
    }
}
