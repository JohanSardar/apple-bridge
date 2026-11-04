use crate::{
    assertions::{await_until, read_all, record_multiset},
    http::Client,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

pub const SCOPE: &str = "phone:read,system:read";
pub const NATIVE_SCENARIOS: [&str; 5] = [
    "CALL-CAPTURE",
    "CALL-IDLE",
    "VM-CAPTURE",
    "VMEMO-CAPTURE",
    "VMEMO-LATE-ASSETS",
];
pub const REPLICA_SCENARIOS: [&str; 1] = ["CALLS-REPLICA"];

pub fn native_build(target: &str) -> Option<PathBuf> {
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !built {
        return None;
    }
    let directory = fs::read_to_string("native/.build/native-output-1").ok()?;
    Some(PathBuf::from(directory.trim()).join(target))
}
pub fn native_run(evidence: &mut Evidence, build: &str, target: &str, args: &[&str], check: &str) {
    let binary = native_build(target);
    evidence.check(
        build,
        binary.is_some(),
        "native fixture checks built on this host",
    );
    let Some(binary) = binary else {
        return;
    };
    let passed = Command::new(binary)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    evidence.check(check, passed, "native fixture checks passed");
}
fn calls_replica(world: &World, evidence: &mut Evidence) -> Result<()> {
    let gateway = Client::new(world, SCOPE)?;
    let replica = crate::replicas::endpoints()
        .first()
        .map(crate::replicas::Endpoint::client);
    let date = jiff::Timestamp::now().to_string();
    let inserted = world
        .control(
            "/phone",
            &json!({"key": "fixture-call", "data": {"direction": "incoming", "handle": "synthetic",
                "name": "Fixture", "date": date, "duration": 42, "service": "cellular", "answered": true}}),
        )
        .is_ok();
    evidence.check("fixture_insert", inserted, "synthetic call inserted");
    let calls = |client: &Client| {
        read_all(client, false, false).map(|read| {
            read.records
                .into_iter()
                .filter(|record| record["collection"] == "phone.call")
                .collect::<Vec<_>>()
        })
    };
    let converged = await_until(30, || {
        match (calls(&gateway), replica.as_ref().map(calls)) {
            (Ok(left), Some(Ok(right))) => {
                left.len() == 1 && record_multiset(&left, true) == record_multiset(&right, true)
            }
            _ => false,
        }
    });
    evidence.check(
        "ubuntu_call_digest_matches",
        converged,
        "replica call records equal the gateway",
    );
    Ok(())
}
fn calls_real(world: &World, evidence: &mut Evidence) -> Result<()> {
    let truth = Command::new("python3").arg("lab/truth/phone.py").output();
    let expected: Option<BTreeMap<String, Map<String, Value>>> = truth
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice(&output.stdout).ok());
    evidence.check(
        "independent_truth_read",
        expected.is_some(),
        "independent phone truth read",
    );
    let Some(expected) = expected else {
        return Ok(());
    };
    let gateway = Client::new(world, SCOPE)?;
    let cutoff = jiff::Timestamp::now().as_millisecond() - 30 * 24 * 3600 * 1000;
    let matches = || {
        let Ok(read) = read_all(&gateway, false, true) else {
            return false;
        };
        let keys: BTreeMap<&str, &str> = read
            .aliases
            .iter()
            .filter_map(|alias| Some((alias["record_id"].as_str()?, alias["native_key"].as_str()?)))
            .collect();
        let mut actual: BTreeMap<String, Value> = BTreeMap::new();
        for record in &read.records {
            if record["collection"] != "phone.call" {
                continue;
            }
            let mut data = record["data"].clone();
            let Some(at) = data["date"]
                .as_str()
                .and_then(|text| text.parse::<jiff::Timestamp>().ok())
            else {
                return false;
            };
            if at.as_millisecond() < cutoff {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            {
                data["date"] = json!(at.as_millisecond() as f64);
            }
            let key = keys
                .get(record["id"].as_str().unwrap_or_default())
                .copied()
                .unwrap_or_default();
            actual.insert(key.to_owned(), data);
        }
        actual.len() == expected.len()
            && expected.len() >= 20
            && expected.iter().all(|(key, want)| {
                actual
                    .get(key)
                    .is_some_and(|got| got == &Value::Object(want.clone()))
            })
    };
    evidence.check(
        "thirty_day_counts_and_twenty_digests",
        await_until(45, matches),
        "gateway calls match the independent truth",
    );
    Ok(())
}
fn fixture(id: &str) -> Option<(&'static str, &'static str, Value)> {
    match id {
        "CALL-CAPTURE" => Some((
            "phone",
            "phone.call",
            json!({"direction": "incoming", "handle": "synthetic", "name": "Fixture",
                "date": "2031-01-01T00:00:00Z", "duration": 42, "service": "cellular", "answered": true}),
        )),
        "VM-CAPTURE" => Some((
            "voicemail",
            "voicemail.message",
            json!({"sender": "Synthetic", "date": "2031-01-01T00:00:00Z", "duration": 42,
                "read": false, "mailbox": "inbox", "type": "carrierVoicemail",
                "transcript_state": "missing", "audio_state": "missing"}),
        )),
        "VMEMO-CAPTURE" | "VMEMO-LATE-ASSETS" => Some((
            "voice_memos",
            "voice_memos.recording",
            json!({"title": "Synthetic", "date": "2031-01-01T00:00:00Z", "duration": 42,
                "folder": "1", "recently_deleted": false,
                "transcript_state": "missing", "audio_state": "missing"}),
        )),
        _ => None,
    }
}
fn put(
    world: &World,
    source: &str,
    collection: &str,
    fields: &Value,
    kind: &str,
    version: u64,
) -> bool {
    if lab_world::session_media_source(source) {
        return world
            .stage_session_media(source, version, kind != "absent")
            .is_ok();
    }
    let mut data = fields.clone();
    data["fixture_version"] = json!(version);
    let at = jiff::Timestamp::now().to_string();
    let mut capture = json!({"source": source, "generation": "native-media-fixture",
        "observations": [{"kind": kind, "collection": collection, "native_key": "native-media-fixture",
            "data": data, "observed_at": at, "revalidated": true}]});
    if kind == "absent" {
        capture["scan_id"] = json!("native-media-fixture");
        capture["scan_started_at"] = json!(at);
        capture["scope"] = json!([collection]);
        capture["completion_evidence"] = json!("complete");
    }
    world.control("/capture", &capture).is_ok()
}
fn gateway_flow(world: &World, evidence: &mut Evidence) -> Result<()> {
    let Some((source, collection, fields)) = fixture(&evidence.scenario) else {
        return Ok(());
    };
    let client = Client::new(world, "admin")?;
    let rows = || -> Option<Vec<Value>> {
        client
            .call(
                "records.query",
                &json!({"collection": collection, "limit": 10}),
                10,
            )
            .ok()
            .flatten()?["records"]
            .as_array()
            .cloned()
    };
    evidence.check("selected_gateway_started", true, "gateway ready");
    let mut id: Option<String> = None;
    for version in 1..=2_u64 {
        let sent = put(world, source, collection, &fields, "upsert", version);
        evidence.check(
            &format!("selected_gateway_capture_{version}"),
            sent,
            "fixture capture",
        );
        if !sent {
            return Ok(());
        }
        let seen = await_until(5, || {
            let Some(rows) = rows().filter(|r| r.len() == 1) else {
                return false;
            };
            let row = &rows[0];
            let seen_version = if lab_world::session_media_source(source) {
                u64::try_from(lab_world::session_media_version(
                    row["data"]["duration"].as_f64().unwrap_or_default(),
                ))
                .ok()
            } else {
                row["data"]["fixture_version"].as_u64()
            };
            if seen_version != Some(version) || row["version"].as_u64() != Some(version) {
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
            "fixture readback",
        );
        if !seen {
            return Ok(());
        }
    }
    let sent = put(world, source, collection, &fields, "absent", 3);
    evidence.check("selected_gateway_absent_capture", sent, "absent capture");
    if sent {
        evidence.check(
            "selected_gateway_absent_readback",
            await_until(5, || rows().is_some_and(|r| r.is_empty())),
            "absent readback",
        );
    }
    Ok(())
}
pub fn phone(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    match id.as_str() {
        "CALL-CAPTURE" => native_run(
            evidence,
            "calls_native_fixture_build",
            "CallsChecks",
            &[],
            "wal_insert_reconcile_delete_partial_scan",
        ),
        "CALL-IDLE" => native_run(
            evidence,
            "calls_native_fixture_build",
            "CallsChecks",
            &["--idle"],
            "five_minute_source_cpu_under_two_percent_and_no_idle_checkpoints",
        ),
        "CALLS-REPLICA" => calls_replica(world, evidence)?,
        "CALL-READ-REAL" => calls_real(world, evidence)?,
        "VM-CAPTURE" | "VMEMO-CAPTURE" | "VMEMO-LATE-ASSETS" => {
            let check = if id == "VMEMO-LATE-ASSETS" {
                "folders_late_transcript_audio_blob_same_record"
            } else {
                "metadata_wal_insert_update_delete"
            };
            native_run(
                evidence,
                "recordings_native_fixture_build",
                "RecordingsChecks",
                &[id.as_str()],
                check,
            );
        }
        _ => return Err(Error::Invalid(format!("phone not ported for {id}"))),
    }
    if fixture(&id).is_some() && evidence.passed {
        gateway_flow(world, evidence)?;
    }
    Ok(())
}
