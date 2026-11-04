use crate::{
    assertions::{await_until, record_multiset},
    http::random_id,
};
use lab_runner::{Evidence, Registry};
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 6] = [
    "HELPER-LIVE-BACKFILL",
    "HELPER-RELEASE-COMMANDS",
    "HELPER-RESTART-REAL",
    "HELPER-SESSION-SOAK",
    "HELPER-SIGNED-APP-CACHE",
    "HELPER-TCC-READY",
];

pub fn register(registry: &mut Registry) {
    registry.register_real("HELPER-LIVE-BACKFILL", live_backfill);
    registry.register_real("HELPER-RELEASE-COMMANDS", release_commands);
    registry.register_real("HELPER-RESTART-REAL", restart_real);
    registry.register_real("HELPER-SESSION-SOAK", session_soak);
    registry.register_real("HELPER-SIGNED-APP-CACHE", signed_app_cache);
    registry.register_real("HELPER-TCC-READY", tcc_ready);
}

fn text(command: &mut Command) -> (bool, String) {
    match command.stdin(Stdio::null()).output() {
        Ok(output) => (
            output.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .trim()
            .to_owned(),
        ),
        Err(error) => (false, error.to_string()),
    }
}
fn invoke(binary: &Path, args: &[&str]) -> (bool, String) {
    let mut child = match Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return (false, error.to_string()),
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().is_ok_and(|status| status.is_none()) {
        if Instant::now() >= deadline {
            let _ = child.kill();
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    match child.wait_with_output() {
        Ok(output) => (
            output.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .trim()
            .to_owned(),
        ),
        Err(error) => (false, error.to_string()),
    }
}
fn scratch(prefix: &str) -> Result<PathBuf> {
    let path = env::temp_dir().join(format!("{prefix}{}", random_id("d")?));
    fs::create_dir(&path)?;
    Ok(path)
}
fn pointer() -> Option<String> {
    fs::read_to_string("native/.build/native-output-1")
        .ok()
        .map(|s| s.trim().to_owned())
}

fn tcc_ready(_: &mut World, evidence: &mut Evidence) -> Result<()> {
    let path = scratch("ab-tcc-evidence-")?.join("report.json");
    let (ran, output) = text(Command::new("python3").args([
        "rust/lab/identity-spikes/run.py",
        &path.to_string_lossy(),
        "--tcc-only",
    ]));
    if !ran {
        evidence.check("rust_probe_run", false, &output);
        return Ok(());
    }
    let results: Option<BTreeMap<String, Value>> = fs::read(&path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok());
    if let Some(parent) = path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
    let Some(results) = results else {
        evidence.check("rust_probe_read", false, "probe report unreadable");
        return Ok(());
    };
    for mode in ["open", "resigned", "control"] {
        for source in ["chat", "wal", "attachment", "notes", "biome", "safari"] {
            let read = results[mode][source].as_object().map(|o| o.get("pass"));
            let valid = read.is_some();
            let pass = read.flatten().and_then(Value::as_bool).unwrap_or(false);
            let passed = if mode == "control" {
                valid && !pass
            } else {
                valid && pass
            };
            evidence.check(&format!("{mode}_{source}"), passed, "TCC read probe");
        }
    }
    evidence.check(
        "launchd_control_removed",
        results.get("control_cleaned") == Some(&json!(true)),
        "launchd control removed",
    );
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn release_commands(_: &mut World, evidence: &mut Evidence) -> Result<()> {
    let directory = scratch("ab-helper-release-")?;
    release_checks(&directory, evidence);
    let _ = fs::remove_dir_all(&directory);
    Ok(())
}
#[allow(clippy::too_many_lines)]
fn release_checks(directory: &Path, evidence: &mut Evidence) {
    let mut apps: BTreeMap<&str, PathBuf> = BTreeMap::new();
    let mut keys: BTreeMap<&str, String> = BTreeMap::new();
    for mode in ["0", "1"] {
        let app = directory.join(mode).join("AppleBridgeHelper.app");
        let built = Command::new("packaging/dev/build-helper.sh")
            .arg(&app)
            .env("APPLE_BRIDGE_NATIVE_LAB", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        evidence.check(
            &format!("signed_helper_build_{mode}"),
            built,
            "helper build",
        );
        if !built {
            return;
        }
        let verified = Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(&app)
            .status()
            .is_ok_and(|status| status.success());
        evidence.check(&format!("signature_valid_{mode}"), verified, "signature");
        let key = app
            .parent()
            .map(|p| p.join("AppleBridgeHelper.cache.json"))
            .and_then(|p| fs::read(p).ok())
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .and_then(|v| v["key"].as_str().map(str::to_owned))
            .unwrap_or_default();
        evidence.check(
            &format!("cache_attestation_{mode}"),
            !key.is_empty(),
            "cache attestation",
        );
        keys.insert(mode, key);
        apps.insert(mode, app.join("Contents/MacOS/AppleBridgeHelper"));
    }
    evidence.check(
        "lab_and_release_cache_keys_differ",
        !keys["0"].is_empty() && keys["0"] != keys["1"],
        "lab and release cache keys differ",
    );
    for command in [
        "reminders-store-blob-fixture",
        "reminders-store-proof",
        "reminders-store-flag-proof",
        "reminders-store-profile",
        "notes-probe",
        "functional",
        "probe",
    ] {
        let (ok, output) = invoke(&apps["0"], &[command]);
        evidence.check(
            &format!("release_rejects_{command}"),
            !ok && output == "unknownCommand",
            "release rejects lab command",
        );
        let (ok, output) = invoke(&apps["1"], &[command]);
        evidence.check(
            &format!("lab_recognizes_{command}"),
            !ok && output == "invalidArguments",
            "lab recognizes command",
        );
    }
    for mode in ["0", "1"] {
        for command in ["session", "status", "onboard", "selftest"] {
            let (ok, output) = invoke(&apps[mode], &[command]);
            evidence.check(
                &format!("product_command_{mode}_{command}"),
                !ok && output == "invalidArguments",
                "product command recognized",
            );
        }
    }
    let wire = ["functional"];
    let before = pointer();
    let packaged = directory.join("Apple Bridge.app");
    let built = Command::new("packaging/build-app.sh")
        .arg(&packaged)
        .env("APPLE_BRIDGE_NATIVE_LAB", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    evidence.check("release_package_build", built, "release package build");
    if !built {
        return;
    }
    let signed = Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&packaged)
        .status()
        .is_ok_and(|status| status.success());
    evidence.check("release_package_signature", signed, "package signature");
    let (ok, output) = invoke(&packaged.join("Contents/MacOS/AppleBridgeHelper"), &wire);
    evidence.check(
        "release_package_overrides_lab_caller",
        !ok && output == "unknownCommand",
        "release package overrides the lab caller",
    );
    let after = pointer();
    let kept = after.as_ref().is_some_and(|dir| {
        let (ok, output) = invoke(&Path::new(dir).join("AppleBridgeHelper"), &wire);
        !ok && output == "invalidArguments"
    });
    evidence.check(
        "lab_reader_keeps_lab_binary_after_release_build",
        before.is_some() && before == after && kept,
        "lab reader keeps the lab binary",
    );
    for mode in ["0", "1"] {
        let symbols = Command::new("nm")
            .arg(&apps[mode])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|nm| {
                let mut demangle = Command::new("xcrun")
                    .arg("swift-demangle")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                    .ok()?;
                std::io::Write::write_all(&mut demangle.stdin.take()?, &nm.stdout).ok()?;
                let out = demangle.wait_with_output().ok()?;
                Some(String::from_utf8_lossy(&out.stdout).into_owned())
            });
        for name in [
            "FunctionalProbe",
            "RemindersStoreProof",
            "RemindersStoreProfile",
            "RemindersStoreResponsiveness",
        ] {
            evidence.check(
                &format!("lab_type_{mode}_{name}"),
                symbols
                    .as_ref()
                    .is_some_and(|s| s.contains(name) == (mode == "1")),
                "lab types present only in the lab build",
            );
        }
    }
}

fn helper_pid(world: &World) -> Option<String> {
    fs::read_to_string(world.gateway_home.join("helper/helper.pid")).ok()
}
fn all_records(world: &World) -> Option<Vec<Value>> {
    world
        .query("records.query", &json!({"collection": "", "limit": 1000}))
        .ok()?["records"]
        .as_array()
        .cloned()
}
fn reminders_ready(world: &World) -> bool {
    all_records(world).is_some_and(|records| {
        records.iter().any(|record| {
            record["collection"] == "system.source"
                && record["data"]["source"] == "reminders"
                && record["data"]["state"] == "ready"
        })
    })
}

pub(crate) fn truth(world: &World, verb: &str, request: &Value) -> Result<Value> {
    let app = env::var("AB_LAB_TRUTH_APP")
        .map_err(|_| Error::Invalid("AB_LAB_TRUTH_APP is not set".into()))?;
    let id = random_id("truth")?;
    let input = world.directory.join(format!("{id}-request.json"));
    let output = world.directory.join(format!("{id}-report.json"));
    fs::write(&input, serde_json::to_vec(request)?)?;
    let _ = Command::new("open")
        .args([
            "-n",
            "-W",
            "-g",
            "-a",
            &app,
            "--args",
            verb,
            "--request-file",
        ])
        .arg(&input)
        .arg("--report")
        .arg(&output)
        .status();
    let _ = fs::remove_file(&input);
    let response: Value = serde_json::from_slice(&fs::read(&output)?)?;
    let _ = fs::remove_file(&output);
    if response["status"] == "verified" {
        Ok(response)
    } else {
        Err(Error::Invalid(format!(
            "truth unverified: {}",
            response["reason"].as_str().unwrap_or("reason_missing")
        )))
    }
}

#[allow(clippy::unnecessary_wraps)]
fn restart_real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let ready = await_until(15, || reminders_ready(world));
    evidence.check("reminders_ready", ready, "reminders source ready");
    if !ready {
        return Ok(());
    }
    let checkpoint = world.gateway_home.join("helper/lab-committed-reminders");
    let committed = await_until(5, || checkpoint.exists());
    evidence.check("native_capture_committed", committed, "capture committed");
    if !committed {
        return Ok(());
    }
    let Some(before) = all_records(world) else {
        evidence.check("before_restart", false, "records before restart");
        return Ok(());
    };
    let marker = fs::read(&checkpoint).unwrap_or_default();
    let pid = helper_pid(world).unwrap_or_default();
    let killed = Command::new("kill")
        .args(["-KILL", pid.trim()])
        .status()
        .is_ok_and(|status| status.success())
        && await_until(10, || {
            helper_pid(world).is_some_and(|current| current != pid)
        })
        && await_until(5, || {
            fs::read(&checkpoint).is_ok_and(|current| current != marker) && reminders_ready(world)
        });
    evidence.check("helper_restarted", killed, "helper restarted");
    if !killed {
        return Ok(());
    }
    let after = all_records(world);
    evidence.check(
        "restart_preserves_records",
        after
            .as_ref()
            .is_some_and(|after| record_multiset(&before, false) == record_multiset(after, false)),
        "records unchanged across the helper restart",
    );
    Ok(())
}

fn session_soak(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let tag = format!("[ab-test {}]", random_id("soak")?);
    let outcome = soak(world, evidence, &tag);
    let cleaned = truth(
        world,
        "fixture",
        &json!({"action": "cleanup", "list": "Bridge Test", "tag": tag}),
    )
    .is_ok();
    if !cleaned {
        evidence.check("soak_cleanup", false, "tagged fixtures removed");
    }
    outcome
}
fn trace_rows(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        })
        .unwrap_or_default()
}
fn soak(world: &World, evidence: &mut Evidence, tag: &str) -> Result<()> {
    let Some(before) = helper_pid(world) else {
        return Ok(());
    };
    let reports =
        PathBuf::from(env::var("HOME").unwrap_or_default()).join("Library/Logs/DiagnosticReports");
    let existing: Vec<String> = fs::read_dir(&reports)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    let started = Instant::now();
    let mut next = started;
    let mut stable = true;
    while started.elapsed() < Duration::from_secs(600) {
        if helper_pid(world).as_deref() != Some(before.as_str()) {
            stable = false;
            break;
        }
        if Instant::now() >= next {
            let created = truth(
                world,
                "fixture",
                &json!({"action": "create", "list": "Bridge Test", "tag": tag,
                    "title": format!("{tag}{}", random_id("deadline")?)}),
            );
            if created.is_err() {
                stable = false;
                break;
            }
            next = Instant::now() + Duration::from_secs(120);
        }
        thread::sleep(Duration::from_millis(500));
    }
    let no_crash = fs::read_dir(&reports).is_ok_and(|entries| {
        entries.filter_map(std::result::Result::ok).all(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            existing.contains(&name)
                || !name.starts_with("AppleBridgeHelper")
                || fs::read_to_string(entry.path()).is_ok_and(|report| {
                    !report.contains(world.directory.to_string_lossy().as_ref())
                })
        })
    });
    let ticks = trace_rows(&world.gateway_home.join("helper/session-timers.jsonl")).len();
    let deadlines = trace_rows(&world.gateway_home.join("helper/capture-trace.jsonl"))
        .iter()
        .filter(|row| row["stage"] == "fetch_timer_fired")
        .count();
    evidence.check(
        "same_helper_pid_ten_minutes",
        stable && started.elapsed() >= Duration::from_secs(600),
        "one helper process for ten minutes",
    );
    evidence.check(
        "no_new_world_helper_crash_reports",
        no_crash,
        "no new helper crash reports",
    );
    evidence.check(
        "six_hundred_lifetime_timer_deadlines",
        ticks >= 600,
        "lifetime timer deadlines",
    );
    evidence.check(
        "multiple_fetch_timer_deadlines",
        deadlines >= 4,
        "fetch timer deadlines",
    );
    Ok(())
}

