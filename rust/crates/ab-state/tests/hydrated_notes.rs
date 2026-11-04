#![allow(clippy::expect_used)]
use ab_state::{Gateway, State};
use ab_wire::messages::Capture;
use rusqlite::Connection;
use serde_json::{Value, json};

fn commit(store: &mut State<Gateway>, number: u32, predecessor: &str, data: &Value) {
    let now = "2026-11-03T00:00:00Z";
    let capture: Capture = serde_json::from_value(json!({
        "source": "notes",
        "generation": "g",
        "predecessor": predecessor,
        "checkpoint": number.to_string(),
        "observations": [{
            "kind": "upsert",
            "collection": "notes.note",
            "native_key": "n1",
            "observed_at": now,
            "data": data,
        }],
        "scan_id": number.to_string(),
        "scope": ["notes.note"],
        "scan_started_at": now,
        "completion_evidence": "complete",
    }))
    .expect("capture");
    store.commit_capture(&capture).expect("commit");
}

fn stored(home: &std::path::Path) -> Value {
    let database = Connection::open(home.join("state.db")).expect("database");
    let payload: Vec<u8> = database
        .query_row(
            "SELECT payload FROM records WHERE collection='notes.note'",
            [],
            |row| row.get(0),
        )
        .expect("record");
    serde_json::from_slice::<Value>(&payload).expect("json")["data"].clone()
}

fn note(fingerprint: &str, pending: bool) -> Value {
    let mut data = json!({
        "title": "T", "plain_text": "T\nbody", "native_fingerprint": fingerprint,
        "locked": false, "folder_id": "", "recently_deleted": false, "read_only": false,
        "preview": "body",
    });
    if pending {
        data["body_status"] = json!("pending");
    } else {
        data["html"] = json!("<p>body</p>");
        data["blocks"] = json!([{"type": "paragraph"}]);
    }
    data
}

#[test]
fn a_metadata_only_note_never_replaces_the_hydrated_one_with_the_same_fingerprint() {
    let directory = tempfile::tempdir().expect("home");
    let mut store = State::<Gateway>::open(directory.path()).expect("open");
    commit(&mut store, 1, "", &note("f1", false));
    commit(&mut store, 2, "1", &note("f1", true));
    let kept = stored(directory.path());
    assert!(kept.get("body_status").is_none());
    assert!(kept.get("html").is_some());
    commit(&mut store, 3, "2", &note("f2", true));
    let changed = stored(directory.path());
    assert_eq!(changed["body_status"], "pending");
    assert!(changed.get("html").is_none());
    commit(&mut store, 4, "3", &note("f2", false));
    assert!(stored(directory.path()).get("html").is_some());
}
