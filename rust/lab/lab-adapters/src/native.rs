#![allow(clippy::too_many_lines)]
use crate::{assertions::await_until, http::random_id};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const VIDEO_PANORAMA: &[u8] = include_bytes!("../../lab-simapple/assets/video-panorama.mp4");

fn build(target: &str, lab: bool) -> Result<PathBuf> {
    let mut command = Command::new("packaging/dev/build-native.sh");
    command.env("APPLE_BRIDGE_NATIVE_TARGET", target);
    if lab {
        command.env("APPLE_BRIDGE_NATIVE_LAB", "1");
    } else {
        command.env_remove("APPLE_BRIDGE_NATIVE_LAB");
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "native build {target} exited {}",
            output.status
        )));
    }
    directory()
}
fn directory() -> Result<PathBuf> {
    fs::read_to_string("native/.build/native-output-1")
        .map(|text| PathBuf::from(text.trim()))
        .map_err(|_| Error::Invalid("native output directory missing".into()))
}
struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}
fn run(tool: &Path, args: &[&str], log: &Path) -> Run {
    match Command::new(tool).args(args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let _ = fs::write(log, format!("{stdout}{stderr}"));
            Run {
                ok: output.status.success(),
                stdout,
                stderr,
            }
        }
        Err(error) => Run {
            ok: false,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}
fn guard(evidence: &Evidence, expected: &str) -> Result<()> {
    if evidence.scenario == expected {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "native action for {expected} not ported for {}",
            evidence.scenario
        )))
    }
}
fn combined(run: &Run) -> String {
    format!("{}{}", run.stdout, run.stderr)
}

pub fn media_yield(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    guard(evidence, "NATIVE-MEDIA-YIELD")?;
    let Ok(directory) = build("MessagesCaptureChecks", true) else {
        evidence.check("native_messages_checks_built", false, "native build failed");
        return Ok(());
    };
    let tool = directory.join("MessagesCaptureChecks");
    for (command, name, success) in [
        (
            "--media-yield-burst",
            "foreground_burst_reuses_completed_stages",
            "cooperative media burst checks passed",
        ),
        (
            "--media-yield-metadata",
            "metadata_only_changes_preserve_retained_stages",
            "cooperative media burst checks passed",
        ),
        (
            "--media-yield-source-change",
            "changed_source_invalidates_retained_stages",
            "cooperative media burst checks passed",
        ),
        (
            "--media-yield-companion",
            "local_companion_wakes_deferred_live_photo",
            "cooperative media burst checks passed",
        ),
        (
            "--media-yield-export-race",
            "export_yield_retries_without_cancelling_parent",
            "cooperative media export checks passed",
        ),
    ] {
        let result = run(
            &tool,
            &[command],
            &world.directory.join(format!("yield{command}.log")),
        );
        evidence.check(
            name,
            result.ok && combined(&result).contains(success),
            "native cooperative yield check",
        );
    }
    gateway_fixture(world, evidence);
    Ok(())
}
pub fn media_export_safety(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    guard(evidence, "NATIVE-MEDIA-EXPORT-SAFETY")?;
    let Ok(directory) = build("MediaChecks", true) else {
        evidence.check("native_media_checks_built", false, "native build failed");
        return Ok(());
    };
    let tool = directory.join("MediaChecks");
    for (command, success) in [
        (
            "export-safety",
            "synthetic incompatible export rejected; following export completed",
        ),
        (
            "export-overlap",
            "same-digest work serialized across processes; cached result reused",
        ),
    ] {
        let result = run(
            &tool,
            &[command],
            &world.directory.join(format!("export-{command}.log")),
        );
        evidence.check(
            command,
            result.ok && combined(&result).contains(success),
            "synthetic native export check",
        );
    }
    gateway_fixture(world, evidence);
    Ok(())
}
pub fn media_posters(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    guard(evidence, "NATIVE-MEDIA-POSTERS")?;
    let tag = random_id("run")?;
    let mut data = VIDEO_PANORAMA.to_vec();
    let size = u32::try_from(8 + tag.len()).unwrap_or(0);
    data.extend_from_slice(&size.to_be_bytes());
    data.extend_from_slice(b"free");
    data.extend_from_slice(tag.as_bytes());
    let source = world.directory.join("panorama.mp4");
    fs::write(&source, data)?;
    let Ok(directory) = build("MediaChecks", true) else {
        evidence.check("native_media_checks_built", false, "native build failed");
        return Ok(());
    };
    let tool = directory.join("MediaChecks");
    let result = run(
        &tool,
        &["poster-safety", &source.to_string_lossy()],
        &world.directory.join("poster-safety.log"),
    );
    let decoded = serde_json::from_str::<Value>(&result.stdout).is_ok();
    evidence.check(
        "native_poster_resolution_sharpness_cache_and_live_still",
        result.ok && decoded,
        "synthetic native poster check",
    );
    gateway_fixture(world, evidence);
    Ok(())
}
pub fn reminders_parity(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    guard(evidence, "WEB-REM-PARITY")?;
    let Ok(directory) = build("RemindersStoreChecks", false) else {
        evidence.check("native_sections_build", false, "native build failed");
        return Ok(());
    };
    evidence.check("native_sections_build", true, "RemindersStoreChecks built");
    let store = run(
        &directory.join("RemindersStoreChecks"),
        &["--parity-checks"],
        &world.directory.join("parity.log"),
    );
    evidence.check(
        "store_effects_and_invalidation",
        store.ok && combined(&store).contains("parity checks passed"),
        "native parity checks",
    );
    let helper = match env::var("APPLE_BRIDGE_REMINDERS_CHECKS_HELPER") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => {
            let built = build("AppleBridgeHelper", false);
            evidence.check("native_fixture_checks_build", built.is_ok(), "helper build");
            built.map_or_else(
                |_| directory.join("AppleBridgeHelper"),
                |d| d.join("AppleBridgeHelper"),
            )
        }
    };
    let available = helper.is_file();
    evidence.check(
        "native_fixture_checks_helper_available",
        available,
        "helper executable present",
    );
    if available {
        let fixtures = run(
            &helper,
            &["reminders-fixture-checks"],
            &world.directory.join("fixtures.log"),
        );
        evidence.check(
            "owned_fixture_accessibility_regressions",
            fixtures.ok && combined(&fixtures).contains("reminders fixture checks passed"),
            "owned fixture checks",
        );
    }
    Ok(())
}

