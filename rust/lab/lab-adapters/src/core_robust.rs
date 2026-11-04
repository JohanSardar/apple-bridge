use crate::replicas::Endpoint;
use crate::{assertions::await_until, replicas};
use jiff::{SignedDuration, Timestamp};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{fs, process::Command, time::Duration};

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
fn doctor_case(world: &World, evidence: &mut Evidence) -> bool {
    let Some(endpoint) = replicas::endpoints().into_iter().next() else {
        return false;
    };
    let gateway = doctor(None, world);
    evidence.check(
        "gateway_clock_not_applicable",
        gateway.is_ok_and(|g| g["clock_skew"] == "n/a"),
        "gateway has no clock estimate",
    );
    let Ok(replica) = doctor(Some(&endpoint.home), world) else {
        evidence.check("replica_doctor", false, "replica doctor failed");
        return false;
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
    true
}
pub(crate) fn clock_delay_case(world: &World, evidence: &mut Evidence) -> bool {
    let Some(endpoint) = replicas::endpoints().into_iter().next() else {
        return false;
    };
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
        return false;
    }
    let (Some(base_offset), Some(base_uncertainty), Some(base_rtt)) = (
        number(&baseline, "clock_offset_ms"),
        number(&baseline, "clock_uncertainty_ms"),
        number(&baseline, "clock_rtt_ms"),
    ) else {
        evidence.check("clock_estimate_fields", false, "clock estimate fields");
        return false;
    };
    if world.delay_both(Duration::from_secs(2)).is_err() {
        return false;
    }
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
        let _ = world.delay_both(Duration::ZERO);
        return false;
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
    let _ = world.delay_both(Duration::ZERO);
    let recovered = await_until(15, || {
        number(&report(), "clock_rtt_ms").is_some_and(|rtt| rtt < 1000.0)
    });
    evidence.check(
        "clock_round_trip_recovers_after_delay_removed",
        recovered,
        "round trip recovers",
    );
    recovered
}
fn cred_lifecycle_case(world: &World, evidence: &mut Evidence) -> bool {
    let home = world.directory.join("credentials").display().to_string();
    let Ok(binary) = replicas::binary() else {
        return false;
    };
    let result = Command::new(binary)
        .args(["lab-credential-lifecycle", "--home", &home])
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .output();
    let parsed = result
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok());
    let Some(report) = parsed else {
        evidence.check(
            "linux_credential_adapter",
            false,
            "credential adapter failed",
        );
        return false;
    };
    let backend = match report["backend"].as_str().unwrap_or_default() {
        "file" => "file_0600",
        other => other,
    };
    evidence.check(
        &format!("credentials_{backend}_create_use_rotate_revoke"),
        ["created", "used", "rotated", "revoked"]
            .iter()
            .all(|k| report[*k] == true),
        "credential created, used, rotated and revoked",
    );
    true
}
pub fn run(case: &str, world: &World, evidence: &mut Evidence) -> bool {
    match case {
        "doctor" => doctor_case(world, evidence),
        "clock-delay" => clock_delay_case(world, evidence),
        _ => cred_lifecycle_case(world, evidence),
    }
}
