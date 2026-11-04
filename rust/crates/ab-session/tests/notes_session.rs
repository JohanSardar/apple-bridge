#![allow(clippy::expect_used)]

use ab_session::readers::{CaptureReader, NotesReader, ScanContext};
use serde_json::Value;
use std::collections::BTreeSet;

const STORE: &str = "CREATE TABLE Z_METADATA(Z_UUID TEXT);INSERT INTO Z_METADATA VALUES('synthetic');CREATE TABLE Z_PRIMARYKEY(Z_ENT INTEGER,Z_NAME TEXT);INSERT INTO Z_PRIMARYKEY VALUES(1,'ICFolder'),(2,'ICNote'),(3,'ICAccount');CREATE TABLE ZICNOTEDATA(Z_PK INTEGER PRIMARY KEY,ZDATA BLOB,Z_OPT INTEGER);CREATE TABLE ZICCLOUDSYNCINGOBJECT(Z_PK INTEGER PRIMARY KEY,Z_ENT INTEGER,ZIDENTIFIER TEXT,ZTITLE1 TEXT,ZTITLE2 TEXT,ZNAME TEXT,ZACCOUNT8 INTEGER,ZPARENT INTEGER,ZFOLDER INTEGER,ZNOTEDATA INTEGER,ZMARKEDFORDELETION INTEGER DEFAULT 0,ZFOLDERTYPE INTEGER DEFAULT 0,ZISPASSWORDPROTECTED INTEGER DEFAULT 0,ZISPINNED INTEGER DEFAULT 0,Z_OPT INTEGER DEFAULT 1,ZNOTE1 INTEGER,ZPARENTATTACHMENT INTEGER,ZMEDIA INTEGER,ZTYPEUTI1 TEXT);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZNAME) VALUES(1,3,'account','Synthetic');INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE2,ZACCOUNT8,ZFOLDERTYPE) VALUES(10,1,'folder','Synthetic',1,0),(11,1,'trash','Recently Deleted',1,1);INSERT INTO ZICCLOUDSYNCINGOBJECT(Z_PK,Z_ENT,ZIDENTIFIER,ZTITLE1,ZFOLDER,ZNOTEDATA) VALUES(20,2,'one','One',10,1),(21,2,'two','Two',10,1),(22,2,'three','Three',10,1);";

struct Rig {
    home: std::path::PathBuf,
    ordinal: u32,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Rig {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let home =
            std::env::temp_dir().join(format!("ab-notes-session-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(home.join("store")).expect("store");
        let db = rusqlite::Connection::open(home.join("store/NoteStore.sqlite")).expect("database");
        db.execute_batch(STORE).expect("schema");
        db.execute(
            "INSERT INTO ZICNOTEDATA VALUES(1,?,1)",
            [include_bytes!("../../../../lab/fixtures/notes/native-plain.gz").as_slice()],
        )
        .expect("body");
        Self { home, ordinal: 0 }
    }

    fn root(&self) -> std::path::PathBuf {
        self.home.join("store")
    }

    fn reader(&self) -> NotesReader {
        NotesReader::new(self.root(), &self.home.join("staging")).expect("reader")
    }

    fn edit(&self, sql: &str) {
        let db =
            rusqlite::Connection::open(self.root().join("NoteStore.sqlite")).expect("database");
        db.execute_batch(sql).expect("edit");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    fn step(&mut self, reader: &mut NotesReader) -> Option<Value> {
        self.ordinal += 1;
        let context = ScanContext {
            checkpoint: format!("c{}", self.ordinal),
            scan_id: format!("s{}", self.ordinal),
            now: 1_793_000_000.0 + f64::from(self.ordinal),
            fence: 1,
        };
        let capture = reader.next(&context).expect("step");
        if capture.is_some() {
            reader.commit().expect("commit");
        }
        capture
    }
}

fn absent(capture: &Value) -> BTreeSet<String> {
    capture["observations"]
        .as_array()
        .expect("observations")
        .iter()
        .filter(|row| row["kind"] == "absent")
        .map(|row| row["native_key"].as_str().expect("key").to_owned())
        .collect()
}

fn hydrating(capture: &Value) -> bool {
    capture["scan_id"]
        .as_str()
        .is_some_and(|id| id.ends_with("-hydrate"))
}

#[test]
fn a_change_does_not_wait_for_the_body_backlog() {
    let mut rig = Rig::new();
    let mut reader = rig.reader();
    let first = rig.step(&mut reader).expect("first scan");
    assert!(!hydrating(&first));
    assert!(hydrating(&rig.step(&mut reader).expect("a body")));
    rig.edit("UPDATE ZICCLOUDSYNCINGOBJECT SET ZFOLDER=11,Z_OPT=2 WHERE Z_PK=22");
    let next = rig.step(&mut reader).expect("the change");
    assert!(absent(&next).contains("three"), "{next}");
}

#[test]
fn a_store_that_never_goes_quiet_still_gets_its_bodies() {
    let mut rig = Rig::new();
    let mut reader = rig.reader();
    rig.step(&mut reader).expect("first scan");
    let mut bodies = 0;
    for turn in 0..6 {
        rig.edit(&format!(
            "UPDATE ZICCLOUDSYNCINGOBJECT SET Z_OPT={},ZTITLE2='x{turn}' WHERE Z_PK=1",
            turn + 2
        ));
        if rig
            .step(&mut reader)
            .is_some_and(|capture| hydrating(&capture))
        {
            bodies += 1;
        }
    }
    assert!(bodies >= 2, "bodies {bodies}");
}

#[test]
fn a_restart_remembers_what_the_gateway_was_told() {
    let mut rig = Rig::new();
    let mut reader = rig.reader();
    rig.step(&mut reader).expect("first scan");
    let token = reader.token().expect("token");
    drop(reader);
    rig.edit("DELETE FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=21");
    let mut restarted = rig.reader();
    assert!(restarted.resume(&token));
    let capture = rig.step(&mut restarted).expect("first scan after restart");
    assert!(absent(&capture).contains("two"), "{capture}");
}

#[test]
fn a_token_from_the_previous_build_rescans_even_a_quiet_store() {
    let mut rig = Rig::new();
    rig.edit("UPDATE ZICCLOUDSYNCINGOBJECT SET ZFOLDER=11,Z_OPT=2 WHERE Z_PK=20");
    let mut reader = rig.reader();
    rig.step(&mut reader).expect("first scan");
    let token: Value = serde_json::from_str(&reader.token().expect("token")).expect("json");
    let old = token["signature"].as_str().expect("signature");
    let mut restarted = rig.reader();
    assert!(restarted.resume(old));
    let capture = rig.step(&mut restarted).expect("a first scan, not silence");
    assert!(absent(&capture).contains("one"), "{capture}");
}

#[test]
fn a_current_token_over_an_unchanged_store_stays_silent() {
    let mut rig = Rig::new();
    let mut reader = rig.reader();
    rig.step(&mut reader).expect("first scan");
    let token = reader.token().expect("token");
    let mut restarted = rig.reader();
    assert!(restarted.resume(&token));
    assert!(rig.step(&mut restarted).is_none());
}

#[test]
fn a_first_scan_names_notes_already_in_recently_deleted() {
    let mut rig = Rig::new();
    rig.edit("UPDATE ZICCLOUDSYNCINGOBJECT SET ZFOLDER=11,Z_OPT=2 WHERE Z_PK=20");
    let mut reader = rig.reader();
    let capture = rig.step(&mut reader).expect("first scan");
    assert_eq!(absent(&capture), BTreeSet::from(["one".to_owned()]));
}
