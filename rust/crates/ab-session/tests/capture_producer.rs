mod support;

use ab_session::{production, production::Options, readers};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn send(socket: &mut UnixStream, value: &Value) -> Fallible<()> {
    let bytes = serde_json::to_vec(value)?;
    socket.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
    socket.write_all(&bytes)?;
    Ok(())
}

fn receive(socket: &mut UnixStream) -> Fallible<(usize, Value)> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let size = usize::try_from(u32::from_be_bytes(header))?;
    let mut bytes = vec![0; size];
    socket.read_exact(&mut bytes)?;
    Ok((size + 4, serde_json::from_slice(&bytes)?))
}

fn history(directory: &Path, visits: u32) -> Fallible<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64() - 978_307_200.0;
    let staged = directory.join("History.db.staged");
    let _ = std::fs::remove_file(&staged);
    let connection = rusqlite::Connection::open(&staged)?;
    connection.execute_batch(
        "BEGIN;CREATE TABLE history_items(id INTEGER PRIMARY KEY,url TEXT,visit_count INTEGER);
         CREATE TABLE history_visits(id INTEGER PRIMARY KEY,history_item INTEGER,visit_time REAL,title TEXT);
         INSERT INTO history_items VALUES(1,'https://example.com/',2);",
    )?;
    for visit in 0..visits {
        connection.execute(
            "INSERT INTO history_visits VALUES(?1,1,?2,?3)",
            rusqlite::params![visit + 1, now - f64::from(visit), format!("Visit {visit}")],
        )?;
    }
    connection.execute_batch("COMMIT")?;
    drop(connection);
    std::fs::rename(&staged, directory.join("History.db"))?;
    Ok(())
}

fn expect_silence(socket: &mut UnixStream) -> Fallible<()> {
    socket.set_read_timeout(Some(Duration::from_millis(400)))?;
    let mut byte = [0; 1];
    let result = socket.read(&mut byte);
    socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    assert!(
        matches!(result, Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut)
    );
    Ok(())
}

fn media_fixture(source: &str, directory: &Path, duration: Option<u32>) -> Fallible<()> {
    let (file, schema, insert) = match source {
        "phone" => (
            "CallHistory.storedata",
            "CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT);
             CREATE TABLE IF NOT EXISTS ZCALLRECORD(Z_PK INTEGER PRIMARY KEY,ZADDRESS TEXT,ZNAME TEXT,ZDATE REAL,ZDURATION REAL,ZORIGINATED INTEGER,ZANSWERED INTEGER,ZSERVICE_PROVIDER TEXT,ZCALLTYPE INTEGER);
             DELETE FROM ZCALLRECORD;",
            "INSERT INTO ZCALLRECORD VALUES(1,'synthetic','Fixture',946684800.0,?1,0,1,'com.apple.Telephony',1)",
        ),
        "voicemail" => (
            "FaceTimeMessageStore-local.sqlitedb",
            "CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT);
             CREATE TABLE IF NOT EXISTS ZSTOREDMESSAGE(Z_PK INTEGER PRIMARY KEY,ZRECORDUUID TEXT,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZMAILBOXTYPE INTEGER,ZPROVIDER TEXT,ZMESSAGETYPE INTEGER);
             DELETE FROM ZSTOREDMESSAGE;",
            "INSERT INTO ZSTOREDMESSAGE VALUES(1,'vm-1','Synthetic',946684800.0,?1,0,0,'com.apple.coretelephony',1)",
        ),
        _ => (
            "CloudRecordings.db",
            "CREATE TABLE IF NOT EXISTS Z_METADATA(Z_UUID TEXT);
             CREATE TABLE IF NOT EXISTS ZCLOUDRECORDING(Z_PK INTEGER PRIMARY KEY,ZUNIQUEID TEXT,ZCUSTOMLABEL TEXT,ZDATE REAL,ZDURATION REAL,ZEVICTIONDATE REAL,ZFOLDER INTEGER);
             DELETE FROM ZCLOUDRECORDING;",
            "INSERT INTO ZCLOUDRECORDING VALUES(1,'vmemo-1','Synthetic',946684800.0,?1,NULL,1)",
        ),
    };
    let connection = rusqlite::Connection::open(directory.join(file))?;
    connection.execute_batch("BEGIN IMMEDIATE")?;
    connection.execute_batch(schema)?;
    if connection.query_row("SELECT COUNT(*) FROM Z_METADATA", [], |r| {
        r.get::<_, i64>(0)
    })? == 0
    {
        connection.execute("INSERT INTO Z_METADATA VALUES('lab')", [])?;
    }
    if let Some(duration) = duration {
        connection.execute(insert, [f64::from(duration)])?;
    }
    connection.execute_batch("COMMIT")?;
    Ok(())
}

