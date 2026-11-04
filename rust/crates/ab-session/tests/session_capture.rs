#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use ab_session::{
    native_facts::Client,
    production::Options,
    queries::{MailBodies, NativeQueries},
    readers, session,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    os::unix::net::UnixStream,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{Fallible, expect_silence, polling, receive, send, temporary, wait_for, wire};

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

#[test]
fn a_reader_capture_reaches_the_gateway_and_the_helper_never_sees_it() -> Fallible<()> {
    let directory = temporary("session-safari")?;
    history(&directory, 2)?;
    let sources = readers::from_config(&json!({"safari":{"root":directory}}).to_string())?;
    let mut world = wire(
        sources,
        polling(None),
        json!({"safari":{"generation":"old","checkpoint":"old-cp"}}),
    )?;
    let (size, first) = receive(&mut world.gateway)?;
    assert_eq!(first["t"], "Capture");
    assert_eq!(first["source"], "safari");
    assert_eq!(first["predecessor"], "");
    assert_eq!(first["completion_evidence"], "complete");
    assert!(first.get("coverage").is_none());
    assert!(
        first["observations"]
            .as_array()
            .is_some_and(|o| !o.is_empty())
    );
    let checkpoint = first["checkpoint"].clone();
    let generation = first["generation"].clone();

    world.commit("safari", &first, size)?;
    expect_silence(&mut world.link)?;

    history(&directory, 3)?;
    let (size, second) = receive(&mut world.gateway)?;
    assert_eq!(second["predecessor"], checkpoint);
    assert_eq!(second["generation"], generation);
    assert_ne!(second["checkpoint"], checkpoint);
    assert_eq!(second["live"], true);
    world.commit("safari", &second, size)?;
    expect_silence(&mut world.gateway)?;

    world.finish()?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn a_helper_scan_of_a_rust_source_is_answered_by_the_session_and_never_forwarded() -> Fallible<()> {
    let directory = temporary("session-safari-scan")?;
    history(&directory, 2)?;
    full_safari(&directory)?;
    let sources = readers::from_config(&json!({"safari":{"root":directory}}).to_string())?;
    let mut world = wire(sources, polling(None), json!({}))?;
    let mut pending = Vec::new();
    for _ in 0..3 {
        let (size, capture) = receive(&mut world.gateway)?;
        pending.push(capture["checkpoint"].clone());
        world.commit("safari", &capture, size)?;
    }
    expect_silence(&mut world.gateway)?;
    world.scan(
        "h1",
        "safari",
        &json!({"scan_id":"s1","scope":["safari.tab"],"completion_evidence":"complete","observations":[
            {"kind":"upsert","collection":"safari.tab","native_key":"1:0","data":{"window":1,"index":0,"title":"T","url":"https://example.com/","active":true},"observed_at":"2026-01-02T03:04:05Z"}
        ]}),
    )?;
    let (_, committed) = receive(&mut world.link)?;
    assert_eq!(committed["t"], "ScanCommitted");
    assert_eq!(committed["id"], "h1");
    assert_eq!(committed["source"], "safari");
    let (size, tabs) = receive(&mut world.gateway)?;
    assert_eq!(tabs["source"], "safari");
    assert!(tabs["observations"].to_string().contains("safari.tab"));
    world.commit("safari", &tabs, size)?;
    world.finish()?;
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn checkpoints_resume_after_a_restart_and_a_mismatch_costs_a_rescan() -> Fallible<()> {
    let directory = temporary("session-resume")?;
    let state = temporary("session-resume-state")?;
    history(&directory, 2)?;
    full_safari(&directory)?;
    let config = json!({"safari":{"root":directory}}).to_string();

    let mut first = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({}),
    )?;
    let (size, mut capture) = receive(&mut first.gateway)?;
    first.commit("safari", &capture, size)?;
    for _ in 0..2 {
        let (size, other) = receive(&mut first.gateway)?;
        first.commit("safari", &other, size)?;
        capture = other;
    }
    wait_for(&state.join("safari.json"))?;
    let saved: Value = serde_json::from_slice(&std::fs::read(state.join("safari.json"))?)?;
    assert_eq!(saved["checkpoint"], capture["checkpoint"]);
    assert_eq!(saved["generation"], capture["generation"]);
    first.finish()?;

    let mut second = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({"safari":{"generation":capture["generation"],"checkpoint":capture["checkpoint"]}}),
    )?;
    expect_silence(&mut second.gateway)?;
    history(&directory, 3)?;
    let (_, changed) = receive(&mut second.gateway)?;
    assert_eq!(changed["predecessor"], capture["checkpoint"]);
    second.finish()?;

    let mut mismatch = wire(
        readers::from_config(&config)?,
        polling(Some(&state)),
        json!({"safari":{"generation":capture["generation"],"checkpoint":"other-checkpoint"}}),
    )?;
    let (_, rescan) = receive(&mut mismatch.gateway)?;
    assert_eq!(rescan["t"], "Capture");
    assert_eq!(rescan["live"], false);
    mismatch.finish()?;
    std::fs::remove_dir_all(directory)?;
    std::fs::remove_dir_all(state)?;
    Ok(())
}

