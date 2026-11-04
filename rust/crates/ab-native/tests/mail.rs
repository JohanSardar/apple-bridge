#![allow(clippy::unwrap_used)]
#[test]
fn foundation_mailbox_url_projections() {
    for line in include_str!("../parity/mail/url.jsonl").lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        let actual = ab_native::mail_url::parse(value["input"].as_str().unwrap());
        if value["host"].is_null() {
            assert!(actual.is_err());
            continue;
        }
        let actual = actual.unwrap_or_else(|error| panic!("{}: {error}", value["input"]));
        let account = value["user"].as_str().map_or_else(
            || value["host"].as_str().unwrap().into(),
            |user| format!("{user}@{}", value["host"].as_str().unwrap()),
        );
        assert_eq!(actual.account, account, "{}", value["input"]);
        assert_eq!(
            actual.kind,
            value["scheme"].as_str().unwrap_or("unknown").to_uppercase(),
            "{}",
            value["input"]
        );
        assert_eq!(actual.path, value["path"], "{}", value["input"]);
        assert_eq!(actual.name, value["last"], "{}", value["input"]);
    }
}

#[test]
fn partial_query_deadlines_reset_and_stale_mail_acknowledgements_fail() {
    use ab_native::{
        capture::{Context, Snapshot},
        mail::Reader,
    };
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let root = std::env::temp_dir().join(format!(
        "ab-mail-race-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("store.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE mailboxes(url TEXT);CREATE TABLE subjects(subject TEXT);CREATE TABLE addresses(address TEXT);CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT);INSERT INTO mailboxes VALUES('imap://synthetic@invalid/INBOX');INSERT INTO subjects VALUES('synthetic');INSERT INTO addresses VALUES('synthetic@invalid');INSERT INTO messages VALUES(1,1,1,1700000000,0,'<synthetic@invalid>');").unwrap();
    let snapshot = Snapshot::open(&path, "SELECT 'fixture'").unwrap();
    assert!(
        !snapshot
            .query_partial("SELECT 1", &[], 10, Duration::ZERO)
            .unwrap()
            .complete
    );
    let limited=snapshot.query_partial("WITH RECURSIVE t(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM t WHERE x<1000000) SELECT SUM(x) AS total FROM t",&[],10,Duration::from_millis(1)).unwrap();
    assert!(!limited.complete && limited.rows.is_empty());
    assert!(
        snapshot
            .query_partial("SELECT 1", &[], 10, Duration::from_secs(1))
            .unwrap()
            .complete
    );
    drop(snapshot);
    let mut reader = Reader::default();
    let context = Context {
        observed_at: "2026-01-01T00:00:00.000Z".into(),
        checkpoint: "a".into(),
        scan_id: "a".into(),
    };
    let first = reader.snapshot(&path, &context, 100, true).unwrap();
    let second = reader.snapshot(&path, &context, 100, true).unwrap();
    reader.commit(second).unwrap();
    assert!(reader.commit(first).is_err());
    db.execute_batch("UPDATE messages SET mailbox=999").unwrap();
    assert!(reader.poll(&path, &context, 100).is_err());
    db.execute_batch("UPDATE messages SET mailbox=1").unwrap();
    assert!(reader.poll(&path, &context, 100).unwrap().is_none());
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_swift_emlx_and_mime_fixture() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    for line in include_str!("../parity/mail/bodies.jsonl").lines() {
        let vector: serde_json::Value = serde_json::from_str(line).unwrap();
        let bytes = if let Some(file) = vector["fixture"].as_str() {
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../lab/fixtures/mail")
                    .join(file),
            )
            .unwrap()
        } else if let Some(count) = vector["repeat"].as_u64() {
            let body = vector["prefix"].as_str().unwrap().to_owned()
                + &"a".repeat(usize::try_from(count).unwrap());
            format!("{}\n{body}", body.len()).into_bytes()
        } else {
            STANDARD.decode(vector["input"].as_str().unwrap()).unwrap()
        };
        let actual = ab_native::mail_body::decode(&bytes, vector["partial"] == true);
        if vector["error"] == true {
            assert!(actual.is_err(), "{}", vector["name"]);
            continue;
        }
        let actual = actual.unwrap();
        if let Some(expected) = vector.get("result") {
            assert_eq!(&actual, expected, "{}", vector["name"]);
        }
        let bytes = ab_native::native_json::record_data(&actual).unwrap();
        assert_eq!(
            ab_native::body_digest(&bytes),
            vector["data_sha256"],
            "{}",
            vector["name"]
        );
    }
}

#[test]
fn a_flag_reaches_the_record_and_clearing_it_returns_to_null() {
    use ab_native::{capture, mail};
    let root = std::env::temp_dir().join(format!("ab-mail-flag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("Envelope Index");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE mailboxes(url TEXT);CREATE TABLE subjects(subject TEXT);CREATE TABLE addresses(address TEXT);CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT,flagged INTEGER,flag_color INTEGER);INSERT INTO mailboxes(rowid,url) VALUES(1,'imap://ACCOUNT/INBOX');INSERT INTO subjects(rowid,subject) VALUES(1,'s');INSERT INTO addresses(rowid,address) VALUES(1,'a');INSERT INTO messages(rowid,mailbox,subject,sender,date_received,read,message_id,flagged,flag_color) VALUES(7,1,1,1,1.0,1,'',0,0);").unwrap();
    let context = capture::Context {
        observed_at: "2026-01-01T00:00:00.000Z".into(),
        checkpoint: "c".into(),
        scan_id: "s".into(),
    };
    let flagged = |reader: &mail::Reader| {
        let pending = reader.snapshot(&path, &context, 100, true).unwrap();
        let row = pending
            .batch
            .observations
            .iter()
            .find(|row| row.collection == "mail.message")
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .get()
            .to_owned();
        let value: serde_json::Value = serde_json::from_str(&row).unwrap();
        (value["flagged"].clone(), value["flag_color"].clone())
    };
    let reader = mail::Reader::default();
    assert_eq!(
        flagged(&reader),
        (serde_json::Value::Null, serde_json::Value::Null)
    );
    db.execute_batch("UPDATE messages SET flagged=1,flag_color=3")
        .unwrap();
    assert_eq!(
        flagged(&reader),
        (serde_json::json!(true), serde_json::json!(3))
    );
    db.execute_batch("UPDATE messages SET flagged=0").unwrap();
    assert_eq!(flagged(&reader).0, serde_json::Value::Null);
    drop(db);
    let _ = std::fs::remove_dir_all(root);
}
