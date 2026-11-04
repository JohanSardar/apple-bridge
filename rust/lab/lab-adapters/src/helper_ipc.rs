use crate::{
    assertions::await_until,
    http::{Client, random_id},
    journey::GATEWAY_SCOPE,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use socket2::SockRef;
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 1] = ["HELPER-IPC-RESPONSIVE"];
pub const BYTE_CREDIT: usize = 4 * 1024 * 1024;
const LIST: &str = "Bridge Test";
const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

type Wired<T> = std::result::Result<T, lab_wire::Error>;

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}
fn timed_out(error: &lab_wire::Error) -> bool {
    matches!(error, lab_wire::Error::Io(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}
fn capture_frame(mut capture: Value) -> Value {
    capture["t"] = json!("Capture");
    capture
}
fn kind(frame: &Value) -> &str {
    frame["t"].as_str().unwrap_or_default()
}

#[derive(Debug)]
pub struct Wire {
    stream: UnixStream,
}
impl Wire {
    pub fn connect(path: &str) -> Result<Self> {
        Ok(Self {
            stream: UnixStream::connect(path)?,
        })
    }
    #[must_use]
    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }
    pub fn send(&mut self, frame: &Value) -> Wired<usize> {
        self.stream
            .set_write_timeout(Some(Duration::from_secs(10)))?;
        lab_wire::send(&mut self.stream, frame)
    }
    pub fn receive_until(&mut self, deadline: Instant) -> Wired<Value> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| lab_wire::Error::Io(ErrorKind::TimedOut.into()))?;
        self.stream.set_read_timeout(Some(remaining))?;
        lab_wire::receive(&mut self.stream).map(|(frame, _)| frame)
    }
    pub fn receive(&mut self) -> Wired<(Value, usize)> {
        self.stream
            .set_read_timeout(Some(Duration::from_secs(15)))?;
        lab_wire::receive(&mut self.stream)
    }
    pub fn authenticate(&mut self, secret: &[u8]) -> Result<()> {
        if secret.len() != 32 {
            return Err(invalid("helper secret length"));
        }
        let (challenge, _) = self.receive().map_err(invalid)?;
        let nonce = challenge["nonce"].as_str().unwrap_or_default().to_owned();
        if kind(&challenge) != "Challenge" || nonce.len() != 64 {
            return Err(invalid("unexpected challenge"));
        }
        let mut bytes = [0u8; 32];
        fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let own = hex::encode(bytes);
        let proof = lab_wire::ipc_proof(secret, "helper", &nonce, &own).map_err(invalid)?;
        self.send(&json!({"t":"Authenticate","nonce":own,"proof":proof}))
            .map_err(invalid)?;
        let (answer, _) = self.receive().map_err(invalid)?;
        let gateway = answer["proof"].as_str().unwrap_or_default();
        if kind(&answer) != "Authenticated"
            || lab_wire::verify_ipc_proof(secret, "gateway", &nonce, &own, gateway).is_err()
        {
            return Err(invalid("gateway proof rejected"));
        }
        Ok(())
    }
    pub fn pong(&mut self, deadline: Instant) -> Result<()> {
        loop {
            let frame = self.receive_until(deadline).map_err(invalid)?;
            match kind(&frame) {
                "Pong" => return Ok(()),
                "Ping" => {
                    self.send(&json!({"t":"Pong"})).map_err(invalid)?;
                }
                other => return Err(invalid(format!("unexpected IPC frame {other} before Pong"))),
            }
        }
    }
    pub fn frame(&mut self, wanted: &str) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let frame = self.receive_until(deadline).map_err(invalid)?;
            if kind(&frame) == wanted {
                return Ok(frame);
            }
            if kind(&frame) != "Ping" {
                return Err(invalid(format!(
                    "expected {wanted}, received {}",
                    kind(&frame)
                )));
            }
            self.send(&json!({"t":"Pong"})).map_err(invalid)?;
        }
    }
    pub fn ping(&mut self, seconds: u64) -> Result<()> {
        self.send(&json!({"t":"Ping"})).map_err(invalid)?;
        self.pong(Instant::now() + Duration::from_secs(seconds))
    }
}

fn record(client: &Client, collection: &str, title: &str) -> Option<Value> {
    let page = client
        .call(
            "records.query",
            &json!({"collection": collection, "limit": 1000}),
            10,
        )
        .ok()??;
    page["records"]
        .as_array()?
        .iter()
        .find(|record| record["data"]["title"] == title)
        .cloned()
}

type Reply = mpsc::Receiver<Option<Value>>;

