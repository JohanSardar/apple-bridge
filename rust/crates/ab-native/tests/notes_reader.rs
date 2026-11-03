#![allow(clippy::unwrap_used)]

#[test]
fn every_swift_plain_text_fixture() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../lab/fixtures/notes");
    for line in include_str!("../parity/notes/plain.jsonl").lines() {
        let vector: serde_json::Value = serde_json::from_str(line).unwrap();
        let bytes = std::fs::read(root.join(vector["fixture"].as_str().unwrap())).unwrap();
        let actual = ab_native::notes::plain_text(&bytes);
        if vector["error"] == true {
            assert!(actual.is_err(), "{}", vector["fixture"]);
            continue;
        }
        let text = actual.unwrap();
        if let Some(expected) = vector.get("text") {
            assert_eq!(&serde_json::json!(text), expected, "{}", vector["fixture"]);
        }
        assert_eq!(
            ab_native::body_digest(text.as_bytes()),
            vector["utf8_sha256"],
            "{}",
            vector["fixture"]
        );
    }
}
#[test]
fn stale_metadata_and_hydration_acknowledgements_cannot_overwrite_new_notes() {
    use ab_native::{
        capture,
        notes_reader::{Context, Facts, Reader},
    };
    use std::time::{SystemTime, UNIX_EPOCH};
    let root = std::env::temp_dir().join(format!(
        "ab-notes-race-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("NoteStore.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount');CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER);CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,Z_OPT INTEGER DEFAULT 1,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME) VALUES(1,3,'account','Synthetic');INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8) VALUES(10,1,'folder','Synthetic',1);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(20,2,'note','Synthetic',10,1);").unwrap();
    db.execute(
        "INSERT INTO ZICNOTEDATA VALUES(1,?,1)",
        [include_bytes!("../../../../lab/fixtures/notes/native-plain.gz").as_slice()],
    )
    .unwrap();
    let context = Context {
        capture: capture::Context {
            observed_at: "2026-01-01T00:00:00.000Z".into(),
            checkpoint: "a".into(),
            scan_id: "a".into(),
        },
        coverage: serde_json::json!({}),
        facts: Facts::default(),
        asset_paths: Vec::new(),
    };
    let mut reader = Reader::open(&path, &root.join("blobs")).unwrap();
    let first = reader.reconcile(&context).unwrap();
    let second = reader.reconcile(&context).unwrap();
    reader.commit(second).unwrap();
    assert!(reader.commit(first).is_err());
    let stale = reader.hydrate_next(&context).unwrap().unwrap();
    db.execute_batch("UPDATE ZICCLOUDSYNCINGOBJECT SET ZTITLE1='Changed',Z_OPT=2 WHERE Z_PK=20")
        .unwrap();
    let changed = reader.reconcile(&context).unwrap();
    reader.commit(changed).unwrap();
    assert!(!reader.hydration_current(&stale).unwrap());
    assert!(!reader.commit_hydration(stale).unwrap());
    let stale = reader.hydrate_next(&context).unwrap().unwrap();
    db.execute_batch("UPDATE Z_METADATA SET Z_UUID='synthetic-2'")
        .unwrap();
    let replacement = reader.reconcile(&context).unwrap();
    assert!(
        !replacement
            .batch
            .observations
            .iter()
            .any(|row| row.kind == "absent")
    );
    reader.commit(replacement).unwrap();
    assert!(!reader.commit_hydration(stale).unwrap());
    let fresh = reader.hydrate_next(&context).unwrap().unwrap();
    assert!(reader.commit_hydration(fresh).unwrap());
    db.execute(
        "UPDATE ZICNOTEDATA SET ZDATA=?,Z_OPT=2 WHERE Z_PK=1",
        [include_bytes!("../../../../lab/fixtures/notes/full.gz").as_slice()],
    )
    .unwrap();
    db.execute_batch("INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNOTE1,ZTYPEUTI1) VALUES(30,4,'fixture-image',20,'test.synthetic')").unwrap();
    let pending = reader.reconcile(&context).unwrap();
    reader.commit(pending).unwrap();
    assert!(matches!(
        reader.hydrate_next(&context),
        Err(capture::Error::FactsRequired(_))
    ));
    assert!(reader.has_hydration());
    let mut facts = context.clone();
    facts.facts.mime.insert("test.synthetic".into(), None);
    let pending = reader.hydrate_next(&facts).unwrap().unwrap();
    let value: serde_json::Value =
        serde_json::from_str(pending.batch.observations[0].data.as_ref().unwrap().get()).unwrap();
    assert_eq!(value["content_status"], "complete");
    assert!(value.get("raw_body_digest").is_none());
    assert!(reader.commit_hydration(pending).unwrap());
    reader.finish_hydration(SystemTime::now()).unwrap();
    drop(reader);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn blob_retention_clock_preserves_active_recent_and_symlink_entries() {
    use ab_native::assets::Store;
    use std::{
        collections::BTreeSet,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    let root = std::env::temp_dir().join(format!(
        "ab-notes-retention-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut store = Store::open(&root.join("blobs"), "notes-blob:").unwrap();
    let old = store.bytes(b"expired").unwrap();
    let active = store.bytes(b"active").unwrap();
    let recent = store.bytes(b"recent").unwrap();
    let boundary = store.bytes(b"boundary").unwrap();
    let now = UNIX_EPOCH + Duration::from_secs(100_000);
    for reference in [&old, &active] {
        std::fs::File::open(store.file(&reference.digest).unwrap())
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(UNIX_EPOCH))
            .unwrap();
    }
    std::fs::File::open(store.file(&recent.digest).unwrap())
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(100)))
        .unwrap();
    std::fs::File::open(store.file(&boundary.digest).unwrap())
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(now - Duration::from_secs(3600)))
        .unwrap();
    let outside = root.join("outside");
    std::fs::write(&outside, b"synthetic outside").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("blobs/link")).unwrap();
    let old_handle = store.handle(&old.digest).unwrap();
    store
        .prune(&BTreeSet::from([active.digest.clone()]), now)
        .unwrap();
    assert!(store.read(&old_handle, 0, 1).is_err());
    assert!(store.file(&active.digest).unwrap().exists());
    assert!(store.file(&recent.digest).unwrap().exists());
    assert!(store.file(&boundary.digest).unwrap().exists());
    assert_eq!(std::fs::read(&outside).unwrap(), b"synthetic outside");
    assert!(root.join("blobs/link").is_symlink());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
