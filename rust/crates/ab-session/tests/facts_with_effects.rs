#![cfg(all(feature = "rust-effects", feature = "rust-capture"))]

use ab_session::{effects, native_facts::Client, readers, routed};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;
type Titles = Arc<Mutex<BTreeMap<String, String>>>;

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

fn row(collection: &str, key: &str, title: &str) -> Value {
    json!({"kind":"upsert","collection":collection,"native_key":key,"scoped_alias":format!("cal/{key}"),"data":{"title":title},"observed_at":"2026-11-03T00:00:00Z"})
}

fn serve_facts(mut stream: UnixStream, titles: Titles) {
    std::thread::spawn(move || {
        let mut counter = 0_u32;
        while let Ok((_, frame)) = receive(&mut stream) {
            let reply = match frame["t"].as_str() {
                Some("Hello") => json!({"t":"Welcome","protocol":"native/1","version":1}),
                Some("Facts") => {
                    counter += 1;
                    let operation = frame["op"].as_str().unwrap_or_default();
                    let items: Vec<Value> = if operation == "agenda.events" {
                        titles
                            .lock()
                            .map(|titles| {
                                titles
                                    .iter()
                                    .map(|(key, title)| row("calendar.event", key, title))
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        vec![row("calendar.calendar", "cal-1", "Home")]
                    };
                    json!({
                        "t":"FactPage","id":frame["id"],"op":operation,"snapshot":format!("snap{counter}"),
                        "items":items,"cursor":Value::Null,"complete":true,
                        "scope":{"collections":["calendar.calendar","calendar.event"],"history_token":""},
                        "generation":"eventkit-calendar-v1",
                    })
                }
                _ => json!({"t":"Error","id":frame["id"],"code":"unsupported"}),
            };
            if send(&mut stream, &reply).is_err() {
                return;
            }
        }
    });
}

fn next_capture(gateway: &mut UnixStream, source: &str) -> Fallible<(usize, Value)> {
    loop {
        let frame = receive(gateway)?;
        if frame.1["t"] == "Capture" && frame.1["source"] == source {
            return Ok(frame);
        }
    }
}

fn upserts(capture: &Value) -> Vec<String> {
    capture["observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == "upsert")
        .filter_map(|row| row["native_key"].as_str().map(str::to_owned))
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn fact_paging_keeps_running_while_a_native_effect_waits_on_its_own_stream() -> Fallible<()> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let home = std::env::temp_dir().join(format!("ab-facts-fx-{}-{nonce}", std::process::id()));
    let titles: Titles = Arc::new(Mutex::new(BTreeMap::from([
        ("a".to_owned(), "A".to_owned()),
        ("b".to_owned(), "B".to_owned()),
    ])));
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    let (mut native, session_native) = UnixStream::pair()?;
    let (facts_server, session_facts) = UnixStream::pair()?;
    for socket in [&gateway, &helper, &native] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
    serve_facts(facts_server, Arc::clone(&titles));
    let client = Arc::new(Client::inherited(session_facts)?);
    let mut sources: BTreeMap<String, Box<dyn readers::CaptureReader>> = BTreeMap::new();
    sources.insert(
        "calendar".into(),
        Box::new(
            readers::AgendaReader::calendar("/nonexistent".into())
                .paged(client, Duration::from_millis(30)),
        ),
    );
    let route_home = home.clone();
    let relay = std::thread::spawn(move || -> Result<(), ab_session::Error> {
        let layer = effects::Layer::open(session_native, &route_home)?;
        routed::relay_layered(
            session_gateway,
            session_helper,
            sources,
            &routed::Options {
                poll: Duration::from_millis(20),
                ..routed::Options::default()
            },
            Some(&layer),
        )
    });
    let result = (|| -> Fallible<()> {
        send(
            &mut gateway,
            &json!({"t":"Open","protocol":"ipc/1","owner_fence":7,"byte_credit":4_194_304,"checkpoints":{}}),
        )?;
        assert_eq!(receive(&mut native)?.1["t"], "Hello");
        send(
            &mut native,
            &json!({"t":"Welcome","protocol":"native/1","version":1}),
        )?;
        assert_eq!(receive(&mut helper)?.1["t"], "Open");
        send(&mut helper, &json!({"t":"Ready"}))?;
        assert_eq!(receive(&mut gateway)?.1["t"], "Ready");

        let (size, first) = next_capture(&mut gateway, "calendar")?;
        assert_eq!(upserts(&first), ["cal-1", "a", "b"]);
        send(
            &mut gateway,
            &json!({"t":"Committed","source":"calendar","checkpoint":first["checkpoint"],"credit":size}),
        )?;

        let command = json!({"t":"Prepare","command_id":"facts-fx-command","principal":"fixture","origin":{"kind":"fixture"},"request_hash":"ipc-hash","expires_at":"2099-01-01T00:00:00Z","action":{"op":"maps.open","args":{"query":"synthetic"}}});
        send(&mut gateway, &command)?;
        assert_eq!(receive(&mut helper)?.1, command);
        send(
            &mut helper,
            &json!({"t":"Prepared","command_id":"facts-fx-command","ticket":"fixture-ticket","fingerprint":"ipc-hash","native_state":{}}),
        )?;
        assert_eq!(receive(&mut gateway)?.1["t"], "Prepared");
        send(
            &mut gateway,
            &json!({"t":"Dispatch","command_id":"facts-fx-command","ticket":"fixture-ticket","origin":{"kind":"fixture"},"owner_fence":7}),
        )?;
        let effect = receive(&mut native)?.1;
        assert_eq!(effect["t"], "Effect");
        assert_eq!(effect["op"], "maps.open");

        titles
            .lock()
            .map_err(|_| "poisoned")?
            .insert("a".into(), "A renamed".into());
        titles
            .lock()
            .map_err(|_| "poisoned")?
            .insert("c".into(), "C".into());
        let (size, second) = next_capture(&mut gateway, "calendar")?;
        assert_eq!(second["predecessor"], first["checkpoint"]);
        assert_eq!(upserts(&second), ["a", "c"]);
        send(
            &mut gateway,
            &json!({"t":"Committed","source":"calendar","checkpoint":second["checkpoint"],"credit":size}),
        )?;

        send(
            &mut native,
            &json!({"t":"EffectResult","id":effect["id"],"evidence":{"observed":true,"fingerprint":"ipc-hash"}}),
        )?;
        let outcome = loop {
            let frame = receive(&mut gateway)?.1;
            if frame["t"] == "Outcome" {
                break frame;
            }
        };
        assert_eq!(outcome["command_id"], "facts-fx-command");
        assert_eq!(outcome["disposition"], "locally_observed");

        titles
            .lock()
            .map_err(|_| "poisoned")?
            .insert("d".into(), "D".into());
        let (_, third) = next_capture(&mut gateway, "calendar")?;
        assert_eq!(third["predecessor"], second["checkpoint"]);
        assert_eq!(upserts(&third), ["d"]);
        Ok(())
    })();
    let _ = gateway.shutdown(Shutdown::Both);
    let _ = helper.shutdown(Shutdown::Both);
    let _ = native.shutdown(Shutdown::Both);
    let _ = relay.join();
    let _ = std::fs::remove_dir_all(home);
    result
}
