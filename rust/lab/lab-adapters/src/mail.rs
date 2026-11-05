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
    if id == "MAIL-COMMANDS" {
        crate::native_core::rust_tests(
            evidence,
            "native_mail_fixture",
            "ab-session",
            &["--lib", "session::mail"],
        );
        crate::native_core::rust_tests(
            evidence,
            "gateway_mail_validation",
            "ab-commands",
            &["--lib", "mail_tests"],
        );
        return Ok(());
    }
    let (check, package, tests): (&str, &str, &[&str]) = match id.as_str() {
        "MAIL-IDLE" => (
            "bounded_fixture_idle_no_checkpoints_and_watcher_resumes",
            "ab-session",
            &["--test", "capture_producer", "rust_mail"],
        ),
        "MAIL-BODY" => ("native_mail_fixture", "ab-native", &["--test", "mail"]),
        _ => (
            "native_mail_fixture",
            "ab-session",
            &["--test", "capture_producer", "rust_mail"],
        ),
    };
    crate::native_core::rust_tests(evidence, check, package, tests);
    if id == "MAIL-CAPTURE" {
        let passed = capture_world(world, &synthetic_mailbox());
        add(evidence, "mail_gateway_capture_and_subject_fts", passed);
    }
    Ok(())
}
fn synthetic_mailbox() -> Value {
    let mut rows = Vec::new();
    let mut row = |collection: &str, key: String, data: Value| {
        rows.push(json!({
            "kind": "upsert", "collection": collection, "native_key": key,
            "observed_at": jiff::Timestamp::now().to_string(), "data": data,
        }));
    };
    for index in 0..3 {
        let account = format!("synthetic{index}@example.com");
        row(
            "mail.account",
            account.clone(),
            json!({"name": account, "kind": "IMAP", "addresses": [account]}),
        );
        row(
            "mail.mailbox",
            format!("mailbox/{index}"),
            json!({"account_id": account, "name": "INBOX", "path": "/INBOX", "role": "inbox", "unread_count": 0, "message_count": 0}),
        );
    }
    for index in 0..1000 {
        row(
            "mail.message",
            index.to_string(),
            json!({
                "subject": format!("synthetic subject {index}"), "from": "sender@example.com",
                "received_at": "2026-01-01T00:00:00Z", "read": false, "flagged": false,
                "mailbox_id": "mailbox/0", "account_id": "synthetic0@example.com",
            }),
        );
    }
    Value::Array(rows)
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
