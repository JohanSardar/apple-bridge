#![allow(clippy::expect_used, clippy::unwrap_used)]
use ab_session::{Error, session};
use ab_wire::{
    crypto::ipc_proof,
    messages::IpcFrame,
    native::{self, Frame, Ready, Scan, SourceState},
};
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    io::{Read, Write},
    os::unix::net::UnixStream,
    thread::{self, JoinHandle},
    time::Duration,
};

const SECRET: [u8; 32] = [7; 32];
const A: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn proof(role: &str, helper_nonce: &str) -> String {
    hex(&ipc_proof(&SECRET, role, A, helper_nonce).expect("proof"))
}

fn raw(value: &Value) -> Box<serde_json::value::RawValue> {
    serde_json::value::RawValue::from_string(value.to_string()).expect("raw")
}

fn ipc_send(stream: &mut UnixStream, frame: &IpcFrame) {
    stream
        .write_all(&frame.encode().expect("encode"))
        .expect("write");
}

fn ipc_read(stream: &mut UnixStream) -> std::io::Result<(Value, usize)> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    let mut bytes = vec![0_u8; size];
    stream.read_exact(&mut bytes)?;
    Ok((serde_json::from_slice(&bytes).expect("json"), size + 4))
}

fn frame(t: &str) -> IpcFrame {
    IpcFrame {
        t: t.into(),
        ..IpcFrame::default()
    }
}

struct World {
    gateway: UnixStream,
    link: UnixStream,
    session: JoinHandle<Result<(), Error>>,
}

fn authenticate(gateway: &mut UnixStream, honest: bool) {
    let mut challenge = frame("Challenge");
    challenge.nonce = A.into();
    ipc_send(gateway, &challenge);
    let (authenticate, _) = ipc_read(gateway).expect("authenticate");
    assert_eq!(authenticate["t"], "Authenticate");
    let nonce = authenticate["nonce"].as_str().expect("nonce").to_owned();
    assert_eq!(authenticate["proof"], proof("helper", &nonce));
    let mut done = frame("Authenticated");
    done.proof = if honest {
        proof("gateway", &nonce)
    } else {
        proof("helper", &nonce)
    };
    ipc_send(gateway, &done);
}

fn open(gateway: &mut UnixStream, credit: i64, checkpoints: &Value) {
    let mut open = frame("Open");
    open.protocol = "ipc/1".into();
    open.owner_fence = 7;
    open.byte_credit = credit;
    if let Some(map) = checkpoints.as_object() {
        for (source, checkpoint) in map {
            open.checkpoints.insert(source.clone(), raw(checkpoint));
        }
    }
    ipc_send(gateway, &open);
}

fn start(credit: i64, checkpoints: &Value) -> World {
    let (mut gateway, session_gateway) = UnixStream::pair().expect("pair");
    let (mut link, session_link) = UnixStream::pair().expect("pair");
    let session = thread::spawn(move || session::serve(session_gateway, session_link, &SECRET));
    authenticate(&mut gateway, true);
    open(&mut gateway, credit, checkpoints);
    assert!(matches!(
        native::read_frame(&mut link),
        Ok(Frame::Hello { owner_fence: 7 })
    ));
    native::write_frame(&mut link, &Frame::Welcome).expect("welcome");
    let Ok(Frame::Session(opened)) = native::read_frame(&mut link) else {
        panic!("no session frame");
    };
    assert_eq!(
        opened.checkpoints.len(),
        checkpoints.as_object().map_or(0, serde_json::Map::len)
    );
    native::write_frame(
        &mut link,
        &Frame::Ready(Ready {
            helper_build: "t".into(),
            os_build: "o".into(),
            coverage: [("reminders".to_owned(), raw(&json!({"state": "ready"})))].into(),
            capabilities: [("reminders".to_owned(), raw(&json!(["reminders.create"])))].into(),
        }),
    )
    .expect("ready");
    let (ready, _) = ipc_read(&mut gateway).expect("ready");
    assert_eq!(ready["t"], "Ready");
    assert_eq!(ready["protocol"], "ipc/1");
    assert_eq!(ready["coverage"]["reminders"]["state"], "ready");
    World {
        gateway,
        link,
        session,
    }
}

fn scan(id: &str, source: &str, rows: &[(&str, &str, usize)], live: bool) -> Scan {
    let observations: Vec<Value> = rows
        .iter()
        .map(|(kind, key, size)| {
            json!({"kind": kind, "collection": format!("{source}.item"), "native_key": key,
                   "data": {"pad": "x".repeat(*size)}, "observed_at": "2026-01-02T03:04:05Z"})
        })
        .collect();
    serde_json::from_value(json!({
        "id": id, "source": source, "generation": "g1", "scan_id": format!("scan-{id}"),
        "scope": [format!("{source}.item")], "scan_started_at": "2026-01-02T03:04:05Z",
        "completion_evidence": "complete", "live": live, "observations": observations
    }))
    .expect("scan")
}