fn media_source_is_routed(source: &str) -> Fallible<()> {
    let directory = std::env::temp_dir().join(format!(
        "ab-media-{source}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    let staging = temporary(&format!("{source}-staging"))?;
    let sources =
        readers::from_config(&json!({source:{"root":directory,"staging":staging}}).to_string())?;
    let options = Options {
        poll: Duration::from_millis(20),
        ..Options::default()
    };
    let support::Wire {
        mut gateway,
        link: mut helper,
        worker,
        home: _,
    } = support::wire(sources, options, json!({}))?;
    expect_silence(&mut gateway)?;

    media_fixture(source, &directory, Some(101))?;
    let (size, first) = receive(&mut gateway)?;
    assert_eq!(first["t"], "Capture");
    assert_eq!(first["source"], source);
    assert_eq!(first["predecessor"], "");
    assert_eq!(first["completion_evidence"], "complete");
    assert!(first.get("coverage").is_none());
    assert_eq!(first["observations"].as_array().map(Vec::len), Some(1));
    assert_eq!(first["observations"][0]["kind"], "upsert");
    let generation = first["generation"].clone();
    send(
        &mut gateway,
        &json!({"t":"Committed","source":source,"checkpoint":first["checkpoint"],"credit":size}),
    )?;
    expect_silence(&mut gateway)?;
    expect_silence(&mut helper)?;

    media_fixture(source, &directory, Some(102))?;
    let (size, second) = receive(&mut gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(second["generation"], generation);
    assert_eq!(second["live"], true);
    assert_eq!(second["observations"][0]["kind"], "upsert");
    send(
        &mut gateway,
        &json!({"t":"Committed","source":source,"checkpoint":second["checkpoint"],"credit":size}),
    )?;

    media_fixture(source, &directory, None)?;
    let (size, third) = receive(&mut gateway)?;
    assert_eq!(third["generation"], generation);
    assert_eq!(third["observations"][0]["kind"], "absent");
    send(
        &mut gateway,
        &json!({"t":"Committed","source":source,"checkpoint":third["checkpoint"],"credit":size}),
    )?;
    expect_silence(&mut gateway)?;

    drop(gateway);
    drop(helper);
    let _ = worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn a_voicemail_record_uuid_stored_as_a_blob_is_a_usable_key() -> Fallible<()> {
    let directory = temporary("voicemail-blob")?;
    let connection =
        rusqlite::Connection::open(directory.join("FaceTimeMessageStore-local.sqlitedb"))?;
    connection.execute_batch(
        "CREATE TABLE Z_METADATA(Z_UUID TEXT);
         INSERT INTO Z_METADATA VALUES('lab');
         CREATE TABLE ZSTOREDMESSAGE(Z_PK INTEGER PRIMARY KEY,ZRECORDUUID BLOB,ZFROM TEXT,ZDATECREATED REAL,ZDURATION REAL,ZISREAD INTEGER,ZMAILBOXTYPE INTEGER,ZPROVIDER TEXT,ZMESSAGETYPE INTEGER);
         INSERT INTO ZSTOREDMESSAGE VALUES(1,x'00AB',  'Synthetic',946684800,12.0,0,0,'com.apple.coretelephony',0);",
    )?;
    drop(connection);
    let sources = readers::from_config(&json!({"voicemail":{"root":directory}}).to_string())?;
    let mut session = wire(
        sources,
        Options {
            poll: Duration::from_millis(20),
            ..Options::default()
        },
        json!({}),
    )?;
    let (size, capture) = receive(&mut session.gateway)?;
    assert_eq!(capture["observations"][0]["native_key"], "00ab");
    commit(&mut session, "voicemail", &capture, size)?;
    session.finish()?;
    Ok(())
}

#[test]
fn rust_calls_voicemail_and_voice_memos_are_routed() -> Fallible<()> {
    for source in ["phone", "voicemail", "voice_memos"] {
        media_source_is_routed(source)?;
    }
    Ok(())
}

fn temporary(name: &str) -> Fallible<std::path::PathBuf> {
    let directory = std::env::temp_dir().join(format!(
        "ab-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

use support::{Wire, wire};

fn commit(wire: &mut Wire, source: &str, capture: &Value, size: usize) -> Fallible<()> {
    send(
        &mut wire.gateway,
        &json!({"t":"Committed","source":source,"checkpoint":capture["checkpoint"],"credit":size}),
    )
}

use support::polling;

fn voice_memo_fixture(directory: &Path, audio: &[u8]) -> Fallible<()> {
    let connection = rusqlite::Connection::open(directory.join("CloudRecordings.db"))?;
    connection.execute_batch(
        "CREATE TABLE Z_METADATA(Z_UUID TEXT);
         INSERT INTO Z_METADATA VALUES('synthetic-uuid');
         CREATE TABLE ZCLOUDRECORDING(Z_PK INTEGER PRIMARY KEY,ZUNIQUEID TEXT,ZCUSTOMLABEL TEXT,ZDATE REAL,ZDURATION REAL,ZEVICTIONDATE REAL,ZFOLDER INTEGER,ZPATH TEXT);
         INSERT INTO ZCLOUDRECORDING VALUES(1,'rec-1','Synthetic memo',700000000.0,3.5,NULL,NULL,'memo.m4a');",
    )?;
    std::fs::write(directory.join("memo.m4a"), audio)?;
    Ok(())
}

#[test]
fn rust_voice_memos_serve_blob_reads_across_restart() -> Fallible<()> {
    let directory = temporary("voice")?;
    let audio: Vec<u8> = (0..200_000_u32)
        .map(|n| u8::try_from(n % 251).unwrap_or(0))
        .collect();
    voice_memo_fixture(&directory, &audio)?;
    let staging = temporary("voice-staging")?;
    let config = json!({"voice_memos":{"root":directory,"staging":staging}}).to_string();
    let build = || readers::from_config(&config);

    let mut first = wire(build()?, polling(None), json!({}))?;
    let (size, capture) = receive(&mut first.gateway)?;
    assert_eq!(capture["source"], "voice_memos");
    let handles = capture["blob_handles"].as_object().ok_or("blob_handles")?;
    assert_eq!(handles.len(), 1);
    let (digest, handle) = handles.iter().next().ok_or("handle")?;
    let handle = handle.as_str().ok_or("handle text")?.to_owned();
    let digest = digest.clone();
    commit(&mut first, "voice_memos", &capture, size)?;
    let mut bytes = Vec::new();
    let mut offset = 0_usize;
    loop {
        send(
            &mut first.gateway,
            &json!({"t":"BlobRead","handle":handle,"offset":offset,"length":131_072}),
        )?;
        let (_, chunk) = receive(&mut first.gateway)?;
        assert_eq!(chunk["t"], "BlobChunk");
        assert_eq!(chunk["digest"], digest);
        let part = ab_native_decode(chunk["bytes"].as_str().ok_or("bytes")?)?;
        offset += part.len();
        bytes.extend(part);
        if chunk["eof"] == true {
            break;
        }
    }
    assert_eq!(bytes, audio);
    send(
        &mut first.gateway,
        &json!({"t":"BlobRead","handle":"voice-memo-blob:missing","offset":0,"length":10}),
    )?;
    assert_eq!(receive(&mut first.gateway)?.1["t"], "BlobUnavailable");
    drop(first.gateway);
    drop(first.link);
    let _ = first.worker.join();

    let mut second = wire(
        build()?,
        polling(None),
        json!({"voice_memos":{"generation":capture["generation"],"checkpoint":capture["checkpoint"]}}),
    )?;
    send(
        &mut second.gateway,
        &json!({"t":"BlobRead","handle":handle,"offset":0,"length":16}),
    )?;
    let (_, again) = loop {
        let frame = receive(&mut second.gateway)?;
        if frame.1["t"] == "BlobChunk" {
            break frame;
        }
    };
    assert_eq!(again["digest"], digest);
    drop(second.gateway);
    drop(second.link);
    let _ = second.worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

fn ab_native_decode(text: &str) -> Fallible<Vec<u8>> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in text.bytes().filter(|b| *b != b'=') {
        let value = table.iter().position(|t| *t == byte).ok_or("base64")?;
        buffer = (buffer << 6) | u32::try_from(value)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff)?);
        }
    }
    Ok(out)
}

#[test]
fn watcher_wakes_the_producer_long_before_the_poll_interval() -> Fallible<()> {
    let directory = temporary("watch")?;
    history(&directory, 2)?;
    let config = json!({"safari":{"root":directory}}).to_string();
    let mut session = wire(
        readers::from_config(&config)?,
        Options {
            poll: Duration::from_secs(120),
            watch: true,
            state: None,
            queries: ab_session::queries::NativeQueries::default(),
            ..Options::default()
        },
        json!({}),
    )?;
    let (size, first) = receive(&mut session.gateway)?;
    commit(&mut session, "safari", &first, size)?;
    std::thread::sleep(Duration::from_millis(600));
    let started = std::time::Instant::now();
    history(&directory, 3)?;
    let (_, second) = receive(&mut session.gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(session.gateway);
    drop(session.link);
    let _ = session.worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn production_config_flips_a_source_to_rust_and_back() -> Fallible<()> {
    let home = temporary("home")?;
    let directory = temporary("home-safari")?;
    history(&directory, 2)?;
    assert!(production::load(&home, None)?.is_none());
    std::fs::create_dir_all(home.join("session"))?;
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"safari":{"root":directory}},"watch":false,"poll_ms":20}).to_string(),
    )?;
    assert_eq!(
        production::load(&home, None)?.ok_or("config")?.sources(),
        ["safari"]
    );
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"calendar":{"root":directory,"facts":"native"},"contacts":{"root":directory,"facts":"native"}}})
            .to_string(),
    )?;
    assert_eq!(
        production::load(&home, None)?.ok_or("config")?.sources(),
        ["calendar", "contacts"]
    );
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"maps":{"root":directory}}}).to_string(),
    )?;
    assert!(production::load(&home, None).is_err());

    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"safari":{"root":directory}},"watch":false,"poll_ms":20}).to_string(),
    )?;
    let attached = production::load(&home, None)?
        .ok_or("config")?
        .attached(None)?;
    let mut world = wire(attached.readers, attached.options, json!({}))?;
    assert_eq!(receive(&mut world.gateway)?.1["source"], "safari");
    world.finish()?;
    std::fs::remove_dir_all(home)?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

