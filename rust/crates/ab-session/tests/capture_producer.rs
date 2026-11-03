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
            "voicemail.db",
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
    let sources = readers::from_config(&json!({source:{"root":directory}}).to_string())?;
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
    let config = json!({"voice_memos":{"root":directory}}).to_string();
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
        json!({"sources":{"calendar":{"root":directory}}}).to_string(),
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