fn submit(client: &Client, operation: &str, args: &Value, version: u64) -> Result<Reply> {
    let body = json!({
        "command_id": random_id("cmd")?,
        "args": args,
        "expected_version": version,
        "wait_ms": 4000
    });
    let client = client.clone();
    let operation = operation.to_owned();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let reply = client
            .call_raw(&operation, &body, 5)
            .ok()
            .flatten()
            .map(|(_, value)| value);
        let _ = sender.send(reply);
    });
    Ok(receiver)
}
#[derive(Debug)]
enum Early {
    Pending,
    Done(Option<Value>),
}
fn early(reply: &Reply) -> Early {
    reply
        .recv_timeout(Duration::from_millis(500))
        .map_or(Early::Pending, Early::Done)
}
fn finish(reply: &Reply, early: Early) -> Option<Value> {
    match early {
        Early::Done(value) => value,
        Early::Pending => reply.recv_timeout(Duration::from_secs(4)).ok().flatten(),
    }
}

#[derive(Debug)]
struct Hold {
    gate: PathBuf,
    released: bool,
}
impl Hold {
    fn release(&mut self) -> Result<()> {
        if self.released {
            return Ok(());
        }
        self.released = true;
        let mut file = fs::OpenOptions::new().write(true).open(&self.gate)?;
        file.write_all(&[1])?;
        Ok(())
    }
}
fn hold(world: &World, wire: &mut Wire, capture: &Value) -> Result<Hold> {
    let home = &world.gateway_home;
    let gate = home.join("lab-command-capture-release");
    let reached = home.join("lab-command-capture-reached");
    let _ = fs::remove_file(&gate);
    let _ = fs::remove_file(&reached);
    let made = Command::new("mkfifo").arg(&gate).status()?;
    if !made.success() {
        return Err(invalid("mkfifo"));
    }
    fs::write(home.join("lab-command-capture-armed"), "armed")?;
    wire.send(&capture_frame(capture.clone()))
        .map_err(invalid)?;
    if !await_until(3, || reached.exists()) {
        let reply = wire.receive_until(Instant::now() + Duration::from_secs(1));
        return Err(invalid(format!("capture boundary not reached: {reply:?}")));
    }
    Ok(Hold {
        gate,
        released: false,
    })
}

fn observation(native: &str, title: &str) -> Value {
    json!({
        "kind": "upsert",
        "collection": "reminders.reminder",
        "native_key": native,
        "data": {"title": title, "completed": false,
            "list_id": {"$ref": {"collection": "reminders.list", "native_key": "list"}}},
        "observed_at": jiff::Timestamp::now().to_string()
    })
}

