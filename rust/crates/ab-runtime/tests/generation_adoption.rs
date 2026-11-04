#[path = "../../ab-session/tests/support/mod.rs"]
mod support;

use ab_session::{production::Options, readers};
use ab_state::{Gateway, State};
use ab_wire::messages::Capture;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn scratch(name: &str) -> Fallible<PathBuf> {
    let directory = std::env::temp_dir().join(format!(
        "ab-adopt-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn send(socket: &mut UnixStream, value: &Value) -> Fallible<()> {
    let bytes = serde_json::to_vec(value)?;
    socket.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
    socket.write_all(&bytes)?;
    Ok(())
}

fn receive(socket: &mut UnixStream) -> Fallible<Value> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let mut bytes = vec![0; usize::try_from(u32::from_be_bytes(header))?];
    socket.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn calls(directory: &Path, rows: &[(u32, f64)], uuid: &str) -> Fallible<()> {
    let _ = std::fs::remove_file(directory.join("CallHistory.storedata"));
    let connection = rusqlite::Connection::open(directory.join("CallHistory.storedata"))?;
    connection.execute_batch(
        "CREATE TABLE Z_METADATA(Z_UUID TEXT);
         CREATE TABLE ZCALLRECORD(Z_PK INTEGER PRIMARY KEY,ZADDRESS TEXT,ZNAME TEXT,ZDATE REAL,ZDURATION REAL,ZORIGINATED INTEGER,ZANSWERED INTEGER,ZSERVICE_PROVIDER TEXT,ZCALLTYPE INTEGER);",
    )?;
    connection.execute("INSERT INTO Z_METADATA VALUES(?1)", [uuid])?;
    for (key, duration) in rows {
        connection.execute(
            "INSERT INTO ZCALLRECORD VALUES(?1,'synthetic','Fixture',946684800.0,?2,0,1,'com.apple.Telephony',1)",
            rusqlite::params![key, duration],
        )?;
    }
    Ok(())
}

fn rust_scan(directory: &Path, checkpoints: &Value, state: Option<&Path>) -> Fallible<Value> {
    let sources = readers::from_config(&json!({"phone":{"root":directory}}).to_string())?;
    let options = Options {
        poll: Duration::from_millis(20),
        state: state.map(Path::to_path_buf),
        ..Options::default()
    };
    let support::Wire {
        mut gateway,
        link: helper,
        worker,
        home: _,
    } = support::wire(sources, options, checkpoints.clone())?;
    let capture = receive(&mut gateway)?;
    assert_eq!(capture["t"], "Capture");
    if let Some(state) = state {
        let size = serde_json::to_vec(&capture)?.len() + 4;
        send(
            &mut gateway,
            &json!({"t":"Committed","source":"phone","checkpoint":capture["checkpoint"],"credit":size}),
        )?;
        let record = state.join("phone.json");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !std::fs::read(&record).is_ok_and(|bytes| {
            serde_json::from_slice::<Value>(&bytes)
                .is_ok_and(|v| v["checkpoint"] == capture["checkpoint"])
        }) {
            assert!(std::time::Instant::now() < deadline, "state never saved");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    drop(gateway);
    drop(helper);
    let _ = worker.join();
    Ok(capture)
}

fn typed(capture: &Value) -> Fallible<Capture> {
    let mut value = capture.clone();
    value.as_object_mut().map(|o| o.remove("t"));
    Ok(serde_json::from_value(value)?)
}

fn swift_shaped(rust: &Value, generation: &str, checkpoint: &str, predecessor: &str) -> Value {
    let mut swift = rust.clone();
    swift["generation"] = json!(generation);
    swift["checkpoint"] = json!(checkpoint);
    swift["predecessor"] = json!(predecessor);
    if let Some(rows) = swift["observations"].as_array_mut() {
        for row in rows {
            let key = row["native_key"].as_str().unwrap_or_default().to_owned();
            row["scoped_alias"] = json!(format!("phone/{generation}/{key}"));
        }
    }
    swift
}

fn checkpoints(state: &State<Gateway>) -> Fallible<Value> {
    let map: BTreeMap<String, Value> = state
        .checkpoints()?
        .into_iter()
        .map(|(source, point)| {
            (
                source,
                json!({"generation":point.generation,"checkpoint":point.checkpoint}),
            )
        })
        .collect();
    Ok(json!(map))
}

fn ids(home: &Path) -> Fallible<BTreeMap<String, bool>> {
    let connection = rusqlite::Connection::open_with_flags(
        home.join("state.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut statement =
        connection.prepare("SELECT id,deleted FROM records WHERE collection='phone.call'")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn seeded(home: &Path, source: &Path, generation: &str) -> Fallible<State<Gateway>> {
    let mut state = State::<Gateway>::open(home)?;
    let first = rust_scan(source, &json!({}), None)?;
    let swift = swift_shaped(&first, generation, "swift-cp-1", "");
    state.commit_capture(&typed(&swift)?)?;
    Ok(state)
}

#[test]
fn flip_on_an_existing_home_keeps_every_record_id() -> Fallible<()> {
    let source = scratch("source")?;
    let home = scratch("home")?;
    calls(&source, &[(1, 5.0), (2, 6.0), (3, 7.0)], "uuid-1")?;
    let mut state = seeded(&home, &source, "swift-gen")?;
    let before = ids(&home)?;
    assert_eq!(before.len(), 3);
    assert!(before.values().all(|deleted| !deleted));

    let rust = rust_scan(&source, &checkpoints(&state)?, None)?;
    state.commit_capture(&typed(&rust)?)?;

    assert_eq!(ids(&home)?, before);
    assert_eq!(rust["generation"], "swift-gen");
    assert_eq!(rust["predecessor"], "swift-cp-1");
    std::fs::remove_dir_all(source)?;
    std::fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn rolling_back_after_a_flip_duplicates_and_loses_nothing() -> Fallible<()> {
    let source = scratch("source")?;
    let home = scratch("home")?;
    calls(&source, &[(1, 5.0), (2, 6.0), (3, 7.0)], "uuid-1")?;
    let mut state = seeded(&home, &source, "swift-gen")?;
    let before = ids(&home)?;

    let rust = rust_scan(&source, &checkpoints(&state)?, None)?;
    state.commit_capture(&typed(&rust)?)?;
    let rust_checkpoint = rust["checkpoint"].as_str().unwrap_or_default().to_owned();

    calls(&source, &[(1, 5.0), (2, 6.0), (3, 7.0), (4, 8.0)], "uuid-1")?;
    let again = rust_scan(&source, &checkpoints(&state)?, None)?;
    let mut changed = swift_shaped(&again, "swift-gen", "swift-cp-2", &rust_checkpoint);
    changed["generation"] = json!("swift-gen");
    state.commit_capture(&typed(&changed)?)?;
    let after_rollback = ids(&home)?;
    assert_eq!(after_rollback.len(), 4);
    assert!(after_rollback.values().all(|deleted| !deleted));
    for (id, deleted) in &before {
        assert_eq!(after_rollback.get(id), Some(deleted));
    }

    let flipped_again = rust_scan(&source, &checkpoints(&state)?, None)?;
    assert_eq!(flipped_again["generation"], "swift-gen");
    state.commit_capture(&typed(&flipped_again)?)?;
    assert_eq!(ids(&home)?, after_rollback);
    std::fs::remove_dir_all(source)?;
    std::fs::remove_dir_all(home)?;
    Ok(())
}

#[test]
fn a_database_regenerated_while_owned_is_a_new_generation() -> Fallible<()> {
    let source = scratch("source")?;
    let home = scratch("home")?;
    let saved = scratch("saved")?;
    calls(&source, &[(1, 5.0)], "uuid-1")?;
    let mut state = seeded(&home, &source, "swift-gen")?;

    let rust = rust_scan(&source, &checkpoints(&state)?, Some(&saved))?;
    assert_eq!(rust["generation"], "swift-gen");
    state.commit_capture(&typed(&rust)?)?;
    let saved_record: Value = serde_json::from_slice(&std::fs::read(saved.join("phone.json"))?)?;
    assert_eq!(saved_record["generation"], "swift-gen");

    std::fs::remove_file(source.join("CallHistory.storedata"))?;
    calls(&source, &[(1, 5.0)], "uuid-2")?;
    let regenerated = rust_scan(&source, &checkpoints(&state)?, Some(&saved))?;
    assert_ne!(regenerated["generation"], "swift-gen");
    assert_eq!(regenerated["predecessor"], "");
    std::fs::remove_dir_all(source)?;
    std::fs::remove_dir_all(home)?;
    std::fs::remove_dir_all(saved)?;
    Ok(())
}
