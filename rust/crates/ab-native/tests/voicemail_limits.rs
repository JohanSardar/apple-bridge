#![allow(clippy::unwrap_used)]
#[path = "support/voicemail.rs"]
mod fixture;
use ab_native::{
    DecodeError,
    capture::{Context, Error},
    telephony::{VoicemailReader, voicemail_transcript},
};
use serde_json::Value;

#[test]
fn keyed_archive_uid_expansion_has_a_cumulative_byte_limit() {
    let bytes = fixture::archive(&"x".repeat(90_000), 200);
    assert!(bytes.len() < 100_000);
    assert!(matches!(
        voicemail_transcript(&bytes),
        Err(Error::Decode(DecodeError::Limit))
    ));
    assert!(matches!(
        ab_native::plist::archive_root(&bytes),
        Err(DecodeError::Limit)
    ));
    assert!(voicemail_transcript(&fixture::archive("synthetic", 4)).is_ok());
}

#[test]
fn binary_plist_object_references_have_a_cumulative_byte_limit() {
    let bytes = fixture::repeated_strings(&"x".repeat(90_000), 200);
    assert!(bytes.len() < 100_000);
    assert!(matches!(
        ab_native::plist::decode(&bytes),
        Err(DecodeError::Limit)
    ));
}

#[test]
fn oversized_serialized_transcripts_keep_metadata_and_audio() {
    for (text, segments) in [
        ("x".repeat(40_000), 4),
        ("<".repeat(8_000), 4),
        ("x".repeat(90_000), 200),
    ] {
        let archive = fixture::archive(&text, segments);
        assert_eq!(voicemail_transcript(&archive).is_ok(), segments == 4);
        let root = std::env::temp_dir().join(format!(
            "ab-vm-limit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("store/Assets/00")).unwrap();
        let store = root.join("store/FaceTimeMessageStore-local.sqlitedb");
        let db = rusqlite::Connection::open(&store).unwrap();
        db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZTRANSCRIPT BLOB,ZFILETYPE TEXT);").unwrap();
        db.execute("INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic',10,2,0,?1,'m4a')", [archive]).unwrap();
        let audio = b"synthetic audio";
        std::fs::write(
            root.join("store/Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.m4a"),
            audio,
        )
        .unwrap();
        let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
        let pending = reader
            .scan(
                &store,
                &Context {
                    observed_at: "2026-01-01T00:00:00.000Z".into(),
                    checkpoint: "one".into(),
                    scan_id: "one".into(),
                },
                false,
                100,
            )
            .unwrap();
        let observation = serde_json::to_value(&pending.batch.observations[0]).unwrap();
        let data = &observation["data"];
        assert_eq!(data["transcript_state"], "unsupported");
        assert!(data.get("transcript").is_none());
        assert_eq!(data["sender"], "Synthetic");
        assert_eq!(data["audio_state"], "present");
        let digest = ab_native::body_digest(audio);
        assert_eq!(data["audio"]["digest"], digest);
        let handle = pending.batch.blob_handles.get(&digest).unwrap();
        assert_eq!(reader.read_blob(handle, 0, 100).unwrap()["eof"], true);
        assert!(
            ab_native::native_json::record_data(&observation)
                .unwrap()
                .len()
                <= 131_072
        );
        assert!(matches!(data, Value::Object(_)));
        std::fs::remove_dir_all(root).unwrap();
    }
}
