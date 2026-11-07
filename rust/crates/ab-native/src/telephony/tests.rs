#![allow(clippy::unwrap_used)]
use super::*;

fn fixture() -> (std::path::PathBuf, std::path::PathBuf, Context) {
    let root = std::env::temp_dir().join(format!(
        "ab-voicemail-race-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("store/Assets/00")).unwrap();
    let db = rusqlite::Connection::open(root.join("store/voicemail.db")).unwrap();
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZFILETYPE TEXT);INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00112233445566778899AABBCCDDEEFF','Synthetic',10,2,0,'m4a');").unwrap();
    let audio = root.join("store/Assets/00/00112233-4455-6677-8899-AABBCCDDEEFF.m4a");
    let context = Context {
        observed_at: "2026-01-01T00:00:00.000Z".into(),
        checkpoint: "one".into(),
        scan_id: "one".into(),
    };
    (root, audio, context)
}

#[test]
fn voicemail_retries_audio_arriving_during_a_missing_staging_attempt() {
    let (root, audio, context) = fixture();
    let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
    let scanned = reader.scan_staged(
        &root.join("store/voicemail.db"),
        &context,
        false,
        100,
        &mut |assets, file, allowed| {
            let staged = assets.stage(file, allowed);
            assert!(staged.is_err());
            std::fs::write(&audio, b"arrived").unwrap();
            staged
        },
    );
    assert!(matches!(scanned, Err(Error::Conflict)));
    let retry = reader
        .scan(&root.join("store/voicemail.db"), &context, false, 100)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&retry.batch.observations[0]).unwrap()["data"]["audio"]["digest"],
        crate::body_digest(b"arrived")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn voicemail_retries_audio_replaced_during_staging() {
    let (root, audio, context) = fixture();
    std::fs::write(&audio, b"old").unwrap();
    let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
    let scanned = reader.scan_staged(
        &root.join("store/voicemail.db"),
        &context,
        false,
        100,
        &mut |assets, file, allowed| {
            let staged = assets.stage(file, allowed);
            let replacement = root.join("replacement");
            std::fs::write(&replacement, b"new").unwrap();
            std::fs::rename(replacement, &audio).unwrap();
            staged
        },
    );
    assert!(matches!(scanned, Err(Error::Conflict)));
    let retry = reader
        .scan(&root.join("store/voicemail.db"), &context, false, 100)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&retry.batch.observations[0]).unwrap()["data"]["audio"]["digest"],
        crate::body_digest(b"new")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn voicemail_retries_transient_staging_failures_without_caching_missing_audio() {
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::StorageFull,
    ] {
        let (root, audio, context) = fixture();
        std::fs::write(&audio, b"synthetic").unwrap();
        let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
        let scanned = reader.scan_staged(
            &root.join("store/voicemail.db"),
            &context,
            false,
            100,
            &mut |_, _, _| Err(std::io::Error::from(kind).into()),
        );
        assert!(
            scanned.is_err(),
            "staging failure must not become a committed missing recording"
        );
        assert_eq!(reader.audio_signature(), &AudioSignature::default());
        let retry = reader
            .scan(&root.join("store/voicemail.db"), &context, false, 100)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&retry.batch.observations[0]).unwrap()["data"]["audio_state"],
            "present"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn voicemail_repairs_corrupt_staged_blobs_without_source_race_retries() {
    let (root, audio, context) = fixture();
    std::fs::write(&audio, b"synthetic").unwrap();
    std::fs::create_dir_all(root.join("staging")).unwrap();
    let digest = crate::body_digest(b"synthetic");
    std::fs::write(root.join("staging").join(&digest), b"corrupted").unwrap();
    let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
    let scan = reader
        .scan(&root.join("store/voicemail.db"), &context, false, 100)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&scan.batch.observations[0]).unwrap()["data"]["audio"]["digest"],
        digest
    );
    assert_eq!(
        std::fs::read(root.join("staging").join(&digest)).unwrap(),
        b"synthetic"
    );
    reader.commit(scan).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn voicemail_acknowledgements_retain_fingerprints_instead_of_transcript_content() {
    let (root, _, context) = fixture();
    let mut fields = ::plist::Dictionary::new();
    fields.insert(
        "transcriptionString".into(),
        ::plist::Value::String("x".repeat(8_000)),
    );
    let mut archive = Vec::new();
    ::plist::Value::Dictionary(fields)
        .to_writer_binary(&mut archive)
        .unwrap();
    let db = rusqlite::Connection::open(root.join("store/voicemail.db")).unwrap();
    db.execute_batch("ALTER TABLE ZSTOREDMESSAGE ADD COLUMN ZTRANSCRIPT BLOB;")
        .unwrap();
    db.execute("UPDATE ZSTOREDMESSAGE SET ZTRANSCRIPT=?1", [archive])
        .unwrap();
    drop(db);
    let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
    let pending = reader
        .scan(&root.join("store/voicemail.db"), &context, false, 100)
        .unwrap();
    reader.commit(pending).unwrap();
    assert!(serde_json::to_vec(&reader.state).unwrap().len() < 1024);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn voicemail_propagates_staging_conflicts_even_with_unchanged_asset_metadata() {
    let (root, audio, context) = fixture();
    std::fs::write(&audio, b"synthetic").unwrap();
    let mut reader = VoicemailReader::open(&root.join("staging")).unwrap();
    let scanned = reader.scan_staged(
        &root.join("store/voicemail.db"),
        &context,
        false,
        100,
        &mut |assets, file, allowed| {
            assets.stage(file, allowed)?;
            Err(Error::Conflict)
        },
    );
    assert!(matches!(scanned, Err(Error::Conflict)));
    let retry = reader
        .scan(&root.join("store/voicemail.db"), &context, false, 100)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&retry.batch.observations[0]).unwrap()["data"]["audio_state"],
        "present"
    );
    std::fs::remove_dir_all(root).unwrap();
}
