use crate::{Result, Shared, handshake, id, invalid, lock, observation, string};
use lab_wire::{receive, send};
use serde_json::{Value, json};
use std::{io::ErrorKind, os::unix::net::UnixStream, time::Duration};

fn ready() -> Value {
    json!({"t":"Ready","protocol":"ipc/1","capabilities":{},"coverage":{},"helper_build":"lab-simapple/rust","os_build":"sim"})
}

fn connect(socket: &str, secret: &[u8]) -> Result<(UnixStream, Value)> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let open = handshake(&mut stream, secret)?;
    send(&mut stream, &ready())?;
    Ok((stream, open))
}

fn reply(stream: &mut UnixStream) -> &'static str {
    if stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .is_err()
    {
        return "closed";
    }
    loop {
        match receive(stream) {
            Ok((frame, _)) => match frame["t"].as_str() {
                Some("Committed") => return "committed",
                Some("Ping" | "Pong") => {}
                _ => return "rejected",
            },
            Err(lab_wire::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                return "silent";
            }
            Err(_) => return "closed",
        }
    }
}

pub fn control_stale(state: &Shared, input: &Value) -> Result<Value> {
    let (socket, secret, generation) = {
        let s = lock(state)?;
        (
            s.socket.clone(),
            s.secret.clone(),
            string(&s.data, "generation"),
        )
    };
    let key = string(input, "key");
    if key.is_empty() {
        return Err(invalid("key required"));
    }
    let collection = input["collection"].as_str().unwrap_or("reminders.reminder");
    let source = crate::source(collection).to_owned();
    let (mut superseded, superseded_open) = connect(&socket, &secret)?;
    let (current, current_open) = connect(&socket, &secret)?;
    let stale_fence = superseded_open["owner_fence"].as_u64().unwrap_or_default();
    let current_fence = current_open["owner_fence"].as_u64().unwrap_or_default();
    let offered = &current_open["checkpoints"][&source];
    let capture = json!({
        "t":"Capture",
        "source":source,
        "generation":offered["generation"].as_str().map_or(generation, str::to_owned),
        "predecessor":string(offered, "checkpoint"),
        "checkpoint":id("cp")?,
        "observations":[observation(collection, &key, input["data"].clone())],
    });
    let result = match send(&mut superseded, &capture) {
        Ok(_) => reply(&mut superseded),
        Err(_) => "closed",
    };
    drop(current);
    let summary = json!({"stale_fence":stale_fence,"current_fence":current_fence,"reply":result,"key":key,"collection":collection,"data":input["data"]});
    let mut s = lock(state)?;
    s.data["protocol"]["stale"] = summary.clone();
    s.save()?;
    Ok(summary)
}
