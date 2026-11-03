use crate::{
    assertions::{Observed, await_until, converged, false_deletion_free},
    core,
    http::{Client, random_id},
    journey::{GATEWAY_SCOPE, gateway_client},
    replicas::{self, Endpoint},
    watch::Watcher,
};
use jiff::{SignedDuration, Timestamp};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 2] = ["CORE-DOCTOR", "CORE-CLOCK-DELAY"];
const REPLICATION: [&str; 0] = [];

struct Robust {
    observed: Observed,
    watchers: Vec<Watcher>,
    aborted: bool,
}
static STATE: Mutex<Option<Robust>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<Robust>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}
fn abort(evidence: &mut Evidence, name: &str) -> Result<()> {
    evidence.check(name, false, name);
    if let Some(state) = lock().as_mut() {
        state.aborted = true;
    }
    Err(Error::Invalid(name.into()))
}
pub fn start(world: &mut World, id: &str, count: usize) -> Result<()> {
    if id == "CORE-CLOCK-DELAY" {
        world.install_proxy()?;
    }
    replicas::start(world, count)
}
fn ensure(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    if lock().is_some() {
        return Ok(());
    }
    let gateway = gateway_client(world)?;
    let observed = Observed::new(gateway, GATEWAY_SCOPE);
    evidence.check(
        "feed_network_loopback",
        true,
        "replicas dial the gateway on loopback",
    );
    if !await_until(15, || converged(world, &observed)) {
        evidence.check("initial_convergence", false, "world did not converge");
        return Err(Error::Invalid("initial_convergence".into()));
    }
    let mut watchers = vec![];
    for client in observed.clients() {
        watchers.push(Watcher::start(&client, false)?);
    }
    *lock() = Some(Robust {
        observed,
        watchers,
        aborted: false,
    });
    Ok(())
}
fn doctor(home: Option<&std::path::Path>, world: &World) -> Result<Value> {
    let text = match home {
        Some(home) => replicas::cli(home, &["doctor"])?,
        None => world.gateway_cli(&["doctor"])?,
    };
    Ok(serde_json::from_str(&text)?)
}
fn number(value: &Value, key: &str) -> Option<f64> {
    value[key].as_f64()
}
fn has_warnings(report: &Value) -> bool {
    report["warnings"].as_array().is_some_and(|w| !w.is_empty())
}
fn clock_warnings(endpoint: &Endpoint) -> Result<(bool, bool)> {
    let path = endpoint.home.join("gateway-clock.json");
    let saved = fs::read_to_string(&path)?;
    replicas::signal(0, "STOP")?;
    let outcome = (|| -> Result<(bool, bool)> {
        let now = Timestamp::now();
        let at = |millis: i64| {
            now.checked_add(SignedDuration::from_millis(millis))
                .map_or_else(|_| now.to_string(), |t| t.to_string())
        };
        let skewed =
            json!({"t0": at(-11), "t1": at(10_000 - 6), "t2": at(10_000 - 5), "t3": at(0)});
        fs::write(&path, skewed.to_string())?;
        let skew = replicas::cli(&endpoint.home, &["doctor"])?;
        let skew: Value = serde_json::from_str(&skew)?;
        fs::write(&path, "{}")?;
        let missing = replicas::cli(&endpoint.home, &["doctor"])?;
        let missing: Value = serde_json::from_str(&missing)?;
        Ok((
            skew["clock_skew"] == "warning_clock_skew" && has_warnings(&skew),
            missing["clock_skew"] == "unverified" && has_warnings(&missing),
        ))
    })();
    let _ = fs::write(&path, saved);
    let _ = replicas::signal(0, "CONT");
    outcome
}
fn doctor_case(world: &World, evidence: &mut Evidence) -> Result<()> {
    let endpoint = replicas::endpoints()
        .into_iter()
        .next()
        .ok_or_else(|| Error::Invalid("replica".into()))?;
    let gateway = doctor(None, world);
    evidence.check(
        "gateway_clock_not_applicable",
        gateway.is_ok_and(|g| g["clock_skew"] == "n/a"),
        "gateway has no clock estimate",
    );
    let Ok(replica) = doctor(Some(&endpoint.home), world) else {
        return abort(evidence, "replica_doctor");
    };
    let sample = number(&replica, "clock_sample_age_ms");
    evidence.check(
        "replica_authenticated_feed_clock_sample",
        number(&replica, "clock_skew_ms").is_some()
            && sample.is_some_and(|age| (0.0..15000.0).contains(&age))
            && replica["clock_method"] == "authenticated_feed_ntp_four_timestamps",
        "replica holds an authenticated feed clock sample",
    );
    let warnings = clock_warnings(&endpoint);
    evidence.check(
        "clock_skew_warning_actionable",
        warnings.as_ref().is_ok_and(|w| w.0),
        "clock skew warns",
    );
    evidence.check(
        "clock_sample_unavailable_warning_actionable",
        warnings.as_ref().is_ok_and(|w| w.1),
        "missing clock sample warns",
    );
    Ok(())
}
fn clock_delay_case(world: &World, evidence: &mut Evidence) -> Result<()> {
    let endpoint = replicas::endpoints()
        .into_iter()
        .next()
        .ok_or_else(|| Error::Invalid("replica".into()))?;
    let report = || doctor(Some(&endpoint.home), world).unwrap_or(Value::Null);
    let mut baseline = Value::Null;
    let ready = await_until(5, || {
        baseline = report();
        baseline["clock_method"] == "authenticated_feed_ntp_four_timestamps"
    });
    evidence.check(
        "authenticated_four_timestamp_clock_sample",
        ready,
        "authenticated clock sample present",
    );
    if !ready {
        return abort_silent();
    }
    let (Some(base_offset), Some(base_uncertainty), Some(base_rtt)) = (
        number(&baseline, "clock_offset_ms"),
        number(&baseline, "clock_uncertainty_ms"),
        number(&baseline, "clock_rtt_ms"),
    ) else {
        evidence.check("clock_estimate_fields", false, "clock estimate fields");
        return abort_silent();
    };
    world.delay_both(Duration::from_secs(2))?;
    let mut delayed = Value::Null;
    let crossed = await_until(25, || {
        delayed = report();
        number(&delayed, "clock_rtt_ms").is_some_and(|rtt| rtt >= 3500.0)
    });
    evidence.check(
        "authenticated_probe_crossed_delayed_feed",
        crossed,
        "probe crossed the delayed feed",
    );
    if !crossed {
        world.delay_both(Duration::ZERO)?;
        return abort_silent();
    }
    let (offset, uncertainty, rtt) = (
        number(&delayed, "clock_offset_ms"),
        number(&delayed, "clock_uncertainty_ms"),
        number(&delayed, "clock_rtt_ms"),
    );
    evidence.check(
        "offset_stays_near_zero_as_transit_grows",
        offset.is_some_and(|o| {
            base_offset.abs() < 250.0 && o.abs() < 250.0 && (o - base_offset).abs() < 250.0
        }),
        "offset stays near zero",
    );
    evidence.check(
        "uncertainty_tracks_round_trip_delay",
        matches!((uncertainty, rtt), (Some(u), Some(r))
            if u > base_uncertainty + 1500.0 && (2.0 * u - r).abs() <= 2.0 && r > base_rtt + 3000.0),
        "uncertainty tracks round trip",
    );
    evidence.check(
        "transit_not_misreported_as_clock_skew",
        delayed["clock_skew"] != "warning_clock_skew",
        "transit is not skew",
    );
    world.delay_both(Duration::ZERO)?;
    let recovered = await_until(15, || {
        number(&report(), "clock_rtt_ms").is_some_and(|rtt| rtt < 1000.0)
    });
    evidence.check(
        "clock_round_trip_recovers_after_delay_removed",
        recovered,
        "round trip recovers",
    );
    if recovered { Ok(()) } else { abort_silent() }
}
fn cred_lifecycle_case(world: &World, evidence: &mut Evidence) -> Result<()> {
    let home = world.directory.join("credentials").display().to_string();
    let result = Command::new(replicas::binary()?)
        .args(["lab-credential-lifecycle", "--home", &home])
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .output();
    let parsed = result
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
    let Some(report) = parsed else {
        return abort(evidence, "linux_credential_adapter");
    };
    let backend = report["backend"].as_str().unwrap_or_default();
    evidence.check(
        &format!("credentials_{backend}_create_use_rotate_revoke"),
        ["created", "used", "rotated", "revoked"]
            .iter()
            .all(|k| report[*k] == true),
        "credential created, used, rotated and revoked",
    );
    Ok(())
}
fn abort_silent() -> Result<()> {
    if let Some(state) = lock().as_mut() {
        state.aborted = true;
    }
    Err(Error::Invalid("case stopped".into()))
}
pub fn robustness(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let case = input["case"].as_str().unwrap_or_default();
    if case == "credential-store" {
        return core::robustness(world, input, evidence);
    }
    ensure(world, evidence)?;
    match case {
        "doctor" => doctor_case(world, evidence),
        "clock-delay" => clock_delay_case(world, evidence),
        "cred-lifecycle" => cred_lifecycle_case(world, evidence),
        other => Err(Error::Invalid(format!(
            "robustness case not ported: {other}"
        ))),
    }
}
fn search_hidden(client: &Client) -> Result<bool> {
    let reply = client.call_raw("search", &json!({"text": "ab"}), 20)?;
    Ok(reply.is_none_or(|(status, value)| {
        matches!(status, 404 | 422)
            || (value["records"].as_array().is_none_or(Vec::is_empty) && value["error"].is_null())
    }))
}
fn system_endpoints(world: &World, observed: &Observed) -> Result<Vec<(Client, String)>> {
    Ok(std::iter::once((
        observed.gateway.clone(),
        world
            .gateway_cli(&["token", "create", "--scope", "system:read"])?
            .trim()
            .to_owned(),
    ))
    .chain(observed.replicas.iter().map(|e| {
        (
            e.client(),
            replicas::cli(&e.home, &["token", "create", "--scope", "system:read"])
                .unwrap_or_default(),
        )
    }))
    .collect())
}
fn automatic(world: &World, observed: &Observed, evidence: &mut Evidence) -> Result<bool> {
    let drained = world.control("/flush", &json!({})).is_ok();
    evidence.check(
        "automatic_recovery_capture_drained",
        drained,
        "captures drained",
    );
    if !drained {
        return Ok(false);
    }
    let fixture = observed
        .gateway
        .call(
            "records.query",
            &json!({"collection":"reminders.reminder","limit":1}),
            20,
        )?
        .unwrap_or(Value::Null);
    let Some(record) = fixture["records"].as_array().and_then(|r| r.first()) else {
        return Ok(false);
    };
    let id = record["id"].as_str().unwrap_or_default().to_owned();
    let mut watchers = vec![];
    let mut scoped = vec![];
    let endpoints = system_endpoints(world, observed)?;
    for (index, (client, token)) in endpoints.iter().enumerate() {
        watchers.push(Watcher::start(client, false)?);
        let limited = Client::at(client.address(), token);
        let page = limited.call(
            "records.query",
            &json!({"collection":"reminders.reminder"}),
            20,
        )?;
        if page.is_none_or(|p| p["records"].as_array().is_some_and(|r| !r.is_empty())) {
            return Ok(false);
        }
        let path = format!("/v1/records/reminders.reminder/{id}");
        if limited.request("GET", &path, &[], None, 20)?.status != 404 {
            return Ok(false);
        }
        if !search_hidden(&limited)? {
            evidence.check(
                &format!("automatic_scope_search_{index}"),
                false,
                "scoped search leaked",
            );
            return Ok(false);
        }
        evidence.check(
            &format!("automatic_scope_query_search_records_{index}"),
            true,
            "scoped reads are empty",
        );
        scoped.push(Watcher::start(&limited, true)?);
    }
    let probe = random_id("automatic")?;
    world.control("/external-change", &json!({"key": probe}))?;
    let effect = Instant::now();
    let title = format!("[ab-test] {probe}");
    if !await_until(10, || {
        watchers
            .iter()
            .all(|w| w.first_seen_title("reminders.reminder", &title).is_some())
    }) {
        return Ok(false);
    }
    for (index, watcher) in watchers.iter().enumerate() {
        let seen = watcher.first_seen_title("reminders.reminder", &title);
        let latency = seen.map_or(f64::MAX, |at| {
            at.saturating_duration_since(effect).as_secs_f64() * 1000.0
        });
        evidence.check(
            &format!("automatic_metadata_latency_and_no_false_deletion_{index}"),
            watcher.valid() && false_deletion_free(world, observed, watcher) && latency < 1000.0,
            "probe visible within one second without false deletes",
        );
    }
    if !await_until(5, || scoped.iter().all(Watcher::advanced)) {
        return Ok(false);
    }
    for (index, watcher) in scoped.iter().enumerate() {
        evidence.check(
            &format!("automatic_scope_sse_watermark_{index}"),
            watcher.valid(),
            "scoped stream is contiguous",
        );
    }
    Ok(true)
}
pub fn finish(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let state = lock().take();
    let Some(state) = state.filter(|s| !s.aborted) else {
        return Ok(());
    };
    let drained = world.control("/flush", &json!({})).is_ok();
    evidence.check(
        "recovery_capture_drained_before_convergence",
        drained,
        "captures drained before convergence",
    );
    if !drained {
        return Ok(());
    }
    if !await_until(30, || converged(world, &state.observed)) {
        evidence.check("truth_gateway_replica_convergence", false, "world diverged");
        return Ok(());
    }
    evidence.check(
        "truth_gateway_replica_convergence",
        true,
        "truth, gateway and replicas agree",
    );
    if !automatic(world, &state.observed, evidence).unwrap_or(false) {
        evidence.check("automatic_sync_guarantees", false, "automatic guarantees");
        return Ok(());
    }
    let gateway = doctor(None, world).ok();
    let valid = |report: Option<&Value>| {
        report.is_some_and(|r| r["ok"] == true && r["exactly_once"] == true)
    };
    evidence.check(
        "sqlite_integrity_gateway",
        valid(gateway.as_ref()),
        "gateway doctor ok and exactly once",
    );
    for endpoint in &state.observed.replicas {
        let report = doctor(Some(&endpoint.home), world).ok();
        evidence.check(
            "sqlite_integrity_replica",
            valid(report.as_ref()),
            "replica doctor ok and exactly once",
        );
    }
    for watcher in &state.watchers {
        evidence.check(
            "no_false_deletion_in_observed_journal_changes",
            false_deletion_free(world, &state.observed, watcher),
            "no observed delete is false",
        );
    }
    if REPLICATION.contains(&evidence.scenario.as_str()) {
        core::replication_resets(world, evidence);
    }
    Ok(())
}