#[derive(Default)]
struct FactState {
    events: Vec<Value>,
    verdict: String,
    fetches: usize,
    revalidated: Vec<Value>,
}

fn facts_server(state: Arc<Mutex<FactState>>) -> Fallible<Arc<Client>> {
    let (mut server, client) = UnixStream::pair()?;
    server.set_read_timeout(None)?;
    std::thread::spawn(move || {
        while let Ok((_, frame)) = receive(&mut server) {
            let reply = match frame["t"].as_str() {
                Some("Hello") => json!({"t":"Welcome","protocol":"native/1","version":1}),
                Some("Facts") => {
                    let mut facts = state.lock().unwrap();
                    facts.fetches += 1;
                    let items = if matches!(
                        frame["op"].as_str(),
                        Some("agenda.events" | "agenda.reminders")
                    ) {
                        facts.events.clone()
                    } else {
                        Vec::new()
                    };
                    json!({"t":"FactPage","id":frame["id"],"op":frame["op"],"snapshot":"snap","items":items,"cursor":null,"complete":true,"scope":"calendar","generation":"swift-g"})
                }
                Some("Revalidate") => {
                    let mut facts = state.lock().unwrap();
                    facts.revalidated.push(frame["args"]["identities"].clone());
                    let results: Vec<Value> = frame["args"]["identities"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|identity| json!({"collection":identity["collection"],"native_key":identity["native_key"],"state":facts.verdict}))
                        .collect();
                    json!({"t":"Revalidated","id":frame["id"],"op":frame["op"],"results":results})
                }
                _ => break,
            };
            if send(&mut server, &reply).is_err() {
                break;
            }
        }
    });
    Ok(Arc::new(Client::inherited(client)?))
}

fn event(key: &str, title: &str) -> Value {
    json!({"kind":"upsert","collection":"calendar.event","native_key":key,"scoped_alias":format!("calendar.event/{key}"),"data":{"title":title}})
}

fn calendar_options() -> Options {
    Options {
        poll: Duration::from_millis(20),
        ..Options::default()
    }
}

fn calendar(
    facts: &Arc<Client>,
    interval: Duration,
) -> BTreeMap<String, Box<dyn readers::CaptureReader>> {
    let reader =
        readers::AgendaReader::calendar("/nonexistent".into()).paged(Arc::clone(facts), interval);
    let boxed: Box<dyn readers::CaptureReader> = Box::new(reader);
    [("calendar".to_owned(), boxed)].into_iter().collect()
}

