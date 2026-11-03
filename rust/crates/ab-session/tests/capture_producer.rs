#![cfg(feature = "rust-capture")]

use ab_session::{readers, routed};
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
    let _ = std::fs::remove_file(directory.join("History.db"));
    let connection = rusqlite::Connection::open(directory.join("History.db"))?;
    connection.execute_batch(
        "CREATE TABLE history_items(id INTEGER PRIMARY KEY,url TEXT,visit_count INTEGER);
         CREATE TABLE history_visits(id INTEGER PRIMARY KEY,history_item INTEGER,visit_time REAL,title TEXT);
         INSERT INTO history_items VALUES(1,'https://example.com/',2);",
    )?;
    for visit in 0..visits {
        connection.execute(
            "INSERT INTO history_visits VALUES(?1,1,?2,?3)",
            rusqlite::params![visit + 1, now - f64::from(visit), format!("Visit {visit}")],
        )?;
    }
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
