#![cfg(feature = "rust-capture")]

use ab_session::{
    production,
    readers::{self, CaptureReader, ScanContext},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn temporary(name: &str) -> Fallible<PathBuf> {
    let directory = std::env::temp_dir().join(format!(
        "ab-stage-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    Ok(directory.canonicalize()?)
}

type Tree = BTreeMap<PathBuf, (bool, u64, SystemTime)>;

fn tree(root: &Path) -> Fallible<Tree> {
    fn walk(base: &Path, path: &Path, out: &mut Tree) -> Fallible<()> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let meta = std::fs::symlink_metadata(entry.path())?;
            out.insert(
                entry.path().strip_prefix(base)?.to_path_buf(),
                (meta.is_dir(), meta.len(), meta.modified()?),
            );
            if meta.is_dir() {
                walk(base, &entry.path(), out)?;
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn names(root: &Path) -> Fallible<Vec<PathBuf>> {
    Ok(tree(root)?.into_keys().collect())
}

fn context(round: u32) -> ScanContext {
    ScanContext {
        checkpoint: format!("checkpoint-{round}"),
        scan_id: format!("scan-{round}"),
        now: 1_800_000_000.0 + f64::from(round),
    }
}

fn voice_memos(root: &Path) -> Fallible<()> {
    let connection = rusqlite::Connection::open(root.join("CloudRecordings.db"))?;
    connection.execute_batch(
        "CREATE TABLE Z_METADATA(Z_UUID TEXT);
         INSERT INTO Z_METADATA VALUES('synthetic-uuid');
         CREATE TABLE ZCLOUDRECORDING(Z_PK INTEGER PRIMARY KEY,ZUNIQUEID TEXT,ZCUSTOMLABEL TEXT,ZDATE REAL,ZDURATION REAL,ZEVICTIONDATE REAL,ZFOLDER INTEGER,ZPATH TEXT);
         INSERT INTO ZCLOUDRECORDING VALUES(1,'rec-1','Synthetic memo',700000000.0,3.5,NULL,NULL,'memo.m4a');",
    )?;
    std::fs::write(root.join("memo.m4a"), vec![7_u8; 4096])?;
    Ok(())
}

fn notes_store(root: &Path) -> Fallible<()> {
    let db = rusqlite::Connection::open(root.join("NoteStore.sqlite"))?;
    db.execute_batch("CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount');CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER);CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,Z_OPT INTEGER DEFAULT 1,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME) VALUES(1,3,'account','Synthetic');INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8) VALUES(10,1,'folder','Synthetic',1);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(20,2,'note','Synthetic',10,1);")?;
    db.execute(
        "INSERT INTO ZICNOTEDATA VALUES(1,?,1)",
        [include_bytes!("../../../../lab/fixtures/notes/native-plain.gz").as_slice()],
    )?;
    Ok(())
}

fn drive(reader: &mut dyn CaptureReader) -> Fallible<Vec<Value>> {
    let mut frames = Vec::new();
    for round in 0..8 {
        match reader.next(&context(round))? {
            Some(frame) => {
                reader.commit()?;
                frames.push(frame);
            }
            None => break,
        }
    }
    Ok(frames)
}

#[test]
fn voice_memos_never_write_inside_the_source_container() -> Fallible<()> {
    let root = temporary("vm-root")?;
    let home = temporary("vm-home")?;
    voice_memos(&root)?;
    let staging = home.join("session/staging/voice_memos");
    let config = json!({"voice_memos":{"root":root,"staging":staging}}).to_string();
    let before = tree(&root)?;
    let mut readers = readers::from_config(&config)?;
    let reader = readers.get_mut("voice_memos").ok_or("reader")?;
    let frames = drive(reader.as_mut())?;
    let handles = frames
        .first()
        .and_then(|frame| frame["blob_handles"].as_object())
        .ok_or("blob handles")?;
    let handle = handles
        .values()
        .next()
        .and_then(Value::as_str)
        .ok_or("handle")?;
    let chunk = reader.blob(handle, 0, 1024)?;
    assert_eq!(chunk["t"], "BlobChunk");
    assert_eq!(tree(&root)?, before);
    assert_ne!(names(&staging)?.len(), 0);
    std::fs::remove_dir_all(root)?;
    std::fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn notes_never_write_inside_the_source_container() -> Fallible<()> {
    let parent = temporary("notes-parent")?;
    let root = parent.join("group.com.apple.notes");
    std::fs::create_dir(&root)?;
    let home = temporary("notes-home")?;
    notes_store(&root)?;
    let staging = home.join("session/staging/notes");
    let config = json!({"notes":{"root":root,"staging":staging}}).to_string();
    let before = tree(&root)?;
    let before_parent = names(&parent)?;
    let mut readers = readers::from_config(&config)?;
    let reader = readers.get_mut("notes").ok_or("reader")?;
    let frames = drive(reader.as_mut())?;
    assert_ne!(frames.len(), 0);
    assert_eq!(tree(&root)?, before);
    assert_eq!(names(&parent)?, before_parent);
    assert!(staging.is_dir());
    std::fs::remove_dir_all(parent)?;
    std::fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn production_defaults_staging_into_the_bridge_home() -> Fallible<()> {
    let root = temporary("prod-root")?;
    let parent = temporary("prod-notes")?;
    let notes = parent.join("group.com.apple.notes");
    std::fs::create_dir(&notes)?;
    let home = temporary("prod-home")?;
    voice_memos(&root)?;
    notes_store(&notes)?;
    std::fs::create_dir_all(home.join("session"))?;
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"voice_memos":{"root":root},"notes":{"root":notes}},"watch":false})
            .to_string(),
    )?;
    let before = (tree(&root)?, tree(&notes)?, names(&parent)?);
    let loaded = production::load(&home)?.ok_or("loaded")?;
    let mut readers = loaded.readers()?;
    for reader in readers.values_mut() {
        drive(reader.as_mut())?;
    }
    assert_eq!((tree(&root)?, tree(&notes)?, names(&parent)?), before);
    let staged = home.join(production::STAGING_DIRECTORY);
    assert!(staged.join("voice_memos").is_dir());
    assert!(staged.join("notes").is_dir());
    for path in [&root, &notes, &parent, &home] {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[test]
fn staging_that_overlaps_the_source_root_is_refused() -> Fallible<()> {
    let root = temporary("overlap-root")?;
    voice_memos(&root)?;
    let outside = temporary("overlap-outside")?;
    let link = outside.join("link");
    std::os::unix::fs::symlink(&root, &link)?;
    let cases = [
        root.join(".ab-session-assets"),
        root.join("a/../.ab-session-assets"),
        root.clone(),
        link.join("staged"),
        root.parent().ok_or("parent")?.to_path_buf(),
        PathBuf::from("relative/staging"),
    ];
    for staging in cases {
        for source in ["voice_memos", "notes"] {
            let config = json!({source:{"root":root,"staging":staging}}).to_string();
            assert!(
                readers::from_config(&config).is_err(),
                "{source} accepted {}",
                staging.display()
            );
        }
    }
    for source in ["voice_memos", "notes"] {
        let config = json!({source:{"root":root}}).to_string();
        assert!(
            readers::from_config(&config).is_err(),
            "{source} without staging"
        );
    }
    let entries = names(&root)?;
    assert_eq!(
        entries,
        vec![
            PathBuf::from("CloudRecordings.db"),
            PathBuf::from("memo.m4a")
        ]
    );
    std::fs::remove_dir_all(root)?;
    std::fs::remove_dir_all(outside)?;
    Ok(())
}
