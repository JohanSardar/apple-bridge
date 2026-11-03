use crate::{
    assertions::{Observed, Observers, await_until, converged},
    http::Client,
    journey::{self, GATEWAY_SCOPE, command, effect_count, gateway_client, observed},
    replicas,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn replica_writer(index: usize) -> Result<Client> {
    replicas::endpoints()
        .get(index)
        .map(replicas::Endpoint::client)
        .ok_or_else(|| Error::Invalid("replica missing".into()))
}
fn create(list_id: &str, title: &str) -> Result<Value> {
    command(
        "reminders.create",
        &json!({"list_id": list_id, "title": format!("[ab-test] {title}")}),
    )
}
fn set_id(command: &mut Value, id: &str) {
    command["command_id"] = json!(id);
}
fn id_of(command: &Value) -> String {
    command["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
fn submit(client: &Client, command: &Value, mcp: bool) -> Result<Value> {
    let op = command["op"].as_str().unwrap_or_default();
    let reply = if mcp {
        client.call_mcp_raw(op, command, 40)?
    } else {
        client.call_raw(op, command, 40)?
    };
    Ok(reply.map_or(Value::Null, |(_, value)| value))
}
fn status(client: &Client, command: &Value, resolve: bool) -> Result<Value> {
    let query = json!({"command_id": command["command_id"], "resolve": resolve});
    Ok(client
        .call_raw("commands.status", &query, 40)?
        .map_or(Value::Null, |(_, value)| value))
}
fn succeeded(value: &Value) -> bool {
    value.is_object() && value["error"].is_null()
}
fn error_code(value: &Value) -> &str {
    value["error"]["code"].as_str().unwrap_or_default()
}
fn count(world: &World, command: &Value, expected: u64) -> bool {
    effect_count(world, &id_of(command)) == Some(expected)
}
fn fault(world: &World, name: &str) -> bool {
    world
        .control("/fault", &json!({"name": name, "ms": 0}))
        .is_ok()
}
fn ledger_loss_blocked(world: &World) -> Result<bool> {
    let mut child = Command::new(replicas::binary()?)
        .args(["start", "--role", "gateway", "--home"])
        .arg(&world.gateway_home)
        .args(["--api-addr", "127.0.0.1:0", "--peer-addr", "127.0.0.1:0"])
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(!output.status.success()
                && String::from_utf8_lossy(&output.stderr).contains("ledger_missing"));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(25));
    }
}
fn remove_ledger(world: &World) -> bool {
    ["", "-wal", "-shm"].iter().all(|suffix| {
        match fs::remove_file(world.gateway_home.join(format!("ledger.db{suffix}"))) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        }
    })
}
fn epoch(gateway: &Client) -> String {
    journey::records(gateway)
        .ok()
        .and_then(|(cursor, _)| cursor["epoch"].as_str().map(str::to_owned))
        .unwrap_or_default()
}
pub fn lost_response(world: &mut World, list_id: &str, evidence: &mut Evidence) -> Result<()> {
    let client = replica_writer(0)?;
    let home = replicas::endpoints()
        .first()
        .map(|endpoint| endpoint.home.clone())
        .ok_or_else(|| Error::Invalid("replica missing".into()))?;
    let c = create(list_id, "peer-lost-response")?;
    evidence.check(
        "replica_feed_lag_injected",
        fs::write(home.join("command-apply-delay"), "1000").is_ok(),
        "replica applies the next feed batch after one second",
    );
    evidence.check(
        "peer_response_drop_armed",
        fs::write(world.gateway_home.join("drop-command-response"), "once").is_ok(),
        "gateway drops the next peer command response",
    );
    let recovered = submit(&client, &c, false)?;
    evidence.check(
        "replica_recovers_lost_peer_response_by_status",
        observed(&recovered) && count(world, &c, 1),
        "replica recovers the dropped peer response by status",
    );
    let local = client.call(
        "records.query",
        &json!({"collection":"reminders.reminder"}),
        20,
    )?;
    let found = local.as_ref().is_some_and(|page| {
        page["records"].as_array().is_some_and(|records| {
            records
                .iter()
                .any(|r| r["data"]["title"] == "[ab-test] peer-lost-response")
        })
    });
    let sequence = local
        .as_ref()
        .and_then(|page| page["cursor"]["sequence"].as_u64())
        .unwrap_or(0);
    evidence.check(
        "lost_peer_response_waits_for_local_write",
        found && sequence >= recovered["commit_seq"].as_u64().unwrap_or(u64::MAX),
        "local read at the commit sequence includes the write",
    );
    let c = create(list_id, "lost-response")?;
    let dropped = client.call_raw(c["op"].as_str().unwrap_or_default(), &c, 40)?;
    evidence.check(
        "response_lost",
        dropped.is_some(),
        "client discards the completed response",
    );
    let looked = status(&client, &c, false)?;
    evidence.check(
        "lookup_original_id_observed",
        observed(&looked),
        "status by the original id is observed",
    );
    let retried = submit(&client, &c, false)?;
    evidence.check(
        "retry_original_id_observed",
        observed(&retried),
        "retry with the original id is observed",
    );
    evidence.check("one_truth_effect", count(world, &c, 1), "one native effect");
    Ok(())
}
#[allow(clippy::too_many_lines)]
pub fn era(
    world: &mut World,
    list_id: &str,
    observers: &Observers,
    evidence: &mut Evidence,
) -> Result<()> {
    let gateway = gateway_client(world)?;
    let client = replica_writer(0)?;
    let head_epoch = epoch(&gateway);
    let backup = world.directory.join("commands-before.db");
    let backup_text = backup.to_string_lossy().into_owned();
    evidence.check(
        "backup_before_command",
        world.gateway_cli(&["backup", &backup_text]).is_ok(),
        "command ledger backup taken",
    );
    let mut c = create(list_id, "after-backup")?;
    set_id(&mut c, "caller-uuid-review-17");
    let first = submit(&client, &c, false)?;
    evidence.check(
        "opaque_caller_id_executes",
        observed(&first) && count(world, &c, 1),
        "opaque caller id executes once",
    );
    let ambiguous = create(list_id, "old-era-ambiguous")?;
    fault(world, "ambiguous-outcome");
    let unresolved = submit(&client, &ambiguous, false)?;
    evidence.check(
        "pre_restore_indeterminate",
        succeeded(&unresolved)
            && unresolved["disposition"] == "indeterminate"
            && count(world, &ambiguous, 1),
        "ambiguous outcome reports indeterminate",
    );
    world.stop_gateway();
    let restore = world.gateway_cli(&["restore", "--backup", &backup_text]);
    evidence.check(
        "restore_older_backup",
        restore.is_ok(),
        "older command backup restored",
    );
    evidence.check(
        "restart_after_restore",
        world.start_gateway().is_ok(),
        "gateway restarts after restore",
    );
    let gateway = gateway_client(world)?;
    let current_view = Observed::new(gateway.clone(), GATEWAY_SCOPE);
    evidence.check(
        "restore_truth_reconverges",
        await_until(10, || converged(world, &current_view)),
        "truth and replicas reconverge after restore",
    );
    let restored_epoch = epoch(&gateway);
    evidence.check(
        "restore_rotates_epoch",
        !restored_epoch.is_empty() && restored_epoch != head_epoch,
        "restore rotates the epoch",
    );
    let retry = submit(&client, &c, false)?;
    evidence.check(
        "post_backup_command_recognized_without_second_effect",
        succeeded(&retry) && retry == first && count(world, &c, 1),
        "post-backup command id is recognized without a second effect",
    );
    let lookup = status(&client, &c, true)?;
    evidence.check(
        "pre_restore_id_lookup_only",
        succeeded(&lookup) && lookup == first && count(world, &c, 1),
        "resolve for a pre-restore id is lookup only",
    );
    let old_unresolved = status(&client, &ambiguous, true)?;
    evidence.check(
        "old_era_resolve_is_lookup_only",
        succeeded(&old_unresolved) && old_unresolved == unresolved && count(world, &ambiguous, 1),
        "old-era resolve never redispatches",
    );
    let mut old = create(list_id, "old-era-new-id")?;
    old["admission_era"] = first["admission_era"].clone();
    let rejected = submit(&client, &old, false)?;
    evidence.check(
        "older_era_cannot_admit_new_id",
        error_code(&rejected) == "expired" && count(world, &old, 0),
        "an older era cannot admit a new id",
    );
    let mut fresh = create(list_id, "fresh-era")?;
    set_id(&mut fresh, "fresh.opaque:caller-17");
    let current = submit(&client, &fresh, false)?;
    evidence.check(
        "fresh_opaque_id_new_era_once",
        observed(&current)
            && current["admission_era"] != first["admission_era"]
            && count(world, &fresh, 1),
        "a fresh opaque id runs once in the new era",
    );
    let again = submit(&client, &fresh, false)?;
    evidence.check(
        "fresh_opaque_retry_once",
        succeeded(&again) && count(world, &fresh, 1),
        "retrying the fresh id adds no effect",
    );
    world.stop_gateway();
    evidence.check(
        "remove_ledger",
        remove_ledger(world),
        "command ledger removed",
    );
    evidence.check(
        "missing_ledger_blocks_gateway",
        ledger_loss_blocked(world)?,
        "gateway refuses to start without its ledger",
    );
    evidence.check(
        "restore_cannot_hide_ledger_loss",
        world
            .gateway_cli(&["restore", "--backup", &backup_text])
            .is_err(),
        "restoring a backup cannot hide ledger loss",
    );
    evidence.check(
        "ledger_loss_still_blocks_gateway",
        ledger_loss_blocked(world)?,
        "ledger loss still blocks the gateway",
    );
    evidence.check(
        "operator_accepts_ledger_loss",
        world
            .gateway_cli(&["restore", "--accept-ledger-loss"])
            .is_ok(),
        "operator accepts ledger loss",
    );
    evidence.check(
        "restart_after_accepted_ledger_loss",
        world.start_gateway().is_ok(),
        "gateway restarts after accepted loss",
    );
    evidence.check(
        "helper_reconnected_after_ledger_loss",
        world.control("/flush", &json!({})).is_ok(),
        "SimApple peer reconnected",
    );
    let gateway = gateway_client(world)?;
    let current_view = Observed::new(gateway.clone(), GATEWAY_SCOPE);
    evidence.check(
        "replica_reconnected_after_ledger_loss",
        await_until(10, || converged(world, &current_view)),
        "replica reconverged after accepted loss",
    );
    let accepted = create(list_id, "accepted-loss-era")?;
    let accepted_result = submit(&client, &accepted, false)?;
    evidence.check(
        "accepted_loss_starts_new_era",
        observed(&accepted_result)
            && accepted_result["admission_era"] != current["admission_era"]
            && count(world, &accepted, 1),
        "accepted loss starts a new era",
    );
    for protocol in ["rest", "mcp"] {
        let mut generated = create(list_id, &format!("generated-{protocol}"))?;
        if let Some(map) = generated.as_object_mut() {
            map.remove("command_id");
        }
        let result = submit(&client, &generated, protocol == "mcp")?;
        let generated_id = result["command"]["command_id"].as_str().unwrap_or_default();
        evidence.check(
            &format!("{protocol}_generated_opaque_id"),
            observed(&result)
                && !generated_id.is_empty()
                && !generated_id.contains(&restored_epoch),
            "generated ids do not embed the epoch",
        );
    }
    resumed_streams(world, &gateway, observers, evidence)?;
    Ok(())
}
fn resumed_streams(
    world: &World,
    gateway: &Client,
    observers: &Observers,
    evidence: &mut Evidence,
) -> Result<()> {
    let watchers: Vec<_> = observers
        .watchers
        .iter()
        .chain(observers.scoped.iter())
        .collect();
    let head = Client::cursor(gateway)?;
    evidence.check(
        "all_sse_observers_resume_new_epoch",
        await_until(5, || watchers.iter().all(|w| w.last().epoch == head.epoch)),
        "every stream observer resumed in the restored epoch",
    );
    evidence.check(
        "post_restore_probe",
        world
            .control("/external-change", &json!({"key": "post-restore-probe"}))
            .is_ok(),
        "post-restore probe change injected",
    );
    evidence.check(
        "post_restore_probe_committed",
        world.control("/flush", &json!({})).is_ok(),
        "post-restore probe committed",
    );
    let head = Client::cursor(gateway)?;
    evidence.check(
        "all_sse_observers_advance_after_restore",
        await_until(5, || {
            watchers.iter().all(|w| {
                let last = w.last();
                last.epoch == head.epoch && last.sequence >= head.sequence
            })
        }),
        "every stream observer advanced past the probe",
    );
    Ok(())
}
