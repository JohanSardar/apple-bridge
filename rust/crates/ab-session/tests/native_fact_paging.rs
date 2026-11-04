mod support;

use ab_session::{native_facts::Client, production::Options, readers};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    time::Duration,
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

#[derive(Clone, Debug)]
struct Operation {
    error: &'static str,
    complete: bool,
    generation: &'static str,
    items: Vec<Value>,
}

type Facts = Arc<Mutex<BTreeMap<String, Operation>>>;

fn serve(mut stream: UnixStream, facts: Facts, page: usize, requests: Arc<Mutex<Vec<Value>>>) {
    std::thread::spawn(move || {
        let mut snapshots: BTreeMap<String, Operation> = BTreeMap::new();
        let mut counter = 0_u32;
        while let Ok((_, frame)) = receive(&mut stream) {
            if frame["t"] == "Hello" {
                assert_eq!(frame["owner_fence"], 1);
                if facts
                    .lock()
                    .is_ok_and(|guard| guard.contains_key("early-changed"))
                {
                    let _ = send(
                        &mut stream,
                        &json!({"t":"Changed","source":"contacts","generation":"native/1"}),
                    );
                }
                let _ = send(
                    &mut stream,
                    &json!({"t":"Welcome","protocol":"native/1","version":1}),
                );
                continue;
            }
            requests
                .lock()
                .map(|mut seen| seen.push(frame.clone()))
                .ok();
            let op = frame["op"].as_str().unwrap_or_default().to_owned();
            let reply = (|| {
                if frame["t"] == "Revalidate" {
                    let results: Vec<Value> = frame["args"]["identities"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|identity| {
                            let key = format!(
                                "verdict:{}/{}",
                                identity["collection"].as_str().unwrap_or_default(),
                                identity["native_key"].as_str().unwrap_or_default()
                            );
                            let state = facts
                                .lock()
                                .ok()
                                .and_then(|guard| guard.get(&key).map(|value| value.error))
                                .unwrap_or("present");
                            json!({"collection":identity["collection"],"native_key":identity["native_key"],"state":state})
                        })
                        .collect();
                    return json!({"t":"Revalidated","id":frame["id"],"op":op,"results":results});
                }
                let (token, offset) = if frame["snapshot"].is_null() {
                    let Ok(guard) = facts.lock() else {
                        return json!({"t":"Error","id":frame["id"],"code":"unavailable"});
                    };
                    let Some(operation) = guard.get(&op).cloned() else {
                        return json!({"t":"Error","id":frame["id"],"code":"unsupported"});
                    };
                    if !operation.error.is_empty() {
                        return json!({"t":"Error","id":frame["id"],"code":operation.error});
                    }
                    counter += 1;
                    let token = format!("snap{counter}");
                    snapshots.insert(token.clone(), operation);
                    (token, 0)
                } else {
                    let token = frame["snapshot"].as_str().unwrap_or_default().to_owned();
                    let cursor = frame["cursor"].as_str().unwrap_or_default();
                    let Some(position) = cursor
                        .strip_prefix(&format!("{token}:"))
                        .and_then(|value| value.parse::<usize>().ok())
                    else {
                        return json!({"t":"Error","id":frame["id"],"code":"invalid"});
                    };
                    (token, position)
                };
                let Some(operation) = snapshots.get(&token) else {
                    return json!({"t":"Error","id":frame["id"],"code":"expired"});
                };
                let end = (offset + page).min(operation.items.len());
                let terminal = end == operation.items.len();
                json!({
                    "t":"FactPage","id":frame["id"],"op":op,"snapshot":token,
                    "items":operation.items[offset..end],
                    "cursor":if terminal { Value::Null } else { json!(format!("{token}:{end}")) },
                    "complete":terminal && operation.complete,
                    "scope":{"collections":["calendar.calendar","calendar.event"],"history_token":""},
                    "generation":operation.generation,
                })
            })();
            if send(&mut stream, &reply).is_err() {
                return;
            }
        }
    });
}

fn verdict(facts: &Facts, identity: &str, state: &'static str) -> Fallible<()> {
    facts.lock().map_err(|_| "poisoned")?.insert(
        format!("verdict:{identity}"),
        Operation {
            error: state,
            complete: true,
            generation: "",
            items: Vec::new(),
        },
    );
    Ok(())
}

