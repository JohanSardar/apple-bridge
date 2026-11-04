#![cfg(feature = "rust-capture")]

use ab_session::{production, readers, routed};
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

#[test]
fn rust_safari_replaces_helper_frames_and_consumes_committed() -> Fallible<()> {
    let directory = std::env::temp_dir().join(format!(
        "ab-capture-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    history(&directory, 2)?;
    let sources = readers::from_config(&json!({"safari":{"root":directory}}).to_string())?;
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        routed::relay(
            session_gateway,
            session_helper,
            sources,
            Duration::from_millis(20),
        )
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{"safari":{"generation":"old","checkpoint":"old-cp"}}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");

    let (size, first) = receive(&mut gateway)?;
    assert_eq!(first["t"], "Capture");
    let capture = &first;
    assert_eq!(capture["source"], "safari");
    assert_eq!(capture["predecessor"], "");
    assert_eq!(capture["completion_evidence"], "complete");
    assert!(capture.get("coverage").is_none());
    assert!(
        capture["observations"]
            .as_array()
            .is_some_and(|o| !o.is_empty())
    );
    let checkpoint = capture["checkpoint"].clone();
    let generation = capture["generation"].clone();

    send(
        &mut helper,
        &json!({"t":"Capture","source":"safari","generation":"swift","checkpoint":"swift-cp","observations":[]}),
    )?;
    let (_, committed) = receive(&mut helper)?;
    assert_eq!(committed["t"], "Committed");
    assert_eq!(committed["source"], "safari");
    assert_eq!(committed["checkpoint"], "swift-cp");

    send(
        &mut helper,
        &json!({"t":"Capture","source":"mail","generation":"swift","checkpoint":"mail-cp","observations":[]}),
    )?;
    let (mail_size, forwarded) = receive(&mut gateway)?;
    assert_eq!(forwarded["checkpoint"], "mail-cp");
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"mail","checkpoint":"mail-cp","credit":mail_size}),
    )?;
    let (_, committed) = receive(&mut helper)?;
    assert_eq!(committed["source"], "mail");

    send(
        &mut gateway,
        &json!({"t":"Committed","source":"safari","checkpoint":checkpoint,"credit":size}),
    )?;
    expect_silence(&mut helper)?;
    expect_silence(&mut gateway)?;

    history(&directory, 3)?;
    let (size, second) = receive(&mut gateway)?;
    assert_eq!(second["predecessor"], checkpoint);
    assert_eq!(second["generation"], generation);
    assert_ne!(second["checkpoint"], checkpoint);
    assert_eq!(second["live"], true);
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"safari","checkpoint":second["checkpoint"],"credit":size}),
    )?;
    expect_silence(&mut gateway)?;

    drop(gateway);
    drop(helper);
    let _ = worker.join();
    std::fs::remove_dir_all(directory)?;
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
    let sources = readers::from_config(&json!({source:{"root":directory,"staging":staging}}).to_string())?;
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        routed::relay(
            session_gateway,
            session_helper,
            sources,
            Duration::from_millis(20),
        )
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");
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
fn rust_calls_voicemail_and_voice_memos_are_routed() -> Fallible<()> {
    for source in ["phone", "voicemail", "voice_memos"] {
        media_source_is_routed(source)?;
    }
    Ok(())
}

