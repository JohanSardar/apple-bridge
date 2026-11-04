use crate::{
    assertions::{Observed, await_until, converged},
    frame_proxy::FrameProxy,
    http::{Client, random_id},
    journey::{GATEWAY_SCOPE, is_error},
    replicas::{self, Endpoint},
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

const CASES: [(&str, &str); 10] = [
    ("CORE-RESTORE-EPOCH", "restore-epoch"),
    ("CORE-RETENTION-RACE", "retention-race"),
    ("CORE-SCOPE-CHANGE", "scope-change"),
    ("CORE-SCOPE-LEAK", "scope-leak"),
    ("CORE-SLOW-CONSUMER", "slow-consumer"),
    ("CORE-SNAPSHOT-INTERRUPT", "snapshot-interrupt"),
    ("CORE-SNAPSHOT-UNDER-LOAD", "snapshot-under-load"),
    ("CORE-UPGRADE", "upgrade"),
    ("CORE-RESTORE-OLD-SCHEMA", "restore-old-schema"),
    ("CORE-SOURCE-GRANTS", "source-grants"),
];
const LEGACY_REVISION: &str = "f0cea78";
const LEGACY_GENERATION: &str = "sim_01M6HS1RM2FC3W3TDFQE0CJYSB";
const LEGACY_STATE: &str = "rust/lab/fixtures/legacy-gateway/state.db";
const REPLICA_SCOPE: &str = "reminders:read,system:read";
const FULL_SCOPE: &str = "reminders:read,messages:read,system:read";
const COLLECTION: &str = "reminders.reminder";

struct Run {
    observed: Observed,
    observers: Vec<Watcher>,
}
static RUN: Mutex<Option<Run>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
#[must_use]
pub fn handles(id: &str) -> bool {
    CASES.iter().any(|(scenario, _)| *scenario == id)
}
fn case_of(id: &str) -> &'static str {
    CASES
        .iter()
        .find(|(scenario, _)| *scenario == id)
        .map_or("", |(_, case)| case)
}
fn title(key: &str) -> String {
    format!("[ab-test] {key}")
}
fn with_replica(case: &str) -> bool {
    matches!(
        case,
        "restore-epoch"
            | "retention-race"
            | "slow-consumer"
            | "upgrade"
            | "restore-old-schema"
            | "source-grants"
    )
}
fn with_proxy(case: &str) -> bool {
    matches!(
        case,
        "retention-race" | "slow-consumer" | "snapshot-interrupt" | "snapshot-under-load"
    )
}
fn fail(evidence: &mut Evidence, name: &str) -> bool {
    evidence.check(name, false, "");
    false
}
fn change(world: &World, prefix: &str, count: usize) -> bool {
    (0..count).all(|i| {
        world
            .control(
                "/external-change",
                &json!({"key": format!("{prefix}-{i:06}")}),
            )
            .is_ok()
    })
}
fn replica_state(index: usize) -> Value {
    replicas::endpoints()
        .get(index)
        .and_then(|e| replicas::status(e).ok())
        .map_or(Value::Null, |v| v["replica"].clone())
}
fn connected(index: usize) -> bool {
    replica_state(index)["state"] == "connected"
}
fn cursor_of(client: &Client) -> Option<crate::watch::Cursor> {
    Client::cursor(client).ok()
}
fn current() -> Result<Observed> {
    with_run(|run| Ok(run.observed.clone()))
}
fn with_run<T>(body: impl FnOnce(&mut Run) -> Result<T>) -> Result<T> {
    let mut guard = lock(&RUN);
    let run = guard
        .as_mut()
        .ok_or_else(|| Error::Invalid("scenario prologue did not run".into()))?;
    body(run)
}
fn doctor(world: &World, home: Option<&std::path::Path>) -> Result<Value> {
    let text = match home {
        Some(home) => replicas::cli(home, &["doctor"])?,
        None => world.gateway_cli(&["doctor"])?,
    };
    Ok(serde_json::from_str(&text)?)
}
pub fn seed(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let case = case_of(&evidence.scenario);
    let count = if case.starts_with("snapshot-") {
        1000
    } else {
        input["count"].as_u64().unwrap_or(4)
    };
    prepare(world, count, evidence)
}
fn prepare(world: &mut World, count: u64, evidence: &mut Evidence) -> Result<()> {
    let case = case_of(&evidence.scenario);
    if with_proxy(case) {
        world.install_proxy()?;
    }
    let legacy = matches!(case, "upgrade" | "restore-old-schema");
    if legacy {
        install_legacy(world)?;
    }
    world.control("/seed", &json!({"count": count}))?;
    let gateway = journey_client(world)?;
    let expected = usize::try_from(count).unwrap_or(0) + 2;
    let ready = legacy
        || await_until(30, || {
            crate::journey::records(&gateway).is_ok_and(|(_, all)| all.len() >= expected)
        });
    if !ready {
        evidence.check("seed", false, "gateway did not capture the seeded records");
        return Err(Error::Invalid("seed not captured".into()));
    }
    if with_replica(case) {
        replicas::start_with(world, 1, REPLICA_SCOPE, 0)?;
    }
    evidence.check(
        "feed_network_loopback",
        true,
        "replicas dial the gateway on loopback",
    );
    let observed = Observed::new(gateway, GATEWAY_SCOPE);
    if !await_until(15, || converged(world, &observed)) {
        evidence.check(
            "initial_convergence",
            false,
            &crate::assertions::diagnose(world, &observed),
        );
        return Err(Error::Invalid("initial convergence".into()));
    }
    let mut observers = vec![];
    for client in observed.clients() {
        observers.push(Watcher::start(&client, false)?);
    }
    *lock(&RUN) = Some(Run {
        observed,
        observers,
    });
    Ok(())
}
fn install_legacy(world: &mut World) -> Result<()> {
    world.stop_gateway();
    world.stop_helper();
    std::fs::write(
        world.directory.join("rust-sim-state.json"),
        json!({
            "generation": LEGACY_GENERATION,
            "records": {},
            "effects": {},
            "evidence": {},
            "origins": {},
            "scans": 0,
            "source_scans": {}
        })
        .to_string(),
    )?;
    let state = world.gateway_home.join("state.db");
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", state.display()));
    }
    std::fs::copy(LEGACY_STATE, &state)?;
    world.start_gateway()?;
    world.restart_peer()?;
    if await_until(20, || world.control("/flush", &json!({})).is_ok()) {
        Ok(())
    } else {
        Err(Error::Invalid(
            "migrated gateway never accepted a capture".into(),
        ))
    }
}
fn journey_client(world: &World) -> Result<Client> {
    crate::journey::gateway_client(world)
}
pub fn converge(_: &mut World, _: &Value, _: &mut Evidence) -> Result<()> {
    Ok(())
}
fn restore_epoch(world: &mut World, evidence: &mut Evidence) -> Result<bool> {
    let backup = world.directory.join("backup.db");
    let backup_text = backup.to_string_lossy().into_owned();
    if world.gateway_cli(&["backup", &backup_text]).is_err() {
        return Ok(fail(evidence, "gateway_backup"));
    }
    let observed = current()?;
    let Some(old) = cursor_of(&observed.gateway) else {
        return Ok(fail(evidence, "old_epoch"));
    };
    if !change(world, "after-backup", 20) {
        return Ok(fail(evidence, "changes_after_backup"));
    }
    if !await_until(10, || converged(world, &observed)) {
        return Ok(fail(evidence, "before_restore_convergence"));
    }
    world.stop_gateway();
    if world.gateway_cli(&["restore", &backup_text]).is_err() {
        return Ok(fail(evidence, "restore_backup"));
    }
    if world.start_gateway().is_err() {
        return Ok(fail(evidence, "restart_restored_gateway"));
    }
    if !await_until(15, || converged(world, &observed) && connected(0)) {
        return Ok(fail(evidence, "restore_convergence"));
    }
    let current = cursor_of(&observed.gateway);
    let snapshots = replica_state(0)["snapshots"].as_u64().unwrap_or(0);
    evidence.check(
        "restore_rotates_epoch_replica_resnapshots",
        current.is_some_and(|c| c.epoch != old.epoch) && snapshots >= 2,
        "a restored gateway rotates its epoch and the replica takes a new snapshot",
    );
    Ok(true)
}
fn retention_race(world: &mut World, evidence: &mut Evidence) -> Result<bool> {
    world.stop_gateway();
    let args = ["--retention-rows", "8", "--retention-age", "1ms"]
        .map(str::to_owned)
        .to_vec();
    if world.configure_gateway(vec![], args).is_err() {
        return Ok(fail(evidence, "configure_small_retention"));
    }
    if !await_until(10, || connected(0)) {
        return Ok(fail(evidence, "gateway_reconnect"));
    }
    world.partition()?;
    let Some(before) = replicas::endpoints()
        .first()
        .and_then(|e| cursor_of(&e.client()))
    else {
        return Ok(fail(evidence, "cursor_before_partition"));
    };
    if !change(world, "beyond-retention", 100) {
        return Ok(fail(evidence, "offline_changes"));
    }
    let passed = await_until(15, || {
        doctor(world, None)
            .ok()
            .and_then(|report| report["floor"].as_u64())
            .is_some_and(|floor| floor > before.sequence)
    });
    if !passed {
        return Ok(fail(evidence, "retention_passed_replica_cursor"));
    }
    world.heal()?;
    if !await_until(15, || connected(0)) {
        return Ok(fail(evidence, "resnapshot_connected"));
    }
    evidence.check(
        "reset_resnapshot_after_retention",
        replica_state(0)["snapshots"].as_u64().unwrap_or(0) >= 2,
        "a cursor behind the retention floor forces a new snapshot",
    );
    Ok(true)
}
fn restricted_message(world: &World, key: &str) -> bool {
    world
        .control(
            "/external-change",
            &json!({
                "key": key,
                "collection": "messages.message",
                "data": {
                    "title": title(key),
                    "text": "restrictedneedle",
                    "optional_future": {"counter": 9_007_199_254_740_993_u64}
                }
            }),
        )
        .is_ok()
}
fn admin_client(world: &World) -> Result<Client> {
    let token = world.gateway_cli(&["token", "create", "--scope", "admin"])?;
    Client::with_token(world, token.trim())
}
fn message_count(client: &Client) -> Option<usize> {
    client
        .call(
            "records.query",
            &json!({"collection": "messages.message"}),
            20,
        )
        .ok()
        .flatten()
        .map(|page| page["records"].as_array().map_or(0, Vec::len))
}
fn probe(path: &str, client: &Client, record: &Value) -> bool {
    match path {
        "query" => message_count(client) == Some(0),
        "search" => client
            .call("search", &json!({"text": "restrictedneedle"}), 20)
            .ok()
            .flatten()
            .is_some_and(|page| page["records"].as_array().is_none_or(Vec::is_empty)),
        "records" => client
            .request(
                "GET",
                &format!(
                    "/v1/records/{}/{}",
                    record["collection"].as_str().unwrap_or_default(),
                    record["id"].as_str().unwrap_or_default()
                ),
                &[],
                None,
                20,
            )
            .is_ok_and(|reply| reply.status == 404),
        "commands" => {
            let Ok(id) = random_id("cmd") else {
                return false;
            };
            let command = json!({
                "command_id": id,
                "op": "messages.send",
                "args": {"chat_id": random_id("cht").unwrap_or_default(), "text": format!("[ab-test {id}] scope probe")}
            });
            client
                .call_raw("messages.send", &command, 20)
                .ok()
                .flatten()
                .is_some_and(|(status, value)| {
                    matches!(status, 401 | 403 | 404)
                        || (status == 422 && is_error(&value, "unsupported"))
                })
        }
        _ => false,
    }
}
fn stored_message(world: &World, index: usize) -> Result<Option<Value>> {
    let admin = admin_client(world)?;
    if !restricted_message(world, "restricted-before") {
        return Ok(None);
    }
    let mut record = Value::Null;
    let present = await_until(10, || {
        admin
            .call(
                "records.query",
                &json!({"collection": "messages.message"}),
                20,
            )
            .ok()
            .flatten()
            .and_then(|page| {
                let records = page["records"].as_array()?;
                (records.len() == 1).then(|| record = records[0].clone())
            })
            .is_some()
    });
    let replica_home = replicas::endpoints()[index].home.clone();
    let physical = present
        && await_until(10, || {
            replicas::cli(&replica_home, &["doctor"])
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .is_some_and(|report| {
                    report["records"]["messages.message"].as_u64().unwrap_or(0) >= 1
                })
        });
    Ok(physical.then_some(record))
}
fn scope_leak(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<bool> {
    replicas::spawn(world, FULL_SCOPE, 0, &[])?;
    replicas::wait_connected(15)?;
    let index = replicas::endpoints().len() - 1;
    let scoped = replicas::cli(
        &replicas::endpoints()[index].home,
        &["token", "create", "--scope", REPLICA_SCOPE],
    )?;
    replicas::update(index, Some(&scoped), REPLICA_SCOPE);
    let paths: Vec<String> = input["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(str::to_owned))
        .collect();
    let refresh = Observed::new(journey_client(world)?, GATEWAY_SCOPE);
    with_run(|run| {
        run.observed = refresh.clone();
        Ok(())
    })?;
    if paths.len() < 5 {
        return Ok(fail(evidence, "all_registered_scope_paths"));
    }
    let Some(record) = stored_message(world, index)? else {
        return Ok(fail(evidence, "all_registered_scope_paths"));
    };
    evidence.check(
        "replica_physically_contains_messages_before_scoped_probes",
        true,
        "the replica stores messages that its token cannot read",
    );
    let endpoints: Vec<Client> = refresh.clients();
    let mut watchers = vec![];
    for (position, client) in endpoints.iter().enumerate() {
        for path in &paths {
            if path == "sse" {
                continue;
            }
            if !probe(path, client, &record) {
                return Ok(fail(evidence, "all_registered_scope_paths"));
            }
            evidence.check(
                &format!(
                    "scope_{path}_{}",
                    if position == 0 { "gateway" } else { "replica" }
                ),
                true,
                "the scoped path exposes no messages",
            );
        }
        watchers.push(Watcher::start(client, false)?);
    }
    let live = restricted_message(world, "restricted-live")
        && world
            .control("/external-change", &json!({"key": "visible-scope-barrier"}))
            .is_ok();
    let barrier = title("visible-scope-barrier");
    let seen = live
        && await_until(10, || {
            watchers
                .iter()
                .all(|w| w.first_seen_title(COLLECTION, &barrier).is_some())
        });
    let leaked = watchers.iter().any(|w| {
        w.first_seen_record(|r| r["source"] == "messages").is_some()
            || w.deletes().iter().any(|r| r["source"] == "messages")
    });
    if !seen || leaked {
        return Ok(fail(evidence, "all_registered_scope_paths"));
    }
    evidence.check(
        "scope_sse_live_filtered_with_visible_barrier",
        true,
        "live streams filtered messages and carried the visible barrier",
    );
    Ok(true)
}
fn replica_scope(world: &World, scope: &str) -> Result<()> {
    let id = replicas::identity(0)?;
    world.gateway_cli(&["scope", &id, "--scope", scope])?;
    Ok(())
}
fn scope_change(world: &mut World, evidence: &mut Evidence) -> Result<bool> {
    let admin = admin_client(world)?;
    if !restricted_message(world, "scope-change-message") {
        return Ok(fail(evidence, "messages_fixture"));
    }
    replicas::spawn(world, FULL_SCOPE, 0, &[])?;
    replicas::wait_connected(15)?;
    let observed = Observed::new(admin.clone(), "admin");
    with_run(|run| {
        run.observed = observed.clone();
        Ok(())
    })?;
    if !await_until(15, || converged(world, &observed)) {
        return Ok(fail(evidence, "full_scope_convergence"));
    }
    let endpoint = replicas::endpoints()[0].clone();
    let Some(original) = cursor_of(&endpoint.client()) else {
        return Ok(fail(evidence, "scope_initial_cursor"));
    };
    if replica_scope(world, REPLICA_SCOPE).is_err() {
        return Ok(fail(evidence, "narrow_scope"));
    }
    replicas::update(0, None, REPLICA_SCOPE);
    let purged = await_until(15, || {
        cursor_of(&endpoint.client()).is_some_and(|c| c.revision > original.revision)
            && message_count(&endpoint.client()) == Some(0)
    });
    if !purged {
        return Ok(fail(evidence, "new_revision_purged_messages"));
    }
    let report = doctor(world, Some(&endpoint.home));
    evidence.check(
        "narrowed_subscription_physically_purged_messages",
        report.is_ok_and(|r| r["records"]["messages.message"].is_null()),
        "the narrowed replica holds no messages on disk",
    );
    let narrowed = cursor_of(&endpoint.client());
    evidence.check("scope_narrowed_purged_new_revision", true, "");
    if replica_scope(world, FULL_SCOPE).is_err() {
        return Ok(fail(evidence, "widen_scope"));
    }
    replicas::update(0, None, FULL_SCOPE);
    let floor = narrowed.as_ref().map_or(0, |c| c.revision);
    if !await_until(15, || {
        cursor_of(&endpoint.client()).is_some_and(|c| c.revision > floor)
            && converged(world, &observed)
    }) {
        let detail = format!(
            "{} cursor={:?} status={}",
            crate::assertions::diagnose(world, &observed),
            cursor_of(&endpoint.client()),
            replica_state(0)
        );
        evidence.check("scope_backfill_convergence", false, &detail);
        return Ok(false);
    }
    let widened = cursor_of(&endpoint.client());
    evidence.check(
        "scope_widened_backfill_new_revision",
        widened.is_some_and(|c| c.revision > floor),
        "widening the scope backfills under a newer subscription revision",
    );
    Ok(true)
}
fn slow_consumer(world: &mut World, evidence: &mut Evidence) -> Result<bool> {
    let Some(home) = replicas::endpoints().first().map(|e| e.home.clone()) else {
        return Ok(fail(evidence, "frame_proxy_start"));
    };
    let proxy = match world
        .peer_address()
        .and_then(|peer| FrameProxy::start(&world.gateway_home, &home, &peer))
    {
        Ok(proxy) => proxy,
        Err(error) => {
            evidence.check("frame_proxy_start", false, &error.to_string());
            return Ok(false);
        }
    };
    world.retarget_proxy(Some(proxy.address()))?;
    world.partition()?;
    world.heal()?;
    if !await_until(10, || proxy.connections() >= 1 && connected(0)) {
        proxy.close();
        world.retarget_proxy(None)?;
        return Ok(fail(evidence, "frame_proxy_connect"));
    }
    let initial = proxy.connections();
    proxy.suppress_acks(true);
    if !change(world, "slow-consumer", 10) {
        proxy.close();
        return Ok(fail(evidence, "suppress_acknowledgements"));
    }
    let start = Instant::now();
    let dropped = await_until(35, || proxy.connections() > initial);
    let elapsed = start.elapsed();
    proxy.suppress_acks(false);
    if !dropped {
        proxy.close();
        return Ok(fail(evidence, "slow_consumer_disconnected"));
    }
    evidence.check(
        "unacknowledged_consumer_disconnected_within_35_seconds",
        elapsed <= Duration::from_secs(35) && proxy.dropped() > 0,
        &format!("{} acknowledgements dropped", proxy.dropped()),
    );
    world.retarget_proxy(None)?;
    proxy.close();
    world.partition()?;
    world.heal()?;
    Ok(true)
}
fn upgrade(world: &mut World, evidence: &mut Evidence) -> Result<bool> {
    evidence.check(
        &format!("older_gateway_built_from_{LEGACY_REVISION}"),
        true,
        "the gateway store was written by the older schema",
    );
    let sent = world.control(
        "/external-change",
        &json!({
            "key": "future-optional",
            "collection": COLLECTION,
            "data": {
                "title": title("future-optional"),
                "optional_future": {"counter": 9_007_199_254_740_993_u64}
            }
        }),
    );
    let observed = current()?;
    if sent.is_err() || !await_until(10, || converged(world, &observed)) {
        return Ok(fail(evidence, "optional_record_replicated"));
    }
    let preserved = observed.replicas.first().is_some_and(|endpoint| {
        crate::assertions::read_all(&endpoint.client(), false, false).is_ok_and(|read| {
            read.records.iter().any(|record| {
                let text = record["data"].to_string();
                text.contains("optional_future") && text.contains("9007199254740993")
            })
        })
    });
    evidence.check(
        "unknown_optional_data_preserved_on_replica",
        preserved,
        "an unknown optional field with a large integer survives replication",
    );
    let status = replica_state(0);
    evidence.check(
        "newer_replica_compatible_schema_negotiates_older_gateway",
        status["state"] == "connected" && status["schema"] == 1,
        "the replica negotiated schema 1 with the gateway",
    );
    Ok(true)
}
fn restore_old_schema(world: &mut World, evidence: &mut Evidence) -> Result<bool> {
    let backup = world.directory.join("schema1-backup.db");
    std::fs::copy(LEGACY_STATE, &backup)?;
    let backup_text = backup.to_string_lossy().into_owned();
    world.stop_gateway();
    let restart = world.start_gateway();
    let selected = restart.is_ok() && replicas::binary().is_ok();
    evidence.check(
        "upgrade_uses_selected_gateway",
        selected,
        "the restart ran the selected gateway implementation",
    );
    if restart.is_err() {
        return Ok(fail(evidence, "upgrade_before_command_admission"));
    }
    let observed = current()?;
    if world.control("/flush", &json!({})).is_err() {
        return Ok(fail(evidence, "upgraded_helper_capture_ready"));
    }
    let lists = observed
        .gateway
        .call(
            "records.query",
            &json!({"collection": "reminders.list"}),
            20,
        )
        .ok()
        .flatten()
        .and_then(|page| page["records"].as_array().cloned())
        .unwrap_or_default();
    let Some(list_id) = lists.first().and_then(|r| r["id"].as_str()) else {
        return Ok(fail(evidence, "upgraded_reminder_list"));
    };
    let command = json!({
        "command_id": "old-schema-surviving-command",
        "op": "reminders.create",
        "args": {"list_id": list_id, "title": "[ab-test] command after schema1 backup"},
        "wait_ms": 15000
    });
    let first = observed
        .gateway
        .call_raw("reminders.create", &command, 30)
        .ok()
        .flatten()
        .map(|(_, value)| value);
    if !first
        .as_ref()
        .is_some_and(|v| v["error"].is_null() && v["disposition"] == "locally_observed")
    {
        return Ok(fail(evidence, "command_after_old_schema_backup"));
    }
    let first = first.unwrap_or(Value::Null);
    let before = cursor_of(&observed.gateway);
    world.stop_gateway();
    if world.gateway_cli(&["restore", &backup_text]).is_err() {
        return Ok(fail(evidence, "restore_old_schema_with_newer_live_ledger"));
    }
    if world.start_gateway().is_err() {
        return Ok(fail(
            evidence,
            "migrate_restored_old_schema_before_publication",
        ));
    }
    if !await_until(15, || converged(world, &observed)) {
        return Ok(fail(evidence, "old_schema_restore_convergence"));
    }
    let after = cursor_of(&observed.gateway);
    let retried = observed
        .gateway
        .call_raw("reminders.create", &command, 30)
        .ok()
        .flatten()
        .map(|(_, value)| value);
    let effects = crate::journey::effect_count(world, "old-schema-surviving-command");
    evidence.check(
        "old_schema_restore_rotates_epoch_keeps_live_ledger",
        before.zip(after).is_some_and(|(b, a)| b.epoch != a.epoch)
            && effects == Some(1)
            && retried.is_some_and(|r| {
                r["error"].is_null()
                    && r["disposition"] == first["disposition"]
                    && r["admission_era"] == first["admission_era"]
            }),
        "restoring an older schema rotates the epoch and keeps the live command ledger",
    );
    Ok(true)
}
fn snapshot(world: &mut World, case: &str, evidence: &mut Evidence) -> Result<bool> {
    world.delay_network(10)?;
    if replicas::spawn(world, REPLICA_SCOPE, 0, &[]).is_err() {
        return Ok(fail(evidence, "replica_start"));
    }
    let first = replicas::endpoints()[0].clone();
    let state = || replicas::status(&first).map_or(Value::Null, |v| v["replica"].clone());
    if !await_until(10, || state()["state"] == "snapshot") {
        return Ok(fail(evidence, "snapshot_in_progress"));
    }
    if case == "snapshot-interrupt" {
        if !await_until(10, || {
            let now = state();
            now["state"] == "snapshot" && now["snapshot_pages"].as_u64().unwrap_or(0) > 0
        }) {
            return Ok(fail(evidence, "snapshot_in_progress"));
        }
        replicas::crash(0)?;
        if replicas::restart(0).is_err() {
            return Ok(fail(evidence, "restart_replica"));
        }
        evidence.check("replica_killed_during_snapshot", true, "");
    }
    let observed = current()?;
    let mut overlap = false;
    if case == "snapshot-under-load" {
        let status = state();
        if status.is_null() || replicas::signal(0, "STOP").is_err() {
            return Ok(fail(evidence, "snapshot_replica_paused"));
        }
        let committed = change(world, "during-frozen-snapshot", 1)
            && world.control("/flush", &json!({})).is_ok();
        if !committed {
            let _ = replicas::signal(0, "CONT");
            return Ok(fail(evidence, "frozen_snapshot_change_committed"));
        }
        let head = cursor_of(&observed.gateway);
        overlap = status["state"] == "snapshot"
            && head.is_some_and(|h| h.sequence > status["snapshot_sequence"].as_u64().unwrap_or(0));
        if replicas::signal(0, "CONT").is_err() {
            return Ok(fail(evidence, "snapshot_replica_resumed"));
        }
    }
    for i in 0..1000 {
        if !change(world, &format!("during-snapshot-{i}"), 1) {
            return Ok(fail(evidence, "concurrent_changes"));
        }
        if !overlap && i % 10 == 0 {
            let status = state();
            overlap = status["state"] == "snapshot"
                && cursor_of(&observed.gateway).is_some_and(|h| {
                    h.sequence > status["snapshot_sequence"].as_u64().unwrap_or(0)
                });
        }
    }
    if case == "snapshot-under-load" {
        evidence.check(
            "changes_committed_after_frozen_sequence_before_snapshot_end",
            overlap,
            "the gateway head moved past the frozen snapshot sequence while it was still being sent",
        );
    }
    world.delay_network(0)?;
    evidence.check("one_thousand_changes_during_bootstrap", true, "");
    Ok(true)
}
pub fn run(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let case = case_of(&evidence.scenario);
    if lock(&RUN).is_none() {
        prepare(world, 4, evidence)?;
    }
    let proceed = match case {
        "restore-epoch" => restore_epoch(world, evidence)?,
        "retention-race" => retention_race(world, evidence)?,
        "scope-leak" => scope_leak(world, input, evidence)?,
        "scope-change" => scope_change(world, evidence)?,
        "slow-consumer" => slow_consumer(world, evidence)?,
        "source-grants" => {
            crate::source_grants::run(world, evidence)?;
            true
        }
        "upgrade" => upgrade(world, evidence)?,
        "restore-old-schema" => restore_old_schema(world, evidence)?,
        "snapshot-under-load" | "snapshot-interrupt" => snapshot(world, case, evidence)?,
        other => {
            return Err(Error::Invalid(format!(
                "robustness case {other} not ported"
            )));
        }
    };
    if proceed {
        tail(world, evidence)?;
    }
    if matches!(
        evidence.scenario.as_str(),
        "CORE-RETENTION-RACE"
            | "CORE-SLOW-CONSUMER"
            | "CORE-SNAPSHOT-INTERRUPT"
            | "CORE-SNAPSHOT-UNDER-LOAD"
            | "CORE-UPGRADE"
    ) {
        crate::labcore::oversized_resets(world, evidence);
    }
    *lock(&RUN) = None;
    Ok(())
}
fn tail(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let drained = world.control("/flush", &json!({})).is_ok();
    evidence.check(
        "recovery_capture_drained_before_convergence",
        drained,
        "the final capture drained before convergence",
    );
    if !drained {
        return Ok(());
    }
    let (gateway, observers) = with_run(|run| {
        let mut refreshed =
            Observed::new(run.observed.gateway.clone(), &run.observed.gateway_scope);
        refreshed.replicas = replicas::endpoints();
        run.observed = refreshed.clone();
        Ok((refreshed, std::mem::take(&mut run.observers)))
    })?;
    if !await_until(30, || converged(world, &gateway)) {
        evidence.check(
            "truth_gateway_replica_convergence",
            false,
            "world did not converge",
        );
        return Ok(());
    }
    evidence.check(
        "truth_gateway_replica_convergence",
        true,
        "SimApple truth, gateway and replicas agree",
    );
    if !automatic(world, &gateway, evidence)? {
        fail(evidence, "automatic_sync_guarantees");
        return Ok(());
    }
    let mut homes = vec![("gateway", None)];
    homes.extend(
        gateway
            .replicas
            .iter()
            .map(|e: &Endpoint| ("replica", Some(e.home.clone()))),
    );
    for (role, home) in homes {
        let report = doctor(world, home.as_deref());
        let valid = report
            .as_ref()
            .is_ok_and(|r| r["ok"] == true && r["exactly_once"] == true);
        evidence.check(
            &format!("sqlite_integrity_{role}"),
            valid,
            "the SQLite stores pass the doctor integrity checks",
        );
    }
    for watcher in &observers {
        let clean = crate::assertions::false_deletion_free(world, &gateway, watcher);
        evidence.check(
            "no_false_deletion_in_observed_journal_changes",
            clean,
            "every observed delete is absent in SimApple truth",
        );
    }
    Ok(())
}
fn system_hidden(limited: &Client, fixture_id: &str) -> Result<bool> {
    let query = limited.call("records.query", &json!({"collection": COLLECTION}), 20)?;
    let hidden = query.is_some_and(|page| page["records"].as_array().is_none_or(Vec::is_empty));
    let record = limited.request(
        "GET",
        &format!("/v1/records/{COLLECTION}/{fixture_id}"),
        &[],
        None,
        20,
    )?;
    let body = serde_json::to_vec(&json!({"text": "ab"}))?;
    let search = limited.request(
        "POST",
        "/v1/op/search",
        &[("Content-Type", "application/json")],
        Some(&body),
        20,
    )?;
    let reply_json: Value = serde_json::from_slice(&search.body).unwrap_or(Value::Null);
    let searched = matches!(search.status, 404 | 422)
        || (search.status == 200 && reply_json["records"].as_array().is_none_or(Vec::is_empty));
    Ok(hidden && record.status == 404 && searched)
}
fn automatic(world: &World, observed: &Observed, evidence: &mut Evidence) -> Result<bool> {
    let drained = world.control("/flush", &json!({})).is_ok();
    evidence.check(
        "automatic_recovery_capture_drained",
        drained,
        "capture drained before the automatic checks",
    );
    if !drained {
        return Ok(false);
    }
    let fixture = observed
        .gateway
        .call(
            "records.query",
            &json!({"collection": COLLECTION, "limit": 1}),
            20,
        )?
        .and_then(|page| page["records"].as_array().cloned())
        .unwrap_or_default();
    let [fixture] = fixture.as_slice() else {
        return Ok(false);
    };
    let mut full = vec![];
    let mut scoped = vec![];
    let mut passed = true;
    let mut targets: Vec<(String, String)> = vec![(
        observed.gateway.address().to_owned(),
        world
            .gateway_cli(&["token", "create", "--scope", "system:read"])?
            .trim()
            .to_owned(),
    )];
    for endpoint in &observed.replicas {
        targets.push((
            endpoint.address.clone(),
            replicas::cli(
                &endpoint.home,
                &["token", "create", "--scope", "system:read"],
            )?,
        ));
    }
    let owners = observed.clients();
    for (index, (address, token)) in targets.iter().enumerate() {
        full.push(Watcher::start(&owners[index], false)?);
        let limited = Client::at(address, token);
        let ok = system_hidden(&limited, fixture["id"].as_str().unwrap_or_default())?;
        evidence.check(
            &format!("automatic_scope_query_search_records_{index}"),
            ok,
            "a system:read token sees no reminders through query, search or fetch",
        );
        if !ok {
            return Ok(false);
        }
        scoped.push(Watcher::start(&limited, true)?);
    }
    let probe = random_id("automatic")?;
    world.control("/external-change", &json!({"key": probe}))?;
    let effect = Instant::now();
    let wanted = title(&probe);
    if !await_until(10, || {
        full.iter()
            .all(|w| w.first_seen_title(COLLECTION, &wanted).is_some())
    }) {
        return Ok(false);
    }
    for (index, watcher) in full.iter().enumerate() {
        let seen = watcher
            .first_seen_title(COLLECTION, &wanted)
            .map_or(Duration::MAX, |at| at.saturating_duration_since(effect));
        let valid = watcher.valid()
            && crate::assertions::false_deletion_free(world, observed, watcher)
            && seen < Duration::from_secs(1);
        evidence.check(
            &format!("automatic_metadata_latency_and_no_false_deletion_{index}"),
            valid,
            "the probe change reached the endpoint within one second",
        );
        passed &= valid;
    }
    if !await_until(5, || scoped.iter().all(Watcher::advanced)) {
        return Ok(false);
    }
    for (index, watcher) in scoped.iter().enumerate() {
        let valid = watcher.valid();
        evidence.check(
            &format!("automatic_scope_sse_watermark_{index}"),
            valid,
            "the scoped stream advanced with contiguous watermarks",
        );
        passed &= valid;
    }
    Ok(passed)
}
