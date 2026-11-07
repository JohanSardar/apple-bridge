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
