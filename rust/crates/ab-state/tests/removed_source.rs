#![allow(clippy::expect_used)]
use ab_domain::{Coverage, Source};
use ab_state::{Cursor, Gateway, ReadQuery, State, Visibility};
use rusqlite::{Connection, params};
use serde_json::json;
use std::path::Path;

const REMOVED_ID: &str = "src_01M6Q2A546QTPRFK1BCDNX16KB";

fn removed_row(source: &str, collection: &str) -> String {
    json!({
        "collection": collection,
        "id": REMOVED_ID,
        "version": 1,
        "source": source,
        "observed_at": "2026-01-01T00:00:00Z",
        "deleted": false,
        "coverage": {"fields": "full", "missing": null},
        "data": {"source": source, "state": "ready"},
    })
    .to_string()
}

fn plant_record(home: &Path, source: &str, collection: &str, payload: &str) {
    Connection::open(home.join("state.db"))
        .expect("database")
        .execute(
            "INSERT INTO records VALUES(?,?,1,?,0,?,1,'2026-01-01T00:00:00Z')",
            params![collection, REMOVED_ID, source, payload.as_bytes()],
        )
        .expect("plant");
}

fn visibility() -> Visibility {
    ["system", "contacts"]
        .iter()
        .map(|name| ((*name).to_owned(), true))
        .collect()
}

fn gateway_with_contacts(home: &Path) -> State<Gateway> {
    let mut state = State::<Gateway>::open(home).expect("state");
    state
        .source_state(
            Source::Contacts,
            "ready",
            Coverage {
                pack_version: String::new(),
                operations: Vec::new(),
                fields: "full".into(),
                missing: None,
            },
            Some(&[]),
            "",
        )
        .expect("source state");
    state
}

fn stored_rows(home: &Path) -> i64 {
    Connection::open(home.join("state.db"))
        .expect("database")
        .query_row(
            "SELECT count(*) FROM records WHERE id=?",
            [REMOVED_ID],
            |row| row.get(0),
        )
        .expect("count")
}

#[test]
fn a_removed_source_row_does_not_stop_a_snapshot_and_stays_on_disk() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    plant_record(
        home.path(),
        "photos",
        "system.source",
        &removed_row("photos", "system.source"),
    );
    let snapshot = state
        .freeze_snapshot(&visibility(), 0)
        .expect("the snapshot is served");
    assert_eq!(snapshot.count, 1, "only the known source is served");
    assert_eq!(stored_rows(home.path()), 1, "nothing is deleted");
}

#[test]
fn a_collection_from_a_removed_source_does_not_stop_a_snapshot() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    plant_record(
        home.path(),
        "photos",
        "photos.asset",
        &removed_row("photos", "photos.asset"),
    );
    assert_eq!(
        state
            .freeze_snapshot(&visibility(), 0)
            .expect("the snapshot is served")
            .count,
        1
    );
    assert_eq!(stored_rows(home.path()), 1, "nothing is deleted");
}

#[test]
fn a_removed_source_change_does_not_stop_the_journal() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    let head = state.head().expect("head");
    let database = Connection::open(home.path().join("state.db")).expect("database");
    let sequence = head.sequence + 1;
    let change = format!(
        r#"{{"sequence":{sequence},"op":"upsert","record":{}}}"#,
        removed_row("photos", "system.source")
    );
    database
        .execute(
            "INSERT INTO journal(seq,epoch,source,payload,committed_at) VALUES(?,?,'photos',?,0)",
            params![
                i64::try_from(sequence).expect("sequence"),
                head.epoch,
                change.as_bytes()
            ],
        )
        .expect("plant journal");
    let advanced = Cursor {
        sequence,
        ..head.clone()
    };
    database
        .execute(
            "UPDATE meta SET value=? WHERE key='cursor'",
            [serde_json::to_string(&advanced).expect("cursor")],
        )
        .expect("advance");
    drop(database);
    let cursor = head;
    state = reopened(state, home.path());
    let mut changes = state.changes(visibility(), cursor).expect("changes");
    let batch = state
        .next_changes(&mut changes)
        .expect("the feed keeps going")
        .expect("a batch");
    assert_eq!(batch.through, sequence, "the cursor moves past the row");
    assert!(batch.changes.is_some_and(|changes| changes.is_empty()));
}

fn reopened(state: State<Gateway>, home: &Path) -> State<Gateway> {
    drop(state);
    State::<Gateway>::open(home).expect("reopen")
}

#[test]
fn reads_and_a_restart_skip_a_removed_source_row() {
    let home = tempfile::tempdir().expect("home");
    let state = gateway_with_contacts(home.path());
    plant_record(
        home.path(),
        "photos",
        "system.source",
        &removed_row("photos", "system.source"),
    );
    let state = reopened(state, home.path());
    let page = state
        .read(&visibility(), &ReadQuery::default())
        .expect("query");
    assert_eq!(page.records.len(), 1);
    let sources = state
        .read_model(&visibility(), "sources.status", &json!({}))
        .expect("sources");
    assert!(!sources.to_string().contains("photos"), "{sources}");
}

#[test]
fn a_damaged_row_from_a_known_source_is_still_an_error() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    let mut damaged: serde_json::Value =
        serde_json::from_str(&removed_row("contacts", "system.source")).expect("row");
    damaged["version"] = json!("not a number");
    plant_record(
        home.path(),
        "contacts",
        "system.source",
        &damaged.to_string(),
    );
    assert!(state.freeze_snapshot(&visibility(), 0).is_err());
}
