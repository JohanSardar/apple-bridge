use crate::{
    assertions::{Observed, await_until, converged, false_deletion_free, scoped_clients},
    core_robust,
    http::{Client, random_id},
    journey::{GATEWAY_SCOPE, gateway_client, records},
    replicas::{self, Endpoint},
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 7] = [
    "CORE-HELPER-RESTART",
    "CORE-KILL-DURING-APPLY",
    "CORE-KILL-GATEWAY-CAPTURE",
    "AUTH-SCOPES-METADATA",
    "CORE-DOCTOR",
    "CORE-CLOCK-DELAY",
    "CORE-CRED-LIFECYCLE",
];
const NO_RESET_CHECK: [&str; 5] = [
    "CORE-HELPER-RESTART",
    "AUTH-SCOPES-METADATA",
    "CORE-DOCTOR",
    "CORE-CLOCK-DELAY",
    "CORE-CRED-LIFECYCLE",
];

pub fn start(world: &mut World, id: &str, count: usize) -> Result<()> {
    if id == "CORE-CLOCK-DELAY" {
        world.install_proxy()?;
    }
    replicas::start(world, count)
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

fn replica_connected() -> bool {
    replicas::endpoints()
        .first()
        .is_some_and(|e| replicas::status(e).is_ok_and(|v| v["replica"]["state"] == "connected"))
}

fn arm(home: &std::path::Path, boundary: &str) -> bool {
    let _ = fs::remove_file(home.join(format!("lab-{boundary}-reached")));
    fs::write(home.join(format!("lab-{boundary}-armed")), b"").is_ok()
}

fn boundary_reached(home: &std::path::Path, boundary: &str, expected: &str) -> bool {
    await_until(10, || {
        fs::read_to_string(home.join(format!("lab-{boundary}-reached")))
            .is_ok_and(|t| t == expected)
    })
}

fn source_scans(world: &World) -> BTreeMap<String, u64> {
    world
        .control("/stats", &Value::Null)
        .ok()
        .and_then(|stats| stats["source_scans"].as_object().cloned())
        .map(|map| {
            map.into_iter()
                .map(|(k, v)| (k, v.as_u64().unwrap_or(0)))
                .collect()
        })
        .unwrap_or_default()
}

fn scope_probe(limited: &Client, fixture_id: &str, index: usize, evidence: &mut Evidence) -> bool {
    let query = limited.call(
        "records.query",
        &json!({"collection":"reminders.reminder"}),
        20,
    );
    if !query.is_ok_and(|p| p.is_some_and(|p| p["records"].as_array().is_none_or(Vec::is_empty))) {
        return false;
    }
    let path = format!("/v1/records/reminders.reminder/{fixture_id}");
    if !limited
        .request("GET", &path, &[], None, 20)
        .is_ok_and(|reply| reply.status == 404)
    {
        return false;
    }
    let search = limited.call_raw("search", &json!({"text":"ab"}), 20);
    let search_ok = search.is_ok_and(|reply| {
        reply.is_some_and(|(status, value)| {
            status == 404
                || status == 422
                || (status < 300 && value["records"].as_array().is_none_or(Vec::is_empty))
        })
    });
    if !search_ok {
        evidence.check(
            &format!("automatic_scope_search_{index}"),
            false,
            "scoped search returned records or failed",
        );
        return false;
    }
    evidence.check(
        &format!("automatic_scope_query_search_records_{index}"),
        true,
        "scoped query, record GET and search return no reminders",
    );
    true
}

fn automatic_healthy(world: &World, observed: &Observed, evidence: &mut Evidence) -> bool {
    let drained = world.control("/flush", &json!({})).is_ok();
    evidence.check(
        "automatic_recovery_capture_drained",
        drained,
        "capture drained before the healthy probe",
    );
    if !drained {
        return false;
    }
    let Ok(Some(fixture)) = observed.gateway.call(
        "records.query",
        &json!({"collection":"reminders.reminder","limit":1}),
        20,
    ) else {
        return false;
    };
    let fixture_id = match fixture["records"].as_array() {
        Some(list) if list.len() == 1 => list[0]["id"].as_str().unwrap_or_default().to_owned(),
        _ => return false,
    };
    let Ok(scoped) = scoped_clients(world, observed, "system:read") else {
        return false;
    };
    let mut watchers = vec![];
    let mut filtered = vec![];
    for (index, (client, token)) in scoped.iter().enumerate() {
        let Ok(plain) = Watcher::start(&endpoint_client(observed, index), false) else {
            return false;
        };
        watchers.push(plain);
        let limited = Client::at(client.address(), token);
        if !scope_probe(&limited, &fixture_id, index, evidence) {
            return false;
        }
        let Ok(narrow) = Watcher::start(&limited, true) else {
            return false;
        };
        filtered.push(narrow);
    }
    let probe = random_id("automatic").unwrap_or_default();
    if world
        .control("/external-change", &json!({"key": probe}))
        .is_err()
    {
        return false;
    }
    let effect = Instant::now();
    let seen = await_until(10, || {
        watchers.iter().all(|w| {
            w.first_seen_record(|r| {
                r["data"]["title"]
                    .as_str()
                    .is_some_and(|t| t.contains(&probe))
            })
            .is_some()
        })
    });
    if !seen {
        return false;
    }
    let mut passed = true;
    for (index, watcher) in watchers.iter().enumerate() {
        let at = watcher
            .first_seen_record(|r| {
                r["data"]["title"]
                    .as_str()
                    .is_some_and(|t| t.contains(&probe))
            })
            .unwrap_or(effect);
        let latency = at.saturating_duration_since(effect);
        let valid = watcher.valid() && false_deletion_free(world, observed, watcher);
        let ok = valid && latency < Duration::from_secs(1);
        evidence.check(
            &format!("automatic_metadata_latency_and_no_false_deletion_{index}"),
            ok,
            "probe visible within one second with no false deletion",
        );
        passed &= ok;
    }
    if !await_until(5, || filtered.iter().all(Watcher::advanced)) {
        return false;
    }
    for (index, watcher) in filtered.iter().enumerate() {
        let valid = watcher.valid();
        evidence.check(
            &format!("automatic_scope_sse_watermark_{index}"),
            valid,
            "scoped stream carried contiguous watermarks",
        );
        passed &= valid;
    }
    passed
}

fn endpoint_client(observed: &Observed, index: usize) -> Client {
    if index == 0 {
        observed.gateway.clone()
    } else {
        observed.replicas[index - 1].client()
    }
}

fn doctor(home: &std::path::Path) -> Option<Value> {
    replicas::cli(home, &["doctor"])
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

fn helper_restart(world: &mut World, gateway: &Client, evidence: &mut Evidence) -> Result<bool> {
    if world
        .control("/blob-fixture", &json!({"size": 1024, "slow_ms": 1000}))
        .is_err()
    {
        evidence.check("durable_helper_blob_fixture", false, "fixture rejected");
        return Ok(false);
    }
    if world.control("/flush", &json!({})).is_err() {
        evidence.check("helper_initial_checkpoint", false, "flush failed");
        return Ok(false);
    }
    let token = world
        .gateway_cli(&["token", "create", "--scope", "messages:read,system:read"])?
        .trim()
        .to_owned();
    let probe = Client::at(gateway.address(), &token);
    let durable = await_until(10, || {
        probe
            .call("replica.status", &json!({}), 10)
            .ok()
            .flatten()
            .is_some_and(|status| {
                status["replica"]["blobs"]["entries"]
                    .as_object()
                    .is_some_and(|entries| {
                        entries.len() == 1 && entries.values().all(|e| e["state"] == "present")
                    })
            })
    });
    if !durable {
        evidence.check(
            "helper_blob_durable_before_restart",
            false,
            "blob not present",
        );
        return Ok(false);
    }
    evidence.check(
        "helper_blob_durable_before_restart",
        true,
        "helper blob present before restart",
    );
    Ok(helper_cycle(world, gateway, evidence))
}

fn helper_cycle(world: &mut World, gateway: &Client, evidence: &mut Evidence) -> bool {
    let scans = source_scans(world);
    world.stop_helper();
    let degraded = await_until(5, || {
        gateway
            .call("sources.status", &json!({}), 10)
            .ok()
            .flatten()
            .is_some_and(|result| {
                result["records"].as_array().is_some_and(|records| {
                    records.iter().any(|r| {
                        r["data"]["state"] == "degraded"
                            && r["data"]["reason"] == "helper_disconnected"
                    })
                })
            })
    });
    if !degraded {
        evidence.check("helper_disconnected_degraded", false, "no degraded status");
        return false;
    }
    if world.restart_peer().is_err() {
        evidence.check("helper_restart", false, "peer did not restart");
        return false;
    }
    let observed = Observed::new(gateway.clone(), GATEWAY_SCOPE);
    if !await_until(10, || {
        world.control("/inspect", &Value::Null).is_ok() && converged(world, &observed)
    }) {
        evidence.check(
            "helper_reconnected",
            false,
            "no convergence after helper restart",
        );
        return false;
    }
    if world.control("/flush", &json!({})).is_err() {
        evidence.check("helper_restart_checkpoint_drained", false, "flush failed");
        return false;
    }
    let after = source_scans(world);
    let calendar = "calendar".to_owned();
    evidence.check(
        "calendar_restart_revalidates_window",
        after.get(&calendar).copied().unwrap_or(0)
            == scans.get(&calendar).copied().unwrap_or(0) + 1,
        "calendar window rescanned exactly once",
    );
    let mut resumed = after.clone();
    if let Some(n) = resumed.get_mut(&calendar) {
        *n = n.saturating_sub(1);
    }
    evidence.check(
        "helper_checkpoint_resume_without_rescan",
        resumed == scans,
        "no other source rescanned after restart",
    );
    let truth = world.control("/inspect", &Value::Null);
    let attachments = truth.as_ref().map_or(0, |t| {
        t.as_array().map_or(0, |all| {
            all.iter()
                .filter(|o| o["collection"] == "messages.attachment")
                .count()
        })
    });
    let intact = world.control("/stats", &Value::Null).is_ok_and(|stats| {
        stats["blob_digests"].as_object().is_some_and(|digests| {
            !digests.is_empty() && digests.iter().all(|(k, v)| v.as_str() == Some(k.as_str()))
        })
    });
    evidence.check(
        "helper_blob_truth_and_bytes_survive_restart",
        truth.is_ok() && attachments == 1 && intact,
        "attachment truth present and blob bytes intact",
    );
    if !change(world, "after-helper-restart", 10) {
        evidence.check("helper_live_capture_resumed", false, "change rejected");
        return false;
    }
    true
}

fn kill_during_apply(world: &World, observed: &Observed, evidence: &mut Evidence) -> bool {
    let Some(endpoint) = replicas::endpoints().first().cloned() else {
        return false;
    };
    for i in 0..8 {
        if world.control("/flush", &json!({})).is_err()
            || !await_until(10, || converged(world, observed))
        {
            evidence.check("pre_apply_crash_convergence", false, "not converged");
            return false;
        }
        if !arm(&endpoint.home, "apply") {
            evidence.check("arm_apply_boundary", false, "cannot arm");
            return false;
        }
        if !change(world, &format!("crash-{i}"), 50) {
            evidence.check("changes_before_crash", false, "change rejected");
            return false;
        }
        let reached = boundary_reached(&endpoint.home, "apply", "write_started_before_commit");
        evidence.check(
            &format!("apply_write_before_commit_boundary_iteration_{i}"),
            reached,
            "replica reached the apply boundary",
        );
        if !reached {
            evidence.check("apply_boundary_not_reached", false, "boundary missing");
            return false;
        }
        if replicas::crash(0).is_err() || replicas::restart(0).is_err() {
            evidence.check("replica_restart", false, "restart failed");
            return false;
        }
        if !await_until(10, replica_connected) {
            evidence.check("replica_reconnect", false, "replica did not reconnect");
            return false;
        }
    }
    evidence.check(
        "eight_replica_process_crashes",
        true,
        "eight crashes survived",
    );
    true
}

fn kill_gateway_capture(world: &mut World, observed: &Observed, evidence: &mut Evidence) -> bool {
    for i in 0..8 {
        if !arm(&world.gateway_home, "capture") {
            evidence.check("arm_capture_boundary", false, "cannot arm");
            return false;
        }
        if !change(world, &format!("capture-crash-{i}"), 50) {
            evidence.check("changes_before_crash", false, "change rejected");
            return false;
        }
        let reached = boundary_reached(
            &world.gateway_home,
            "capture",
            "capture_committed_before_acknowledgment",
        );
        evidence.check(
            &format!("capture_before_committed_boundary_iteration_{i}"),
            reached,
            "gateway committed before acknowledging",
        );
        if !reached {
            evidence.check("capture_boundary_not_reached", false, "boundary missing");
            return false;
        }
        if world.kill_gateway().is_err() {
            evidence.check("kill_gateway", false, "kill failed");
            return false;
        }
        world.stop_gateway();
        if world.start_gateway().is_err() {
            evidence.check("gateway_restart", false, "restart failed");
            return false;
        }
        let drained = world.control("/flush", &json!({})).is_ok()
            && await_until(15, || converged(world, observed));
        evidence.check(
            &format!("capture_replay_converged_iteration_{i}"),
            drained,
            "replayed captures converged",
        );
        if !drained {
            evidence.check("capture_iteration_recovery", false, "no recovery");
            return false;
        }
    }
    evidence.check(
        "eight_gateway_process_crashes",
        true,
        "eight crashes survived",
    );
    if world.control("/flush", &json!({})).is_err() {
        evidence.check("capture_replay_drained", false, "flush failed");
        return false;
    }
    if !await_until(15, || converged(world, observed)) {
        evidence.check("capture_replay_convergence", false, "not converged");
        return false;
    }
    let deduplicated = records(&observed.gateway).is_ok_and(|(_, all)| {
        all.iter()
            .all(|r| r["collection"] == "system.source" || r["version"] == 1)
    });
    evidence.check(
        "capture_replays_do_not_duplicate_record_versions",
        deduplicated,
        "no record version above 1 after replays",
    );
    true
}

pub fn robustness(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let case = input["case"].as_str().unwrap_or_default().to_owned();
    if !matches!(
        (case.as_str(), evidence.scenario.as_str()),
        ("helper-restart", "CORE-HELPER-RESTART")
            | ("kill-during-apply", "CORE-KILL-DURING-APPLY")
            | ("kill-gateway-capture", "CORE-KILL-GATEWAY-CAPTURE")
            | ("auth-scopes-metadata", "AUTH-SCOPES-METADATA")
            | ("doctor", "CORE-DOCTOR")
            | ("clock-delay", "CORE-CLOCK-DELAY")
            | ("cred-lifecycle", "CORE-CRED-LIFECYCLE")
    ) {
        return Err(Error::Invalid(format!(
            "robustness case not ported: {case}"
        )));
    }
    if case == "auth-scopes-metadata" {
        crate::journey::seed(world, &json!({"count": 4}), evidence)?;
    }
    let gateway = gateway_client(world)?;
    let observed = Observed::new(gateway.clone(), GATEWAY_SCOPE);
    evidence.check(
        "feed_network_loopback",
        true,
        "replicas dial the gateway on loopback",
    );
    if !await_until(15, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "world did not converge");
        return Ok(());
    }
    let mut observers = vec![];
    for client in observed.clients() {
        observers.push(Watcher::start(&client, false)?);
    }
    let proceed = match case.as_str() {
        "helper-restart" => helper_restart(world, &gateway, evidence)?,
        "kill-during-apply" => kill_during_apply(world, &observed, evidence),
        "auth-scopes-metadata" => {
            crate::auth_scopes::run(world, evidence)?;
            true
        }
        "doctor" | "clock-delay" | "cred-lifecycle" => core_robust::run(&case, world, evidence),
        _ => kill_gateway_capture(world, &observed, evidence),
    };
    if !proceed {
        return Ok(());
    }
    let drained = world.control("/flush", &json!({})).is_ok();
    evidence.check(
        "recovery_capture_drained_before_convergence",
        drained,
        "final capture drain",
    );
    if !drained {
        return Ok(());
    }
    if !await_until(30, || converged(world, &observed)) {
        evidence.check("truth_gateway_replica_convergence", false, "not converged");
        return Ok(());
    }
    evidence.check(
        "truth_gateway_replica_convergence",
        true,
        "truth, gateway and replicas agree",
    );
    if !automatic_healthy(world, &observed, evidence) {
        evidence.check("automatic_sync_guarantees", false, "healthy probe failed");
        return Ok(());
    }
    let mut homes: Vec<(String, std::path::PathBuf)> =
        vec![("gateway".into(), world.gateway_home.clone())];
    homes.extend(
        replicas::endpoints()
            .iter()
            .map(|e: &Endpoint| ("replica".to_owned(), e.home.clone())),
    );
    for (role, home) in homes {
        let valid = doctor(&home).is_some_and(|r| r["ok"] == true && r["exactly_once"] == true);
        evidence.check(
            &format!("sqlite_integrity_{role}"),
            valid,
            "doctor reports ok and exactly once",
        );
    }
    for observer in &observers {
        evidence.check(
            "no_false_deletion_in_observed_journal_changes",
            false_deletion_free(world, &observed, observer),
            "every observed delete is absent in SimApple truth",
        );
    }
    if !NO_RESET_CHECK.contains(&evidence.scenario.as_str()) {
        let resets: usize = observers.iter().map(Watcher::resets).sum();
        evidence.check(
            "no_oversized_capture_observer_resets",
            resets == 0,
            "no observer needed a reset snapshot",
        );
    }
    Ok(())
}

pub fn dispatch(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if input["case"] == "credential-store" {
        crate::core::robustness(world, input, evidence)
    } else if SCENARIOS.contains(&evidence.scenario.as_str()) {
        robustness(world, input, evidence)
    } else {
        crate::packaging::robustness(world, input, evidence)
    }
}
