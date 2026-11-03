use crate::{
    assertions::{Observed, await_until, converged},
    journey::{GATEWAY_SCOPE, gateway_client, records},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const SCENARIOS: [&str; 4] = [
    "REM-STORE-READ",
    "REM-STORE-DECODE",
    "REM-STORE-PERF",
    "REM-SECTIONS",
];

fn build(target: &str, lab: bool) -> Option<PathBuf> {
    let mut command = Command::new("packaging/dev/build-native.sh");
    command.env("APPLE_BRIDGE_NATIVE_TARGET", target);
    if lab {
        command.env("APPLE_BRIDGE_NATIVE_LAB", "1");
    } else {
        command.env_remove("APPLE_BRIDGE_NATIVE_LAB");
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    output_directory()
}
fn output_directory() -> Option<PathBuf> {
    fs::read_to_string("native/.build/native-output-1")
        .ok()
        .map(|text| PathBuf::from(text.trim()))
}
fn combined(tool: &Path, args: &[&str]) -> (bool, String) {
    match Command::new(tool).args(args).output() {
        Ok(output) => (
            output.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        ),
        Err(error) => (false, error.to_string()),
    }
}
fn store_script(args: &[&str]) -> (bool, String) {
    combined(Path::new("packaging/dev/check-reminders-store.sh"), args)
}
fn data_of(record: &Value) -> &Value {
    &record["data"]
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "REM-STORE-PERF" => perf(world, evidence),
        "REM-SECTIONS" => sections(world, evidence),
        _ => read(world, evidence),
    }
    Ok(())
}

fn read(world: &World, evidence: &mut Evidence) {
    evidence.check("world_start", true, "gateway and SimApple peer ready");
    let report = world.directory.join("reminders-store-fixture.json");
    let report_text = report.to_string_lossy().into_owned();
    let mut args = vec!["--report", report_text.as_str()];
    if evidence.scenario == "REM-STORE-DECODE" {
        args.push("--blob-checks");
    }
    let (ok, output) = store_script(&args);
    evidence.check(
        "synthetic_sqlite_arbitrary_entities_wal_isolation_and_projection",
        ok,
        &output,
    );
    if !ok {
        return;
    }
    let observations: Vec<Value> = fs::read(&report)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value["observations"].as_array().cloned())
        .unwrap_or_default();
    evidence.check(
        "synthetic_reader_observations",
        observations.len() == 3,
        "three reader observations decoded",
    );
    if observations.len() != 3 {
        return;
    }
    for observation in &observations {
        let injected = world.control(
            "/external-change",
            &json!({
                "collection": observation["collection"],
                "key": observation["native_key"],
                "data": observation["data"]
            }),
        );
        if injected.is_err() {
            evidence.check("reader_capture_committed", false, "capture rejected");
            return;
        }
    }
    let Ok(gateway) = gateway_client(world) else {
        evidence.check("gateway_records", false, "gateway client unavailable");
        return;
    };
    let observed = Observed::new(gateway.clone(), GATEWAY_SCOPE);
    evidence.check(
        "store_fields_truth_gateway_and_replica",
        await_until(5, || converged(world, &observed)),
        "truth, gateway and replica agree",
    );
    let all = records(&gateway);
    evidence.check("gateway_records", all.is_ok(), "gateway records read");
    let all = all.map(|r| r.1).unwrap_or_default();
    let (mut parent, mut child) = (Value::Null, Value::Null);
    for record in &all {
        match data_of(record)["title"].as_str().unwrap_or_default() {
            "Bridge Test Parent" => parent = record.clone(),
            "Bridge Test Child" => child = record.clone(),
            "Bridge Test List" => {
                let d = data_of(record);
                evidence.check(
                    "list_native_emblem_pin_order_grocery_sharing",
                    d["badge_emblem"] == "star"
                        && d["pinned"] == true
                        && d["display_order"] == 12
                        && d["categorize_groceries"] == true
                        && d["shared_owner_name"] == "Bridge Test Owner",
                    "list fields round-trip",
                );
            }
            _ => {}
        }
    }
    evidence.check(
        "flagged_exact_and_parent_linked",
        !parent.is_null()
            && !child.is_null()
            && data_of(&parent)["flagged"] == true
            && data_of(&child)["parent_id"] == parent["id"],
        "flag and parent link round-trip",
    );
    evidence.check(
        "unsupported_order_and_sections_not_guessed",
        data_of(&parent)["manual_order"].is_null() && data_of(&parent)["section_id"].is_null(),
        "unsupported fields stay absent",
    );
}

