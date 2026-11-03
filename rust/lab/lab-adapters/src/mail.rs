#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::needless_pass_by_value,
    clippy::ref_option,
    clippy::semicolon_if_nothing_returned,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::unnecessary_wraps
)]
use crate::{
    assertions::await_until,
    http::{Client, random_id},
    replicas::{self, Endpoint},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const SCOPE: &str = "mail:read,system:read";
pub const NATIVE: [&str; 6] = [
    "MAIL-BODY",
    "MAIL-CAPTURE",
    "MAIL-COMMANDS",
    "MAIL-IDLE",
    "MAIL-LIVE",
    "MAIL-NO-FALSE-DELETE",
];
pub const REAL_ONLY: [&str; 6] = [
    "MAIL-AGENT",
    "MAIL-BODY-REAL",
    "MAIL-DRAFT-REAL",
    "MAIL-ORGANIZE-REAL",
    "MAIL-READ-REAL",
    "MAIL-SEND-SELF-REAL",
];
pub const SCENARIOS: [&str; 7] = [
    "MAIL-BODY",
    "MAIL-CAPTURE",
    "MAIL-COMMANDS",
    "MAIL-IDLE",
    "MAIL-LIVE",
    "MAIL-NO-FALSE-DELETE",
    "MAIL-REPLICA",
];

fn add(evidence: &mut Evidence, name: &str, ok: bool) {
    evidence.check(name, ok, "");
}
fn mail_checks() -> Option<(PathBuf, Vec<u8>)> {
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", "MailChecks")
        .output()
        .ok()?;
    if !built.status.success() {
        return None;
    }
    let directory = fs::read_to_string("native/.build/native-output-1").ok()?;
    Some((Path::new(directory.trim()).join("MailChecks"), built.stdout))
}
pub fn run(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if id == "MAIL-REPLICA" {
        return replica(world, evidence);
    }
    if REAL_ONLY.contains(&id.as_str()) {
        return Err(Error::Invalid(format!(
            "{id} needs a real or agent world, not ported to the sim-only Rust runner"
        )));
    }
    if !NATIVE.contains(&id.as_str()) {
        return Err(Error::Invalid(format!("mail not ported for {id}")));
    }
    let Some((binary, _)) = mail_checks() else {
        add(evidence, "native_mail_build", false);
        return Ok(());
    };
    add(evidence, "native_mail_build", true);
    let output = Command::new(&binary).arg(&id).output();
    let ok = output.as_ref().is_ok_and(|o| o.status.success());
    let name = if id == "MAIL-IDLE" {
        "bounded_fixture_idle_no_checkpoints_and_watcher_resumes"
    } else {
        "native_mail_fixture"
    };
    add(evidence, name, ok);
    if !ok || !matches!(id.as_str(), "MAIL-CAPTURE" | "MAIL-BODY") {
        return Ok(());
    }
    let fixture: Value = serde_json::from_slice(&output.map(|o| o.stdout).unwrap_or_default())
        .unwrap_or(Value::Null);
    let observations = fixture["observations"].clone();
    let mut passed = capture_world(world, &observations);
    if passed && id == "MAIL-BODY" {
        passed = body_world(world);
    }
    add(evidence, "mail_gateway_capture_and_subject_fts", passed);
    Ok(())
}
fn capture_world(world: &mut World, observations: &Value) -> bool {
    if replicas::start_with(world, 1, SCOPE, 0).is_err() {
        return false;
    }
    let Ok(gateway) = Client::new(world, SCOPE) else {
        return false;
    };
    if world
        .control("/dataset", &json!({"records": observations}))
        .is_err()
    {
        return false;
    }
    await_until(10, || {
        let Ok(read) = crate::assertions::read_all(&gateway, false, false) else {
            return false;
        };
        let count = |collection: &str| {
            read.records
                .iter()
                .filter(|r| r["collection"] == collection)
                .count()
        };
        if count("mail.account") != 3 || count("mail.mailbox") != 3 || count("mail.message") != 1000
        {
            return false;
        }
        gateway
            .call(
                "records.query",
                &json!({"text":"synthetic","limit":1000}),
                20,
            )
            .ok()
            .flatten()
            .is_some_and(|p| p["records"].as_array().is_some_and(|r| r.len() == 1000))
    })
}
fn body_world(world: &World) -> bool {
    let Ok(gateway) = Client::new(world, SCOPE) else {
        return false;
    };
    await_until(10, || {
        let Ok(read) = crate::assertions::read_all(&gateway, false, false) else {
            return false;
        };
        let Some(message) = read
            .records
            .iter()
            .find(|r| r["collection"] == "mail.message")
        else {
            return false;
        };
        let reply = gateway
            .call(
                "mail.body",
                &json!({"args": {"message_id": message["id"]}}),
                20,
            )
            .ok()
            .flatten();
        reply.is_some_and(|result| {
            let body = if result["result_json"].is_object() {
                result["result_json"].clone()
            } else {
                serde_json::from_str(result["result_json"].as_str().unwrap_or("null"))
                    .unwrap_or(Value::Null)
            };
            body["state"] == "downloaded" && body["plain_text"] == "Hello fixture"
        })
    })
}
fn replica(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let started = replicas::start_with(world, 1, SCOPE, 0).is_ok();
    add(evidence, "ubuntu_replica_started", started);
    if !started {
        return Ok(());
    }
    let gateway = Client::new(world, SCOPE)?;
    let tag = random_id("mailtag")?;
    let record = json!({
        "kind": "upsert", "collection": "mail.message", "native_key": tag,
        "data": {
            "account_id": "synthetic@example.com", "message_id": "<fixture@example.com>",
            "read": false, "to": null, "cc": null, "sent_at": null, "flagged": null,
            "flag_color": null, "size": null, "snippet": null, "subject": tag,
            "from": "sender@example.com", "received_at": "2026-10-28T00:00:00Z",
            "mailbox_ids": ["mailbox/1"], "body_state": null, "has_attachments": null
        }
    });
    let sent = world
        .control("/dataset", &json!({"records": [record]}))
        .is_ok();
    add(evidence, "metadata_only_fixture", sent);
    if !sent {
        return Ok(());
    }
    let replica = replicas::endpoints()
        .first()
        .map(Endpoint::client)
        .ok_or_else(|| Error::Invalid("replica".into()))?;
    let equal = await_until(20, || {
        let pages: Vec<Option<Value>> = [&gateway, &replica]
            .iter()
            .map(|client| {
                client
                    .call("records.query", &json!({"text": tag, "limit": 100}), 20)
                    .ok()
                    .flatten()
            })
            .collect();
        let rows: Vec<Vec<Value>> = pages
            .iter()
            .map(|p| {
                p.as_ref()
                    .and_then(|v| v["records"].as_array().cloned())
                    .unwrap_or_default()
            })
            .collect();
        rows.iter()
            .all(|r| r.len() == 1 && r[0]["collection"] == "mail.message")
            && rows[0] == rows[1]
    });
    add(evidence, "ubuntu_records_and_subject_fts_equal", equal);
    Ok(())
}
