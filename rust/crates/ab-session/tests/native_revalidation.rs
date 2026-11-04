#![cfg(feature = "rust-capture")]

use ab_session::{
    native_facts::Client,
    readers::{AgendaReader, CaptureReader, ScanContext},
};
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

fn receive(socket: &mut UnixStream) -> Fallible<Value> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let mut bytes = vec![0; usize::try_from(u32::from_be_bytes(header))?];
    socket.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Default)]
struct World {
    items: BTreeMap<String, Vec<Value>>,
    generation: String,
    verdicts: BTreeMap<String, String>,
    refuse: bool,
    short: bool,
    swapped: bool,
    requests: Vec<Value>,
}

type Shared = Arc<Mutex<World>>;

fn serve(mut stream: UnixStream, world: Shared) {
    std::thread::spawn(move || {
        let mut counter = 0_u32;
        while let Ok(frame) = receive(&mut stream) {
            if frame["t"] == "Hello" {
                let _ = send(
                    &mut stream,
                    &json!({"t":"Welcome","protocol":"native/1","version":1}),
                );
                continue;
            }
            let Ok(mut guard) = world.lock() else {
                return;
            };
            guard.requests.push(frame.clone());
            let op = frame["op"].as_str().unwrap_or_default().to_owned();
            let reply = match frame["t"].as_str() {
                Some("Facts") => {
                    counter += 1;
                    json!({
                        "t":"FactPage","id":frame["id"],"op":op,"snapshot":format!("s{counter}"),
                        "items":guard.items.get(&op).cloned().unwrap_or_default(),
                        "cursor":Value::Null,"complete":true,
                        "scope":{"collections":["x"],"history_token":""},
                        "generation":guard.generation,
                    })
                }
                Some("Revalidate") if guard.refuse => {
                    json!({"t":"Error","id":frame["id"],"code":"unsupported"})
                }
                Some("Revalidate") => {
                    let results: Vec<Value> = frame["args"]["identities"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|identity| {
                            let key = format!(
                                "{}/{}",
                                identity["collection"].as_str().unwrap_or_default(),
                                identity["native_key"].as_str().unwrap_or_default()
                            );
                            json!({
                                "collection":identity["collection"],
                                "native_key":identity["native_key"],
                                "state":guard.verdicts.get(&key).cloned().unwrap_or_else(|| "present".into()),
                            })
                        })
                        .collect();
                    let mut results = results;
                    if guard.short {
                        results.pop();
                    }
                    if guard.swapped {
                        results.reverse();
                    }
                    json!({"t":"Revalidated","id":frame["id"],"op":op,"results":results})
                }
                _ => json!({"t":"Error","id":frame["id"],"code":"unsupported"}),
            };
            drop(guard);
            if send(&mut stream, &reply).is_err() {
                return;
            }
        }
    });
}

fn event(key: &str, title: &str) -> Value {
    json!({"kind":"upsert","collection":"calendar.event","native_key":key,"scoped_alias":format!("cal/{key}"),"data":{"title":title,"start":"2026-11-10T10:00:00Z","end":"2026-11-10T11:00:00Z","all_day":false}})
}

fn calendar(key: &str) -> Value {
    json!({"kind":"upsert","collection":"calendar.calendar","native_key":key,"scoped_alias":"","data":{"title":key}})
}

fn contact(key: &str) -> Value {
    json!({"kind":"upsert","collection":"contacts.contact","native_key":key,"scoped_alias":"","data":{"name":key}})
}

fn container(key: &str) -> Value {
    json!({"kind":"upsert","collection":"contacts.container","native_key":key,"scoped_alias":"","data":{"name":key}})
}

fn rig() -> Fallible<(Shared, Arc<Client>)> {
    let (native, server) = UnixStream::pair()?;
    let world: Shared = Arc::new(Mutex::new(World {
        generation: "eventkit-calendar-v1".into(),
        ..World::default()
    }));
    serve(server, Arc::clone(&world));
    Ok((world, Arc::new(Client::inherited(native)?)))
}

fn calendar_reader(client: &Arc<Client>) -> AgendaReader {
    AgendaReader::calendar("/nonexistent".into())
        .paged(Arc::clone(client), Duration::from_millis(1))
}

struct Clock(u32);

