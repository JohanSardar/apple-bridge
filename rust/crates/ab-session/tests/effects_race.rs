#![cfg(feature = "rust-effects")]

use ab_session::{Error, effects};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn send(socket: &mut UnixStream, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(value)?;
    socket.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
    socket.write_all(&bytes)?;
    Ok(())
}

fn receive(socket: &mut UnixStream) -> Result<Value, Box<dyn std::error::Error>> {
    let mut header = [0; 4];
    socket.read_exact(&mut header)?;
    let mut bytes = vec![0; usize::try_from(u32::from_be_bytes(header))?];
    socket.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn query_fixture(
    gateway: &mut UnixStream,
    native: &mut UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    send(
        gateway,
        &json!({"t":"Query","id":"fixture-query","op":"maps.search","args":{"query":"synthetic"}}),
    )?;
    let effect = receive(native)?;
    assert_eq!(effect["t"], "Effect");
    assert_eq!(effect["permit"]["operation"], "maps.search");
    assert_eq!(effect["args"]["session_query"], true);
    send(
        native,
        &json!({"t":"EffectResult","id":"fixture-query","evidence":{"observed":true,"native_query_result":{"places":[]}}}),
    )?;
    assert_eq!(
        receive(gateway)?,
        json!({"t":"QueryResult","id":"fixture-query","result_json":{"places":[]},"blobs":[]})
    );
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn native_route(commit_capture: bool, wrong_id: bool) -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::temp_dir().join(format!(
        "ab-effects-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    let (mut native, session_native) = UnixStream::pair()?;
    for socket in [&gateway, &helper, &native] {
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    }
    let route_home = home.clone();
    let relay = std::thread::spawn(move || {
        effects::relay_with(
            session_gateway,
            session_helper,
            session_native,
            &route_home,
            effects::Config {
                correlation_wait: Duration::from_millis(300),
                ..effects::Config::default()
            },
        )
    });
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        send(
            &mut gateway,
            &json!({"t":"Open","protocol":"ipc/1","owner_fence":7}),
        )?;
        assert_eq!(
            receive(&mut native)?,
            json!({"t":"Hello","protocol":"native/1","version":1,"owner_fence":7})
        );
        send(
            &mut native,
            &json!({"t":"Welcome","protocol":"native/1","version":1}),
        )?;
        assert_eq!(receive(&mut helper)?["t"], "Open");
        send(&mut helper, &json!({"t":"Ready"}))?;
        assert_eq!(receive(&mut gateway)?["t"], "Ready");
        let command = json!({"t":"Prepare","command_id":"fixture-command","principal":"fixture","origin":{"kind":"fixture"},"request_hash":"ipc-hash","expires_at":"2099-01-01T00:00:00Z","action":{"op":"calendar.create_event","args":{"title":"synthetic"}}});
        send(&mut gateway, &command)?;
        assert_eq!(receive(&mut helper)?, command);
        send(
            &mut helper,
            &json!({"t":"Prepared","command_id":"fixture-command","ticket":"fixture-ticket","fingerprint":"ipc-hash","native_state":{}}),
        )?;
        assert_eq!(receive(&mut gateway)?["t"], "Prepared");
        send(
            &mut gateway,
            &json!({"t":"Dispatch","command_id":"fixture-command","ticket":"fixture-ticket","origin":{"kind":"fixture"},"owner_fence":7}),
        )?;
        let effect = receive(&mut native)?;
        assert_eq!(effect["t"], "Effect");
        assert_eq!(effect["args"], command);
        assert_eq!(
            effect["permit"]["request_hash"],
            effects::request_hash(&command)?
        );
        assert_eq!(effect["permit"]["owner_fence"], 7);
        assert_eq!(effect["permit"]["nonce"].as_str().ok_or("nonce")?.len(), 64);
        let durable = std::fs::read_to_string(home.join("dispatch-guard.log"))?;
        assert_eq!(
            serde_json::from_str::<Value>(durable.trim())?["t"],
            "DispatchStarted"
        );
        let capture = json!({"t":"Capture","source":"calendar","checkpoint":"committed-fixture","observations":[{"kind":"upsert","collection":"calendar.event","native_key":"fixture-event","data":{"title":"synthetic"}}]});
        let credit = serde_json::to_vec(&capture)?.len() + 4;
        send(&mut helper, &capture)?;
        assert_eq!(receive(&mut gateway)?, capture);
        if commit_capture {
            send(
                &mut gateway,
                &json!({"t":"Committed","source":"calendar","checkpoint":"committed-fixture","credit":credit}),
            )?;
            assert_eq!(receive(&mut helper)?["t"], "Committed");
        }
        send(
            &mut native,
            &json!({"t":"EffectResult","id":if wrong_id { "wrong-command" } else { "fixture-command" },"evidence":{"observed":true,"fingerprint":"ipc-hash","native_key":"fixture-event"}}),
        )?;
        let outcome = receive(&mut gateway)?;
        if commit_capture && !wrong_id {
            assert_eq!(outcome["disposition"], "locally_observed");
            assert_eq!(
                outcome["evidence"]["capture_checkpoint"],
                "committed-fixture"
            );
        } else {
            assert_eq!(outcome["disposition"], "indeterminate");
            assert!(outcome["evidence"]["capture_checkpoint"].is_null());
        }
        send(
            &mut gateway,
            &json!({"t":"Resolve","command_id":"fixture-command"}),
        )?;
        let forwarded = receive(&mut helper)?;
        assert_eq!(forwarded["t"], "Resolve");
        assert_eq!(forwarded["saved"], outcome);
        send(&mut helper, &outcome)?;
        assert_eq!(receive(&mut gateway)?, outcome);
        if commit_capture && !wrong_id {
            query_fixture(&mut gateway, &mut native)?;
        }
        Ok(())
    })();
    let _ = gateway.shutdown(Shutdown::Both);
    let _ = helper.shutdown(Shutdown::Both);
    let _ = native.shutdown(Shutdown::Both);
    assert!(matches!(relay.join(), Ok(Err(Error::Io(_)))));
    std::fs::remove_dir_all(home)?;
    result
}

#[test]
fn native_route_requires_matching_committed_evidence() -> Result<(), Box<dyn std::error::Error>> {
    native_route(true, false)?;
    native_route(false, false)?;
    native_route(true, true)
}

#[test]
fn canonical_permit_hash_rejects_ambiguous_numbers() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        effects::request_hash(&json!({"b":"x/y","a":1}))?,
        effects::request_hash(&json!({"a":1,"b":"x/y"}))?
    );
    assert!(effects::request_hash(&json!({"x":1e-7})).is_err());
    assert_eq!(
        effects::request_hash(&json!({"x":2.0}))?,
        effects::request_hash(&json!({"x":2}))?
    );
    assert!(effects::request_hash(&json!({"x":-9_007_199_254_740_992_i64})).is_err());
    assert!(effects::request_hash(&json!({"x":9_007_199_254_740_992_u64})).is_ok());
    Ok(())
}