#[allow(clippy::too_many_lines)]
fn ordering(
    client: &Client,
    world: &World,
    wire: &mut Wire,
    generation: &str,
    predecessor: &str,
    evidence: &mut Evidence,
) -> Result<bool> {
    let list = record(client, "reminders.list", LIST);
    evidence.check(
        "causal_command_list_available",
        list.is_some() && !generation.is_empty(),
        "reminders list and checkpoint generation available",
    );
    let Some(list) = list.filter(|_| !generation.is_empty()) else {
        return Ok(false);
    };
    let title = format!("[ab-test {}]", random_id("ipc")?);
    let native = random_id("native")?;
    let create = submit(
        client,
        "reminders.create",
        &json!({"title": title, "list_id": list["id"]}),
        0,
    )?;
    let dispatched = (|| -> Result<Value> {
        let prepare = wire.frame("Prepare")?;
        wire.send(&json!({"t":"Prepared","command_id":prepare["command_id"],
            "ticket":random_id("ticket")?,"fingerprint":prepare["request_hash"]}))
            .map_err(invalid)?;
        wire.frame("Dispatch")?;
        Ok(prepare)
    })();
    evidence.check(
        "causal_create_dispatched",
        dispatched.is_ok(),
        "create prepared and dispatched",
    );
    let Ok(prepare) = dispatched else {
        return Ok(false);
    };
    let first_checkpoint = random_id("cp")?;
    let first = json!({"source":"reminders","generation":generation,"predecessor":predecessor,
        "checkpoint":first_checkpoint,"observations":[observation(&native, &title)]});
    let held = hold(world, wire, &first);
    evidence.check(
        "observed_capture_held_before_receipt",
        held.is_ok(),
        &held.as_ref().err().map_or_else(
            || "capture commit held at the gateway".to_owned(),
            ToString::to_string,
        ),
    );
    let Ok(mut release) = held else {
        return Ok(false);
    };
    let sent = wire
        .send(&json!({"t":"Outcome","command_id":prepare["command_id"],
            "disposition":"locally_observed",
            "evidence":{"native_key":native,"capture_checkpoint":first_checkpoint,
                "fingerprint":prepare["request_hash"],"observed":true}}))
        .map_err(invalid)
        .and_then(|_| wire.ping(1));
    evidence.check(
        "causal_control_progresses_before_receipt",
        sent.is_ok(),
        "control frames answered while the capture is held",
    );
    if sent.is_err() {
        return Ok(false);
    }
    let waiting = early(&create);
    evidence.check(
        "observed_reply_waits_for_capture_receipt",
        matches!(waiting, Early::Pending),
        "no command reply before the capture receipt",
    );
    if release.release().is_err() || wire.frame("Committed").is_err() {
        return Ok(false);
    }
    let result = finish(&create, waiting);
    let observed = record(client, "reminders.reminder", &title);
    let ok = result.as_ref().is_some_and(|r| {
        r["error"].is_null()
            && r["state"] == "observed"
            && r["disposition"] == "locally_observed"
            && r["commit_seq"].as_u64().unwrap_or(0) > 0
            && r["observed_record_ids"].as_array().is_some_and(|ids| {
                ids.len() == 1 && observed.as_ref().is_some_and(|o| ids[0] == o["id"])
            })
    });
    evidence.check(
        "cited_capture_produces_observed_commit_and_record_id",
        ok,
        "cited capture yields an observed commit and record id",
    );
    let Some(observed) = observed else {
        return Ok(false);
    };
    let version = observed["version"].as_u64().unwrap_or(0);
    let update = submit(
        client,
        "reminders.update",
        &json!({"id": observed["id"], "title": format!("{title} requested")}),
        version,
    )?;
    let prepared = wire.frame("Prepare");
    evidence.check(
        "causal_update_prepared",
        prepared.is_ok(),
        "update prepared",
    );
    let Ok(prepare) = prepared else {
        return Ok(false);
    };
    let second_checkpoint = random_id("cp")?;
    let second = json!({"source":"reminders","generation":generation,"predecessor":first_checkpoint,
        "checkpoint":second_checkpoint,
        "observations":[observation(&native, &format!("{title} external"))]});
    let held = hold(world, wire, &second);
    evidence.check(
        "conflict_capture_held_before_receipt",
        held.is_ok(),
        "conflicting capture held at the gateway",
    );
    let Ok(mut release) = held else {
        return Ok(false);
    };
    let sent = wire
        .send(&json!({"t":"Rejected","command_id":prepare["command_id"],
            "disposition":"rejected_before_dispatch",
            "error":{"code":"conflict","message":"state conflict","retryable":false},
            "evidence":{"native_key":native,"capture_checkpoint":second_checkpoint}}))
        .map_err(invalid)
        .and_then(|_| wire.ping(1));
    if sent.is_err() {
        evidence.check(
            "conflict_control_progresses_before_receipt",
            false,
            "control frames answered while the capture is held",
        );
        return Ok(false);
    }
    let waiting = early(&update);
    evidence.check(
        "conflict_reply_waits_for_capture_receipt",
        matches!(waiting, Early::Pending),
        "no conflict reply before the capture receipt",
    );
    if release.release().is_err() || wire.frame("Committed").is_err() {
        return Ok(false);
    }
    let result = finish(&update, waiting);
    let problem = result.as_ref().map(|r| r["error"].clone());
    let conflict = problem.as_ref().is_some_and(|p| {
        p["code"] == "conflict"
            && p["current"]["id"] == observed["id"]
            && p["current"]["version"].as_u64().unwrap_or(0) > version
            && p["current"]["data"]["title"] == format!("{title} external")
    });
    evidence.check(
        "cited_capture_returns_conflict_with_current",
        conflict,
        "cited capture returns a conflict carrying the current record",
    );
    Ok(true)
}

fn credit_exhausted(wire: &mut Wire, outstanding: usize) -> bool {
    let frame = capture_frame(json!({"source":"reminders","generation":"probe",
        "predecessor":"","checkpoint":"queued","observations":[{"kind":"upsert",
        "collection":"reminders.reminder","native_key":"probe",
        "data":{"title":"x".repeat(64 << 10)},"observed_at":ZERO_TIME}]}));
    let Ok(encoded) = lab_wire::encode_json(&frame).and_then(|b| lab_wire::frame_bytes(&b)) else {
        return false;
    };
    let mut outstanding = outstanding;
    while outstanding + encoded.len() <= BYTE_CREDIT {
        match wire.send(&frame) {
            Ok(n) => outstanding += n,
            Err(_) => return false,
        }
    }
    let mut error = wire.send(&frame).err();
    let deadline = Instant::now() + Duration::from_secs(1);
    while error.is_none() {
        match wire.receive_until(deadline) {
            Ok(reply) if kind(&reply) == "Ping" => error = wire.send(&json!({"t":"Pong"})).err(),
            Ok(_) => return false,
            Err(e) => error = Some(e),
        }
    }
    error.is_some_and(|e| !timed_out(&e))
}

