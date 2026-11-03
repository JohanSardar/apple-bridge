use super::{CHUNK, Context, progress, records, references, sleep};
use crate::{assertions::await_until, replicas};
use lab_world::Result;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn received(client: &crate::http::Client, digest: &str) -> u64 {
    progress(client).map_or(0, |p| {
        p["entries"][digest]["received"].as_u64().unwrap_or(0)
    })
}

fn gateway_state(context: &Context, digest: &str) -> String {
    progress(&context.gateway)
        .map(|p| {
            p["entries"][digest]["state"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .unwrap_or_default()
}

fn replica_connected(context: &Context, want: &str) -> bool {
    context
        .replicas
        .first()
        .and_then(|e| replicas::status(e).ok())
        .is_some_and(|v| v["replica"]["state"] == want)
}

pub(super) fn interrupt(context: &mut Context, digest: &str, size: u64) -> bool {
    let replica = context.replicas[0].client();
    if !context.check(
        "gateway_blob_present",
        await_until(10, || gateway_state(context, digest) == "present"),
    ) {
        return false;
    }
    if !context.check(
        "partial_blob_persisted_before_interrupt",
        await_until(15, || {
            let n = received(&replica, digest);
            n > 0 && n < size
        }),
    ) {
        return false;
    }
    let window = context.world.directory.join("blob-cut-window");
    if !context.check(
        "interrupted_halfway_through_binary_chunk",
        await_until(10, || {
            window.exists() && received(&replica, digest) == CHUNK
        }),
    ) {
        return false;
    }
    let _ = context.world.partition();
    if !context.check(
        "partition_disconnect_observed",
        await_until(10, || replica_connected(context, "disconnected")),
    ) {
        return false;
    }
    let healed = context.world.heal().is_ok();
    if !context.check("partition_healed", healed) {
        return false;
    }
    if !context.check(
        "replica_reconnected_before_healthy_latency",
        await_until(10, || replica_connected(context, "connected")),
    ) {
        return false;
    }
    if !context.check(
        "reconnected_feed",
        await_until(10, || {
            replica
                .call("replica.status", &json!({}), 10)
                .ok()
                .flatten()
                .is_some_and(|v| v["replica"]["state"] == "connected")
        }),
    ) {
        return false;
    }
    let before = received(&replica, digest);
    if !context.check(
        "active_bulk_transfer_barrier",
        await_until(3, || {
            let n = received(&replica, digest);
            n > before && size.saturating_sub(n) >= 8 * CHUNK
        }),
    ) {
        return false;
    }
    let sent = context
        .world
        .control("/external-change", &json!({"key":"during-blob"}))
        .is_ok();
    let live = sent
        && await_until(1, || {
            records(&replica).is_ok_and(|(_, all)| {
                all.iter()
                    .any(|r| r["data"].to_string().contains("during-blob"))
            }) && received(&replica, digest) < size
                && progress(&replica).is_some_and(|p| p["entries"][digest]["state"] != "present")
        });
    context.check("live_metadata_during_bulk", live)
}

pub(super) fn resumed(context: &mut Context) -> bool {
    let replica = context.replicas[0].client();
    let resumed = progress(&replica).map_or(0, |p| p["resumed_bytes"].as_u64().unwrap_or(0));
    context.check("transfer_resumed_from_nonzero_offset", resumed > 0)
}

pub(super) fn retry_before(context: &mut Context, digest: &str, size: u64) -> bool {
    if !context.check(
        "gateway_retries_timed_out_read",
        await_until(18, || {
            let n = received_gateway(context, digest);
            n > 0 && n < size
        }),
    ) {
        return false;
    }
    let restarted = context.world.restart_peer().is_ok();
    if !context.check("helper_restarts_during_staged_download", restarted) {
        return false;
    }
    context.check(
        "gateway_resumes_with_fresh_handles",
        await_until(15, || {
            gateway_state(context, digest) == "present"
                && progress(&context.gateway)
                    .is_some_and(|p| p["resumed_bytes"].as_u64().unwrap_or(0) > 0)
        }),
    )
}

fn received_gateway(context: &Context, digest: &str) -> u64 {
    received(&context.gateway, digest)
}

fn failed_digest(context: &Context, record: &str, digest: &str) -> Option<String> {
    let (_, all) = records(&context.gateway).ok()?;
    let found = all.iter().find(|r| r["id"] == record)?;
    let mut refs = vec![];
    references(&found["data"], &mut refs);
    match refs.as_slice() {
        [one] if one != digest => Some(one.clone()),
        _ => None,
    }
}

fn manifest_entry(home: &Path, digest: &str) -> Option<Value> {
    let data = fs::read(home.join("blobs/manifest.json")).ok()?;
    let value: Value = serde_json::from_slice(&data).ok()?;
    let entries = value.get("entries").or_else(|| value.get("Entries"))?;
    Some(entries[digest].clone())
}

pub(super) fn retry_after(context: &mut Context, record: &str, digest: &str) -> bool {
    let started = Instant::now();
    let fixture = context.world.control(
        "/blob-fixture",
        &json!({"size":128 << 10,"unavailable_reads":8,"refresh_before_unavailable":true}),
    );
    if !context.check("retry_exhaustion_fixture", fixture.is_ok()) {
        return false;
    }
    let mut failed = String::new();
    let faulted = await_until(8, || {
        let remaining = context
            .world
            .control("/blob-faults", &Value::Null)
            .ok()
            .and_then(|v| v["unavailable_remaining"].as_u64());
        if remaining != Some(3) {
            return false;
        }
        match failed_digest(context, record, digest) {
            Some(found) => {
                failed = found;
                true
            }
            None => false,
        }
    });
    if !context.check("five_blob_read_faults_before_gateway_restart", faulted) {
        return false;
    }
    let visible = progress(&context.gateway).is_some();
    if !context.check("retry_manifest_visible_before_restart", visible) {
        return false;
    }
    let killed = context.world.kill_gateway().is_ok();
    if !context.check("gateway_killed_during_retry_backoff", killed) {
        return false;
    }
    context.world.stop_gateway();
    let durable = manifest_entry(&context.world.gateway_home, &failed);
    if !context.check("retry_manifest_read_after_crash", durable.is_some()) {
        return false;
    }
    let durable = durable.unwrap_or(Value::Null);
    let restarted = context.world.start_gateway().is_ok();
    if !context.check("gateway_restarts_during_retry_backoff", restarted) {
        return false;
    }
    let after = progress(&context.gateway).map_or(Value::Null, |p| p["entries"][&failed].clone());
    let budget = context.check(
        "retry_budget_and_deadline_survive_gateway_restart",
        durable["retry_attempts"] == 5
            && after["retry_attempts"] == 5
            && !is_zero_time(&durable["next_attempt_at"])
            && after["next_attempt_at"] == durable["next_attempt_at"],
    );
    let terminal = await_until(22, || {
        if let Some(found) = failed_digest(context, record, digest) {
            failed = found;
        }
        progress(&context.gateway).is_some_and(|p| {
            !failed.is_empty()
                && p["entries"][&failed]["state"] == "failed"
                && p["entries"][&failed]["reason"] == "ipc_retry_exhausted"
        })
    });
    if !context.check("bounded_retries_surface_terminal_reason", terminal) {
        return false;
    }
    if !budget {
        return false;
    }
    let elapsed = started.elapsed();
    if !context.check(
        "retry_backoff_is_bounded",
        elapsed >= Duration::from_secs(15) && elapsed < Duration::from_secs(22),
    ) {
        return false;
    }
    let replica = context.replicas[0].client();
    context.check(
        "replica_surfaces_terminal_failure",
        await_until(1, || {
            progress(&replica).is_some_and(|p| {
                p["entries"][&failed]["state"] == "failed"
                    && p["entries"][&failed]["reason"]
                        .as_str()
                        .is_some_and(|r| !r.is_empty())
            })
        }),
    )
}

fn is_zero_time(value: &Value) -> bool {
    value.as_str().is_none_or(|s| s.starts_with("0001-01-01"))
}

fn blob_path(home: &Path, digest: &str) -> PathBuf {
    home.join("blobs/sha256")
        .join(&digest[..2])
        .join(&digest[2..])
}

fn control(context: &Context, input: &Value) -> Result<Value> {
    context.world.blob_control(input)
}

pub(super) fn gc_migration(context: &mut Context, record: &str, digest: &str) -> bool {
    context.world.stop_gateway();
    let db = context.world.gateway_home.join("state.db");
    let removed = Connection::open(&db).and_then(|conn| {
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS blob_refs_record_delete; DROP TRIGGER IF EXISTS blob_refs_staging_delete; DROP TABLE IF EXISTS blob_refs; DELETE FROM meta WHERE key='blob_refs_version'",
        )
    });
    if !context.check("legacy_reference_index_open", db.exists()) {
        return false;
    }
    if !context.check("legacy_reference_index_removed", removed.is_ok()) {
        return false;
    }
    let restarted = context.world.start_gateway().is_ok();
    if !context.check("reference_index_migration_restart", restarted) {
        return false;
    }
    if !context.check(
        "reference_index_migration_converges",
        await_until(10, || context.converged()),
    ) {
        return false;
    }
    let conn = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY);
    if !context.check("migrated_reference_index_open", conn.is_ok()) {
        return false;
    }
    let references: Option<i64> = conn.ok().and_then(|c| {
        c.query_row(
            "SELECT count(*) FROM blob_refs WHERE owner=?1 AND digest=?2",
            [format!("record:{record}"), digest.to_owned()],
            |row| row.get(0),
        )
        .ok()
    });
    context.check("legacy_reference_index_backfilled", references == Some(1))
}

pub(super) fn gc_lifecycle(context: &mut Context, digest: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let home = context.world.gateway_home.clone();
    let path = blob_path(&home, digest);
    let private = fs::metadata(&path).is_ok_and(|m| m.permissions().mode() & 0o777 == 0o600);
    if !context.check("private_immutable_blob", private) {
        return false;
    }
    sleep(700);
    if !context.check("referenced_blob_survives_grace", true) {
        return false;
    }
    if !context.check("record_protects_blob", path.exists()) {
        return false;
    }
    let snapshot = control(context, &json!({"action":"snapshot","owner":"gc-snapshot"}));
    if !context.check("snapshot_reference_acquired", snapshot.is_ok()) {
        return false;
    }
    let staged = control(context, &json!({"action":"stage"})).ok();
    let pin = staged
        .as_ref()
        .and_then(|v| v["pin"].as_str())
        .unwrap_or_default()
        .to_owned();
    let staged_digest = staged
        .as_ref()
        .and_then(|v| v["digest"].as_str())
        .unwrap_or_default()
        .to_owned();
    if !context.check(
        "stage_returns_pin",
        !pin.is_empty() && staged_digest.len() == 64,
    ) {
        return false;
    }
    let pinned = blob_path(&home, &staged_digest);
    let deleted = context
        .world
        .control("/blob-fixture", &json!({"delete":true}));
    if !context.check("delete_attachment_and_message", deleted.is_ok()) {
        return false;
    }
    if !context.check(
        "deletion_metadata_converges",
        await_until(5, || context.converged()),
    ) {
        return false;
    }
    sleep(800);
    if !context.check(
        "snapshot_only_and_pin_survive_grace",
        path.exists() && pinned.exists(),
    ) {
        return false;
    }
    let command = control(
        context,
        &json!({"action":"command","owner":"gc-command","digest":digest}),
    );
    if !context.check("command_reference_acquired", command.is_ok()) {
        return false;
    }
    let released = control(
        context,
        &json!({"action":"snapshot-release","owner":"gc-snapshot"}),
    );
    if !context.check("snapshot_release", released.is_ok()) {
        return false;
    }
    sleep(700);
    if !context.check("command_only_survives_grace", path.exists()) {
        return false;
    }
    let released = control(
        context,
        &json!({"action":"command-release","owner":"gc-command"}),
    );
    if !context.check("command_release", released.is_ok()) {
        return false;
    }
    sleep(700);
    if !context.check("uncommitted_pin_still_protects_blob", pinned.exists()) {
        return false;
    }
    let released = control(context, &json!({"action":"release","pin":pin}));
    if !context.check("staging_pin_release", released.is_ok()) {
        return false;
    }
    if !context.check(
        "released_pin_collected",
        await_until(5, || !pinned.exists()),
    ) {
        return false;
    }
    let replica = context.replicas[0].client();
    context.check(
        "unreferenced_blob_collected",
        await_until(10, || {
            !path.exists() && progress(&replica).is_some_and(|p| p["entries"][digest].is_null())
        }),
    )
}
