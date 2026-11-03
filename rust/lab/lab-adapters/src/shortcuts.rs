use crate::http::Client;
use lab_runner::{Evidence, Scenario};
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime},
};

pub const SCENARIOS: [(&str, &str); 14] = [
    ("FILES-TRANSFORM", "files_transform"),
    ("SC-ACTIONS", "shortcuts_actions"),
    ("SC-CAPTURE", "shortcuts_checks"),
    ("SC-RUN-REAL", "shortcuts_checks"),
    ("SC-RUN", "shortcuts_run"),
    ("FOC-SIM", "focus_sim"),
    ("FOC-PHONE-SYNC", "focus_sim"),
    ("CLK-SIM", "shortcuts_checks"),
    ("CLK-ALARM-REAL", "shortcuts_checks"),
    ("CLK-TIMER-REAL", "shortcuts_checks"),
    ("FOC-IDLE", "shortcuts_real_only"),
    ("FOC-SET-REAL", "shortcuts_real_only"),
    ("FOC-STATUS-REAL", "shortcuts_real_only"),
    ("SC-IDLE", "shortcuts_real_only"),
];

pub fn inject(scenario: &mut Scenario) {
    if !scenario.given.is_empty() || !scenario.when.is_empty() || !scenario.then.is_empty() {
        return;
    }
    if let Some((_, action)) = SCENARIOS.iter().find(|(id, _)| *id == scenario.id) {
        scenario.when.push(json!({ *action: {} }));
    }
}