fn envelope(directory: &Path, rows: &[u32]) -> Fallible<()> {
    let target = directory.join("Envelope Index");
    let staged = directory.join(".staged-envelope");
    let _ = std::fs::remove_file(&staged);
    let connection = rusqlite::Connection::open(&staged)?;
    connection.execute_batch(
        "CREATE TABLE mailboxes(url TEXT);
         CREATE TABLE subjects(subject TEXT);
         CREATE TABLE addresses(address TEXT);
         CREATE TABLE messages(mailbox INTEGER,subject INTEGER,sender INTEGER,date_received REAL,read INTEGER,message_id TEXT);
         INSERT INTO mailboxes(ROWID,url) VALUES(1,'imap://one@example.com/INBOX');
         INSERT INTO subjects(ROWID,subject) VALUES(1,'synthetic subject');
         INSERT INTO addresses(ROWID,address) VALUES(1,'sender@example.com');",
    )?;
    for row in rows {
        connection.execute(
            "INSERT INTO messages(ROWID,mailbox,subject,sender,date_received,read,message_id) VALUES(?1,1,1,1,1700000000,0,?2)",
            rusqlite::params![row, format!("<synthetic-{row}@example.com>")],
        )?;
    }
    drop(connection);
    std::fs::rename(staged, target)?;
    Ok(())
}

