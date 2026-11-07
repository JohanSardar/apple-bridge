#![allow(clippy::expect_used)]
use ab_domain::{Coverage, Source};
use ab_state::{Cursor, Gateway, ReadQuery, State, Visibility};
use ab_wire::messages::Capture;
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
                permissions: std::collections::BTreeMap::new(),
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
fn legacy_photos_collections_are_not_served_to_photos_principals_after_restart() {
    for collection in ["photos.asset", "photos.album"] {
        let home = tempfile::tempdir().expect("home");
        let mut state = gateway_with_contacts(home.path());
        plant_record(
            home.path(),
            "photos",
            collection,
            &removed_row("photos", collection),
        );
        let mut visible = visibility();
        visible.insert("photos".into(), true);
        assert_eq!(
            state.freeze_snapshot(&visible, 0).expect("snapshot").count,
            1
        );
        let state = reopened(state, home.path());
        assert_eq!(
            state
                .read(&visible, &ReadQuery::default())
                .expect("read")
                .records
                .len(),
            1
        );
        assert_eq!(
            stored_rows(home.path()),
            1,
            "legacy data is retained, never migrated or deleted"
        );
    }
}

#[test]
fn legacy_photos_changes_advance_without_exporting_metadata() {
    let home = tempfile::tempdir().expect("home");
    let state = gateway_with_contacts(home.path());
    let head = state.head().expect("head");
    let sequence = head.sequence + 1;
    let database = Connection::open(home.path().join("state.db")).expect("database");
    let change = format!(
        r#"{{"sequence":{sequence},"op":"upsert","record":{}}}"#,
        removed_row("photos", "photos.asset")
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
        .expect("journal");
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
    let state = reopened(state, home.path());
    let mut visible = visibility();
    visible.insert("photos".into(), true);
    let mut changes = state.changes(visible, head).expect("changes");
    let batch = state
        .next_changes(&mut changes)
        .expect("feed")
        .expect("batch");
    assert_eq!(batch.through, sequence);
    assert!(batch.changes.is_some_and(|changes| changes.is_empty()));
}

#[test]
fn photos_live_readiness_is_served_but_damaged_known_records_still_fail() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    state
        .source_state(
            Source::Photos,
            "ready",
            Coverage {
                fields: "live_only".into(),
                permissions: std::collections::BTreeMap::new(),
                pack_version: String::new(),
                operations: Vec::new(),
                missing: None,
            },
            Some(&[]),
            "",
        )
        .expect("readiness");
    let mut visible = visibility();
    visible.insert("photos".into(), true);
    let page = state.read(&visible, &ReadQuery::default()).expect("read");
    assert!(
        page.records
            .iter()
            .any(|record| record.source == Source::Photos
                && record.collection == ab_domain::Collection::SystemSource)
    );
    let mut damaged: serde_json::Value =
        serde_json::from_str(&removed_row("photos", "system.source")).expect("row");
    damaged["version"] = json!("invalid");
    plant_record(home.path(), "photos", "system.source", &damaged.to_string());
    assert!(state.freeze_snapshot(&visible, 0).is_err());
}

#[test]
fn unknown_sources_still_skip_stored_records_with_explicit_visibility() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    plant_record(
        home.path(),
        "retired_fixture",
        "retired_fixture.asset",
        &removed_row("retired_fixture", "retired_fixture.asset"),
    );
    let mut visible = visibility();
    visible.insert("retired_fixture".into(), true);
    assert_eq!(
        state.freeze_snapshot(&visible, 0).expect("snapshot").count,
        1
    );
    assert_eq!(stored_rows(home.path()), 1);
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

fn payload_of(home: &Path, collection: &str) -> (i64, Vec<u8>) {
    Connection::open(home.join("state.db"))
        .expect("database")
        .query_row(
            "SELECT version,payload FROM records WHERE collection=?",
            [collection],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("row")
}

fn mislabel_payload_source(home: &Path, collection: &str, known: &str) -> (i64, Vec<u8>) {
    let (version, payload) = payload_of(home, collection);
    let text = String::from_utf8(payload).expect("text");
    let damaged = text.replace(
        &format!("\"source\":\"{known}\""),
        &format!("\"source\":\"{known}_typo\""),
    );
    assert_ne!(text, damaged);
    Connection::open(home.join("state.db"))
        .expect("database")
        .execute(
            "UPDATE records SET payload=? WHERE collection=?",
            params![damaged.as_bytes(), collection],
        )
        .expect("damage");
    (version, damaged.into_bytes())
}

#[test]
fn source_state_never_overwrites_a_row_it_cannot_read() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    let before = mislabel_payload_source(home.path(), "system.source", "contacts");
    let write = state.source_state(
        Source::Contacts,
        "denied",
        Coverage {
            permissions: std::collections::BTreeMap::new(),
            pack_version: String::new(),
            operations: Vec::new(),
            fields: "full".into(),
            missing: None,
        },
        Some(&[]),
        "changed",
    );
    assert!(write.is_err(), "the write refuses instead of replacing");
    assert_eq!(payload_of(home.path(), "system.source"), before);
}

