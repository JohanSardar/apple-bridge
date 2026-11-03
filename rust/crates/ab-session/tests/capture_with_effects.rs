#![cfg(all(feature = "rust-effects", feature = "rust-capture"))]

use ab_session::{effects, readers, routed};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
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

fn history(directory: &std::path::Path, visits: u32) -> Fallible<()> {
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

#[test]
#[allow(clippy::too_many_lines)]
fn one_session_serves_rust_capture_and_permit_gated_effects() -> Fallible<()> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let home = std::env::temp_dir().join(format!("ab-both-{}-{nonce}", std::process::id()));
    let directory =
        std::env::temp_dir().join(format!("ab-both-safari-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    history(&directory, 2)?;
    let sources = readers::from_config(&json!({"safari":{"root":directory}}).to_string())?;
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    let (mut native, session_native) = UnixStream::pair()?;
    for socket in [&gateway, &helper, &native] {
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
    }
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

        let (size, rust) = receive(&mut gateway)?;
        assert_eq!(rust["source"], "safari");
        let command = json!({"t":"Prepare","command_id":"fixture-command","principal":"fixture","origin":{"kind":"fixture"},"request_hash":"ipc-hash","expires_at":"2099-01-01T00:00:00Z","action":{"op":"calendar.create_event","args":{"title":"synthetic"}}});
        send(&mut gateway, &command)?;
        assert_eq!(receive(&mut helper)?.1, command);
        send(
            &mut helper,
            &json!({"t":"Prepared","command_id":"fixture-command","ticket":"fixture-ticket","fingerprint":"ipc-hash","native_state":{}}),
        )?;
        assert_eq!(receive(&mut gateway)?.1["t"], "Prepared");
        send(
            &mut gateway,
            &json!({"t":"Dispatch","command_id":"fixture-command","ticket":"fixture-ticket","origin":{"kind":"fixture"},"owner_fence":7}),
        )?;
        let effect = receive(&mut native)?.1;
        assert_eq!(effect["t"], "Effect");
        assert_eq!(effect["permit"]["owner_fence"], 7);

        send(
            &mut gateway,
            &json!({"t":"Committed","source":"safari","checkpoint":rust["checkpoint"],"credit":size}),
        )?;
        let capture = json!({"t":"Capture","source":"calendar","checkpoint":"committed-fixture","observations":[{"kind":"upsert","collection":"calendar.event","native_key":"fixture-event","data":{"title":"synthetic"}}]});
        let credit = serde_json::to_vec(&capture)?.len() + 4;
        send(&mut helper, &capture)?;
        let (_, forwarded) = loop {
            let frame = receive(&mut gateway)?;
            if frame.1["source"] == "calendar" {
                break frame;
            }
        };
        assert_eq!(forwarded, capture);
        send(
            &mut gateway,
            &json!({"t":"Committed","source":"calendar","checkpoint":"committed-fixture","credit":credit}),
        )?;
        assert_eq!(receive(&mut helper)?.1["t"], "Committed");
        send(
            &mut native,
            &json!({"t":"EffectResult","id":"fixture-command","evidence":{"observed":true,"fingerprint":"ipc-hash","native_key":"fixture-event"}}),
        )?;
        let outcome = loop {
            let frame = receive(&mut gateway)?.1;
            if frame["t"] != "Capture" {
                break frame;
            }
        };
        assert_eq!(outcome["disposition"], "locally_observed");
        assert_eq!(
            outcome["evidence"]["capture_checkpoint"],
            "committed-fixture"
        );

        history(&directory, 3)?;
        let (_, second) = loop {
            let frame = receive(&mut gateway)?;
            if frame.1["t"] == "Capture" && frame.1["source"] == "safari" {
                break frame;
            }
        };
        assert_eq!(second["predecessor"], rust["checkpoint"]);
        Ok(())
    })();
    let _ = gateway.shutdown(Shutdown::Both);
    let _ = helper.shutdown(Shutdown::Both);
    let _ = native.shutdown(Shutdown::Both);
    let _ = relay.join();
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_dir_all(directory);
    result
}