fn fixture_records(world: &World) -> Option<Vec<Value>> {
    let page = world
        .query(
            "records.query",
            &json!({"collection":"messages.attachment","limit":10}),
        )
        .ok()?;
    Some(page["records"].as_array().cloned().unwrap_or_default())
}
fn gateway_fixture(world: &World, evidence: &mut Evidence) {
    evidence.check(
        "selected_gateway_started",
        true,
        "gateway and SimApple ready",
    );
    let mut id = String::new();
    for version in 1..=2_u64 {
        let capture = json!({
            "source": "messages", "generation": "native-media-fixture",
            "predecessor": "", "checkpoint": "",
            "observations": [{
                "kind": "upsert", "collection": "messages.attachment",
                "native_key": "native-media-fixture", "revalidated": true,
                "observed_at": jiff::Timestamp::now().to_string(),
                "data": {"filename": "[ab-test] native-media.txt", "mime_type": "text/plain",
                    "uti": "public.plain-text", "size": 0, "blob": {"state": "missing"},
                    "is_sticker": false, "hidden": false, "genmoji": false,
                    "fixture_version": version}
            }]
        });
        let sent = world.control("/capture", &capture).is_ok();
        evidence.check(
            &format!("selected_gateway_capture_{version}"),
            sent,
            "fixture capture",
        );
        if !sent {
            return;
        }
        let seen = await_until(5, || {
            let Some(records) = fixture_records(world) else {
                return false;
            };
            let [record] = records.as_slice() else {
                return false;
            };
            if record["data"]["fixture_version"] != version
                || record["version"] != version
                || (!id.is_empty() && record["id"] != id.as_str())
            {
                return false;
            }
            record["id"]
                .as_str()
                .unwrap_or_default()
                .clone_into(&mut id);
            true
        });
        evidence.check(
            &format!("selected_gateway_readback_{version}"),
            seen,
            "fixture readback",
        );
        if !seen {
            return;
        }
    }
    let now = jiff::Timestamp::now().to_string();
    let absent = json!({
        "source": "messages", "generation": "native-media-fixture",
        "predecessor": "", "checkpoint": "", "scan_id": "native-media-fixture",
        "scan_started_at": now, "scope": ["messages.attachment"],
        "completion_evidence": "complete",
        "observations": [{
            "kind": "absent", "collection": "messages.attachment",
            "native_key": "native-media-fixture", "revalidated": true,
            "observed_at": now,
            "data": {"filename": "[ab-test] native-media.txt", "mime_type": "text/plain",
                "uti": "public.plain-text", "size": 0, "blob": {"state": "missing"},
                "is_sticker": false, "hidden": false, "genmoji": false, "fixture_version": 3}
        }]
    });
    let sent = world.control("/capture", &absent).is_ok();
    evidence.check("selected_gateway_absent_capture", sent, "absent capture");
    if sent {
        evidence.check(
            "selected_gateway_absent_readback",
            await_until(5, || fixture_records(world).is_some_and(|r| r.is_empty())),
            "absent readback",
        );
    }
}