fn live_backfill(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let tag = format!("[ab-test {}]", random_id("bulk")?);
    let original = format!("{tag} original");
    let updated = format!("{tag} updated");
    let held = world.gateway_home.join("helper/lab-backfill-held");
    backfill(world, evidence, &tag, &original, &updated, &held);
    let cleaned = truth(
        world,
        "fixture",
        &json!({"action": "cleanup", "list": "Bridge Test", "tag": tag}),
    )
    .is_ok();
    if !cleaned {
        evidence.check("tagged_fixture_cleanup", false, "tagged fixtures removed");
    }
    Ok(())
}
fn titles(world: &World) -> Option<Vec<String>> {
    Some(
        all_records(world)?
            .iter()
            .filter_map(|record| record["data"]["title"].as_str().map(str::to_owned))
            .collect(),
    )
}
fn backfill(
    world: &World,
    evidence: &mut Evidence,
    tag: &str,
    original: &str,
    updated: &str,
    held: &Path,
) {
    let is_held = await_until(5, || held.exists());
    evidence.check("initial_backfill_held", is_held, "initial backfill held");
    if !is_held {
        return;
    }
    let edited = truth(
        world,
        "fixture",
        &json!({"action": "modify", "list": "Bridge Test", "tag": tag,
            "title": original, "new_title": updated}),
    );
    evidence.check(
        "external_edit_during_backfill",
        edited.is_ok(),
        "external edit during the held backfill",
    );
    let visible = || {
        titles(world).is_some_and(|all| {
            !all.iter().any(|t| t == original) && all.iter().filter(|t| *t == updated).count() == 1
        })
    };
    evidence.check(
        "live_visible_before_initial_backfill_commit",
        await_until(5, visible),
        "live edit visible before the backfill commits",
    );
    let trace = trace_rows(&world.gateway_home.join("helper/capture-trace.jsonl"));
    let no_bulk = !trace
        .iter()
        .any(|row| row["stage"] == "capture_sent" && row["live"] != true);
    let checkpoint = world.gateway_home.join("helper/lab-committed-reminders");
    let prior = fs::read(&checkpoint).unwrap_or_default();
    let _ = fs::remove_file(world.gateway_home.join("helper/lab-hold-backfill"));
    evidence.check(
        "backfill_committed_after_live",
        await_until(10, || fs::read(&checkpoint).is_ok_and(|c| c != prior)),
        "backfill committed after the live edit",
    );
    evidence.check(
        "no_bulk_capture_before_live",
        no_bulk,
        "no bulk capture first",
    );
    evidence.check(
        "backfill_does_not_overwrite_live",
        visible(),
        "backfill keeps the live edit",
    );
    let observed = truth(
        world,
        "observe",
        &json!({"list": "Bridge Test", "tag": tag, "title": updated}),
    );
    evidence.check(
        "independent_updated_truth",
        observed
            .as_ref()
            .is_ok_and(|v| v["records"].as_array().is_some_and(|r| r.len() == 1)),
        "independent truth shows the updated record",
    );
}

