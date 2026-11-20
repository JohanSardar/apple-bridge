use ab_native::messages::{Batch, Reader};
use rusqlite::Connection;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn directory(name: &str) -> Fallible<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "ab-native-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

fn fixture(root: &Path, rows: i64) -> Fallible<Connection> {
    let connection = Connection::open(root.join("chat.db"))?;
    connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))?;
    connection.execute_batch(
        "CREATE TABLE message(ROWID INTEGER PRIMARY KEY,guid TEXT,date INTEGER,text TEXT,handle_id INTEGER,is_from_me INTEGER,associated_message_guid TEXT,associated_message_type INTEGER);
         CREATE TABLE chat(ROWID INTEGER PRIMARY KEY,guid TEXT,service_name TEXT,display_name TEXT);
         CREATE TABLE handle(ROWID INTEGER PRIMARY KEY,id TEXT,service TEXT);
         CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER);
         CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);
         INSERT INTO handle VALUES(1,'+15550100','iMessage');
         INSERT INTO chat VALUES(1,'iMessage;-;+15550100','iMessage','');
         INSERT INTO chat_handle_join VALUES(1,1);",
    )?;
    insert(&connection, 1, rows)?;
    Ok(connection)
}

fn insert(connection: &Connection, from: i64, to: i64) -> Fallible<()> {
    connection.execute_batch("BEGIN")?;
    for row in from..=to {
        connection.execute(
            "INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me) VALUES(?1,?2,?3,?4,1,0)",
            rusqlite::params![
                row,
                format!("msg-{row}"),
                700_000_000_000_000_000_i64 + row,
                format!("text {row}")
            ],
        )?;
        connection.execute("INSERT INTO chat_message_join VALUES(1,?1)", [row])?;
    }
    connection.execute_batch("COMMIT")?;
    Ok(())
}

fn number(key: &str) -> Option<i64> {
    key.strip_prefix("msg-")?.parse().ok()
}

fn history_keys(batches: &[Batch]) -> Vec<i64> {
    batches
        .iter()
        .filter(|batch| !batch.live)
        .flat_map(|batch| &batch.observations)
        .filter(|observation| observation.kind == "upsert")
        .filter_map(|observation| number(&observation.native_key))
        .collect()
}

#[test]
fn history_scan_survives_data_changes_between_pages() -> Fallible<()> {
    let root = directory("pages")?;
    let writer = fixture(&root, 2600)?;
    let mut reader = Reader::open(&root.join("chat.db"), &root)?;
    let mut cursor = 0;
    let mut complete = false;
    let mut emitted = BTreeSet::new();
    let mut next_row = 2601;
    for step in 0..200 {
        let batches = reader.step(step == 0)?;
        for batch in &batches {
            complete |= batch.complete;
            emitted.extend(
                batch
                    .observations
                    .iter()
                    .filter(|o| o.kind == "upsert")
                    .filter_map(|o| number(&o.native_key)),
            );
        }
        if let Some(position) = reader.scan_cursor() {
            assert!(position >= cursor, "scan restarted: {position} < {cursor}");
            cursor = position;
        }
        if step % 3 == 0 {
            insert(&writer, next_row, next_row)?;
            next_row += 1;
        }
        if complete {
            break;
        }
    }
    assert!(complete, "history scan never completed");
    assert!(cursor >= 2000, "cursor {cursor}");
    for row in 1..=2600 {
        assert!(emitted.contains(&row), "msg-{row} missing");
    }
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn history_scan_resumes_from_a_saved_cursor() -> Fallible<()> {
    let root = directory("resume")?;
    let writer = fixture(&root, 2600)?;
    let mut first = Reader::open(&root.join("chat.db"), &root)?;
    let mut saved = 0;
    for step in 0..6 {
        first.step(step == 0)?;
        saved = first.scan_cursor().unwrap_or(saved);
    }
    assert!(saved >= 256, "saved {saved}");
    drop(first);

    let mut second = Reader::open(&root.join("chat.db"), &root)?;
    second.resume_scan(saved)?;
    let mut complete = false;
    let mut history = Vec::new();
    let mut emitted = BTreeSet::new();
    for _ in 0..200 {
        let batches = second.step(false)?;
        history.extend(history_keys(&batches));
        emitted.extend(
            batches
                .iter()
                .flat_map(|batch| &batch.observations)
                .filter_map(|o| number(&o.native_key)),
        );
        complete |= batches.iter().any(|batch| batch.complete);
        if complete {
            break;
        }
    }
    assert!(complete);
    assert!(
        history.iter().all(|row| *row > saved),
        "resumed scan re-read rows at or before {saved}"
    );
    for row in (saved + 1)..=2600 {
        assert!(emitted.contains(&row), "msg-{row} missing after resume");
    }
    drop(writer);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