fn messages(capture: &Value, kind: &str) -> Vec<String> {
    capture["observations"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| row["collection"] == "mail.message" && row["kind"] == kind)
                .filter_map(|row| row["native_key"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn rust_mail_scans_fixture_and_never_invents_deletions() -> Fallible<()> {
    let directory = std::env::temp_dir().join(format!(
        "ab-capture-mail-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    envelope(&directory, &[1, 2, 3])?;
    let sources = readers::from_config(&json!({"mail":{"root":directory}}).to_string())?;
    let options = Options {
        poll: Duration::from_millis(20),
        ..Options::default()
    };
    let support::Wire {
        mut gateway,
        link: helper,
        worker,
        home: _,
    } = support::wire(sources, options, json!({}))?;

    let (size, first) = receive(&mut gateway)?;
    assert_eq!(first["source"], "mail");
    assert_eq!(first["completion_evidence"], "complete");
    assert_eq!(first["live"], false);
    assert!(first.get("coverage").is_none());
    assert_eq!(messages(&first, "upsert"), ["1", "2", "3"]);
    assert_eq!(messages(&first, "absent"), Vec::<String>::new());
    let generation = first["generation"].clone();
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"mail","checkpoint":first["checkpoint"],"credit":size}),
    )?;
    expect_silence(&mut gateway)?;

    envelope(&directory, &[1, 3])?;
    let (size, rebuilt) = receive(&mut gateway)?;
    assert_ne!(rebuilt["generation"], generation);
    assert_eq!(messages(&rebuilt, "absent"), Vec::<String>::new());
    assert_eq!(messages(&rebuilt, "upsert"), ["1", "3"]);
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"mail","checkpoint":rebuilt["checkpoint"],"credit":size}),
    )?;

    std::fs::write(directory.join("Envelope Index"), b"not a database")?;
    expect_silence(&mut gateway)?;

    drop(gateway);
    drop(helper);
    let _ = worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn rust_mail_splits_large_scan_into_committed_chunks() -> Fallible<()> {
    let directory = std::env::temp_dir().join(format!(
        "ab-capture-mail-chunks-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    let rows: Vec<u32> = (1..=1000).collect();
    envelope(&directory, &rows)?;
    let sources = readers::from_config(&json!({"mail":{"root":directory}}).to_string())?;
    let options = Options {
        poll: Duration::from_millis(20),
        ..Options::default()
    };
    let support::Wire {
        mut gateway,
        link: helper,
        worker,
        home: _,
    } = support::wire(sources, options, json!({}))?;

    let mut keys = Vec::new();
    let mut checkpoints = Vec::new();
    let mut scans = std::collections::BTreeSet::new();
    let mut predecessor = String::new();
    loop {
        let (size, frame) = receive(&mut gateway)?;
        assert_eq!(frame["t"], "Capture");
        assert!(size <= 262_144);
        assert_eq!(frame["predecessor"], predecessor);
        scans.insert(frame["scan_id"].as_str().unwrap_or_default().to_owned());
        keys.extend(messages(&frame, "upsert"));
        let checkpoint = frame["checkpoint"].as_str().unwrap_or_default().to_owned();
        assert_ne!(checkpoint, "");
        send(
            &mut gateway,
            &json!({"t":"Committed","source":"mail","checkpoint":checkpoint,"credit":size}),
        )?;
        predecessor.clone_from(&checkpoint);
        checkpoints.push(checkpoint);
        if keys.len() == 1000 {
            break;
        }
    }
    assert!(checkpoints.len() > 1);
    assert_eq!(scans.len(), 1);
    assert_eq!(
        checkpoints
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        checkpoints.len()
    );
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 1000);
    expect_silence(&mut gateway)?;

    drop(gateway);
    drop(helper);
    let _ = worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn concurrent_checkpoint_saves_never_race_on_the_temporary_file() -> Fallible<()> {
    let directory = std::env::temp_dir().join(format!(
        "ab-session-store-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let store = ab_session::store::Store::open(&directory)?;
    let record = |round: usize| ab_session::store::Record {
        generation: "g".into(),
        checkpoint: format!("c{round}"),
        token: "t".into(),
        native: String::new(),
    };
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| -> Result<(), ab_session::Error> {
                    for round in 0..200 {
                        store.save("notes", &record(round))?;
                    }
                    Ok(())
                })
            })
            .collect();
        for worker in workers {
            worker.join().map_err(|_| "worker panicked")??;
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    assert!(store.load("notes").is_some());
    std::fs::remove_dir_all(&directory)?;
    Ok(())
}

fn messages_fixture(
    root: &Path,
    texts: &[&str],
    attachment: Option<&[u8]>,
) -> Fallible<rusqlite::Connection> {
    let attachments = root.join("Attachments");
    std::fs::create_dir_all(&attachments)?;
    let connection = rusqlite::Connection::open(root.join("chat.db"))?;
    connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))?;
    connection.execute_batch(
        "CREATE TABLE message(ROWID INTEGER PRIMARY KEY,guid TEXT,date INTEGER,text TEXT,handle_id INTEGER,is_from_me INTEGER,associated_message_guid TEXT,associated_message_type INTEGER);
         CREATE TABLE chat(ROWID INTEGER PRIMARY KEY,guid TEXT,service_name TEXT,display_name TEXT);
         CREATE TABLE handle(ROWID INTEGER PRIMARY KEY,id TEXT,service TEXT);
         CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER);
         CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);
         CREATE TABLE attachment(ROWID INTEGER PRIMARY KEY,guid TEXT,filename TEXT,mime_type TEXT,uti TEXT,total_bytes INTEGER,transfer_state INTEGER);
         CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER);
         INSERT INTO handle VALUES(1,'+15550100','iMessage');
         INSERT INTO chat VALUES(1,'iMessage;-;+15550100','iMessage','');
         INSERT INTO chat_handle_join VALUES(1,1);",
    )?;
    for (index, text) in texts.iter().enumerate() {
        let row = i64::try_from(index)? + 1;
        connection.execute(
            "INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me) VALUES(?1,?2,?3,?4,1,0)",
            rusqlite::params![row, format!("msg-{row}"), 700_000_000_000_000_000_i64 + row, text],
        )?;
        connection.execute("INSERT INTO chat_message_join VALUES(1,?1)", [row])?;
    }
    if let Some(bytes) = attachment {
        let directory = attachments.join("aa");
        std::fs::create_dir_all(&directory)?;
        std::fs::write(directory.join("photo.bin"), bytes)?;
        connection.execute(
            "INSERT INTO attachment VALUES(1,'att-1',?1,'image/png','public.png',?2,5)",
            rusqlite::params![
                format!("{}/photo.bin", directory.display()),
                i64::try_from(bytes.len())?
            ],
        )?;
        connection.execute("INSERT INTO message_attachment_join VALUES(1,1)", [])?;
    }
    Ok(connection)
}