fn full_safari(directory: &Path) -> Fallible<()> {
    let connection = rusqlite::Connection::open(directory.join("CloudTabs.db"))?;
    connection.execute_batch(
        "CREATE TABLE cloud_tab_devices(device_uuid TEXT,device_name TEXT);
         CREATE TABLE cloud_tabs(tab_uuid TEXT,device_uuid TEXT,title TEXT,url TEXT);
         INSERT INTO cloud_tab_devices VALUES('d','Synthetic');
         INSERT INTO cloud_tabs VALUES('t','d','Synthetic','https://example.com/');",
    )?;
    std::fs::write(
        directory.join("Bookmarks.plist"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Children</key><array><dict><key>WebBookmarkType</key><string>WebBookmarkTypeList</string><key>Title</key><string>Favorites</string><key>Children</key><array><dict><key>WebBookmarkType</key><string>WebBookmarkTypeLeaf</string><key>WebBookmarkUUID</key><string>a</string><key>URLString</key><string>https://example.com/</string><key>URIDictionary</key><dict><key>title</key><string>Synthetic</string></dict></dict></array></dict></array></dict></plist>",
    )?;
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

struct Wire {
    gateway: UnixStream,
    helper: UnixStream,
    worker: std::thread::JoinHandle<Result<(), ab_session::Error>>,
}

fn wire(
    sources: std::collections::BTreeMap<String, Box<dyn readers::CaptureReader>>,
    options: routed::Options,
    checkpoints: impl Into<Value>,
) -> Fallible<Wire> {
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        routed::relay_layered(session_gateway, session_helper, sources, &options, None)
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":checkpoints.into()}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");
    Ok(Wire {
        gateway,
        helper,
        worker,
    })
}

fn commit(wire: &mut Wire, source: &str, capture: &Value, size: usize) -> Fallible<()> {
    send(
        &mut wire.gateway,
        &json!({"t":"Committed","source":source,"checkpoint":capture["checkpoint"],"credit":size}),
    )
}

fn wait_for(path: &Path) -> Fallible<()> {
    for _ in 0..200 {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err("timed out waiting for file".into())
}

fn polling(state: Option<&Path>) -> routed::Options {
    routed::Options {
        poll: Duration::from_millis(20),
        watch: false,
        state: state.map(Path::to_path_buf),
    }
}

#[test]
fn rust_checkpoints_resume_after_restart_without_rescan() -> Fallible<()> {
    let directory = temporary("resume")?;
    let state = temporary("resume-state")?;
    history(&directory, 2)?;
    full_safari(&directory)?;
    let config = json!({"safari":{"root":directory}}).to_string();

    let mut first = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({}),
    )?;
    let (size, capture) = receive(&mut first.gateway)?;
    commit(&mut first, "safari", &capture, size)?;
    let mut capture = capture;
    for _ in 0..2 {
        let (size, other) = receive(&mut first.gateway)?;
        commit(&mut first, "safari", &other, size)?;
        capture = other;
    }
    wait_for(&state.join("safari.json"))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(state.join("safari.json"))?)?;
    assert_eq!(saved["checkpoint"], capture["checkpoint"]);
    assert_eq!(saved["generation"], capture["generation"]);
    drop(first.gateway);
    drop(first.helper);
    let _ = first.worker.join();

    let mut second = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({"safari":{"generation":capture["generation"],"checkpoint":capture["checkpoint"]}}),
    )?;
    expect_silence(&mut second.gateway)?;
    history(&directory, 3)?;
    let (_, changed) = receive(&mut second.gateway)?;
    assert_eq!(changed["predecessor"], capture["checkpoint"]);
    drop(second.gateway);
    drop(second.helper);
    let _ = second.worker.join();

    let mut mismatch = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({"safari":{"generation":capture["generation"],"checkpoint":"other-checkpoint"}}),
    )?;
    let (_, rescan) = receive(&mut mismatch.gateway)?;
    assert_eq!(rescan["t"], "Capture");
    assert_eq!(rescan["live"], false);
    drop(mismatch.gateway);
    drop(mismatch.helper);
    let _ = mismatch.worker.join();
    std::fs::remove_dir_all(directory)?;
    std::fs::remove_dir_all(state)?;
    Ok(())
}

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
    drop(first.helper);
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
    drop(second.helper);
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
        routed::Options {
            poll: Duration::from_secs(120),
            watch: true,
            state: None,
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
    drop(session.helper);
    let _ = session.worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn production_config_flips_a_source_to_rust_and_back() -> Fallible<()> {
    let home = temporary("home")?;
    let directory = temporary("home-safari")?;
    history(&directory, 2)?;
    assert!(production::load(&home)?.is_none());
    std::fs::create_dir_all(home.join("session"))?;
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"safari":{"root":directory}},"watch":false,"poll_ms":20}).to_string(),
    )?;
    let loaded = production::load(&home)?.ok_or("config")?;
    assert_eq!(loaded.sources(), ["safari"]);
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"calendar":{"root":directory},"contacts":{"root":directory}}})
            .to_string(),
    )?;
    assert_eq!(
        production::load(&home)?.ok_or("config")?.sources(),
        ["calendar", "contacts"]
    );
    std::fs::write(
        home.join(production::CONFIG_FILE),
        json!({"sources":{"messages":{"root":directory}}}).to_string(),
    )?;
    assert!(production::load(&home).is_err());

    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        production::serve(session_gateway, session_helper, Some(&loaded), None)
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");
    assert_eq!(receive(&mut gateway)?.1["source"], "safari");
    drop(gateway);
    drop(helper);
    let _ = worker.join();

    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker =
        std::thread::spawn(move || production::serve(session_gateway, session_helper, None, None));
    send(
        &mut helper,
        &json!({"t":"Capture","source":"safari","generation":"swift","checkpoint":"swift-cp","observations":[]}),
    )?;
    let (_, forwarded) = receive(&mut gateway)?;
    assert_eq!(forwarded["checkpoint"], "swift-cp");
    expect_silence(&mut gateway)?;
    drop(gateway);
    drop(helper);
    let _ = worker.join();
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
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        routed::relay(
            session_gateway,
            session_helper,
            sources,
            Duration::from_millis(20),
        )
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");

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
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        routed::relay(
            session_gateway,
            session_helper,
            sources,
            Duration::from_millis(20),
        )
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");

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

