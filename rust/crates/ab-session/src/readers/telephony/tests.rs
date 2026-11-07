#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn voicemail_caches_the_captured_asset_version_not_a_later_sample() {
    for replacement in [false, true] {
        let root = std::env::temp_dir().join(format!(
            "ab-vm-signature-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("store/Assets/00")).unwrap();
        let store = root.join("store/voicemail.db");
        let db = rusqlite::Connection::open(&store).unwrap();
        db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZFILETYPE TEXT);INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic',10,2,0,'m4a');").unwrap();
        drop(db);
        let audio = root.join("store/Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.m4a");
        if replacement {
            std::fs::write(&audio, b"old").unwrap();
        }
        let mut reader =
            TelephonyReader::voicemail(&root.join("store"), &root.join("staging")).unwrap();
        let before = file_signature(&store);
        let context = ScanContext {
            checkpoint: "one".into(),
            scan_id: "one".into(),
            now: 1_767_225_600.0,
            fence: 1,
        };
        let pending = reader
            .assets()
            .unwrap()
            .scan(&store, &context_for(&context).unwrap(), false, 100)
            .unwrap();
        let new = root.join("new");
        std::fs::write(&new, b"new").unwrap();
        std::fs::rename(new, &audio).unwrap();
        reader.accept(pending, before).unwrap().unwrap();
        reader.commit().unwrap();
        let context = ScanContext {
            checkpoint: "two".into(),
            scan_id: "two".into(),
            ..context
        };
        let refreshed = reader.next(&context).unwrap();
        assert!(
            refreshed.is_some(),
            "asset change must force another capture"
        );
        let refreshed = refreshed.unwrap();
        assert_eq!(
            refreshed["observations"][0]["data"]["audio"]["digest"],
            ab_native::body_digest(b"new")
        );
        reader.commit().unwrap();
        assert!(reader.next(&context).unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
