use crate::{assertions::await_until, http::Client};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};

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
