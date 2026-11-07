#![allow(clippy::unwrap_used, dead_code)]
#[path = "support/voicemail.rs"]
mod fixture;
use ab_native::{capture::Context, telephony::VoicemailReader};
use std::collections::BTreeSet;

#[test]
fn cumulative_transcript_budget_keeps_metadata_and_eventually_enriches_every_record() {
    let root = std::env::temp_dir().join(format!(
        "ab-vm-scan-budget-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("store")).unwrap();
    let store = root.join("store/voicemail.db");
    let mut db = rusqlite::Connection::open(&store).unwrap();
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZTRANSCRIPT BLOB);").unwrap();
    let archive = fixture::archive(&"x".repeat(4_000), 4);
    let transaction = db.transaction().unwrap();
    for id in 1..=1000 {
        transaction
            .execute(
                "INSERT INTO ZSTOREDMESSAGE VALUES(?1,printf('%032X',?1),'Synthetic',10,2,0,?2)",
                rusqlite::params![id, archive],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    drop(db);
    let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
    let mut present = BTreeSet::new();
    for pass in 0..16 {
        let context = Context {
            observed_at: "2026-01-01T00:00:00.000Z".into(),
            checkpoint: format!("pass-{pass}"),
            scan_id: format!("pass-{pass}"),
        };
        let pending = reader.scan(&store, &context, pass != 0, 10_000).unwrap();
        let mut retained = 0;
        let mut deferred = 0;
        if pass == 0 {
            assert_eq!(pending.batch.observations.len(), 1000);
        }
        for row in &pending.batch.observations {
            let value = serde_json::to_value(row).unwrap();
            let data = &value["data"];
            if let Some(transcript) = data.get("transcript") {
                retained += ab_native::native_json::record_data(transcript)
                    .unwrap()
                    .len();
                present.insert(row.native_key.clone());
            } else if data["transcript_state"] == "pending" {
                assert!(
                    !present.contains(&row.native_key),
                    "deferral must not remove a previously captured transcript"
                );
                deferred += 1;
            }
        }
        assert!(
            retained <= 4 * 1024 * 1024,
            "retained transcript bytes must be bounded across the scan"
        );
        if pass == 0 {
            assert!(deferred > 0);
        }
        reader.commit(pending).unwrap();
        if present.len() == 1000 {
            break;
        }
    }
    assert_eq!(
        present.len(),
        1000,
        "budget deferrals must remain eligible for later captures"
    );
    std::fs::remove_dir_all(root).unwrap();
}