fn blob_progress(wire: &mut Wire, evidence: &mut Evidence) -> bool {
    use sha2::{Digest, Sha256};
    let payload = vec![1u8; 128 << 10];
    let digest = hex::encode(Sha256::digest(&payload));
    let _ = SockRef::from(wire.stream()).set_send_buffer_size(4096);
    let _ = wire
        .stream()
        .set_write_timeout(Some(Duration::from_secs(1)));
    let started = Instant::now();
    let frame = json!({"t":"BlobChunk","handle":"ipc-probe","digest":digest,
        "bytes":STANDARD.encode(&payload),"eof":true});
    let written = lab_wire::send(&mut wire.stream(), &frame);
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    evidence.check(
        "blob_reply_progresses_while_capture_commit_is_held",
        written.is_ok() && elapsed < 1000.0,
        "blob reply written promptly while the capture commit is held",
    );
    written.is_ok()
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("isolated_gateway_started", true, "gateway ready");
    world.control("/seed", &json!({"count": 0}))?;
    let client = Client::new(world, GATEWAY_SCOPE)?;
    let seeded = await_until(3, || record(&client, "reminders.list", LIST).is_some());
    evidence.check(
        "command_fixture_seed_ready",
        seeded,
        "command fixture list seeded",
    );
    if !seeded {
        return Ok(());
    }
    world.stop_helper();
    let Some(socket) = world.runtime["socket"].as_str().map(str::to_owned) else {
        evidence.check("gateway_configuration_read", false, "runtime socket");
        return Ok(());
    };
    let Ok(secret) = fs::read(world.gateway_home.join("apple-secret")) else {
        evidence.check("helper_authentication_secret_read", false, "helper secret");
        return Ok(());
    };
    let Ok(mut wire) = Wire::connect(&socket) else {
        evidence.check("helper_socket_connected", false, "helper socket");
        return Ok(());
    };
    if wire.authenticate(&secret).is_err() {
        evidence.check("helper_authenticated", false, "helper authentication");
        return Ok(());
    }
    let opened = wire.receive();
    let (open, checkpoint) = match &opened {
        Ok((frame, _)) => (frame.clone(), frame["checkpoints"]["reminders"].clone()),
        Err(_) => (Value::Null, Value::Null),
    };
    evidence.check(
        "helper_authenticated_with_bounded_credit",
        opened.is_ok() && kind(&open) == "Open" && open["byte_credit"] == BYTE_CREDIT,
        "gateway opened the session with bounded byte credit",
    );
    if opened.is_err() {
        return Ok(());
    }
    let ready = wire
        .send(&json!({"t":"Ready","protocol":"ipc/1",
            "coverage":{"reminders":{"fields":"complete","missing":null}},
            "capabilities":{"reminders":["reminders.create","reminders.update"]}}))
        .map_err(invalid)
        .and_then(|_| wire.ping(5));
    evidence.check(
        "helper_ready_before_capture_hold",
        ready.is_ok(),
        "helper ready and answering before the capture hold",
    );
    if ready.is_err() {
        return Ok(());
    }
    let generation = checkpoint["generation"].as_str().unwrap_or_default();
    let predecessor = checkpoint["checkpoint"].as_str().unwrap_or_default();
    if !ordering(&client, world, &mut wire, generation, predecessor, evidence)? {
        return Ok(());
    }
    fs::write(world.gateway_home.join("lab-capture-armed"), "armed")?;
    let sent = wire.send(&capture_frame(json!({"source":"reminders",
        "generation":random_id("generation")?,"predecessor":"",
        "checkpoint":random_id("checkpoint")?,"observations":null,"live":true})));
    let reached = world.gateway_home.join("lab-capture-reached");
    let held = sent.is_ok() && await_until(3, || reached.exists());
    evidence.check(
        "capture_commit_held_before_acknowledgment",
        held,
        "capture commit held before acknowledgment",
    );
    let Ok(captured) = sent else {
        return Ok(());
    };
    if !held || !blob_progress(&mut wire, evidence) {
        return Ok(());
    }
    let started = Instant::now();
    let control = wire.ping(1);
    evidence.check(
        "control_progresses_without_early_capture_ack",
        control.is_ok() && started.elapsed().as_secs_f64() * 1000.0 < 1000.0,
        "control round trip without an early capture acknowledgment",
    );
    if control.is_ok() {
        evidence.check(
            "capture_credit_exhaustion_closes_session",
            credit_exhausted(&mut wire, captured),
            "exceeding byte credit closes the session",
        );
    }
    Ok(())
}

pub fn cases(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "HELPER-IPC-RESPONSIVE" => run(world, evidence),
        other => Err(Error::Invalid(format!("helper_ipc not ported for {other}"))),
    }
}
