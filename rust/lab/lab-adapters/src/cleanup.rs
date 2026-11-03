#![allow(clippy::too_many_lines)]
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{env, fs, path::PathBuf, process::Command};

type Patch = fn(&mut Value);

fn binary() -> Option<PathBuf> {
    let directory = env::var("AB_LAB_NATIVE_OUTPUT").ok().or_else(|| {
        fs::read_to_string("native/.build/native-output-1")
            .ok()
            .map(|s| s.trim().to_owned())
    })?;
    let path = PathBuf::from(directory).join("MessagesSafetyChecks");
    path.is_file().then_some(path)
}
fn candidate(outgoing: bool, values: &[&str]) -> Value {
    json!({"outgoing": outgoing, "values": values})
}
fn description(text: &str) -> Value {
    json!({"description": text})
}
const FILE: &str = "ab-run_media.heic";
fn base() -> Value {
    json!({"tag": "[ab-test run_media]", "registration": {"scenario": "WEB-MEDIA-VARIANTS", "run_id": "run_media", "baseline_max_row_id": 30, "media_attachment_names": [FILE]}, "own": ["self@example.com"], "row": {"row_id": 31, "attachments": [FILE], "peers": ["self@example.com"], "is_from_me": true}, "candidates": [{"outgoing": true, "values": [format!("file:///private/test/{FILE}")]}, {"outgoing": false, "values": [FILE]}]})
}
fn cases() -> Vec<(&'static str, Patch, bool, bool)> {
    vec![
        ("attachment_only_target", |_| {}, true, true),
        (
            "attachment_only_mov",
            |v| {
                v["registration"]["media_attachment_names"] = json!(["ab-run_media.mov"]);
                v["row"]["attachments"] = json!(["ab-run_media.mov"]);
                v["candidates"] = json!([description(
                    "Your iMessage, 0-ab-run_media.mov, Media, 3:45 PM"
                )]);
            },
            true,
            true,
        ),
        (
            "attachment_only_caf",
            |v| {
                v["registration"]["media_attachment_names"] = json!(["ab-run_media.caf"]);
                v["row"]["attachments"] = json!(["ab-run_media.caf"]);
                v["candidates"] = json!([description(
                    "Your iMessage, 0-ab-run_media.caf, Media, 3:45 PM"
                )]);
            },
            true,
            true,
        ),
        (
            "empty_database_baseline_is_valid",
            |v| {
                v["registration"]["baseline_max_row_id"] = json!(0);
                v["row"]["row_id"] = json!(1);
            },
            true,
            true,
        ),
        (
            "incoming_echo_is_separate_target",
            |v| v["row"]["is_from_me"] = json!(false),
            true,
            true,
        ),
        (
            "native_file_description_outgoing",
            |v| {
                v["candidates"] = json!([description(&format!(
                    "Your iMessage, 0-{FILE}, Text Document · 66 bytes, 3:45 PM"
                ))]);
            },
            true,
            true,
        ),
        (
            "native_file_description_incoming",
            |v| {
                v["row"]["is_from_me"] = json!(false);
                v["candidates"] = json!([description(&format!(
                    "Self Test, 0-{FILE}, Text Document · 66 bytes, 3:45 PM"
                ))]);
            },
            true,
            true,
        ),
        (
            "native_file_description_substring_denied",
            |v| {
                v["candidates"] = json!([description(&format!(
                    "Your iMessage, prefix-{FILE}, Text Document · 66 bytes, 3:45 PM"
                ))]);
            },
            true,
            false,
        ),
        (
            "native_file_description_ambiguous_denied",
            |v| {
                let one = description(&format!(
                    "Your iMessage, 0-{FILE}, Text Document · 66 bytes, 3:45 PM"
                ));
                v["candidates"] = json!([one.clone(), one]);
            },
            true,
            false,
        ),
        (
            "photo_preview_filename_with_generic_description",
            |v| {
                v["candidates"] =
                    json!([{"description": "Your iMessage, Photo, 3:45 PM", "values": [FILE]}]);
            },
            true,
            true,
        ),
        (
            "duplicate_copies_require_complete_ownership",
            |v| {
                let one = candidate(true, &[FILE]);
                v["candidates"] = json!([one.clone(), one]);
                v["owned_copies"] = json!(2);
            },
            true,
            true,
        ),
        (
            "unaccounted_duplicate_is_denied",
            |v| {
                let one = candidate(true, &[FILE]);
                v["candidates"] = json!([one.clone(), one.clone(), one]);
                v["owned_copies"] = json!(2);
            },
            true,
            false,
        ),
        (
            "baseline_data_denied",
            |v| v["row"]["row_id"] = json!(30),
            false,
            false,
        ),
        (
            "other_chat_denied",
            |v| v["row"]["peers"] = json!(["other@example.com"]),
            false,
            false,
        ),
        (
            "unregistered_attachment_denied",
            |v| v["row"]["attachments"] = json!(["personal.heic"]),
            false,
            false,
        ),
        (
            "mixed_attachment_bubble_denied",
            |v| v["row"]["attachments"] = json!([FILE, "personal.heic"]),
            false,
            false,
        ),
        (
            "similar_filename_denied",
            |v| v["candidates"] = json!([candidate(true, &[&format!("prefix-{FILE}")])]),
            true,
            false,
        ),
        (
            "other_registered_file_is_not_the_target",
            |v| {
                v["registration"]["media_attachment_names"] = json!([FILE, "ab-run_media.mov"]);
                v["candidates"] = json!([candidate(true, &["ab-run_media.mov"])]);
            },
            true,
            false,
        ),
        (
            "ambiguous_ui_target_denied",
            |v| v["candidates"] = json!([candidate(true, &[FILE]), candidate(true, &[FILE])]),
            true,
            false,
        ),
    ]
}
pub fn prove(evidence: &mut Evidence, prefix: &str) -> bool {
    let Some(binary) = binary() else {
        evidence.check(
            &format!("{prefix}native_cleanup_checks_build"),
            false,
            "MessagesSafetyChecks binary unavailable on this host",
        );
        return false;
    };
    let Ok(directory) = tempdir() else {
        evidence.check(
            &format!("{prefix}private_world"),
            false,
            "temporary directory",
        );
        return false;
    };
    let mut passed = true;
    for (name, patch, authorized, target) in cases() {
        let mut input = base();
        patch(&mut input);
        let path = directory.join("fixture.json");
        let result = fs::write(&path, input.to_string())
            .ok()
            .and_then(|()| {
                Command::new(&binary)
                    .args(["media-cleanup", "--fixtures"])
                    .arg(&path)
                    .output()
                    .ok()
            })
            .filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
        let ok = result.is_some_and(|v| {
            v["authorized"].as_bool() == Some(authorized)
                && v["target_verified"].as_bool() == Some(target)
        });
        passed &= ok;
        evidence.check(&format!("{prefix}{name}"), ok, "native media cleanup check");
    }
    let _ = fs::remove_dir_all(&directory);
    passed
}
fn tempdir() -> std::io::Result<PathBuf> {
    let base = env::temp_dir().join(format!("media-cleanup-{}", std::process::id()));
    fs::create_dir_all(&base)?;
    Ok(base)
}
pub fn messages_commands(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if crate::messages_a::commands::ALL.contains(&evidence.scenario.as_str()) {
        return crate::messages_a::commands::commands(world, evidence);
    }
    if evidence.scenario != "MSG-MEDIA-CLEANUP" {
        return Err(lab_world::Error::Invalid(format!(
            "messages_commands not ported for {}",
            evidence.scenario
        )));
    }
    prove(evidence, "");
    Ok(())
}