fn event(key: &str, title: &str) -> Value {
    json!({"kind":"upsert","collection":"calendar.event","native_key":key,"scoped_alias":format!("cal/{key}"),"data":{"title":title},"observed_at":"2026-11-03T00:00:00Z"})
}

fn names(capture: &Value, kind: &str) -> Vec<String> {
    capture["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["kind"] == kind)
        .filter_map(|o| o["native_key"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn rust_calendar_diffs_helper_facts_without_false_deletion() -> Fallible<()> {
    let sources = readers::from_config(&json!({"calendar":{"root":"/nonexistent"}}).to_string())?;
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        routed::relay(
            session_gateway,
            session_helper,
            sources,
            Duration::from_millis(20),
        )
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");

    send(
        &mut helper,
        &json!({"t":"Capture","source":"calendar","generation":"swift-g","checkpoint":"h1","observations":[event("a","A"),event("b","B")]}),
    )?;
    expect_silence(&mut helper)?;
    let (size, first) = receive(&mut gateway)?;
    assert_eq!(first["source"], "calendar");
    assert_eq!(first["predecessor"], "");
    assert_eq!(first["generation"], "swift-g");
    assert_eq!(names(&first, "upsert"), ["a", "b"]);
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"calendar","checkpoint":first["checkpoint"],"credit":size}),
    )?;
    let (_, committed) = receive(&mut helper)?;
    assert_eq!(committed["checkpoint"], "h1");
    expect_silence(&mut gateway)?;

    send(
        &mut helper,
        &json!({"t":"Capture","source":"calendar","generation":"swift-g","checkpoint":"h2","observations":[event("a","A2")]}),
    )?;
    let (size, second) = receive(&mut gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(names(&second, "upsert"), ["a"]);
    assert_eq!(names(&second, "absent"), Vec::<String>::new());
    assert_eq!(names(&second, "evict"), Vec::<String>::new());
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"calendar","checkpoint":second["checkpoint"],"credit":size}),
    )?;
    assert_eq!(receive(&mut helper)?.1["checkpoint"], "h2");

    let mut gone = event("b", "B");
    gone["kind"] = json!("evict");
    send(
        &mut helper,
        &json!({"t":"Capture","source":"calendar","generation":"swift-g","checkpoint":"h3","observations":[event("a","A2"),gone]}),
    )?;
    let (size, third) = receive(&mut gateway)?;
    assert_eq!(names(&third, "upsert"), Vec::<String>::new());
    assert_eq!(names(&third, "evict"), ["b"]);
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"calendar","checkpoint":third["checkpoint"],"credit":size}),
    )?;
    assert_eq!(receive(&mut helper)?.1["checkpoint"], "h3");

    send(
        &mut helper,
        &json!({"t":"Capture","source":"calendar","generation":"swift-g","checkpoint":"h4","observations":[]}),
    )?;
    assert_eq!(receive(&mut helper)?.1["checkpoint"], "h4");
    expect_silence(&mut gateway)?;

    drop(gateway);
    drop(helper);
    let _ = worker.join();
    Ok(())
}