#[test]
fn a_capture_never_overwrites_a_row_it_cannot_read() {
    let home = tempfile::tempdir().expect("home");
    let mut state = State::<Gateway>::open(home.path()).expect("state");
    let capture = |checkpoint: &str, predecessor: &str, title: &str| -> Capture {
        serde_json::from_value(json!({
            "source": "contacts",
            "generation": "g",
            "predecessor": predecessor,
            "checkpoint": checkpoint,
            "scan_id": checkpoint,
            "scope": ["contacts.container"],
            "scan_started_at": "2026-11-05T00:00:00Z",
            "completion_evidence": "complete",
            "observations": [{
                "kind": "upsert", "collection": "contacts.container", "native_key": "synthetic",
                "observed_at": "2026-11-05T00:00:00Z",
                "data": {"name": title, "type": 1},
            }],
        }))
        .expect("capture")
    };
    state
        .commit_capture(&capture("1", "", "first"))
        .expect("first capture");
    let before = mislabel_payload_source(home.path(), "contacts.container", "contacts");
    assert!(
        state.commit_capture(&capture("2", "1", "second")).is_err(),
        "the capture refuses instead of replacing"
    );
    assert_eq!(payload_of(home.path(), "contacts.container"), before);
}

#[test]
fn a_known_source_with_an_unknown_collection_is_still_an_error() {
    let home = tempfile::tempdir().expect("home");
    let mut state = gateway_with_contacts(home.path());
    plant_record(
        home.path(),
        "contacts",
        "contacts.removed",
        &removed_row("contacts", "contacts.removed"),
    );
    assert!(state.freeze_snapshot(&visibility(), 0).is_err());
}

fn capture_of(
    source: &str,
    checkpoint: &str,
    predecessor: &str,
    row: &serde_json::Value,
) -> Capture {
    serde_json::from_value(json!({
        "source": source,
        "generation": "g",
        "predecessor": predecessor,
        "checkpoint": checkpoint,
        "scan_id": checkpoint,
        "scope": [row["collection"]],
        "scan_started_at": "2026-11-05T00:00:00Z",
        "completion_evidence": "complete",
        "observations": [row],
    }))
    .expect("capture")
}

fn contact(key: &str, phone: &str) -> serde_json::Value {
    json!({
        "kind": "upsert", "collection": "contacts.contact", "native_key": key,
        "observed_at": "2026-11-05T00:00:00Z",
        "data": {"display_name": key, "phones": [{"value": phone}], "emails": []},
    })
}

#[test]
fn an_unreadable_contact_never_relinks_a_handle() {
    let home = tempfile::tempdir().expect("home");
    let mut state = State::<Gateway>::open(home.path()).expect("state");
    state
        .commit_capture(&capture_of(
            "contacts",
            "1",
            "",
            &contact("synthetic-a", "+15550000001"),
        ))
        .expect("contact");
    let handle = json!({
        "kind": "upsert", "collection": "messages.handle", "native_key": "synthetic-handle",
        "observed_at": "2026-11-05T00:00:00Z",
        "data": {"address": "+15550000001", "service": "iMessage"},
    });
    state
        .commit_capture(&capture_of("messages", "1", "", &handle))
        .expect("handle");
    let linked = payload_of(home.path(), "messages.handle");
    assert!(
        String::from_utf8_lossy(&linked.1).contains("contact_id"),
        "the handle starts linked"
    );
    mislabel_payload_source(home.path(), "contacts.contact", "contacts");
    assert!(
        state
            .commit_capture(&capture_of(
                "contacts",
                "2",
                "1",
                &contact("synthetic-b", "+15550000002")
            ))
            .is_err(),
        "the capture refuses instead of relinking from a partial index"
    );
    assert_eq!(payload_of(home.path(), "messages.handle"), linked);
}

#[test]
fn a_collection_read_does_not_hide_an_unreadable_row() {
    let home = tempfile::tempdir().expect("home");
    let mut state = State::<Gateway>::open(home.path()).expect("state");
    state
        .commit_capture(&capture_of(
            "contacts",
            "1",
            "",
            &contact("synthetic-a", "+15550000001"),
        ))
        .expect("contact");
    let contacts: Visibility = [("contacts".to_owned(), true)].into_iter().collect();
    assert!(
        state
            .read_model(&contacts, "contacts.list", &json!({}))
            .is_ok()
    );
    mislabel_payload_source(home.path(), "contacts.contact", "contacts");
    assert!(
        state
            .read_model(&contacts, "contacts.list", &json!({}))
            .is_err()
    );
}
