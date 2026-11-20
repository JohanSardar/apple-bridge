use ab_session::readers::{CaptureReader, MessagesReader, ScanContext};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::path::PathBuf;

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

const HISTORY: i64 = 600;

fn world(name: &str) -> Fallible<(PathBuf, Connection)> {
    let root = std::env::temp_dir().join(format!(
        "ab-live-first-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir_all(&root)?;
    let db = Connection::open(root.join("chat.db"))?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
        CREATE TABLE message(ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB, is_from_me INTEGER DEFAULT 1, is_read INTEGER DEFAULT 1, is_delivered INTEGER DEFAULT 0, is_finished INTEGER DEFAULT 1, service TEXT DEFAULT 'iMessage', item_type INTEGER DEFAULT 0, associated_message_type INTEGER DEFAULT 0, associated_message_guid TEXT, associated_message_emoji TEXT, date INTEGER DEFAULT 1, date_read INTEGER DEFAULT 0, date_delivered INTEGER DEFAULT 0, date_edited INTEGER DEFAULT 0, date_retracted INTEGER DEFAULT 0, error INTEGER DEFAULT 0, handle_id INTEGER DEFAULT 0, other_handle INTEGER DEFAULT 0);
        CREATE TABLE chat(ROWID INTEGER PRIMARY KEY, guid TEXT, chat_identifier TEXT, service_name TEXT DEFAULT 'iMessage', display_name TEXT, is_archived INTEGER DEFAULT 0, is_filtered INTEGER DEFAULT 0, properties BLOB, last_read_message_timestamp INTEGER DEFAULT 0);
        CREATE TABLE handle(ROWID INTEGER PRIMARY KEY, id TEXT, service TEXT DEFAULT 'iMessage');
        CREATE TABLE chat_handle_join(chat_id INTEGER, handle_id INTEGER);
        CREATE TABLE chat_message_join(chat_id INTEGER, message_id INTEGER);
        CREATE TABLE message_attachment_join(message_id INTEGER, attachment_id INTEGER);
        CREATE TABLE attachment(ROWID INTEGER PRIMARY KEY, guid TEXT, filename TEXT, transfer_name TEXT, mime_type TEXT, uti TEXT);
        CREATE TABLE chat_recoverable_message_join(chat_id INTEGER, message_id INTEGER, delete_date INTEGER);
        CREATE TABLE recoverable_message_part(chat_id INTEGER, message_id INTEGER, part_index INTEGER, delete_date INTEGER, part_text BLOB);
        INSERT INTO chat(ROWID,guid,chat_identifier) VALUES (1,'chat','me@example.com');
        INSERT INTO handle(ROWID,id) VALUES (1,'me@example.com');
        INSERT INTO chat_handle_join VALUES (1,1);",
    )?;
    for row in 1..=HISTORY {
        db.execute(
            "INSERT INTO message(ROWID,guid,text) VALUES (?,?,?)",
            params![row, format!("old-{row}"), "x".repeat(400)],
        )?;
        db.execute("INSERT INTO chat_message_join VALUES (1,?)", params![row])?;
    }
    Ok((root, db))
}

fn context(ordinal: u64) -> ScanContext {
    ScanContext {
        checkpoint: format!("c{ordinal}"),
        scan_id: format!("s{ordinal}"),
        now: 1_800_000_000.0,
        fence: 1,
    }
}

fn keys(piece: &Value) -> Vec<String> {
    piece["observations"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["native_key"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn run(name: &str, waited_on: bool) -> Fallible<(Vec<String>, PathBuf)> {
    let (root, db) = world(name)?;
    let mut reader = MessagesReader::new(root.clone(), root.clone());
    let mut seen = 0_u64;
    let mut first = None;
    while first.is_none() {
        seen += 1;
        let piece = reader.next(&context(seen))?;
        if piece.is_some() {
            reader.commit()?;
        }
        first = piece.filter(|piece| keys(piece).iter().any(|key| key.starts_with("old-")));
        assert!(seen < 500, "history never started");
    }
    db.execute(
        "INSERT INTO message(ROWID,guid,text) VALUES (?,?,?)",
        params![HISTORY + 1, "fresh", "just sent"],
    )?;
    db.execute(
        "INSERT INTO chat_message_join VALUES (1,?)",
        params![HISTORY + 1],
    )?;
    if waited_on {
        assert!(reader.refresh());
    }
    let piece = reader.next(&context(seen + 1))?.ok_or("no piece")?;
    reader.commit()?;
    Ok((keys(&piece), root))
}

#[test]
fn a_new_message_a_command_waits_for_goes_ahead_of_the_history_scan() -> Fallible<()> {
    let (waiting, root) = run("waiting", true)?;
    let _ = std::fs::remove_dir_all(root);
    let fresh = waiting.iter().position(|key| key == "fresh");
    let history = waiting.iter().position(|key| key.starts_with("old-"));
    assert!(
        fresh.is_some() && history.is_none_or(|at| fresh < Some(at)),
        "{waiting:?}"
    );
    Ok(())
}

#[test]
fn with_nobody_waiting_the_history_keeps_its_place() -> Fallible<()> {
    let (idle, root) = run("idle", false)?;
    let _ = std::fs::remove_dir_all(root);
    assert!(!idle.iter().any(|key| key == "fresh"), "{idle:?}");
    Ok(())
}
