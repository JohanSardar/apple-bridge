#![allow(clippy::unwrap_used)]
use ab_native::{capture::Context, telephony::VoicemailReader};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::PathBuf;

fn directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ab-voicemail-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn vectors() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/voicemail-transcripts.json")).unwrap()
}
fn context(checkpoint: &str) -> Context {
    Context {
        observed_at: "2026-01-01T00:00:00.000Z".into(),
        checkpoint: checkpoint.into(),
        scan_id: checkpoint.into(),
    }
}
#[test]
fn carrier_and_facetime_keyed_archives_are_bounded_and_decoded() {
    for vector in vectors() {
        let bytes = STANDARD
            .decode(vector["archive"].as_str().unwrap())
            .unwrap();
        let result = ab_native::telephony::voicemail_transcript(&bytes);
        if vector["error"] == true {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), vector["transcript"]);
        }
    }
}
#[test]
fn voicemail_assets_transcripts_and_absences_reconcile() {
    let root = directory();
    let store = root.join("store");
    std::fs::create_dir(&store).unwrap();
    let path = store.join("FaceTimeMessageStore-local.sqlitedb");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER PRIMARY KEY,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZMAILBOXTYPE INTEGER,ZPROVIDER TEXT,ZMESSAGETYPE INTEGER,ZTRANSCRIPT BLOB,ZFILETYPE TEXT,ZDATEDELETED REAL);").unwrap();
    let archive = STANDARD
        .decode(vectors()[0]["archive"].as_str().unwrap())
        .unwrap();
    db.execute("INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic',10,2,0,0,'com.apple.coretelephony',0,?1,'MOV',NULL)", [archive]).unwrap();
    let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
    let first = reader.scan(&path, &context("one"), false, 100).unwrap();
    assert_eq!(first.batch.observations.len(), 1);
    assert_eq!(
        serde_json::to_value(&first.batch.observations[0]).unwrap()["data"]["transcript"]["text"],
        "Synthetic voicemail"
    );
    assert_eq!(
        serde_json::to_value(&first.batch.observations[0]).unwrap()["data"]["audio_state"],
        "missing"
    );
    reader.commit(first).unwrap();
    let audio = store.join("Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.MOV");
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    let bytes = vec![42; 200_000];
    std::fs::write(&audio, &bytes).unwrap();
    let second = reader.scan(&path, &context("two"), true, 100).unwrap();
    assert_eq!(
        serde_json::to_value(&second.batch.observations[0]).unwrap()["data"]["audio"]["mime_type"],
        "audio/mp4"
    );
    let (digest, handle) = second.batch.blob_handles.iter().next().unwrap();
    assert_eq!(*digest, ab_native::body_digest(&bytes));
    let mut fetched = Vec::new();
    loop {
        let chunk = reader
            .read_blob(handle, u64::try_from(fetched.len()).unwrap(), 131_072)
            .unwrap();
        fetched.extend(STANDARD.decode(chunk["bytes"].as_str().unwrap()).unwrap());
        if chunk["eof"] == true {
            break;
        }
    }
    assert_eq!(fetched, bytes);
    let handle = handle.clone();
    reader.commit(second).unwrap();
    let restarted = VoicemailReader::open(&root.join("staging")).unwrap();
    assert_eq!(
        restarted.read_blob(&handle, 199_999, 10).unwrap()["eof"],
        true
    );
    assert!(
        restarted
            .read_blob("voicemail-blob:missing", 0, 10)
            .is_err()
    );
    db.execute("UPDATE ZSTOREDMESSAGE SET ZDATEDELETED=20", [])
        .unwrap();
    let third = reader.scan(&path, &context("three"), true, 100).unwrap();
    assert_eq!(third.batch.observations[0].kind, "absent");
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn optional_columns_do_not_hide_metadata_or_invent_mailboxes() {
    let root = directory();
    let path = root.join("store.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER);INSERT INTO ZSTOREDMESSAGE VALUES(1,'fixture','Synthetic',10,2,0);").unwrap();
    let mut reader = VoicemailReader::open(&root.join("blobs")).unwrap();
    let scan = reader.scan(&path, &context("one"), false, 100).unwrap();
    let data = &serde_json::to_value(&scan.batch.observations[0]).unwrap()["data"];
    assert_eq!(data["mailbox"], "unknown");
    assert_eq!(data["type"], "unknown");
    assert_eq!(data["transcript_state"], "unavailable");
    assert_eq!(data["audio_state"], "unavailable");
    assert!(data.get("audio").is_none());
    assert!(reader.scan(&path, &context("two"), true, 0).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn unsafe_assets_and_malformed_transcripts_never_publish_content() {
    let root = directory();
    let store = root.join("store");
    std::fs::create_dir_all(store.join("Assets/00")).unwrap();
    let path = store.join("store.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZTRANSCRIPT BLOB,ZFILETYPE TEXT);INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic',10,2,0,x'FFFF','m4a');").unwrap();
    let outside = root.join("outside.m4a");
    std::fs::write(&outside, b"must not be exposed").unwrap();
    std::os::unix::fs::symlink(
        outside,
        store.join("Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.m4a"),
    )
    .unwrap();
    let mut reader = VoicemailReader::open(&root.join("blobs")).unwrap();
    let scan = reader.scan(&path, &context("one"), false, 100).unwrap();
    assert_eq!(
        serde_json::to_value(&scan.batch.observations[0]).unwrap()["data"]["transcript_state"],
        "unsupported"
    );
    assert_eq!(
        serde_json::to_value(&scan.batch.observations[0]).unwrap()["data"]["audio_state"],
        "missing"
    );
    assert!(scan.batch.blob_handles.is_empty());
    assert!(
        serde_json::to_value(&scan.batch.observations[0]).unwrap()["data"]
            .get("audio")
            .is_none()
    );
    assert!(
        serde_json::to_value(&scan.batch.observations[0]).unwrap()["data"]
            .get("transcript")
            .is_none()
    );
    assert_eq!(json!(scan.batch.source), "voicemail");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn facetime_video_is_not_published_as_audio_even_with_an_audio_extension() {
    let root = directory();
    let path = root.join("store.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZFILETYPE TEXT,ZPROVIDER TEXT,ZMESSAGETYPE INTEGER);INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic',10,2,0,'amr','com.apple.telephonyutilities.callservicesd.FaceTimeProvider',2);").unwrap();
    let file = root.join("Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.amr");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, b"not an audio-only message").unwrap();
    let mut reader = VoicemailReader::open(&root.join("blobs")).unwrap();
    let scan = reader.scan(&path, &context("one"), false, 100).unwrap();
    let data = &serde_json::to_value(&scan.batch.observations[0]).unwrap()["data"];
    assert_eq!(data["type"], "faceTimeVideo");
    assert_eq!(data["audio_state"], "unsupported");
    assert!(data.get("audio").is_none());
    assert!(scan.batch.blob_handles.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