fn commit(gateway: &mut UnixStream, capture: &Value, size: usize) {
    let mut committed = frame("Committed");
    committed.source = serde_json::from_value(capture["source"].clone()).ok();
    committed.checkpoint = capture["checkpoint"].as_str().expect("checkpoint").into();
    committed.credit = i64::try_from(size).expect("size");
    ipc_send(gateway, &committed);
}

fn keys(capture: &Value) -> Vec<String> {
    capture["observations"]
        .as_array()
        .expect("observations")
        .iter()
        .map(|row| row["native_key"].as_str().expect("key").to_owned())
        .collect()
}

fn finish(world: World) -> Result<(), Error> {
    let World {
        gateway,
        link,
        session,
    } = world;
    drop(link);
    drop(gateway);
    session.join().expect("session panicked")
}

#[test]
fn session_owns_chunking_checkpoints_and_commit_acknowledgement() {
    let mut world = start(
        4_194_304,
        &json!({"reminders": {"generation": "g1", "checkpoint": "c0"}}),
    );
    let rows = [
        ("absent", "gone", 10),
        ("upsert", "a", 9000),
        ("upsert", "b", 9000),
    ];
    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("s1", "reminders", &rows, false)),
    )
    .expect("scan");

    let (first, first_size) = ipc_read(&mut world.gateway).expect("first");
    assert_eq!(first["t"], "Capture");
    assert_eq!(first["source"], "reminders");
    assert_eq!(first["predecessor"], "c0");
    assert_eq!(first["scan_id"], "scan-s1");
    assert_eq!(first["live"], Value::Null);
    assert_eq!(keys(&first), ["a"]);
    let first_checkpoint = first["checkpoint"].as_str().expect("checkpoint").to_owned();
    assert!(first_checkpoint.starts_with("ses-") && first_checkpoint.ends_with("-0"));
    world
        .link
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("timeout");
    assert!(
        native::read_frame(&mut world.link).is_err(),
        "scan acknowledged before the gateway committed it"
    );
    commit(&mut world.gateway, &first, first_size);

    let (second, second_size) = ipc_read(&mut world.gateway).expect("second");
    assert_eq!(second["predecessor"], first_checkpoint.as_str());
    assert_eq!(keys(&second), ["b", "gone"]);
    assert!(
        !second["checkpoint"]
            .as_str()
            .expect("checkpoint")
            .ends_with("-0")
    );
    world
        .link
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("timeout");
    assert!(native::read_frame(&mut world.link).is_err());
    commit(&mut world.gateway, &second, second_size);

    world
        .link
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let Ok(Frame::ScanCommitted(done)) = native::read_frame(&mut world.link) else {
        panic!("no acknowledgement");
    };
    assert_eq!(done.id, "s1");
    assert_eq!(
        done.checkpoint,
        second["checkpoint"].as_str().expect("checkpoint")
    );

    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("s2", "reminders", &[("upsert", "c", 10)], true)),
    )
    .expect("scan");
    let (next, next_size) = ipc_read(&mut world.gateway).expect("next");
    assert_eq!(next["predecessor"], second["checkpoint"]);
    assert_eq!(next["live"], true);
    commit(&mut world.gateway, &next, next_size);
    assert!(matches!(
        native::read_frame(&mut world.link),
        Ok(Frame::ScanCommitted(_))
    ));
    assert!(finish(world).is_ok());
}

#[test]
fn a_different_generation_has_no_predecessor() {
    let mut world = start(
        4_194_304,
        &json!({"reminders": {"generation": "old", "checkpoint": "c0"}}),
    );
    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("s1", "reminders", &[("upsert", "a", 10)], false)),
    )
    .expect("scan");
    let (capture, _) = ipc_read(&mut world.gateway).expect("capture");
    assert!(capture.get("predecessor").is_none() || capture["predecessor"] == "");
    let _ = finish(world);
}

#[test]
fn credit_is_shared_across_sources_and_returned_on_commit() {
    let mut world = start(10_000, &json!({}));
    let row = [("upsert", "a", 9000)];
    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("m", "mail", &row, false)),
    )
    .expect("scan");
    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("n", "notes", &row, false)),
    )
    .expect("scan");
    let (first, first_size) = ipc_read(&mut world.gateway).expect("first");
    world
        .gateway
        .set_read_timeout(Some(Duration::from_millis(400)))
        .expect("timeout");
    assert!(
        ipc_read(&mut world.gateway).is_err(),
        "second capture sent without credit"
    );
    world
        .gateway
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    commit(&mut world.gateway, &first, first_size);
    let (second, second_size) = ipc_read(&mut world.gateway).expect("second");
    assert_ne!(first["source"], second["source"]);
    commit(&mut world.gateway, &second, second_size);
    let _ = finish(world);
}