fn keys(capture: &Value, kind: &str) -> Vec<String> {
    capture["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == kind)
        .filter_map(|row| row["native_key"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn facts_rust_decides_a_deletion_only_after_the_helper_revalidates() -> Fallible<()> {
    let facts = Arc::new(Mutex::new(FactState {
        events: vec![event("a", "A"), event("b", "B")],
        verdict: "present".into(),
        ..FactState::default()
    }));
    let client = facts_server(Arc::clone(&facts))?;
    let mut world = wire(
        calendar(&client, Duration::from_millis(30)),
        calendar_options(),
        json!({}),
    )?;
    let (size, first) = receive(&mut world.gateway)?;
    assert_eq!(first["source"], "calendar");
    assert_eq!(first["generation"], "swift-g");
    assert_eq!(keys(&first, "upsert"), ["a", "b"]);
    world.commit("calendar", &first, size)?;
    expect_silence(&mut world.gateway)?;

    facts.lock().unwrap().events = vec![event("a", "A")];
    expect_silence(&mut world.gateway)?;
    assert_ne!(facts.lock().unwrap().revalidated.len(), 0);

    facts.lock().unwrap().verdict = "missing".into();
    let (size, second) = receive(&mut world.gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(keys(&second, "absent"), ["b"]);
    world.commit("calendar", &second, size)?;
    world.finish()?;
    Ok(())
}

#[test]
fn a_helper_scan_of_a_facts_source_is_held_until_the_reader_capture_that_followed_it_commits()
-> Fallible<()> {
    let facts = Arc::new(Mutex::new(FactState {
        events: vec![event("a", "A")],
        verdict: "present".into(),
        ..FactState::default()
    }));
    let client = facts_server(Arc::clone(&facts))?;
    let mut world = wire(
        calendar(&client, Duration::from_secs(3600)),
        calendar_options(),
        json!({}),
    )?;
    let (size, first) = receive(&mut world.gateway)?;
    world.commit("calendar", &first, size)?;
    expect_silence(&mut world.gateway)?;

    facts.lock().unwrap().events = vec![event("a", "A"), event("b", "B")];
    world.scan(
        "h1",
        "calendar",
        &json!({"scan_id":"s1","scope":["calendar.event"],"completion_evidence":"complete"}),
    )?;
    expect_silence(&mut world.link)?;
    let (size, second) = receive(&mut world.gateway)?;
    assert_eq!(keys(&second, "upsert"), ["b"]);
    expect_silence(&mut world.link)?;
    world.commit("calendar", &second, size)?;
    let (_, committed) = receive(&mut world.link)?;
    assert_eq!(committed["t"], "ScanCommitted");
    assert_eq!(committed["id"], "h1");
    assert_eq!(committed["checkpoint"], second["checkpoint"]);

    world.scan(
        "h2",
        "calendar",
        &json!({"scan_id":"s2","completion_evidence":"complete"}),
    )?;
    let (_, committed) = receive(&mut world.link)?;
    assert_eq!(committed["id"], "h2");
    expect_silence(&mut world.gateway)?;

    world.scan(
        "h3",
        "calendar",
        &json!({"scan_id":"s3","completion_evidence":"partial"}),
    )?;
    let (_, committed) = receive(&mut world.link)?;
    assert_eq!(committed["id"], "h3");
    world.finish()?;
    Ok(())
}

#[test]
fn a_committed_generation_is_adopted_so_record_ids_stay() -> Fallible<()> {
    let facts = Arc::new(Mutex::new(FactState {
        events: vec![event("a", "A")],
        verdict: "present".into(),
        ..FactState::default()
    }));
    let client = facts_server(Arc::clone(&facts))?;
    let mut world = wire(
        calendar(&client, Duration::from_secs(3600)),
        calendar_options(),
        json!({"calendar":{"generation":"committed-g","checkpoint":"committed-cp"}}),
    )?;
    let (_, first) = receive(&mut world.gateway)?;
    assert_eq!(first["generation"], "committed-g");
    assert_eq!(first["predecessor"], "committed-cp");
    world.finish()?;
    Ok(())
}

#[test]
fn losing_the_facts_link_ends_the_session_so_the_helper_restarts_it() -> Fallible<()> {
    let (server, client) = UnixStream::pair()?;
    let client = Arc::new(Client::inherited(client)?);
    drop(server);
    let world = wire(
        calendar(&client, Duration::from_millis(10)),
        calendar_options(),
        json!({}),
    )?;
    let started = std::time::Instant::now();
    while !world.worker.is_finished() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "session did not end"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let error = world.finish().expect_err("session should fail");
    assert!(error.to_string().contains("facts link"), "{error}");
    Ok(())
}

#[test]
fn mail_bodies_and_reader_blobs_never_reach_the_helper() -> Fallible<()> {
    let root = temporary("session-mail")?;
    let folder = root.join("Mailboxes/INBOX.mbox/Data/Messages");
    std::fs::create_dir_all(&folder)?;
    let message = "From: a@example.com\r\nSubject: hello\r\n\r\nbody text\r\n";
    std::fs::write(
        folder.join("42.emlx"),
        format!("{}\n{message}", message.len()),
    )?;
    let options = Options {
        poll: Duration::from_secs(3600),
        queries: NativeQueries {
            mail_bodies: Some(Arc::new(MailBodies::new(root.clone()))),
            shortcut_roots: None,
        },
        ..Options::default()
    };
    let safari = temporary("session-mail-safari")?;
    history(&safari, 1)?;
    full_safari(&safari)?;
    let sources = readers::from_config(&json!({"safari":{"root":safari}}).to_string())?;
    let mut world = wire(sources, options, json!({}))?;
    let (size, capture) = receive(&mut world.gateway)?;
    world.commit("safari", &capture, size)?;

    send(
        &mut world.gateway,
        &json!({"t":"Query","id":"q1","op":"mail.body","args":{"message_id":"42"}}),
    )?;
    loop {
        let (size, frame) = receive(&mut world.gateway)?;
        if frame["t"] == "Capture" {
            world.commit("safari", &frame, size)?;
            continue;
        }
        assert_eq!(frame["t"], "QueryResult");
        assert_eq!(frame["id"], "q1");
        break;
    }
    send(
        &mut world.gateway,
        &json!({"t":"BlobRead","handle":"query-body:unknown","offset":0,"length":10}),
    )?;
    loop {
        let (size, frame) = receive(&mut world.gateway)?;
        if frame["t"] == "Capture" {
            world.commit("safari", &frame, size)?;
            continue;
        }
        assert_eq!(frame["t"], "BlobUnavailable");
        assert_eq!(frame["handle"], "query-body:unknown");
        break;
    }
    send(
        &mut world.gateway,
        &json!({"t":"BlobRead","handle":"media:other","offset":0,"length":10}),
    )?;
    let (_, forwarded) = receive(&mut world.link)?;
    assert_eq!(forwarded["t"], "BlobRead");
    assert_eq!(forwarded["handle"], "media:other");
    world.finish()?;
    std::fs::remove_dir_all(root)?;
    std::fs::remove_dir_all(safari)?;
    Ok(())
}

fn reminder(key: &str, title: &str) -> Value {
    json!({"kind":"upsert","collection":"reminders.reminder","native_key":key,"scoped_alias":format!("reminders.reminder/{key}"),"data":{"title":title,"notes":null,"priority":0,"completed":false,"list_id":{"$ref":{"collection":"reminders.list","native_key":"work"}}}})
}

fn owned_reminders(facts: &Arc<Mutex<FactState>>) -> Fallible<support::Wire> {
    let client = facts_server(Arc::clone(facts))?;
    let reader = readers::AgendaReader::reminders("/nonexistent".into())
        .paged(client, Duration::from_secs(3600));
    let boxed: Box<dyn readers::CaptureReader> = Box::new(reader);
    let mut world = support::wire_with(
        [("reminders".to_owned(), boxed)].into_iter().collect(),
        calendar_options(),
        json!({}),
        &session::Options {
            rust_commands: vec!["reminders".into()],
        },
    )?;
    let (size, first) = receive(&mut world.gateway)?;
    world.commit("reminders", &first, size)?;
    expect_silence(&mut world.gateway)?;
    Ok(world)
}

fn prepare(world: &mut support::Wire, id: &str, title: &str) -> Fallible<()> {
    send(
        &mut world.gateway,
        &json!({"t":"Prepare","command_id":id,"principal":"owner","request_hash":format!("hash-{id}"),"origin":"interactive",
            "expires_at":"2099-01-02T03:04:05Z",
            "action":{"op":"reminders.update","native_key":"r1","list_key":"work","args":{"title":"oat milk"}},
            "expected_state":reminder("r1", title)["data"]}),
    )
}

fn answer_read(world: &mut support::Wire, operation: &str, result: &Value) -> Fallible<Value> {
    let (_, asked) = receive(&mut world.link)?;
    assert_eq!(asked["t"], "Primitive");
    assert_eq!(asked["op"], operation);
    send(
        &mut world.link,
        &json!({"t":"PrimitiveResult","id":asked["id"],"result":result}),
    )?;
    Ok(asked)
}

fn read_reminder(title: &str) -> Value {
    json!({"exists":true,"writable":true,"list_key":"work","data":reminder("r1", title)["data"]})
}

fn read_list() -> Value {
    json!({"exists":true,"writable":true,"data":{"title":"Work","writable":true}})
}

#[test]
fn a_rust_command_on_a_rust_captured_source_is_bound_to_the_readers_checkpoint() -> Fallible<()> {
    let facts = Arc::new(Mutex::new(FactState {
        events: vec![reminder("r1", "milk")],
        verdict: "present".into(),
        ..FactState::default()
    }));
    let mut world = owned_reminders(&facts)?;
    prepare(&mut world, "c1", "milk")?;
    answer_read(
        &mut world,
        "reminders.read_reminder",
        &read_reminder("milk"),
    )?;
    answer_read(&mut world, "reminders.read_list", &read_list())?;
    let (_, prepared) = receive(&mut world.gateway)?;
    assert_eq!(prepared["t"], "Prepared");
    send(
        &mut world.gateway,
        &json!({"t":"Dispatch","command_id":"c1","ticket":prepared["ticket"],"origin":"interactive","owner_fence":1}),
    )?;
    answer_read(&mut world, "reminders.read_list", &read_list())?;
    answer_read(
        &mut world,
        "reminders.read_reminder",
        &read_reminder("milk"),
    )?;
    let (_, write) = receive(&mut world.link)?;
    assert_eq!(write["op"], "reminders.write");
    assert_eq!(write["args"]["session_captures"], true);

    let saved = reminder("r1", "oat milk")["data"].clone();
    send(
        &mut world.link,
        &json!({"t":"PrimitiveResult","id":write["id"],"result":{"native_key":"r1","saved":saved,"observed":saved,"exists_after":true}}),
    )?;
    expect_silence(&mut world.gateway)?;
    facts.lock().unwrap().events = vec![reminder("r1", "oat milk")];
    let (size, capture) = receive(&mut world.gateway)?;
    assert_eq!(capture["t"], "Capture");
    assert_eq!(keys(&capture, "upsert"), ["r1"]);
    world.commit("reminders", &capture, size)?;
    let (_, outcome) = receive(&mut world.gateway)?;
    assert_eq!(outcome["t"], "Outcome");
    assert_eq!(outcome["disposition"], "locally_observed");
    assert_eq!(
        outcome["evidence"]["capture_checkpoint"],
        capture["checkpoint"]
    );
    world.finish()?;
    Ok(())
}

#[derive(Debug)]
struct ScanningReader {
    root: std::path::PathBuf,
    sent: u32,
}

impl readers::CaptureReader for ScanningReader {
    fn source(&self) -> &'static str {
        "reminders"
    }
    fn root(&self) -> &Path {
        &self.root
    }
    fn next(&mut self, context: &readers::ScanContext) -> Result<Option<Value>, ab_session::Error> {
        self.sent += 1;
        Ok(Some(json!({
            "source": "reminders",
            "generation": "g1",
            "checkpoint": context.checkpoint,
            "scan_id": context.scan_id,
            "scan_started_at": "2026-01-01T00:00:00Z",
            "scope": ["reminders.reminder"],
            "completion_evidence": "",
            "observations": [reminder("r1", &format!("seen {}", self.sent))],
            "blob_handles": [],
            "live": false,
        })))
    }
    fn commit(&mut self) -> Result<(), ab_session::Error> {
        Ok(())
    }
    fn token(&self) -> Option<String> {
        None
    }
    fn resume(&mut self, _: &str) -> bool {
        false
    }
    fn more(&self) -> bool {
        true
    }
    fn refresh(&mut self) -> bool {
        true
    }
}

#[test]
fn a_reconcile_during_a_long_scan_never_waits_for_the_scan_to_end() -> Fallible<()> {
    let reader: Box<dyn readers::CaptureReader> = Box::new(ScanningReader {
        root: "/nonexistent".into(),
        sent: 0,
    });
    let mut world = support::wire_with(
        [("reminders".to_owned(), reader)].into_iter().collect(),
        calendar_options(),
        json!({}),
        &session::Options {
            rust_commands: vec!["reminders".into()],
        },
    )?;
    let (size, first) = receive(&mut world.gateway)?;
    world.commit("reminders", &first, size)?;
    prepare(&mut world, "c1", "bread")?;
    answer_read(
        &mut world,
        "reminders.read_reminder",
        &read_reminder("milk two"),
    )?;
    let started = std::time::Instant::now();
    loop {
        let (size, frame) = receive(&mut world.gateway)?;
        if frame["t"] == "Capture" {
            world.commit("reminders", &frame, size)?;
            continue;
        }
        assert_eq!(frame["t"], "Rejected", "{frame}");
        assert_eq!(frame["error"]["code"], "conflict");
        assert!(
            frame["evidence"]["capture_checkpoint"].is_string(),
            "{frame}"
        );
        break;
    }
    assert!(started.elapsed() < Duration::from_secs(10));
    Ok(())
}

#[test]
fn a_conflict_reconciles_through_the_reader_without_asking_the_helper() -> Fallible<()> {
    let facts = Arc::new(Mutex::new(FactState {
        events: vec![reminder("r1", "milk")],
        verdict: "present".into(),
        ..FactState::default()
    }));
    let mut world = owned_reminders(&facts)?;
    prepare(&mut world, "c1", "bread")?;
    facts.lock().unwrap().events = vec![reminder("r1", "milk two")];
    answer_read(
        &mut world,
        "reminders.read_reminder",
        &read_reminder("milk two"),
    )?;
    let (size, capture) = receive(&mut world.gateway)?;
    assert_eq!(capture["t"], "Capture");
    world.commit("reminders", &capture, size)?;
    let (_, rejected) = receive(&mut world.gateway)?;
    assert_eq!(rejected["t"], "Rejected");
    assert_eq!(rejected["error"]["code"], "conflict");
    assert_eq!(rejected["evidence"]["native_key"], "r1");
    assert_eq!(
        rejected["evidence"]["capture_checkpoint"],
        capture["checkpoint"]
    );
    expect_silence(&mut world.link)?;
    world.finish()?;
    Ok(())
}