fn perf(world: &World, evidence: &mut Evidence) {
    let directory = world.directory.join("reminders-store-perf");
    let created = fs::create_dir_all(&directory).is_ok();
    evidence.check(
        "private_profile_directory",
        created,
        "profile directory created",
    );
    if !created {
        return;
    }
    let report = directory.join("profile.json");
    let report_text = report.to_string_lossy().into_owned();
    let (ok, output) = store_script(&["--perf-report", &report_text]);
    evidence.check("store_profile_completed", ok, &output);
    if !ok {
        return;
    }
    let profile: Option<Value> = fs::read(&report)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let decoded = profile.as_ref().is_some_and(Value::is_object);
    evidence.check(
        "timing_only_profile_decoded",
        decoded,
        "profile decoded as an object",
    );
    let Some(profile) = profile.filter(|p| p.as_object().is_some_and(|o| !o.is_empty())) else {
        return;
    };
    let total = |mode: &str| profile[mode]["total"].clone();
    let samples = |mode: &str| total(mode)["samples"].as_u64().unwrap_or(0);
    for mode in [
        "warm_full",
        "warm_incremental_unchanged",
        "warm_incremental_changed_1",
        "warm_incremental_changed_10",
        "warm_incremental_changed_100",
        "warm_incremental_changed_1000",
    ] {
        if profile[mode].is_null() || !profile[mode].is_object() {
            continue;
        }
        let budget = match mode {
            "warm_full" => 100.0,
            "warm_incremental_changed_1" | "warm_incremental_changed_10" => 5.0,
            _ => 50.0,
        };
        let p95 = total(mode)["p95_ms"].as_f64().unwrap_or(f64::MAX);
        evidence.check(
            &format!("{mode}_200_samples_within_budget"),
            samples(mode) == 200 && p95 < budget,
            &format!("p95 {p95:.2} ms against {budget} ms"),
        );
    }
    evidence.check(
        "full_and_incremental_measured",
        samples("warm_full") == 200 && samples("warm_incremental_unchanged") == 200,
        "full and unchanged incremental modes measured",
    );
    let complete = [
        "warm_incremental_changed_1",
        "warm_incremental_changed_10",
        "warm_incremental_changed_100",
        "warm_incremental_changed_1000",
    ]
    .iter()
    .all(|mode| samples(mode) == 200);
    evidence.check(
        "changed_history_measured",
        complete,
        "every changed-history mode measured",
    );
}

fn sections(_: &World, evidence: &mut Evidence) {
    let Some(directory) = build("RemindersStoreChecks", false) else {
        evidence.check("native_sections_build", false, "native build failed");
        return;
    };
    evidence.check("native_sections_build", true, "RemindersStoreChecks built");
    let (ok, output) = combined(&directory.join("RemindersStoreChecks"), &["--section-checks"]);
    evidence.check(
        "store_effects_and_invalidation",
        ok && output.contains("section checks passed"),
        "native section checks",
    );
    let helper = match env::var("APPLE_BRIDGE_REMINDERS_CHECKS_HELPER") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => {
            let built = build("AppleBridgeHelper", false);
            evidence.check(
                "native_fixture_checks_build",
                built.is_some(),
                "helper build",
            );
            directory.join("AppleBridgeHelper")
        }
    };
    let available = helper.is_file();
    evidence.check(
        "native_fixture_checks_helper_available",
        available,
        "helper executable present",
    );
    if available {
        let (ok, output) = combined(&helper, &["reminders-fixture-checks"]);
        evidence.check(
            "owned_fixture_accessibility_regressions",
            ok && output.contains("reminders fixture checks passed"),
            "owned fixture checks",
        );
    }
}