fn row(collection: &str, key: &str, title: &str) -> Value {
    json!({"kind":"upsert","collection":collection,"native_key":key,"scoped_alias":format!("cal/{key}"),"data":{"title":title},"observed_at":"2026-11-03T00:00:00Z"})
}

fn operation(items: Vec<Value>) -> Operation {
    Operation {
        error: "",
        complete: true,
        generation: "eventkit-calendar-v1",
        items,
    }
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

struct Wire {
    gateway: UnixStream,
    helper: UnixStream,
    worker: std::thread::JoinHandle<Result<(), ab_session::Error>>,
}

fn wire(sources: BTreeMap<String, Box<dyn readers::CaptureReader>>) -> Fallible<Wire> {
    let options = Options {
        poll: Duration::from_millis(20),
        watch: false,
        state: None,
        queries: ab_session::queries::NativeQueries::default(),
    };
    let support::Wire {
        gateway,
        link: helper,
        worker,
        home: _,
    } = support::wire(sources, options, json!({}))?;
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

struct Fixture {
    facts: Facts,
    requests: Arc<Mutex<Vec<Value>>>,
    wire: Wire,
}

fn fixture(interval: Duration, page: usize) -> Fallible<Fixture> {
    let (native, server) = UnixStream::pair()?;
    let facts: Facts = Arc::new(Mutex::new(BTreeMap::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    serve(server, Arc::clone(&facts), page, Arc::clone(&requests));
    let client = Arc::new(Client::inherited(native)?);
    let reader = readers::AgendaReader::calendar("/nonexistent".into()).paged(client, interval);
    let mut sources: BTreeMap<String, Box<dyn readers::CaptureReader>> = BTreeMap::new();
    sources.insert("calendar".into(), Box::new(reader));
    Ok(Fixture {
        facts,
        requests,
        wire: wire(sources)?,
    })
}

fn set(facts: &Facts, op: &str, value: Operation) -> Fallible<()> {
    facts
        .lock()
        .map_err(|_| "poisoned")?
        .insert(op.to_owned(), value);
    Ok(())
}

#[test]
fn rust_pages_calendar_facts_from_native_and_diffs_them() -> Fallible<()> {
    let mut f = fixture(Duration::from_millis(60), 2)?;
    set(
        &f.facts,
        "agenda.calendars",
        operation(vec![row("calendar.calendar", "cal-1", "Home")]),
    )?;
    set(
        &f.facts,
        "agenda.events",
        operation(vec![
            row("calendar.event", "a", "A"),
            row("calendar.event", "b", "B"),
            row("calendar.event", "c", "C"),
            row("calendar.event", "d", "D"),
            row("calendar.event", "e", "E"),
        ]),
    )?;
    let (size, first) = receive(&mut f.wire.gateway)?;
    assert_eq!(first["source"], "calendar");
    assert_eq!(first["predecessor"], "");
    assert_eq!(first["generation"], "eventkit-calendar-v1");
    assert_eq!(names(&first, "upsert"), ["cal-1", "a", "b", "c", "d", "e"]);
    commit(&mut f.wire, "calendar", &first, size)?;
    expect_silence(&mut f.wire.gateway)?;
    assert!(
        f.requests
            .lock()
            .map_err(|_| "poisoned")?
            .iter()
            .all(|request| request["args"] == json!({}) && request["limit"] == 500)
    );

    set(
        &f.facts,
        "agenda.events",
        operation(vec![
            row("calendar.event", "a", "A2"),
            row("calendar.event", "b", "B"),
            row("calendar.event", "c", "C"),
            row("calendar.event", "d", "D"),
        ]),
    )?;
    verdict(&f.facts, "calendar.event/e", "outside_scope")?;
    let (size, second) = receive(&mut f.wire.gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(names(&second, "upsert"), ["a"]);
    assert_eq!(names(&second, "evict"), ["e"]);
    commit(&mut f.wire, "calendar", &second, size)?;

    set(
        &f.facts,
        "agenda.events",
        operation(vec![
            row("calendar.event", "a", "A2"),
            row("calendar.event", "b", "B"),
            row("calendar.event", "c", "C"),
            row("calendar.event", "d", "D"),
        ]),
    )?;
    expect_silence(&mut f.wire.gateway)?;
    drop((f.wire.gateway, f.wire.helper));
    let _ = f.wire.worker.join();
    Ok(())
}

#[test]
fn incomplete_or_failed_native_pages_never_delete_or_emit() -> Fallible<()> {
    let mut f = fixture(Duration::from_millis(60), 10)?;
    set(
        &f.facts,
        "agenda.calendars",
        operation(vec![row("calendar.calendar", "cal-1", "Home")]),
    )?;
    set(
        &f.facts,
        "agenda.events",
        operation(vec![
            row("calendar.event", "a", "A"),
            row("calendar.event", "b", "B"),
        ]),
    )?;
    let (size, first) = receive(&mut f.wire.gateway)?;
    commit(&mut f.wire, "calendar", &first, size)?;

    set(
        &f.facts,
        "agenda.events",
        Operation {
            complete: false,
            ..operation(vec![row("calendar.event", "a", "A")])
        },
    )?;
    expect_silence(&mut f.wire.gateway)?;
    set(
        &f.facts,
        "agenda.events",
        Operation {
            error: "permission_missing",
            ..operation(Vec::new())
        },
    )?;
    expect_silence(&mut f.wire.gateway)?;
    set(
        &f.facts,
        "agenda.events",
        Operation {
            generation: "other-generation",
            ..operation(vec![row("calendar.event", "a", "A")])
        },
    )?;
    expect_silence(&mut f.wire.gateway)?;

    set(
        &f.facts,
        "agenda.events",
        operation(vec![
            row("calendar.event", "a", "A"),
            row("calendar.event", "b", "B"),
            row("calendar.event", "c", "C"),
        ]),
    )?;
    let (_, third) = receive(&mut f.wire.gateway)?;
    assert_eq!(names(&third, "upsert"), ["c"]);
    assert_eq!(names(&third, "absent"), Vec::<String>::new());
    assert_eq!(names(&third, "evict"), Vec::<String>::new());
    drop((f.wire.gateway, f.wire.helper));
    let _ = f.wire.worker.join();
    Ok(())
}

#[test]
fn helper_capture_is_only_a_hint_and_its_committed_waits_for_the_rust_commit() -> Fallible<()> {
    let mut f = fixture(Duration::from_secs(3600), 50)?;
    set(
        &f.facts,
        "agenda.calendars",
        operation(vec![row("calendar.calendar", "cal-1", "Home")]),
    )?;
    set(
        &f.facts,
        "agenda.events",
        operation(vec![row("calendar.event", "a", "A")]),
    )?;
    let (size, first) = receive(&mut f.wire.gateway)?;
    commit(&mut f.wire, "calendar", &first, size)?;
    expect_silence(&mut f.wire.gateway)?;

    set(
        &f.facts,
        "agenda.events",
        operation(vec![
            row("calendar.event", "a", "A"),
            row("calendar.event", "b", "B"),
        ]),
    )?;
    send(
        &mut f.wire.helper,
        &json!({"t":"Scan","id":"h1","source":"calendar","generation":"swift-g","observations":[row("calendar.event","zzz","ignored")]}),
    )?;
    expect_silence(&mut f.wire.helper)?;
    let (size, second) = receive(&mut f.wire.gateway)?;
    assert_eq!(names(&second, "upsert"), ["b"]);
    commit(&mut f.wire, "calendar", &second, size)?;
    assert_eq!(receive(&mut f.wire.helper)?.1["id"], "h1");
    drop((f.wire.gateway, f.wire.helper));
    let _ = f.wire.worker.join();
    Ok(())
}

#[test]
fn partial_helper_parts_release_at_once_and_only_the_final_part_pulls() -> Fallible<()> {
    let mut f = fixture(Duration::from_secs(3600), 50)?;
    set(
        &f.facts,
        "agenda.calendars",
        operation(vec![row("calendar.calendar", "cal-1", "Home")]),
    )?;
    set(
        &f.facts,
        "agenda.events",
        operation(vec![row("calendar.event", "a", "A")]),
    )?;
    let (size, first) = receive(&mut f.wire.gateway)?;
    commit(&mut f.wire, "calendar", &first, size)?;
    expect_silence(&mut f.wire.gateway)?;
    set(
        &f.facts,
        "agenda.events",
        operation(vec![
            row("calendar.event", "a", "A"),
            row("calendar.event", "b", "B"),
        ]),
    )?;
    send(
        &mut f.wire.helper,
        &json!({"t":"Scan","id":"p1","source":"calendar","generation":"swift-g","scan_id":"s1","observations":[]}),
    )?;
    assert_eq!(receive(&mut f.wire.helper)?.1["id"], "p1");
    expect_silence(&mut f.wire.gateway)?;
    send(
        &mut f.wire.helper,
        &json!({"t":"Scan","id":"p2","source":"calendar","generation":"swift-g","scan_id":"s1","completion_evidence":"complete","observations":[]}),
    )?;
    let (size, second) = receive(&mut f.wire.gateway)?;
    assert_eq!(names(&second, "upsert"), ["b"]);
    commit(&mut f.wire, "calendar", &second, size)?;
    assert_eq!(receive(&mut f.wire.helper)?.1["id"], "p2");
    drop((f.wire.gateway, f.wire.helper));
    let _ = f.wire.worker.join();
    Ok(())
}

#[test]
fn contacts_pull_all_three_collections_over_the_socket_link() -> Fallible<()> {
    let directory = std::env::temp_dir().join(format!("ab-native-facts-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("native.sock");
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path)?;
    let facts: Facts = Arc::new(Mutex::new(BTreeMap::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server_facts = Arc::clone(&facts);
    let server_requests = Arc::clone(&requests);
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve(stream, server_facts, 1, server_requests);
        }
    });
    let contact = |collection: &str, key: &str| json!({"kind":"upsert","collection":collection,"native_key":key,"scoped_alias":format!("c/{key}"),"data":{"name":key},"observed_at":"2026-11-03T00:00:00Z"});
    for (op, rows) in [
        (
            "contacts.containers",
            vec![contact("contacts.container", "box")],
        ),
        (
            "contacts.groups",
            vec![
                contact("contacts.group", "g1"),
                contact("contacts.group", "g2"),
            ],
        ),
        ("contacts.contacts", vec![contact("contacts.contact", "p1")]),
    ] {
        set(
            &facts,
            op,
            Operation {
                generation: "contacts-v1",
                ..operation(rows)
            },
        )?;
    }
    set(
        &facts,
        "contacts.contacts",
        Operation {
            generation: "contacts-v1",
            ..operation(vec![
                contact("contacts.contact", "p1"),
                json!({"kind":"blob_handle","digest":"d1","handle":"1:d1"}),
            ])
        },
    )?;
    let sources = readers::from_config(
        &json!({"contacts":{"root":"/nonexistent","facts":{"socket":path},"facts_poll_ms":60}})
            .to_string(),
    )?;
    let mut wire = wire(sources)?;
    let (_, first) = receive(&mut wire.gateway)?;
    assert_eq!(first["source"], "contacts");
    assert_eq!(first["generation"], "contacts-v1");
    let mut keys = names(&first, "upsert");
    keys.sort();
    assert_eq!(keys, ["box", "g1", "g2", "p1"]);
    assert_eq!(first["blob_handles"], json!({"d1":"1:d1"}));
    drop((wire.gateway, wire.helper));
    let _ = wire.worker.join();
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

fn reminder(collection: &str, key: &str, alias: &str, title: &str) -> Value {
    let mut value = json!({"kind":"upsert","collection":collection,"native_key":key,"data":{"title":title},"observed_at":"2026-11-03T00:00:00Z"});
    if !alias.is_empty() {
        value["scoped_alias"] = json!(alias);
    }
    value
}

fn reminders_fixture(page: usize) -> Fallible<Fixture> {
    let (native, server) = UnixStream::pair()?;
    let facts: Facts = Arc::new(Mutex::new(BTreeMap::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    serve(server, Arc::clone(&facts), page, Arc::clone(&requests));
    let client = Arc::new(Client::inherited(native)?);
    let reader = readers::AgendaReader::reminders("/nonexistent".into())
        .paged(client, Duration::from_millis(60));
    let mut sources: BTreeMap<String, Box<dyn readers::CaptureReader>> = BTreeMap::new();
    sources.insert("reminders".into(), Box::new(reader));
    Ok(Fixture {
        facts,
        requests,
        wire: wire(sources)?,
    })
}

fn reminder_operation(items: Vec<Value>) -> Operation {
    Operation {
        generation: "eventkit-lineage",
        ..operation(items)
    }
}

#[test]
fn reminders_pull_pages_both_collections_and_follows_absent_rows_and_alias_moves() -> Fallible<()> {
    let mut f = reminders_fixture(2)?;
    set(
        &f.facts,
        "agenda.reminder_lists",
        reminder_operation(vec![reminder("reminders.list", "list-1", "", "Home")]),
    )?;
    set(
        &f.facts,
        "agenda.reminders",
        reminder_operation(vec![
            reminder("reminders.reminder", "r1", "list-1/ext1", "One"),
            reminder("reminders.reminder", "r2", "list-1/ext2", "Two"),
            reminder("reminders.reminder", "r3", "", "Three"),
            reminder("reminders.reminder", "r4", "", "Four"),
        ]),
    )?;
    let (size, first) = receive(&mut f.wire.gateway)?;
    assert_eq!(first["source"], "reminders");
    assert_eq!(first["generation"], "eventkit-lineage");
    assert_eq!(
        first["scope"],
        json!(["reminders.list", "reminders.reminder"])
    );
    assert_eq!(names(&first, "upsert"), ["list-1", "r1", "r2", "r3", "r4"]);
    let observations = first["observations"].as_array().ok_or("observations")?;
    assert!(observations[0].get("scoped_alias").is_none());
    assert_eq!(observations[1]["scoped_alias"], "list-1/ext1");
    commit(&mut f.wire, "reminders", &first, size)?;
    expect_silence(&mut f.wire.gateway)?;
    assert!(
        f.requests
            .lock()
            .map_err(|_| "poisoned")?
            .iter()
            .any(|request| request["op"] == "agenda.reminders" && request["args"] == json!({}))
    );

    set(
        &f.facts,
        "agenda.reminders",
        reminder_operation(vec![
            reminder("reminders.reminder", "r1", "list-1/ext1", "One renamed"),
            reminder("reminders.reminder", "r2", "list-1/ext2-moved", "Two"),
            reminder("reminders.reminder", "r4", "", "Four"),
        ]),
    )?;
    verdict(&f.facts, "reminders.reminder/r3", "missing")?;
    let (size, second) = receive(&mut f.wire.gateway)?;
    assert_eq!(second["predecessor"], first["checkpoint"]);
    assert_eq!(names(&second, "upsert"), ["r1", "r2"]);
    assert_eq!(names(&second, "absent"), ["r3"]);
    commit(&mut f.wire, "reminders", &second, size)?;

    set(
        &f.facts,
        "agenda.reminders",
        reminder_operation(vec![
            reminder(
                "reminders.reminder",
                "r1-rekeyed",
                "list-1/ext1",
                "One renamed",
            ),
            reminder("reminders.reminder", "r2", "list-1/ext2-moved", "Two"),
        ]),
    )?;
    let (size, third) = receive(&mut f.wire.gateway)?;
    assert_eq!(names(&third, "upsert"), ["r1-rekeyed"]);
    assert_eq!(names(&third, "absent"), Vec::<String>::new());
    commit(&mut f.wire, "reminders", &third, size)?;

    set(
        &f.facts,
        "agenda.reminders",
        reminder_operation(vec![reminder(
            "reminders.reminder",
            "r1-rekeyed",
            "list-1/ext1",
            "One renamed",
        )]),
    )?;
    expect_silence(&mut f.wire.gateway)?;
    Ok(())
}

#[test]
fn reminders_list_absence_and_incomplete_pages_never_delete() -> Fallible<()> {
    let mut f = reminders_fixture(5)?;
    set(
        &f.facts,
        "agenda.reminder_lists",
        reminder_operation(vec![
            reminder("reminders.list", "list-1", "", "Home"),
            reminder("reminders.list", "list-2", "", "Work"),
        ]),
    )?;
    set(
        &f.facts,
        "agenda.reminders",
        reminder_operation(vec![reminder("reminders.reminder", "r1", "", "One")]),
    )?;
    let (size, first) = receive(&mut f.wire.gateway)?;
    commit(&mut f.wire, "reminders", &first, size)?;
    set(
        &f.facts,
        "agenda.reminder_lists",
        Operation {
            complete: false,
            ..reminder_operation(vec![reminder("reminders.list", "list-1", "", "Home")])
        },
    )?;
    expect_silence(&mut f.wire.gateway)?;
    set(
        &f.facts,
        "agenda.reminder_lists",
        reminder_operation(vec![reminder("reminders.list", "list-1", "", "Home")]),
    )?;
    verdict(&f.facts, "reminders.list/list-2", "missing")?;
    let (_, second) = receive(&mut f.wire.gateway)?;
    assert_eq!(names(&second, "absent"), ["list-2"]);
    assert_eq!(names(&second, "upsert"), Vec::<String>::new());
    Ok(())
}

#[test]
fn a_facts_link_without_a_native_endpoint_is_refused() {
    assert!(
        readers::from_config(
            &json!({"calendar":{"root":"/nonexistent","facts":"native"}}).to_string()
        )
        .is_err()
    );
    assert!(
        readers::from_config(
            &json!({"calendar":{"root":"/nonexistent","facts":{"socket":"relative.sock"}}})
                .to_string()
        )
        .is_err()
    );
}

fn contacts_over_socket(
    early_changed: bool,
) -> Fallible<(Wire, std::path::PathBuf, std::thread::JoinHandle<()>)> {
    let directory = std::env::temp_dir().join(format!(
        "ab-native-early-{}-{early_changed}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("native.sock");
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path)?;
    let facts: Facts = Arc::new(Mutex::new(BTreeMap::new()));
    let contact = |collection: &str, key: &str| json!({"kind":"upsert","collection":collection,"native_key":key,"scoped_alias":format!("c/{key}"),"data":{"name":key},"observed_at":"2026-11-03T00:00:00Z"});
    for (op, rows) in [
        ("contacts.containers", vec![]),
        ("contacts.groups", vec![]),
        ("contacts.contacts", vec![contact("contacts.contact", "p1")]),
    ] {
        set(
            &facts,
            op,
            Operation {
                generation: "contacts-v1",
                ..operation(rows)
            },
        )?;
    }
    if early_changed {
        set(&facts, "early-changed", operation(vec![]))?;
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server = std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve(stream, facts, 10, requests);
        }
    });
    let sources = readers::from_config(
        &json!({"contacts":{"root":"/nonexistent","facts":{"socket":path},"facts_poll_ms":60000}})
            .to_string(),
    )?;
    Ok((wire(sources)?, directory, server))
}

#[test]
fn a_changed_push_before_welcome_does_not_kill_the_facts_link() -> Fallible<()> {
    let (mut wire, directory, _server) = contacts_over_socket(true)?;
    let (_, first) = receive(&mut wire.gateway)?;
    assert_eq!(first["source"], "contacts");
    assert_eq!(names(&first, "upsert"), ["p1"]);
    drop((wire.gateway, wire.helper));
    let _ = wire.worker.join();
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

#[test]
fn a_dead_inherited_facts_link_ends_the_session_instead_of_stalling_every_poll() -> Fallible<()> {
    let (native, server) = UnixStream::pair()?;
    let client = Arc::new(Client::inherited(native)?);
    drop(server);
    let reader = readers::AgendaReader::contacts("/nonexistent".into())
        .paged(client, Duration::from_millis(20));
    let mut sources: BTreeMap<String, Box<dyn readers::CaptureReader>> = BTreeMap::new();
    sources.insert("contacts".into(), Box::new(reader));
    let wire = wire(sources)?;
    let (done, finished) = std::sync::mpsc::channel();
    let worker = wire.worker;
    std::thread::spawn(move || {
        let _ = done.send(worker.join());
    });
    let ended = finished.recv_timeout(Duration::from_secs(10))?;
    assert!(matches!(ended, Ok(Err(_))));
    drop((wire.gateway, wire.helper));
    Ok(())
}
