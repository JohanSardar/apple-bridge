#![cfg(feature = "rust-capture")]

use ab_session::native_facts::Client;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
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

fn page(request: &Value) -> Value {
    json!({"t":"FactPage","id":request["id"],"op":request["op"],"snapshot":"s","items":[],"cursor":null,"complete":true,"scope":"all","generation":"g"})
}

#[test]
fn a_slow_answer_fails_that_request_and_leaves_the_shared_link_usable() -> Fallible<()> {
    let (client_end, mut server) = UnixStream::pair()?;
    let client = Client::inherited(client_end)?.with_patience(Duration::from_millis(200));
    let worker = std::thread::spawn(move || -> Result<(), String> {
        serve(&mut server).map_err(|error| error.to_string())
    });
    let late = client.fetch(1, "agenda.calendars");
    assert!(late.is_err(), "the slow request must fail");
    std::thread::sleep(Duration::from_millis(600));
    let next = client.fetch_patient(1, "agenda.calendars", &json!({}), Duration::from_secs(5))?;
    assert!(next.complete, "the link must survive a timed out request");
    worker.join().map_err(|_| "server panicked")??;
    Ok(())
}

fn serve(server: &mut UnixStream) -> Fallible<()> {
    assert_eq!(receive(server)?["t"], "Hello");
    send(
        server,
        &json!({"t":"Welcome","protocol":"native/1","version":1}),
    )?;
    let slow = receive(server)?;
    std::thread::sleep(Duration::from_millis(700));
    send(server, &page(&slow))?;
    let quick = receive(server)?;
    send(server, &page(&quick))?;
    Ok(())
}
