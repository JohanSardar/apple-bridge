#![allow(clippy::expect_used)]
use ab_state::{Gateway, State};
use ab_wire::messages::Capture;
use rusqlite::Connection;
use serde_json::{Value, json};

const NOW: &str = "2026-11-04T00:00:00Z";

fn upsert(key: &str) -> Value {
    json!({
        "kind": "upsert", "collection": "notes.note", "native_key": key, "observed_at": NOW,
        "data": {"title": key, "plain_text": "body", "native_fingerprint": key, "locked": false,
                 "folder_id": "", "recently_deleted": false, "read_only": false, "preview": "body"},
    })
}

fn absent(key: &str) -> Value {
    json!({"kind": "absent", "collection": "notes.note", "native_key": key, "observed_at": NOW, "revalidated": true})
}

fn commit(store: &mut State<Gateway>, number: u32, observations: &[Value]) {
    let capture: Capture = serde_json::from_value(json!({
        "source": "notes",
        "generation": "g",
        "predecessor": if number == 1 { String::new() } else { (number - 1).to_string() },
        "checkpoint": number.to_string(),
        "observations": observations,
        "scan_id": number.to_string(),
        "scope": ["notes.folder", "notes.note"],
        "scan_started_at": NOW,
        "completion_evidence": "complete",
    }))
    .expect("capture");
    store.commit_capture(&capture).expect("commit");
}

fn tally(home: &std::path::Path) -> (i64, i64, i64) {
    let database = Connection::open(home.join("state.db")).expect("database");
    let count = |sql: &str| -> i64 {
        database
            .query_row(sql, [], |row| row.get(0))
            .expect("count")
    };
    (
        count("SELECT COUNT(*) FROM records WHERE collection='notes.note' AND deleted=0"),
        count("SELECT COUNT(*) FROM records WHERE collection='notes.note' AND deleted=1"),
        count("SELECT COALESCE(MAX(seq),0) FROM records"),
    )
}

#[test]
fn a_first_scan_after_a_switch_removes_only_what_is_still_live() {
    let directory = tempfile::tempdir().expect("home");
    let mut store = State::<Gateway>::open(directory.path()).expect("open");
    commit(
        &mut store,
        1,
        &[
            upsert("kept"),
            upsert("trashed-long-ago"),
            upsert("trashed-unseen"),
        ],
    );
    commit(&mut store, 2, &[absent("trashed-long-ago")]);
    let (live, removed, sequence) = tally(directory.path());
    assert_eq!((live, removed), (2, 1));

    commit(
        &mut store,
        3,
        &[
            absent("trashed-long-ago"),
            absent("trashed-unseen"),
            absent("never-in-the-gateway"),
        ],
    );
    assert_eq!(
        tally(directory.path()),
        (1, 2, sequence + 1),
        "one change: the note that was still live"
    );

    commit(
        &mut store,
        4,
        &[
            absent("trashed-long-ago"),
            absent("trashed-unseen"),
            absent("never-in-the-gateway"),
        ],
    );
    assert_eq!(
        tally(directory.path()),
        (1, 2, sequence + 1),
        "another first scan changes nothing"
    );
}

#[test]
fn a_restored_note_comes_back_under_its_old_record() {
    let directory = tempfile::tempdir().expect("home");
    let mut store = State::<Gateway>::open(directory.path()).expect("open");
    commit(&mut store, 1, &[upsert("n")]);
    let id = |home: &std::path::Path| -> String {
        Connection::open(home.join("state.db"))
            .expect("database")
            .query_row(
                "SELECT id FROM records WHERE collection='notes.note'",
                [],
                |row| row.get(0),
            )
            .expect("id")
    };
    let before = id(directory.path());
    commit(&mut store, 2, &[absent("n")]);
    assert_eq!(tally(directory.path()).1, 1);
    commit(&mut store, 3, &[upsert("n")]);
    assert_eq!(tally(directory.path()).0, 1);
    assert_eq!(id(directory.path()), before);
}