fn signed_app_cache(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let available = Path::new("packaging/dev/signed-app-cache.py").is_file();
    evidence.check(
        "signed_app_cache_available",
        available,
        "cache tool present",
    );
    if !available {
        return Ok(());
    }
    let cache = scratch("ab-signed-app-cache-")?;
    let outcome = cache_checks(world, evidence, &cache);
    let _ = fs::remove_dir_all(&cache);
    outcome
}
fn cache_checks(world: &World, evidence: &mut Evidence, cache: &Path) -> Result<()> {
    let apps = world.directory.join("apps");
    let mut keys: BTreeMap<&str, String> = BTreeMap::new();
    for role in ["AppleBridgeHelper", "AppleTruth"] {
        let report: Option<Value> = fs::read(apps.join(format!("{role}.cache.json")))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok());
        let key = report
            .as_ref()
            .and_then(|r| r["key"].as_str())
            .unwrap_or_default();
        let bundle = report
            .as_ref()
            .and_then(|r| r["bundle_sha256"].as_str())
            .unwrap_or_default();
        evidence.check(
            &format!("cold_{role}_expected_cache_result"),
            key.len() == 64 && bundle.len() == 64,
            "cache report present",
        );
        keys.insert(role, format!("{key}/{bundle}"));
        let signed = Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(apps.join(format!("{role}.app")))
            .status()
            .is_ok_and(|status| status.success());
        evidence.check(
            &format!("cold_{role}_signature_valid"),
            signed,
            "signature valid",
        );
    }
    for n in 0..3u8 {
        let entry = cache.join(char::from(b'a' + n).to_string().repeat(64));
        fs::create_dir(&entry)?;
    }
    let trimmed = Command::new("python3")
        .args(["packaging/dev/signed-app-cache.py", "--trim"])
        .env("APPLE_BRIDGE_SIGNED_APP_CACHE", cache)
        .status()
        .is_ok_and(|status| status.success());
    let count = fs::read_dir(cache)?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count();
    evidence.check(
        "cache_retains_last_three_entries",
        trimmed && count == 3,
        "cache trimmed to three entries",
    );
    Ok(())
}