#[test]
fn ambiguous_float_refuses_the_command_and_keeps_the_gateway_stream() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::temp_dir().join(format!(
        "ab-effects-float-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let (mut gateway, session_gateway) = UnixStream::pair()?;
    let (mut helper, session_helper) = UnixStream::pair()?;
    let (mut native, session_native) = UnixStream::pair()?;
    for socket in [&gateway, &helper, &native] {
        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    }
    let route_home = home.clone();
    let relay = std::thread::spawn(move || {
        effects::relay_with(
            session_gateway,
            session_helper,
            session_native,
            &route_home,
            effects::Config::default(),
        )
    });
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        send(&mut gateway, &json!({"t":"Open","protocol":"ipc/1","owner_fence":7}))?;
        assert_eq!(receive(&mut native)?["t"], "Hello");
        send(&mut native, &json!({"t":"Welcome","protocol":"native/1","version":1}))?;
        assert_eq!(receive(&mut helper)?["t"], "Open");
        send(&mut helper, &json!({"t":"Ready"}))?;
        assert_eq!(receive(&mut gateway)?["t"], "Ready");
        for (id, magnitude) in [("float-small", 1e-7), ("float-large", 1e16)] {
            send(
                &mut gateway,
                &json!({"t":"Prepare","command_id":id,"principal":"fixture","origin":{"kind":"fixture"},"request_hash":"ipc-hash","expires_at":"2099-01-01T00:00:00Z","action":{"op":"calendar.create_event","args":{"offset":magnitude}}}),
            )?;
            let rejected = receive(&mut gateway)?;
            assert_eq!(rejected["t"], "Rejected");
            assert_eq!(rejected["command_id"], id);
            assert_eq!(rejected["error"]["code"], "invalid");
        }
        send(
            &mut gateway,
            &json!({"t":"Query","id":"float-query","op":"maps.search","args":{"radius":1e-7}}),
        )?;
        assert_eq!(
            receive(&mut gateway)?,
            json!({"t":"QueryError","id":"float-query","code":"invalid"})
        );
        let command = json!({"t":"Prepare","command_id":"after-float","principal":"fixture","origin":{"kind":"fixture"},"request_hash":"ipc-hash","expires_at":"2099-01-01T00:00:00Z","action":{"op":"calendar.create_event","args":{"offset":0.5}}});
        send(&mut gateway, &command)?;
        assert_eq!(receive(&mut helper)?, command);
        Ok(())
    })();
    let _ = gateway.shutdown(Shutdown::Both);
    let _ = helper.shutdown(Shutdown::Both);
    let _ = native.shutdown(Shutdown::Both);
    let _ = relay.join();
    std::fs::remove_dir_all(home)?;
    result
}
