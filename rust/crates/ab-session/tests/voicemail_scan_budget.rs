#![allow(clippy::unwrap_used, dead_code)]
#[path = "../../ab-native/tests/support/voicemail.rs"]
mod fixture;
use ab_session::readers::{self, ScanContext};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn unchanged_voicemail_inputs_retry_until_every_deferred_transcript_is_captured() {
    let root = std::env::temp_dir().join(format!(
        "ab-session-vm-budget-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("store")).unwrap();
    let mut db = rusqlite::Connection::open(root.join("store/voicemail.db")).unwrap();
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZTRANSCRIPT BLOB);").unwrap();
    let archive = fixture::archive(&"x".repeat(4_000), 4);
    let transaction = db.transaction().unwrap();
    for id in 1..=200 {
        transaction
            .execute(
                "INSERT INTO ZSTOREDMESSAGE VALUES(?1,printf('%032X',?1),'Synthetic',10,2,0,?2)",
                rusqlite::params![id, archive],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    drop(db);
    let mut readers = readers::from_config(
        &json!({"voicemail":{"root":root.join("store"),"staging":root.join("staging")}})
            .to_string(),
    )
    .unwrap();
    let reader = readers.get_mut("voicemail").unwrap();
    let mut present = BTreeSet::new();
    let mut passes = 0;
    for pass in 0..4 {
        let context = ScanContext {
            checkpoint: format!("pass-{pass}"),
            scan_id: format!("pass-{pass}"),
            now: 1_767_225_600.0,
            fence: 1,
        };
        let Some(capture) = reader.next(&context).unwrap() else {
            break;
        };
        passes += 1;
        for row in capture["observations"].as_array().unwrap() {
            if row["data"]["transcript_state"] == "present" {
                present.insert(row["native_key"].as_str().unwrap().to_owned());
            }
        }
        reader.commit().unwrap();
    }
    assert!(passes > 1);
    assert_eq!(present.len(), 200);
    std::fs::remove_dir_all(root).unwrap();
}
