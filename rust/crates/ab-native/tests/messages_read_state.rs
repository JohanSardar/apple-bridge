use ab_native::messages::Reader;
use rusqlite::Connection;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn chat(reader: &mut Reader, first: bool) -> Fallible<Option<Value>> {
    let mut found = None;
    for _ in 0..4 {
        for batch in reader.step(first)? {
            for observation in batch.observations {
                if observation.collection == "messages.chat" && observation.kind == "upsert" {
                    found = Some(observation.data);
                }
            }
        }
    }
    Ok(found)
}

#[test]
fn an_incoming_row_messages_counts_as_read_never_shows_as_unread() -> Fallible<()> {
    let root = std::env::temp_dir().join(format!(
        "ab-native-read-at-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&root)?;
    let connection = Connection::open(root.join("chat.db"))?;
    connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))?;
    connection.execute_batch(
        "CREATE TABLE message(ROWID INTEGER PRIMARY KEY,guid TEXT,date INTEGER,text TEXT,handle_id INTEGER,is_from_me INTEGER,is_read INTEGER,date_read INTEGER,associated_message_guid TEXT,associated_message_type INTEGER);
         CREATE TABLE chat(ROWID INTEGER PRIMARY KEY,guid TEXT,service_name TEXT,display_name TEXT);
         CREATE TABLE handle(ROWID INTEGER PRIMARY KEY,id TEXT,service TEXT);
         CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER);
         CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);
         INSERT INTO handle VALUES(1,'+15550100','iMessage');
         INSERT INTO chat VALUES(1,'iMessage;-;+15550100','iMessage','');
         INSERT INTO chat_handle_join VALUES(1,1);
         INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me,is_read,date_read) VALUES
           (1,'read-no-date',700000000000000000,'a',1,0,1,0),
           (2,'read-dated',700000000000000100,'b',1,0,1,700000500000000000),
           (3,'unread',700000000000000200,'c',1,0,0,0),
           (4,'mine',700000000000000300,'d',1,1,1,0);
         INSERT INTO chat_message_join VALUES(1,1),(1,2),(1,3),(1,4);",
    )?;
    let mut reader = Reader::open(&root.join("chat.db"), &root)?;
    let mut found = std::collections::BTreeMap::new();
    for step in 0..4 {
        for batch in reader.step(step == 0)? {
            for observation in batch.observations {
                if observation.collection == "messages.message" && observation.kind == "upsert" {
                    found.insert(observation.native_key.clone(), observation.data);
                }
            }
        }
    }
    let read_at = |key: &str| found[key].get("read_at").cloned();
    assert_eq!(
        read_at("read-no-date"),
        found["read-no-date"].get("sent_at").cloned(),
        "read with no recorded time counts from when it arrived"
    );
    assert!(read_at("read-no-date").is_some());
    assert_ne!(
        read_at("read-dated"),
        found["read-dated"].get("sent_at").cloned()
    );
    assert_eq!(read_at("unread"), None);
    assert_eq!(read_at("mine"), None, "my own row is not read by me");
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn a_chat_marked_unread_reaches_the_record_and_clears_when_read() -> Fallible<()> {
    let root = std::env::temp_dir().join(format!(
        "ab-native-read-state-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&root)?;
    let connection = Connection::open(root.join("chat.db"))?;
    connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))?;
    connection.execute_batch(
        "CREATE TABLE message(ROWID INTEGER PRIMARY KEY,guid TEXT,date INTEGER,text TEXT,handle_id INTEGER,is_from_me INTEGER,is_read INTEGER,date_read INTEGER,associated_message_guid TEXT,associated_message_type INTEGER);
         CREATE TABLE chat(ROWID INTEGER PRIMARY KEY,guid TEXT,service_name TEXT,display_name TEXT,last_read_message_timestamp INTEGER);
         CREATE TABLE handle(ROWID INTEGER PRIMARY KEY,id TEXT,service TEXT);
         CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER);
         CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);
         INSERT INTO handle VALUES(1,'+15550100','iMessage');
         INSERT INTO chat VALUES(1,'iMessage;-;+15550100','iMessage','',700000000000000500);
         INSERT INTO chat_handle_join VALUES(1,1);
         INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me,is_read,date_read) VALUES(1,'old',700000000000000000,'hello',1,0,1,700000000000000500);
         INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me,is_read,date_read) VALUES(2,'new',700000000000000600,'never opened',1,0,0,0);
         INSERT INTO chat_message_join VALUES(1,1);",
    )?;
    let mut reader = Reader::open(&root.join("chat.db"), &root)?;
    let first = chat(&mut reader, true)?.ok_or("no chat record")?;
    assert_eq!(
        first.get("is_unread"),
        None,
        "a read chat carries no marker"
    );

    connection.execute("UPDATE message SET is_read=0 WHERE guid='old'", [])?;
    let marked = chat(&mut reader, false)?.ok_or("no chat update")?;
    assert_eq!(
        marked["is_unread"], true,
        "Mark as Unread keeps date_read and clears is_read"
    );

    connection.execute("UPDATE message SET is_read=1 WHERE guid='old'", [])?;
    let cleared = chat(&mut reader, false)?.ok_or("no chat update")?;
    assert_eq!(cleared.get("is_unread"), None);

    connection.execute_batch("INSERT INTO chat_message_join VALUES(1,2);")?;
    let natural = chat(&mut reader, false)?;
    assert_eq!(
        natural.and_then(|data| data.get("is_unread").cloned()),
        None,
        "a new message nobody opened counts through read_at, not the marker"
    );
    std::fs::remove_dir_all(root)?;
    Ok(())
}