#[test]
#[allow(clippy::too_many_lines)]
fn messages_reader_streams_history_then_live_rows_and_serves_attachment_bytes() -> Fallible<()> {
    let home = temporary("messages-home")?;
    let root = home.join("Library/Messages");
    std::fs::create_dir_all(&root)?;
    let bytes: Vec<u8> = (0..300_000_u32)
        .map(|n| u8::try_from(n % 239).unwrap_or(0))
        .collect();
    let database = messages_fixture(&root, &["first", "second"], Some(&bytes))?;
    let config = json!({"messages":{"root":root}}).to_string();
    let mut session = wire(readers::from_config(&config)?, polling(None), json!({}))?;

    let mut seen = std::collections::BTreeSet::new();
    let mut handles = serde_json::Map::new();
    let mut complete = false;
    let mut attachment = Value::Null;
    for _ in 0..8 {
        let (size, frame) = receive(&mut session.gateway)?;
        assert_eq!(frame["source"], "messages");
        assert_eq!(frame["scope"].as_array().map(Vec::len), Some(5));
        for row in frame["observations"].as_array().ok_or("rows")? {
            if row["native_key"] == "att-1" && row["kind"] == "upsert" {
                attachment = row.clone();
            }
            if row["kind"] == "upsert" {
                seen.insert(format!(
                    "{}/{}",
                    row["collection"].as_str().unwrap_or(""),
                    row["native_key"].as_str().unwrap_or("")
                ));
            }
        }
        if let Some(found) = frame["blob_handles"].as_object() {
            handles.extend(found.clone());
        }
        complete |= frame["completion_evidence"] == "complete";
        commit(&mut session, "messages", &frame, size)?;
        if complete {
            break;
        }
    }
    assert!(complete, "history scan never completed");
    for key in [
        "messages.message/msg-1",
        "messages.message/msg-2",
        "messages.chat/iMessage;-;+15550100",
        "messages.attachment/att-1",
    ] {
        assert!(seen.contains(key), "missing {key}: {seen:?}");
    }
    assert_eq!(handles.len(), 1, "{attachment}");
    let (digest, handle) = handles.iter().next().ok_or("handle")?;
    assert_eq!(handle, "att-1");
    assert_eq!(
        attachment["data"]["blob"]["digest"], *digest,
        "{attachment}"
    );
    assert!(attachment["data"]["blob"].get("handle").is_none());

    let mut read = Vec::new();
    let mut offset = 0_usize;
    loop {
        send(
            &mut session.gateway,
            &if offset == 0 {
                json!({"t":"BlobRead","handle":"att-1","length":131_072})
            } else {
                json!({"t":"BlobRead","handle":"att-1","offset":offset,"length":131_072})
            },
        )?;
        let (_, chunk) = loop {
            let frame = receive(&mut session.gateway)?;
            if frame.1["t"] == "BlobChunk" {
                break frame;
            }
        };
        assert_eq!(chunk["digest"], *digest);
        let part = ab_native_decode(chunk["bytes"].as_str().ok_or("bytes")?)?;
        offset += part.len();
        read.extend(part);
        if chunk["eof"] == true {
            break;
        }
    }
    assert_eq!(read, bytes);
    send(
        &mut session.gateway,
        &json!({"t":"BlobRead","handle":"unknown","offset":0,"length":10}),
    )?;

    expect_silence(&mut session.gateway)?;
    database.execute_batch(
        "BEGIN;
         INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me) VALUES(3,'msg-3',700000000000000003,'third',1,0);
         INSERT INTO chat_message_join VALUES(1,3);
         COMMIT;",
    )?;
    let (size, live) = receive(&mut session.gateway)?;
    assert_eq!(live["live"], true);
    let keys: Vec<&str> = live["observations"]
        .as_array()
        .ok_or("rows")?
        .iter()
        .filter(|row| row["kind"] == "upsert")
        .map(|row| row["native_key"].as_str().unwrap_or_default())
        .collect();
    assert!(keys.contains(&"msg-3"), "{keys:?}");
    assert!(!keys.contains(&"msg-1"), "{keys:?}");
    commit(&mut session, "messages", &live, size)?;

    drop(session.gateway);
    drop(session.link);
    let _ = session.worker.join();
    drop(database);
    std::fs::remove_dir_all(home)?;
    Ok(())
}