impl Clock {
    fn context(&mut self) -> ScanContext {
        self.0 += 1;
        std::thread::sleep(Duration::from_millis(5));
        ScanContext {
            checkpoint: format!("checkpoint-{}", self.0),
            scan_id: format!("scan-{}", self.0),
            now: 1_793_000_000.0 + f64::from(self.0),
            fence: 1,
        }
    }
}

fn set(world: &Shared, op: &str, items: Vec<Value>) -> Fallible<()> {
    world
        .lock()
        .map_err(|_| "poisoned")?
        .items
        .insert(op.into(), items);
    Ok(())
}

fn pull(reader: &mut AgendaReader, clock: &mut Clock) -> Fallible<Option<Value>> {
    let capture = reader.next(&clock.context())?;
    if capture.is_some() {
        reader.commit()?;
    }
    Ok(capture)
}

fn kinds(capture: &Value, kind: &str) -> Vec<String> {
    capture["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == kind)
        .filter_map(|row| row["native_key"].as_str().map(str::to_owned))
        .collect()
}

fn revalidations(world: &Shared) -> Fallible<Vec<Value>> {
    Ok(world
        .lock()
        .map_err(|_| "poisoned")?
        .requests
        .iter()
        .filter(|request| request["t"] == "Revalidate")
        .cloned()
        .collect())
}

fn seed(world: &Shared, reader: &mut AgendaReader, clock: &mut Clock) -> Fallible<()> {
    set(world, "agenda.calendars", vec![calendar("home")])?;
    set(
        world,
        "agenda.events",
        vec![event("a", "A"), event("b", "B"), event("c", "C")],
    )?;
    let first = pull(reader, clock)?.ok_or("first capture")?;
    assert_eq!(kinds(&first, "upsert"), ["home", "a", "b", "c"]);
    Ok(())
}

#[test]
fn rust_asks_swift_to_revalidate_what_the_pull_no_longer_lists() -> Fallible<()> {
    let (world, client) = rig()?;
    let mut reader = calendar_reader(&client);
    let mut clock = Clock(0);
    seed(&world, &mut reader, &mut clock)?;
    {
        let mut guard = world.lock().map_err(|_| "poisoned")?;
        guard
            .verdicts
            .insert("calendar.event/c".into(), "missing".into());
        assert!(
            guard
                .requests
                .iter()
                .filter(|request| request["t"] == "Facts")
                .all(|request| request["args"] == json!({}))
        );
    }
    set(
        &world,
        "agenda.events",
        vec![event("a", "A"), event("b", "B")],
    )?;
    let second = pull(&mut reader, &mut clock)?.ok_or("second capture")?;
    assert_eq!(kinds(&second, "absent"), ["c"]);
    assert_eq!(kinds(&second, "upsert"), Vec::<String>::new());
    let asked = revalidations(&world)?;
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0]["op"], "agenda.events");
    assert_eq!(
        asked[0]["args"]["identities"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(asked[0]["args"]["identities"][0]["native_key"], "c");
    assert_eq!(
        asked[0]["args"]["identities"][0]["data"]["start"],
        "2026-11-10T10:00:00Z"
    );
    Ok(())
}

#[test]
fn present_and_evict_verdicts_keep_or_evict_and_unlisted_rows_are_never_guessed() -> Fallible<()> {
    let (world, client) = rig()?;
    let mut reader = calendar_reader(&client);
    let mut clock = Clock(0);
    seed(&world, &mut reader, &mut clock)?;
    world.lock().map_err(|_| "poisoned")?.verdicts.extend([
        ("calendar.event/b".into(), "present".into()),
        ("calendar.event/c".into(), "outside_scope".into()),
    ]);
    set(&world, "agenda.events", vec![event("a", "A")])?;
    let second = pull(&mut reader, &mut clock)?.ok_or("second capture")?;
    assert_eq!(kinds(&second, "evict"), ["c"]);
    assert_eq!(kinds(&second, "absent"), Vec::<String>::new());
    set(&world, "agenda.events", vec![event("a", "A")])?;
    assert!(pull(&mut reader, &mut clock)?.is_none());
    let asked = revalidations(&world)?;
    assert_eq!(asked.len(), 2);
    assert_eq!(
        asked[1]["args"]["identities"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(asked[1]["args"]["identities"][0]["native_key"], "b");
    Ok(())
}

#[test]
fn a_refused_or_failed_revalidation_emits_nothing_and_deletes_nothing() -> Fallible<()> {
    let (world, client) = rig()?;
    let mut reader = calendar_reader(&client);
    let mut clock = Clock(0);
    seed(&world, &mut reader, &mut clock)?;
    {
        let mut guard = world.lock().map_err(|_| "poisoned")?;
        guard.refuse = true;
        guard
            .verdicts
            .insert("calendar.event/c".into(), "missing".into());
    }
    set(
        &world,
        "agenda.events",
        vec![event("a", "A"), event("b", "B")],
    )?;
    assert!(pull(&mut reader, &mut clock)?.is_none());
    assert!(pull(&mut reader, &mut clock)?.is_none());
    world.lock().map_err(|_| "poisoned")?.refuse = false;
    let recovered = pull(&mut reader, &mut clock)?.ok_or("capture after recovery")?;
    assert_eq!(kinds(&recovered, "absent"), ["c"]);
    Ok(())
}

#[test]
fn a_new_generation_never_asks_and_never_deletes() -> Fallible<()> {
    let (world, client) = rig()?;
    let mut reader = calendar_reader(&client);
    let mut clock = Clock(0);
    seed(&world, &mut reader, &mut clock)?;
    {
        let mut guard = world.lock().map_err(|_| "poisoned")?;
        guard.generation = "eventkit-calendar-v2".into();
        guard
            .verdicts
            .insert("calendar.event/c".into(), "missing".into());
    }
    set(
        &world,
        "agenda.events",
        vec![event("a", "A"), event("b", "B")],
    )?;
    let replay = pull(&mut reader, &mut clock)?.ok_or("replay capture")?;
    assert_eq!(kinds(&replay, "absent"), Vec::<String>::new());
    assert_eq!(kinds(&replay, "upsert"), ["home", "a", "b"]);
    assert_eq!(revalidations(&world)?.len(), 0);
    Ok(())
}

#[test]
fn the_baseline_survives_a_restart_through_the_reader_token() -> Fallible<()> {
    let (world, client) = rig()?;
    let mut reader = calendar_reader(&client);
    let mut clock = Clock(0);
    seed(&world, &mut reader, &mut clock)?;
    let token = reader.token().ok_or("baseline token")?;
    let mut restarted = calendar_reader(&client);
    assert!(restarted.resume(&token));
    world
        .lock()
        .map_err(|_| "poisoned")?
        .verdicts
        .insert("calendar.event/c".into(), "missing".into());
    set(
        &world,
        "agenda.events",
        vec![event("a", "A"), event("b", "B")],
    )?;
    let capture = pull(&mut restarted, &mut clock)?.ok_or("capture after restart")?;
    assert_eq!(kinds(&capture, "absent"), ["c"]);
    assert_eq!(kinds(&capture, "upsert"), ["home", "a", "b"]);
    assert!(pull(&mut restarted, &mut clock)?.is_none());
    let plain = AgendaReader::calendar("/nonexistent".into());
    assert!(plain.token().is_none());
    let mut refusing = plain;
    assert!(!refusing.resume(&token));
    assert!(!restarted.resume("{"));
    Ok(())
}

#[test]
fn contacts_ask_about_a_missing_container_and_delete_it_only_on_a_missing_verdict() -> Fallible<()>
{
    let (world, client) = rig()?;
    world.lock().map_err(|_| "poisoned")?.generation = "contacts-g1".into();
    let mut reader = AgendaReader::contacts("/nonexistent".into())
        .paged(Arc::clone(&client), Duration::from_millis(1));
    let mut clock = Clock(0);
    set(
        &world,
        "contacts.containers",
        vec![container("c1"), container("c2"), container("c3")],
    )?;
    set(&world, "contacts.groups", vec![])?;
    set(
        &world,
        "contacts.contacts",
        vec![contact("p"), contact("q")],
    )?;
    pull(&mut reader, &mut clock)?.ok_or("first capture")?;
    world.lock().map_err(|_| "poisoned")?.verdicts.extend([
        ("contacts.contact/q".into(), "missing".into()),
        ("contacts.container/c2".into(), "missing".into()),
        ("contacts.container/c3".into(), "present".into()),
    ]);
    set(&world, "contacts.containers", vec![container("c1")])?;
    set(&world, "contacts.contacts", vec![contact("p")])?;
    let second = pull(&mut reader, &mut clock)?.ok_or("second capture")?;
    let mut absent = kinds(&second, "absent");
    absent.sort();
    assert_eq!(absent, ["c2", "q"]);
    let mut operations: Vec<String> = revalidations(&world)?
        .iter()
        .filter_map(|request| request["op"].as_str().map(str::to_owned))
        .collect();
    operations.sort();
    assert_eq!(operations, ["contacts.contacts", "contacts.containers"]);
    Ok(())
}

#[test]
fn a_container_the_pull_omits_survives_a_refused_or_present_revalidation() -> Fallible<()> {
    for refuse in [false, true] {
        let (world, client) = rig()?;
        world.lock().map_err(|_| "poisoned")?.generation = "contacts-g1".into();
        let mut reader = AgendaReader::contacts("/nonexistent".into())
            .paged(Arc::clone(&client), Duration::from_millis(1));
        let mut clock = Clock(0);
        set(
            &world,
            "contacts.containers",
            vec![container("c1"), container("c2")],
        )?;
        set(&world, "contacts.groups", vec![])?;
        set(&world, "contacts.contacts", vec![contact("p")])?;
        pull(&mut reader, &mut clock)?.ok_or("first capture")?;
        world.lock().map_err(|_| "poisoned")?.refuse = refuse;
        set(&world, "contacts.containers", vec![container("c1")])?;
        set(
            &world,
            "contacts.contacts",
            vec![contact("p"), contact("r")],
        )?;
        let second = pull(&mut reader, &mut clock)?;
        if let Some(capture) = second {
            assert!(kinds(&capture, "absent").is_empty(), "refuse={refuse}");
        }
        assert_eq!(revalidations(&world)?.len(), 1);
        set(
            &world,
            "contacts.containers",
            vec![container("c1"), container("c2")],
        )?;
        world.lock().map_err(|_| "poisoned")?.refuse = false;
        let third = pull(&mut reader, &mut clock)?;
        if let Some(capture) = third {
            assert!(kinds(&capture, "absent").is_empty(), "refuse={refuse}");
        }
    }
    Ok(())
}

#[test]
fn reminders_ask_per_collection_and_only_a_present_verdict_keeps_a_record() -> Fallible<()> {
    let (world, client) = rig()?;
    world.lock().map_err(|_| "poisoned")?.generation = "eventkit-abc".into();
    let mut reader = AgendaReader::reminders("/nonexistent".into())
        .paged(Arc::clone(&client), Duration::from_millis(1));
    let mut clock = Clock(0);
    let list = |key: &str| json!({"kind":"upsert","collection":"reminders.list","native_key":key,"scoped_alias":"","data":{"title":key}});
    let reminder = |key: &str| json!({"kind":"upsert","collection":"reminders.reminder","native_key":key,"scoped_alias":format!("l/{key}"),"data":{"title":key}});
    set(
        &world,
        "agenda.reminder_lists",
        vec![list("l1"), list("l2")],
    )?;
    set(
        &world,
        "agenda.reminders",
        vec![reminder("r1"), reminder("r2")],
    )?;
    pull(&mut reader, &mut clock)?.ok_or("first capture")?;
    world.lock().map_err(|_| "poisoned")?.verdicts.extend([
        ("reminders.list/l2".into(), "missing".into()),
        ("reminders.reminder/r2".into(), "present".into()),
    ]);
    set(&world, "agenda.reminder_lists", vec![list("l1")])?;
    set(&world, "agenda.reminders", vec![reminder("r1")])?;
    let second = pull(&mut reader, &mut clock)?.ok_or("second capture")?;
    assert_eq!(kinds(&second, "absent"), ["l2"]);
    let mut operations: Vec<String> = revalidations(&world)?
        .iter()
        .filter_map(|request| request["op"].as_str().map(str::to_owned))
        .collect();
    operations.sort();
    assert_eq!(operations, ["agenda.reminder_lists", "agenda.reminders"]);
    Ok(())
}

#[test]
fn a_partial_or_reordered_revalidation_response_deletes_nothing() -> Fallible<()> {
    let (world, client) = rig()?;
    let mut reader = calendar_reader(&client);
    let mut clock = Clock(0);
    seed(&world, &mut reader, &mut clock)?;
    {
        let mut guard = world.lock().map_err(|_| "poisoned")?;
        guard.short = true;
        guard.verdicts.extend([
            ("calendar.event/b".into(), "missing".into()),
            ("calendar.event/c".into(), "missing".into()),
        ]);
    }
    set(&world, "agenda.events", vec![event("a", "A")])?;
    assert!(pull(&mut reader, &mut clock)?.is_none());
    {
        let mut guard = world.lock().map_err(|_| "poisoned")?;
        guard.short = false;
        guard.swapped = true;
    }
    assert!(pull(&mut reader, &mut clock)?.is_none());
    world.lock().map_err(|_| "poisoned")?.swapped = false;
    let recovered = pull(&mut reader, &mut clock)?.ok_or("capture after recovery")?;
    assert_eq!(kinds(&recovered, "absent"), ["b", "c"]);
    Ok(())
}

#[test]
fn a_dropped_connection_mid_revalidation_deletes_nothing() -> Fallible<()> {
    let (native, mut server) = UnixStream::pair()?;
    let client = Arc::new(Client::inherited(native)?);
    let mut reader = calendar_reader(&client);
    let mut clock = Clock(0);
    let script = std::thread::spawn(move || -> Result<(), String> {
        let mut pages = 0;
        loop {
            let Ok(frame) = receive(&mut server) else {
                return Ok(());
            };
            match frame["t"].as_str() {
                Some("Hello") => send(
                    &mut server,
                    &json!({"t":"Welcome","protocol":"native/1","version":1}),
                )
                .map_err(|error| error.to_string())?,
                Some("Facts") => {
                    pages += 1;
                    let op = frame["op"].as_str().unwrap_or_default();
                    let items = match (op, pages > 2) {
                        ("agenda.calendars", _) => vec![calendar("home")],
                        (_, false) => vec![event("a", "A"), event("b", "B")],
                        (_, true) => vec![event("a", "A")],
                    };
                    send(
                        &mut server,
                        &json!({"t":"FactPage","id":frame["id"],"op":op,"snapshot":format!("s{pages}"),"items":items,"cursor":Value::Null,"complete":true,"scope":{"collections":["x"],"history_token":""},"generation":"eventkit-calendar-v1"}),
                    )
                    .map_err(|error| error.to_string())?;
                }
                Some("Revalidate") => return Ok(()),
                _ => {}
            }
        }
    });
    let first = pull(&mut reader, &mut clock)?.ok_or("first capture")?;
    assert_eq!(kinds(&first, "upsert"), ["home", "a", "b"]);
    assert!(pull(&mut reader, &mut clock)?.is_none());
    script.join().map_err(|_| "script panicked")??;
    Ok(())
}

#[test]
fn a_changed_push_makes_the_pull_due_and_without_one_the_long_interval_holds() -> Fallible<()> {
    let (native, server) = UnixStream::pair()?;
    let mut pusher = server.try_clone()?;
    let world: Shared = Arc::new(Mutex::new(World {
        generation: "eventkit-calendar-v1".into(),
        ..World::default()
    }));
    serve(server, Arc::clone(&world));
    let client = Arc::new(Client::inherited(native)?);
    let mut reader = AgendaReader::calendar("/nonexistent".into())
        .paged(Arc::clone(&client), Duration::from_secs(3600));
    let mut clock = Clock(0);
    seed(&world, &mut reader, &mut clock)?;
    set(
        &world,
        "agenda.events",
        vec![
            event("a", "A"),
            event("b", "B"),
            event("c", "C"),
            event("d", "D"),
        ],
    )?;
    assert!(pull(&mut reader, &mut clock)?.is_none());
    send(
        &mut pusher,
        &json!({"t":"Changed","source":"contacts","generation":"x"}),
    )?;
    std::thread::sleep(Duration::from_millis(200));
    assert!(pull(&mut reader, &mut clock)?.is_none());
    send(
        &mut pusher,
        &json!({"t":"Changed","source":"calendar","generation":"eventkit-calendar-v1"}),
    )?;
    std::thread::sleep(Duration::from_millis(200));
    let pulled = pull(&mut reader, &mut clock)?.ok_or("capture after Changed")?;
    assert_eq!(kinds(&pulled, "upsert"), ["d"]);
    assert!(pull(&mut reader, &mut clock)?.is_none());
    Ok(())
}

#[test]
fn the_rust_store_enriches_pulled_calendar_events_and_serves_their_attachments() -> Fallible<()> {
    let root = std::env::temp_dir().join(format!("ab-session-calstore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Attachments"))?;
    rusqlite::Connection::open(root.join("Calendar.sqlitedb"))?.execute_batch(include_str!(
        "../../ab-native/tests/parity/calendar-store/schema.sql"
    ))?;
    std::fs::write(root.join("Attachments/a.pdf"), b"pdf-bytes-1")?;
    std::fs::write(root.join("Attachments/b.txt"), b"hello")?;
    let (world, client) = rig()?;
    let mut reader = AgendaReader::calendar("/nonexistent".into())
        .paged(Arc::clone(&client), Duration::from_millis(1))
        .with_calendar_store(root);
    let mut clock = Clock(0);
    let cal = "AAAAAAAA-0000-0000-0000-000000000001";
    let mut plain = event("ID-H", "H");
    plain["data"]["calendar_id"] =
        json!({"$ref":{"collection":"calendar.calendar","native_key":cal}});
    plain["scoped_alias"] = json!("src/cal/EXT-H");
    set(&world, "agenda.calendars", vec![calendar(cal)])?;
    set(&world, "agenda.events", vec![plain])?;
    let capture = pull(&mut reader, &mut clock)?.ok_or("capture")?;
    let enriched = capture["observations"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["native_key"] == "ID-H"))
        .ok_or("event row")?;
    assert_eq!(
        enriched["data"]["store_identifier_match"],
        "calendarItemIdentifier:unique_identifier"
    );
    assert_eq!(
        enriched["data"]["attachments"].as_array().map(Vec::len),
        Some(2)
    );
    let handles = capture["blob_handles"].as_object().ok_or("blob handles")?;
    assert_eq!(handles.len(), 2);
    let handle = handles
        .values()
        .next()
        .and_then(Value::as_str)
        .ok_or("handle")?;
    assert!(reader.owns(handle));
    assert_eq!(reader.handle_prefix(), Some("calendar-attachment:"));
    assert_eq!(reader.blob(handle, 0, 3)?["t"], "BlobChunk");
    assert!(!AgendaReader::calendar("/nonexistent".into()).owns(handle));
    Ok(())
}

#[test]
fn the_rust_store_enriches_pulled_reminders_from_the_store_and_never_invents_fields() -> Fallible<()>
{
    let stores = std::env::temp_dir().join(format!("ab-session-remstore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stores);
    std::fs::create_dir_all(&stores)?;
    rusqlite::Connection::open(stores.join("Data-fixture.sqlite"))?.execute_batch(include_str!(
        "../../ab-native/tests/parity/reminders-store/schema.sql"
    ))?;
    let (world, client) = rig()?;
    world.lock().map_err(|_| "poisoned")?.generation = "eventkit-abc".into();
    let mut reader = AgendaReader::reminders("/nonexistent".into())
        .paged(Arc::clone(&client), Duration::from_millis(1))
        .with_reminders_store(stores);
    let mut clock = Clock(0);
    let item = |collection: &str, key: &str| json!({"kind":"upsert","collection":collection,"native_key":key,"scoped_alias":"","data":{"title":key}});
    set(
        &world,
        "agenda.reminder_lists",
        vec![item(
            "reminders.list",
            "00000010-0000-4000-8000-000000000010",
        )],
    )?;
    set(
        &world,
        "agenda.reminders",
        vec![
            item("reminders.reminder", "000000A0-0000-4000-8000-0000000000A0"),
            item("reminders.reminder", "not-in-the-store"),
        ],
    )?;
    let capture = pull(&mut reader, &mut clock)?.ok_or("capture")?;
    let rows = capture["observations"].as_array().ok_or("rows")?;
    let find = |key: &str| rows.iter().find(|row| row["native_key"] == key);
    let known = find("000000A0-0000-4000-8000-0000000000A0").ok_or("known")?;
    assert_eq!(known["data"]["store_status"], "partial");
    assert_eq!(known["data"]["tags"], json!(["home"]));
    assert_eq!(
        known["data"]["title"],
        "000000A0-0000-4000-8000-0000000000A0"
    );
    let unknown = find("not-in-the-store").ok_or("unknown")?;
    assert_eq!(unknown["data"]["store_status"], "unavailable");
    assert_eq!(unknown["data"]["tags"], Value::Null);
    let list = find("00000010-0000-4000-8000-000000000010").ok_or("list")?;
    assert_eq!(list["data"]["badge_emblem"], "star");
    Ok(())
}