#[test]
fn outcome_waits_for_rust_capture_of_its_key_and_rebinds_checkpoint() -> Fallible<()> {
    let directory = std::env::temp_dir().join(format!(
        "ab-capture-outcome-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    history(&directory, 2)?;
    let sources = readers::from_config(&json!({"safari":{"root":directory}}).to_string())?;
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    for socket in [&gateway, &helper] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    let worker = std::thread::spawn(move || {
        routed::relay(
            session_gateway,
            session_helper,
            sources,
            Duration::from_millis(20),
        )
    });
    send(
        &mut gateway,
        &json!({"t":"Open","protocol":"ipc/1","owner_fence":1,"byte_credit":4_194_304,"checkpoints":{}}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Open");
    send(&mut helper, &json!({"t":"Ready","protocol":"ipc/1"}))?;
    assert_eq!(receive(&mut gateway)?.1["t"], "Ready");
    let (size, capture) = receive(&mut gateway)?;
    let key = capture["observations"][0]["native_key"].clone();
    let checkpoint = capture["checkpoint"].clone();

    send(
        &mut gateway,
        &json!({"t":"Dispatch","command_id":"c1","ticket":"t1"}),
    )?;
    assert_eq!(receive(&mut helper)?.1["t"], "Dispatch");
    send(
        &mut helper,
        &json!({"t":"Outcome","command_id":"c1","disposition":"locally_observed","evidence":{"native_key":key,"observed":true,"capture_checkpoint":"stale"}}),
    )?;
    expect_silence(&mut gateway)?;
    send(
        &mut gateway,
        &json!({"t":"Committed","source":"safari","checkpoint":checkpoint,"credit":size}),
    )?;
    let outcome = receive(&mut gateway)?.1;
    assert_eq!(outcome["t"], "Outcome");
    assert_eq!(outcome["evidence"]["capture_checkpoint"], checkpoint);
    drop((gateway, helper));
    let _ = worker.join();
    Ok(())
}

#[test]
fn calendar_parts_merge_and_command_evidence_rebinds_to_the_rust_checkpoint() -> Fallible<()> {
    let sources = readers::from_config(&json!({"calendar":{"root":"/nonexistent"}}).to_string())?;
    let mut wire = wire(sources, polling(None), json!({}))?;

    send(
        &mut wire.helper,
        &json!({"t":"Capture","source":"calendar","generation":"swift-g","checkpoint":"h1","scan_id":"s1","observations":[event("a","A")]}),
    )?;
    assert_eq!(receive(&mut wire.helper)?.1["checkpoint"], "h1");
    expect_silence(&mut wire.gateway)?;
    send(
        &mut wire.helper,
        &json!({"t":"Capture","source":"calendar","generation":"swift-g","checkpoint":"h2","scan_id":"s1","completion_evidence":"complete","observations":[event("b","B")]}),
    )?;
    expect_silence(&mut wire.helper)?;
    let (size, first) = receive(&mut wire.gateway)?;
    assert_eq!(names(&first, "upsert"), ["a", "b"]);
    commit(&mut wire, "calendar", &first, size)?;
    assert_eq!(receive(&mut wire.helper)?.1["checkpoint"], "h2");

    send(
        &mut wire.gateway,
        &json!({"t":"Dispatch","command_id":"c1","ticket":"t1"}),
    )?;
    assert_eq!(receive(&mut wire.helper)?.1["t"], "Dispatch");
    send(
        &mut wire.helper,
        &json!({"t":"Capture","source":"calendar","generation":"swift-g","checkpoint":"h3","scan_id":"s2","completion_evidence":"complete","observations":[event("a","A")]}),
    )?;
    assert_eq!(receive(&mut wire.helper)?.1["checkpoint"], "h3");
    expect_silence(&mut wire.gateway)?;
    send(
        &mut wire.helper,
        &json!({"t":"Outcome","command_id":"c1","disposition":"locally_observed","evidence":{"native_key":"a","observed":true,"capture_checkpoint":"h3"}}),
    )?;
    let outcome = receive(&mut wire.gateway)?.1;
    assert_eq!(
        outcome["evidence"]["capture_checkpoint"],
        first["checkpoint"]
    );

    send(
        &mut wire.helper,
        &json!({"t":"Rejected","command_id":"c2","disposition":"rejected_before_dispatch","evidence":{"native_key":"b","capture_checkpoint":"h3"}}),
    )?;
    let rejected = receive(&mut wire.gateway)?.1;
    assert_eq!(rejected["t"], "Rejected");
    assert_eq!(
        rejected["evidence"]["capture_checkpoint"],
        first["checkpoint"]
    );

    drop((wire.gateway, wire.helper));
    let _ = wire.worker.join();
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

fn tab_frame(checkpoint: &str, tabs: &[(i64, i64, &str)], partial: bool) -> Value {
    let rows: Vec<Value> = tabs
        .iter()
        .map(|(window, index, title)| {
            json!({"kind":"upsert","collection":"safari.tab","native_key":format!("{window}:{index}"),
                   "scoped_alias":format!("safari/safari.tab/{window}:{index}"),"observed_at":"2026-01-01T00:00:00.000Z",
                   "data":{"window":window,"index":index,"title":title,"url":"https://example.com/","active":*index == 1}})
        })
        .collect();
    json!({"t":"Capture","source":"safari","generation":"swift-tabs","checkpoint":checkpoint,"scan_id":checkpoint,
           "scope":["safari.tab"],"completion_evidence":if partial {"partial"} else {"complete"},"observations":rows,"live":true})
}

#[test]
fn safari_live_tabs_arrive_as_helper_facts_and_leave_as_rust_captures() -> Fallible<()> {
    let directory = temporary("tabs")?;
    let mut session = wire(
        readers::from_config(&json!({"safari":{"root":directory}}).to_string())?,
        polling(None),
        json!({}),
    )?;
    expect_silence(&mut session.gateway)?;

    send(&mut session.helper, &tab_frame("t1", &[(7, 1, "One"), (7, 2, "Two")], false))?;
    let (_, committed) = receive(&mut session.helper)?;
    assert_eq!(committed["t"], "Committed");
    assert_eq!(committed["checkpoint"], "t1");
    let (size, first) = receive(&mut session.gateway)?;
    assert_eq!(first["source"], "safari");
    assert_eq!(first["scope"], json!(["safari.tab"]));
    assert_eq!(first["completion_evidence"], "complete");
    assert_eq!(first["live"], false);
    assert_eq!(first["predecessor"], "");
    let keys: Vec<&str> = first["observations"]
        .as_array()
        .ok_or("observations")?
        .iter()
        .map(|row| row["native_key"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["7:1", "7:2"]);
    assert_eq!(first["observations"][0]["data"]["title"], "One");
    commit(&mut session, "safari", &first, size)?;

    send(&mut session.helper, &tab_frame("t2", &[(7, 1, "One renamed")], false))?;
    assert_eq!(receive(&mut session.helper)?.1["t"], "Committed");
    let (size, second) = receive(&mut session.gateway)?;
    assert_eq!(second["live"], true);
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(second["generation"], first["generation"]);
    let kinds: Vec<(&str, &str)> = second["observations"]
        .as_array()
        .ok_or("observations")?
        .iter()
        .map(|row| {
            (
                row["kind"].as_str().unwrap_or_default(),
                row["native_key"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(kinds, [("upsert", "7:1"), ("absent", "7:2")]);
    commit(&mut session, "safari", &second, size)?;

    send(&mut session.helper, &tab_frame("t3", &[(7, 1, "One renamed")], false))?;
    assert_eq!(receive(&mut session.helper)?.1["t"], "Committed");
    expect_silence(&mut session.gateway)?;

    send(&mut session.helper, &tab_frame("t4", &[], true))?;
    assert_eq!(receive(&mut session.helper)?.1["t"], "Committed");
    let mut other = tab_frame("t5", &[], false);
    other["scope"] = json!(["safari.history"]);
    send(&mut session.helper, &other)?;
    assert_eq!(receive(&mut session.helper)?.1["t"], "Committed");
    expect_silence(&mut session.gateway)?;

    send(&mut session.helper, &tab_frame("t6", &[], false))?;
    assert_eq!(receive(&mut session.helper)?.1["t"], "Committed");
    let (_, closed) = receive(&mut session.gateway)?;
    assert_eq!(closed["observations"][0]["kind"], "absent");
    assert_eq!(closed["observations"][0]["native_key"], "7:1");

    drop(session.gateway);
    drop(session.helper);
    let _ = session.worker.join();
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