fn grow_messages(connection: &rusqlite::Connection, to: i64) -> Fallible<()> {
    connection.execute_batch("BEGIN")?;
    for row in 3..=to {
        connection.execute(
            "INSERT INTO message(ROWID,guid,date,text,handle_id,is_from_me) VALUES(?1,?2,?3,?4,1,0)",
            rusqlite::params![row, format!("msg-{row}"), 700_000_000_000_000_000_i64 + row, format!("text {row}")],
        )?;
        connection.execute("INSERT INTO chat_message_join VALUES(1,?1)", [row])?;
    }
    connection.execute_batch("COMMIT")?;
    Ok(())
}

fn upserted(frame: &Value) -> Vec<i64> {
    frame["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == "upsert")
        .filter_map(|row| {
            row["native_key"]
                .as_str()?
                .strip_prefix("msg-")?
                .parse()
                .ok()
        })
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn messages_history_commits_in_single_frame_pages_and_resumes_after_a_reset() -> Fallible<()> {
    let home = temporary("messages-resume-home")?;
    let state = temporary("messages-resume-state")?;
    let root = home.join("Library/Messages");
    std::fs::create_dir_all(&root)?;
    let database = messages_fixture(&root, &["first", "second"], None)?;
    grow_messages(&database, 3000)?;
    let config = json!({"messages":{"root":root}}).to_string();

    let mut first = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({}),
    )?;
    let mut history_frames = 0;
    let mut last = Value::Null;
    let mut committed_history: Vec<i64> = Vec::new();
    while history_frames < 14 {
        let (size, frame) = receive(&mut first.gateway)?;
        assert_eq!(frame["completion_evidence"], "", "scan finished too early");
        let checkpoint = frame["checkpoint"].as_str().ok_or("checkpoint")?;
        assert_eq!(
            checkpoint.matches('-').count(),
            2,
            "page split: {checkpoint}"
        );
        assert!(size < 70_000, "frame of {size} bytes");
        if frame["live"] == false {
            history_frames += 1;
            committed_history.extend(upserted(&frame));
        }
        commit(&mut first, "messages", &frame, size)?;
        last = frame;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let saved = loop {
        let saved: Option<Value> = std::fs::read(state.join("messages.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        if let Some(saved) = saved
            && saved["checkpoint"] == last["checkpoint"]
        {
            break saved;
        }
        assert!(std::time::Instant::now() < deadline, "progress never saved");
        std::thread::sleep(Duration::from_millis(20));
    };
    let progress: Value = serde_json::from_str(saved["token"].as_str().ok_or("token")?)?;
    let cursor = progress["cursor"].as_i64().ok_or("cursor")?;
    assert!(cursor >= 128, "cursor {cursor}");
    drop(first.gateway);
    drop(first.link);
    let _ = first.worker.join();

    let mut second = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({"messages":{"generation":last["generation"],"checkpoint":last["checkpoint"]}}),
    )?;
    let mut replayed: Vec<i64> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut complete = false;
    for _ in 0..400 {
        let (size, frame) = receive(&mut second.gateway)?;
        if frame["live"] == false {
            replayed.extend(upserted(&frame));
        }
        seen.extend(upserted(&frame));
        complete |= frame["completion_evidence"] == "complete";
        commit(&mut second, "messages", &frame, size)?;
        if complete {
            break;
        }
    }
    assert!(complete, "resumed scan never completed");
    assert!(
        replayed.iter().all(|row| *row > cursor),
        "resume restarted below {cursor}"
    );
    for row in 1..=3000 {
        assert!(
            seen.contains(&row) || committed_history.contains(&row) || row <= cursor,
            "msg-{row} lost"
        );
    }
    expect_silence(&mut second.gateway)?;
    drop(second.gateway);
    drop(second.link);
    let _ = second.worker.join();
    drop(database);
    std::fs::remove_dir_all(home)?;
    std::fs::remove_dir_all(state)?;
    Ok(())
}

#[derive(Debug)]
struct Slow(bool);

impl readers::CaptureReader for Slow {
    fn source(&self) -> &'static str {
        "safari"
    }

    fn root(&self) -> &Path {
        Path::new("/")
    }

    fn token(&self) -> Option<String> {
        None
    }

    fn resume(&mut self, _token: &str) -> bool {
        false
    }

    fn next(
        &mut self,
        _context: &readers::ScanContext,
    ) -> Result<Option<Value>, ab_session::Error> {
        if !self.0 {
            self.0 = true;
            std::thread::sleep(Duration::from_secs(3));
        }
        Ok(None)
    }

    fn commit(&mut self) -> Result<(), ab_session::Error> {
        Ok(())
    }
}

#[test]
fn a_busy_reader_never_delays_blob_reads_for_other_sources() -> Fallible<()> {
    let mut sources: std::collections::BTreeMap<String, Box<dyn readers::CaptureReader>> =
        std::collections::BTreeMap::new();
    sources.insert("safari".into(), Box::new(Slow(false)));
    let mut session = wire(sources, polling(None), json!({}))?;
    std::thread::sleep(Duration::from_millis(200));
    let started = std::time::Instant::now();
    send(
        &mut session.gateway,
        &json!({"t":"BlobRead","handle":"swift-notes-handle","offset":0,"length":10}),
    )?;
    let (_, forwarded) = receive(&mut session.link)?;
    assert_eq!(forwarded["t"], "BlobRead");
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "blob read waited {:?} behind a busy reader",
        started.elapsed()
    );
    drop(session.gateway);
    drop(session.link);
    let _ = session.worker.join();
    Ok(())
}