pub(crate) fn native_directory() -> Option<PathBuf> {
    env::var("AB_LAB_NATIVE_OUTPUT")
        .ok()
        .or_else(|| {
            fs::read_to_string("native/.build/native-output-1")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .map(PathBuf::from)
}

fn newest(path: &Path) -> Option<SystemTime> {
    let meta = fs::metadata(path).ok()?;
    let mut latest = meta.modified().ok()?;
    if meta.is_dir() {
        for entry in fs::read_dir(path).ok()? {
            latest = latest.max(newest(&entry.ok()?.path())?);
        }
    }
    Some(latest)
}

fn fresh(binary: &Path, inputs: &[&str]) -> bool {
    let Some(built) = fs::metadata(binary).ok().and_then(|m| m.modified().ok()) else {
        return false;
    };
    inputs
        .iter()
        .all(|input| newest(Path::new(input)).is_some_and(|t| t <= built))
}

fn native_binary(
    evidence: &mut Evidence,
    target: &str,
    inputs: &[&str],
    always_output: bool,
) -> Option<PathBuf> {
    let binary = native_directory().map(|d| d.join(target));
    if binary.as_ref().is_some_and(|b| fresh(b, inputs)) {
        if always_output {
            evidence.check("native_output", true, "native output directory");
        }
        return binary;
    }
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", target)
        .output();
    let ok = built.as_ref().is_ok_and(|o| o.status.success());
    evidence.check("native_build", ok, "native checks build");
    if !ok {
        return None;
    }
    let directory = native_directory();
    evidence.check(
        "native_output",
        directory.is_some(),
        "native output directory",
    );
    directory.map(|d| d.join(target))
}

pub(crate) fn run_native(binary: &Path, args: &[&str]) -> (bool, String) {
    match Command::new(binary).args(args).output() {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ),
        Err(error) => (false, error.to_string()),
    }
}

pub(crate) fn await_until(seconds: u64, mut ready: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if ready() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn put(world: &World, kind: &str, version: u64) -> Result<()> {
    let collection = "files.item";
    let fields = json!({"root":"/Bridge Test","path":"fixture.txt","name":"fixture.txt","kind":"file","size":0,"created":null,"modified":null,"tags":[],"downloaded":true,"icloud_status":"local","fixture_version":version});
    let at = now();
    let mut capture = json!({"source":"files","generation":"native-media-fixture","observations":[{"kind":kind,"collection":collection,"native_key":"native-media-fixture","data":fields,"observed_at":at,"revalidated":true}]});
    if kind == "absent" {
        capture["scan_id"] = json!("native-media-fixture");
        capture["scan_started_at"] = json!(at);
        capture["scope"] = json!([collection]);
        capture["completion_evidence"] = json!("complete");
    }
    world.control("/capture", &capture)?;
    Ok(())
}

fn records(client: &Client) -> Option<Vec<Value>> {
    let out = client
        .call(
            "records.query",
            &json!({"collection":"files.item","limit":10}),
            10,
        )
        .ok()??;
    out["records"].as_array().cloned()
}

fn gateway_flow(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, "admin")?;
    evidence.check("selected_gateway_started", true, "gateway ready");
    let mut id: Option<String> = None;
    for version in 1..=2u64 {
        let sent = put(world, "upsert", version);
        evidence.check(
            &format!("selected_gateway_capture_{version}"),
            sent.is_ok(),
            &sent
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        if sent.is_err() {
            return Ok(());
        }
        let seen = await_until(5, || {
            let Some(rows) = records(&client).filter(|r| r.len() == 1) else {
                return false;
            };
            let row = &rows[0];
            if row["data"]["fixture_version"].as_u64() != Some(version)
                || row["version"].as_u64() != Some(version)
            {
                return false;
            }
            let current = row["id"].as_str().map(str::to_owned);
            if id.is_some() && id != current {
                return false;
            }
            id = current;
            true
        });
        evidence.check(
            &format!("selected_gateway_readback_{version}"),
            seen,
            "record visible at the expected version",
        );
        if !seen {
            return Ok(());
        }
    }
    let sent = put(world, "absent", 3);
    evidence.check("selected_gateway_absent_capture", sent.is_ok(), "");
    if sent.is_err() {
        return Ok(());
    }
    evidence.check(
        "selected_gateway_absent_readback",
        await_until(5, || records(&client).is_some_and(|r| r.is_empty())),
        "record removed",
    );
    Ok(())
}

pub fn files_transform(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let Some(binary) = native_binary(
        evidence,
        "FilesChecks",
        &[
            "native/Package.swift",
            "native/Sources/FilesSource",
            "native/Sources/FilesChecks",
            "native/Sources/SourceSupport",
        ],
        false,
    ) else {
        return Ok(());
    };
    let (ok, _) = run_native(&binary, &["FILES-TRANSFORM"]);
    evidence.check("native_files_checks", ok, "native Files checks");
    if evidence.passed {
        gateway_flow(world, evidence)?;
    }
    Ok(())
}

pub fn shortcuts_actions(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let scope = "calendar:read,reminders:read,shortcuts:read,system:read";
    let client = Client::new(world, scope)?;
    let denied = Client::new(world, "calendar:read,system:read")?;
    let gated = denied
        .call_raw("shortcuts.actions", &json!({"args":{}}), 10)?
        .is_some_and(|(status, _)| status == 403);
    let readable = client
        .call("shortcuts.actions", &json!({"args":{}}), 10)
        .is_ok();
    evidence.check(
        "query_registered",
        gated && readable,
        "shortcuts.actions is a query gated by shortcuts:read",
    );
    evidence.check("public_gateway", true, "gateway ready");
    evidence.check(
        "helper_ready",
        await_until(5, || {
            client
                .call("helper.version", &json!({"args":{}}), 5)
                .ok()
                .flatten()
                .is_some()
        }),
        "helper answered",
    );
    let out = client.call("shortcuts.actions", &json!({"args":{}}), 10)?;
    let items = out
        .as_ref()
        .and_then(|o| o["result_json"]["items"].as_array());
    evidence.check(
        "public_actions",
        out.as_ref()
            .is_some_and(|o| o["result_json"]["complete"] == true)
            && items.is_some_and(|i| i.len() == 1 && i[0]["identifier"] == "Export"),
        "one complete Export action",
    );
    let rejected = client
        .call(
            "shortcuts.actions",
            &json!({"args":{"path":"/etc/passwd"}}),
            10,
        )?
        .is_none();
    evidence.check("caller_paths_rejected", rejected, "path argument rejected");
    let Some(binary) = native_binary(
        evidence,
        "ActionsChecks",
        &[
            "native/Package.swift",
            "native/Sources/ActionsChecks",
            "native/Sources/ShortcutsSource",
            "native/Sources/SourceSupport",
        ],
        true,
    ) else {
        return Ok(());
    };
    let (ok, _) = run_native(&binary, &["sim"]);
    evidence.check(
        "bounded_metadata_discovery",
        ok,
        "native metadata discovery",
    );
    Ok(())
}

pub fn shortcuts_real_only(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    evidence.unverified(
        "real_level_only",
        "this scenario only runs at the real level, which the Rust lab never executes",
    );
    Ok(())
}

pub(crate) fn checks_binary() -> Result<PathBuf> {
    let gateway = crate::replicas::binary()?;
    let path = env::var("AB_LAB_SHORTCUTS_CHECKS").map_or_else(
        |_| {
            gateway
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("ab-shortcuts-checks")
        },
        PathBuf::from,
    );
    if path.is_file() {
        Ok(path)
    } else {
        Err(lab_world::Error::Invalid(format!(
            "ab-shortcuts-checks not found at {}",
            path.display()
        )))
    }
}

pub(crate) fn native_target(
    evidence: &mut Evidence,
    target: &str,
    inputs: &[&str],
    names: (&str, &str),
) -> Option<PathBuf> {
    let existing = native_directory().map(|d| d.join(target));
    if let Some(binary) = existing.filter(|b| !inputs.is_empty() && fresh(b, inputs)) {
        evidence.check(names.1, true, "native output directory");
        return Some(binary);
    }
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", target)
        .output();
    let ok = built.as_ref().is_ok_and(|o| o.status.success());
    evidence.check(names.0, ok, "native checks build");
    if !ok {
        return None;
    }
    let directory = native_directory();
    evidence.check(names.1, directory.is_some(), "native output directory");
    directory.map(|d| d.join(target))
}

pub(crate) const SHORTCUTS_NATIVE_INPUTS: [&str; 4] = [
    "native/Package.swift",
    "native/Sources/ShortcutsChecks",
    "native/Sources/ShortcutsSource",
    "native/Sources/SourceSupport",
];

pub(crate) fn scratch_directory(id: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    env::temp_dir().join(format!("ab-lab-{id}-{}-{nanos}", std::process::id()))
}

pub(crate) fn run_checks(checks: &Path, args: &[&str], evidence: &mut Evidence) -> Option<Value> {
    let output = Command::new(checks).args(args).output();
    let parsed = output
        .as_ref()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
    if parsed.is_none() {
        evidence.check("checks_binary", false, "ab-shortcuts-checks failed");
    }
    parsed
}

pub fn shortcuts_checks(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let checks = checks_binary()?;
    let directory = scratch_directory(&id);
    let directory_text = directory.to_string_lossy().into_owned();
    let root = env::current_dir()?;
    let root_text = root.to_string_lossy().into_owned();
    let fixture_scenario = matches!(id.as_str(), "SC-CAPTURE" | "SC-RUN-REAL");
    if fixture_scenario {
        let prepared = Command::new(&checks)
            .args(["prepare", &directory_text])
            .output()
            .is_ok_and(|o| o.status.success());
        evidence.check("fixture_directory", prepared, "temporary directory");
        evidence.check("fake_cli", prepared, "fake shortcuts executable");
        if !prepared {
            return Ok(());
        }
    } else {
        fs::create_dir_all(&directory)?;
    }
    if id == "SC-CAPTURE" {
        let binary = native_target(
            evidence,
            "ShortcutsChecks",
            &SHORTCUTS_NATIVE_INPUTS,
            ("native_shortcuts_build", "native_shortcuts_output"),
        );
        let Some(binary) = binary else { return Ok(()) };
        let executable = directory.join("shortcuts").to_string_lossy().into_owned();
        let (ok, _) = run_native(&binary, &[&executable, "SC-CAPTURE"]);
        evidence.check("native_source_fixture", ok, "native source fixture");
    }
    if id == "CLK-TIMER-REAL" {
        let binary = native_target(
            evidence,
            "ShortcutsChecks",
            &[],
            (
                "native_timer_snapshot_build",
                "native_timer_snapshot_output",
            ),
        );
        let Some(binary) = binary else { return Ok(()) };
        let (ok, _) = run_native(&binary, &["synthetic", "CLK-TIMER-REAL"]);
        for name in [
            "native_timer_snapshot_states_and_readonly",
            "native_timer_duration_adjustment_verified",
            "native_timer_localized_component_values",
            "native_timer_recent_snapshot_readonly",
        ] {
            evidence.check(name, ok, "native timer snapshot checks");
        }
    }
    let report = run_checks(
        &checks,
        &["run", &id, &directory_text, &root_text],
        evidence,
    );
    let _ = fs::remove_dir_all(&directory);
    let Some(report) = report else { return Ok(()) };
    for item in report["assertions"].as_array().into_iter().flatten() {
        evidence.check(
            item["name"].as_str().unwrap_or("unnamed"),
            item["passed"] == true,
            "",
        );
    }
    if let Some(reason) = report["limited"].as_str() {
        evidence.check("not_limited", false, reason);
    }
    if id == "CLK-SIM" {
        let client = Client::new(world, "calendar:read,reminders:read,system:read")?;
        for op in [
            "clock.create_alarm",
            "clock.toggle_alarm",
            "clock.delete_alarm",
            "clock.start_timer",
            "clock.world_time",
        ] {
            let advertised = client
                .call_raw(op, &json!({"args":{}}), 10)?
                .is_some_and(|(status, _)| (200..300).contains(&status));
            evidence.check(
                "unqualified_operation_not_advertised",
                !advertised,
                "operation is not registered",
            );
        }
    }
    Ok(())
}