#[test]
fn a_scan_larger_than_the_whole_credit_fails_without_closing_the_session() {
    let mut world = start(2_000, &json!({}));
    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("big", "mail", &[("upsert", "a", 9000)], false)),
    )
    .expect("scan");
    let Ok(Frame::ScanFailed(failed)) = native::read_frame(&mut world.link) else {
        panic!("no failure");
    };
    assert_eq!(
        (failed.id.as_str(), failed.code.as_str()),
        ("big", "invalid")
    );
    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("ok", "mail", &[("upsert", "a", 10)], false)),
    )
    .expect("scan");
    let (capture, size) = ipc_read(&mut world.gateway).expect("capture");
    commit(&mut world.gateway, &capture, size);
    assert!(matches!(
        native::read_frame(&mut world.link),
        Ok(Frame::ScanCommitted(_))
    ));
    let _ = finish(world);
}

#[test]
fn a_commit_that_does_not_match_the_flight_closes_the_session() {
    let mut world = start(4_194_304, &json!({}));
    native::write_frame(
        &mut world.link,
        &Frame::Scan(scan("s", "mail", &[("upsert", "a", 10)], false)),
    )
    .expect("scan");
    let (capture, size) = ipc_read(&mut world.gateway).expect("capture");
    commit(&mut world.gateway, &capture, size - 1);
    assert!(matches!(finish(world), Err(Error::Invalid)));
}

#[test]
fn a_commit_without_a_flight_closes_the_session() {
    let mut world = start(4_194_304, &json!({}));
    let mut committed = frame("Committed");
    committed.source = serde_json::from_value(json!("mail")).ok();
    committed.checkpoint = "x".into();
    committed.credit = 10;
    ipc_send(&mut world.gateway, &committed);
    assert!(matches!(finish(world), Err(Error::Invalid)));
}

#[test]
fn a_wrong_gateway_proof_never_reaches_the_helper() {
    let (mut gateway, session_gateway) = UnixStream::pair().expect("pair");
    let (mut link, session_link) = UnixStream::pair().expect("pair");
    let session = thread::spawn(move || session::serve(session_gateway, session_link, &SECRET));
    authenticate(&mut gateway, false);
    assert!(session.join().expect("panicked").is_err());
    assert!(native::read_frame(&mut link).is_err());
}

#[test]
fn an_unknown_helper_frame_closes_the_session() {
    let mut world = start(4_194_304, &json!({}));
    native::write_frame(&mut world.link, &Frame::Hello { owner_fence: 7 }).expect("hello");
    assert!(matches!(finish(world), Err(Error::Invalid)));
}

#[test]
fn source_state_is_forwarded_and_commands_fail_closed() {
    let mut world = start(4_194_304, &json!({}));
    native::write_frame(
        &mut world.link,
        &Frame::SourceState(SourceState {
            source: serde_json::from_value(json!("reminders")).expect("source"),
            state: "degraded".into(),
            coverage: [("reminders".to_owned(), raw(&json!({"k": 1})))].into(),
            capabilities: [("reminders".to_owned(), raw(&json!([])))].into(),
            reason: "capture_failed".into(),
        }),
    )
    .expect("state");
    let (state, _) = ipc_read(&mut world.gateway).expect("state");
    assert_eq!(state["t"], "SourceState");
    assert_eq!(state["source"], "reminders");
    assert_eq!(state["state"], "degraded");
    assert_eq!(state["reason"], "capture_failed");

    ipc_send(&mut world.gateway, &frame("Ping"));
    assert_eq!(ipc_read(&mut world.gateway).expect("pong").0["t"], "Pong");

    let mut prepare = frame("Prepare");
    prepare.command_id = "cmd".into();
    ipc_send(&mut world.gateway, &prepare);
    let (rejected, _) = ipc_read(&mut world.gateway).expect("rejected");
    assert_eq!(rejected["t"], "Rejected");
    assert_eq!(rejected["command_id"], "cmd");
    assert_eq!(rejected["error"]["code"], "unsupported");

    let mut query = frame("Query");
    query.id = "q".into();
    ipc_send(&mut world.gateway, &query);
    let (answer, _) = ipc_read(&mut world.gateway).expect("query");
    assert_eq!(
        (answer["t"].as_str(), answer["code"].as_str()),
        (Some("QueryError"), Some("unsupported"))
    );

    let mut blob = frame("BlobRead");
    blob.handle = "h".into();
    ipc_send(&mut world.gateway, &blob);
    assert_eq!(
        ipc_read(&mut world.gateway).expect("blob").0["t"],
        "BlobUnavailable"
    );

    let mut dispatch = frame("Dispatch");
    dispatch.command_id = "cmd".into();
    ipc_send(&mut world.gateway, &dispatch);
    assert_eq!(
        ipc_read(&mut world.gateway).expect("dispatch").0["error"]["code"],
        "conflict"
    );
    let _ = finish(world);
}

#[test]
fn a_gateway_close_ends_the_session_cleanly() {
    let mut world = start(4_194_304, &json!({}));
    ipc_send(&mut world.gateway, &frame("Close"));
    assert!(native::read_frame(&mut world.link).is_err());
    assert!(finish(world).is_ok());
}
