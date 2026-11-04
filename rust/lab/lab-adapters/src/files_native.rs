use crate::{assertions::await_until, http::Client};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

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

pub(crate) fn native_output() -> Option<PathBuf> {
    env::var("AB_LAB_NATIVE_OUTPUT")
        .ok()
        .or_else(|| {
            fs::read_to_string("native/.build/native-output-1")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .map(PathBuf::from)
}

pub(crate) fn checks(evidence: &mut Evidence, args: &[&str]) -> bool {
    let binary = native_output().map(|d| d.join("FilesChecks"));
    let fresh = binary.as_ref().is_some_and(|binary| {
        fs::metadata(binary)
            .and_then(|m| m.modified())
            .is_ok_and(|built| {
                [
                    "native/Package.swift",
                    "native/Sources/FilesSource",
                    "native/Sources/FilesChecks",
                    "native/Sources/SourceSupport",
                ]
                .iter()
                .all(|root| newest(Path::new(root)).is_some_and(|t| t <= built))
            })
    });
    let mut binary = binary;
    if !fresh {
        let built = Command::new("packaging/dev/build-native.sh")
            .env("APPLE_BRIDGE_NATIVE_TARGET", "FilesChecks")
            .output();
        let ok = built.as_ref().is_ok_and(|o| o.status.success());
        evidence.check("native_build", ok, "");
        if !ok {
            return false;
        }
        binary = native_output().map(|d| d.join("FilesChecks"));
        evidence.check("native_output", binary.is_some(), "");
    }
    let Some(binary) = binary else {
        return false;
    };
    let output = Command::new(binary).args(args).output();
    let ok = output.as_ref().is_ok_and(|o| o.status.success());
    evidence.check("native_files_checks", ok, "");
    if let Ok(o) = &output
        && !ok
    {
        eprint!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
    }
    ok
}

fn capture(world: &World, kind: &str, version: u64) -> Result<()> {
    let now = jiff::Timestamp::now().to_string();
    let mut frame = json!({
        "source": "files",
        "generation": "native-media-fixture",
        "observations": [{
            "kind": kind,
            "collection": "files.item",
            "native_key": "native-media-fixture",
            "data": {
                "root": "/Bridge Test", "path": "fixture.txt", "name": "fixture.txt",
                "kind": "file", "size": 0, "created": null, "modified": null,
                "tags": [], "downloaded": true, "icloud_status": "local",
                "fixture_version": version
            },
            "observed_at": now,
            "revalidated": true
        }]
    });
    if kind == "absent" {
        frame["scan_id"] = json!("native-media-fixture");
        frame["scan_started_at"] = json!(now);
        frame["scope"] = json!(["files.item"]);
        frame["completion_evidence"] = json!("complete");
    }
    world.control("/capture", &frame)?;
    Ok(())
}

fn gateway(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, "admin")?;
    evidence.check("selected_gateway_started", true, "");
    let mut id = String::new();
    for version in 1..=2u64 {
        let sent = capture(world, "upsert", version);
        evidence.check(
            &format!("selected_gateway_capture_{version}"),
            sent.is_ok(),
            "",
        );
        if sent.is_err() {
            return Ok(());
        }
        let ok = await_until(5, || {
            let Ok(Some(result)) = client.call(
                "records.query",
                &json!({"collection":"files.item","limit":10}),
                5,
            ) else {
                return false;
            };
            let rows = result["records"].as_array().cloned().unwrap_or_default();
            let [row] = rows.as_slice() else {
                return false;
            };
            if row["data"]["fixture_version"].as_u64() != Some(version)
                || row["version"].as_u64() != Some(version)
                || (!id.is_empty() && row["id"].as_str() != Some(id.as_str()))
            {
                return false;
            }
            row["id"].as_str().unwrap_or_default().clone_into(&mut id);
            true
        });
        evidence.check(&format!("selected_gateway_readback_{version}"), ok, "");
        if !ok {
            return Ok(());
        }
    }
    let sent = capture(world, "absent", 3);
    evidence.check("selected_gateway_absent_capture", sent.is_ok(), "");
    if sent.is_err() {
        return Ok(());
    }
    let gone = await_until(5, || {
        matches!(
            client.call("records.query", &json!({"collection":"files.item","limit":10}), 5),
            Ok(Some(r)) if r["records"].as_array().is_some_and(Vec::is_empty)
        )
    });
    evidence.check("selected_gateway_absent_readback", gone, "");
    Ok(())
}

pub fn files_native(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "FILE-IDLE" => {
            crate::native_core::rust_tests(
                evidence,
                "incomplete_walk_keeps_rows_complete_walk_removes",
                "ab-native",
                &["--test", "files_walk"],
            );
            Ok(())
        }
        "FILE-CAPTURE" => {
            crate::native_core::rust_tests(
                evidence,
                "reader_walk_and_ubiquity_fact",
                "ab-session",
                &["--test", "files_mac_facts"],
            );
            gateway(world, evidence)
        }
        other => Err(Error::Invalid(format!(
            "files_native not ported for {other}"
        ))),
    }
}
