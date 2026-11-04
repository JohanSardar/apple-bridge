use crate::{
    assertions::await_until,
    core_frames::{self, pinned_client},
    journey::{command, effect_count, gateway_client},
    labcore::{self, change},
    replicas::{self, Endpoint},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

static CODE: Mutex<String> = Mutex::new(String::new());

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
fn free_address() -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.to_string())
}

pub fn start(world: &mut World, count: usize) -> Result<()> {
    if count != 1 {
        return Err(Error::Invalid(
            "gateway dials world needs one replica".into(),
        ));
    }
    let listen = free_address()?;
    world.install_proxy_to(&listen)?;
    let proxy = world.proxy_address()?;
    let peer = world.peer_address()?;
    let home = world.directory.join("replica0");
    let code = world.gateway_cli(&[
        "pair",
        "--peer-addr",
        &peer,
        "--name",
        "replica0",
        "--scope",
        replicas::SCOPE,
        "--dial-replica",
        &proxy,
    ])?;
    let code = code.trim().to_owned();
    let binary = replicas::binary()?;
    let mut joining = Command::new(binary)
        .args(["join", &code, "--listen", &listen, "--home"])
        .arg(&home)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = joining.try_wait()? {
            if status.success() {
                break;
            }
            return Err(Error::Invalid("dial join failed".into()));
        }
        if Instant::now() >= deadline {
            let _ = joining.kill();
            let _ = joining.wait();
            return Err(Error::Invalid("dial join timed out".into()));
        }
        thread::sleep(Duration::from_millis(50));
    }
    *lock(&CODE) = code;
    let token = replicas::cli(&home, &["token", "create", "--scope", replicas::SCOPE])?;
    let log = fs::File::create(world.directory.join("replica0.log"))?;
    let child = Command::new(binary)
        .args(["start", "--role", "replica", "--home"])
        .arg(&home)
        .args([
            "--api-addr",
            "127.0.0.1:0",
            "--blob-cap",
            "0",
            "--listen",
            &listen,
        ])
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let address = replicas::await_runtime(&home)?;
    let endpoint = Endpoint {
        home,
        address,
        token,
        scope: replicas::SCOPE.to_owned(),
    };
    replicas::adopt(child, endpoint.clone());
    if !await_until(15, || {
        replicas::status(&endpoint).is_ok_and(|v| v["replica"]["state"] == "connected")
    }) {
        return Err(Error::Invalid("replica not connected".into()));
    }
    Ok(())
}

fn replica() -> Result<Endpoint> {
    replicas::endpoints()
        .into_iter()
        .next()
        .ok_or_else(|| Error::Invalid("replica missing".into()))
}
fn replica_state(endpoint: &Endpoint) -> Value {
    replicas::status(endpoint).map_or(Value::Null, |v| v["replica"].clone())
}
fn connected(endpoint: &Endpoint) -> bool {
    replica_state(endpoint)["state"] == "connected"
}

pub fn changes(world: &mut World, input: &Value, _: &mut Evidence) -> Result<()> {
    let count = input["count"].as_u64().unwrap_or(200);
    let rate = input["per_second"].as_u64().unwrap_or(5);
    if count < 1 || rate < 1 {
        return Err(Error::Invalid("change_parameters".into()));
    }
    labcore::with_run(|run| {
        run.forget_effect("probe");
        let step = Duration::from_secs(1) / u32::try_from(rate).unwrap_or(1);
        let mut next = Instant::now();
        for i in 0..count {
            change(world, run, &format!("live-{i:03}"), true)?;
            next += step;
            thread::sleep(next.saturating_duration_since(Instant::now()));
        }
        Ok(())
    })
}

fn unavailable(reply: Option<&(u16, Value)>) -> bool {
    reply.is_some_and(|(status, value)| {
        value["error"]["code"] == "unavailable" || (*status != 200 && value["error"].is_null())
    })
}
fn locally_observed(reply: Option<&(u16, Value)>) -> bool {
    reply.is_some_and(|(_, value)| {
        value["error"].is_null() && value["disposition"] == "locally_observed"
    })
}
fn effects(world: &World, id: &str) -> Option<u64> {
    effect_count(world, id)
}

fn pairing_replay_rejected(endpoint: &Endpoint) -> bool {
    let Ok(binary) = replicas::binary() else {
        return false;
    };
    let code = lock(&CODE).clone();
    let listen = fs::read_to_string(endpoint.home.join("credentials.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v["enrollment"]["listen"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "127.0.0.1:0".to_owned());
    Command::new(binary)
        .args(["join", &code, "--listen", &listen, "--home"])
        .arg(&endpoint.home)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .output()
        .is_ok_and(|output| {
            !output.status.success()
                && String::from_utf8_lossy(&output.stderr)
                    .to_lowercase()
                    .contains("unauthorized")
        })
}

fn protocol_call(
    client: &crate::http::Client,
    mcp: bool,
    op: &str,
    input: &Value,
) -> Result<Option<(u16, Value)>> {
    if mcp {
        client.call_mcp_raw(op, input, 20)
    } else {
        client.call_raw(op, input, 20)
    }
}
fn feed_connected(endpoint: &Endpoint) -> bool {
    await_until(15, || connected(endpoint))
}

fn round(
    world: &World,
    endpoint: &Endpoint,
    list: &str,
    protocol: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let client = endpoint.client();
    let mcp = protocol == "mcp";
    let make = || {
        command(
            "reminders.create",
            &json!({"list_id": list, "title": format!("[ab-test] gateway-dials-{protocol}")}),
        )
    };
    let first_command = make()?;
    let op = first_command["op"].as_str().unwrap_or_default().to_owned();
    let first = protocol_call(&client, mcp, &op, &first_command)?;
    let first_id = first_command["command_id"].as_str().unwrap_or_default();
    evidence.check(
        &format!("feed_command_{protocol}"),
        locally_observed(first.as_ref()) && effects(world, first_id) == Some(1),
        "command over the gateway dialed feed took one effect",
    );
    evidence.check(
        &format!("drop_feed_response_{protocol}"),
        fs::write(world.gateway_home.join("drop-command-response"), "once").is_ok(),
        "the next feed command response is dropped",
    );
    let dropped = make()?;
    let dropped_id = dropped["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let ignored = protocol_call(&client, mcp, &op, &dropped)?;
    evidence.check(
        &format!("feed_response_lost_{protocol}"),
        unavailable(ignored.as_ref()),
        "the dropped response surfaces as unavailable",
    );
    evidence.check(
        &format!("feed_reconnect_{protocol}"),
        feed_connected(endpoint),
        "the feed reconnects",
    );
    let retry = protocol_call(&client, mcp, &op, &dropped)?;
    evidence.check(
        &format!("original_id_retry_one_effect_{protocol}"),
        locally_observed(retry.as_ref())
            && retry
                .as_ref()
                .is_some_and(|(_, v)| v["command"]["command_id"] == dropped_id.as_str())
            && effects(world, &dropped_id) == Some(1),
        "retry with the original id takes exactly one effect",
    );
    let status = protocol_call(
        &client,
        mcp,
        "commands.status",
        &json!({"command_id": dropped_id}),
    )?;
    evidence.check(
        &format!("feed_status_{protocol}"),
        status.as_ref().is_some_and(|(_, v)| {
            v["error"].is_null()
                && v["command"]["command_id"] == dropped_id.as_str()
                && Some(&v["disposition"]) == retry.as_ref().map(|(_, r)| &r["disposition"])
        }),
        "status by the original id matches the retry",
    );
    world.partition()?;
    evidence.check(
        &format!("feed_disconnect_{protocol}"),
        await_until(5, || replica_state(endpoint)["state"] == "disconnected"),
        "partitioning disconnects the feed",
    );
    let offline = make()?;
    let offline_id = offline["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let started = Instant::now();
    let reply = protocol_call(&client, mcp, &op, &offline)?;
    evidence.check(
        &format!("feed_offline_fail_fast_{protocol}"),
        unavailable(reply.as_ref())
            && started.elapsed() < Duration::from_secs(1)
            && effects(world, &offline_id) == Some(0),
        "an offline command fails fast without an effect",
    );
    world.heal()?;
    evidence.check(
        &format!("feed_heal_{protocol}"),
        feed_connected(endpoint),
        "healing reconnects the feed",
    );
    Ok(())
}

pub fn commands(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let endpoint = replica()?;
    let gateway_replay = pairing_replay_rejected(&endpoint);
    evidence.check(
        "dial_pairing_code_replay_rejected",
        gateway_replay,
        "a consumed dial pairing code is refused",
    );
    let list = crate::journey::record_set(&endpoint.client(), "reminders.list")
        .ok()
        .and_then(|lists| {
            lists
                .first()
                .and_then(|r| r["id"].as_str().map(str::to_owned))
        });
    let Some(list) = list else {
        evidence.check("command_fixture", false, "no reminders list on the replica");
        return Ok(());
    };
    for protocol in ["rest", "mcp"] {
        round(world, &endpoint, &list, protocol, evidence)?;
    }
    Ok(())
}

pub fn latency(_: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let limit = input["p95_ms"].as_f64().unwrap_or(1000.0);
    let samples = usize::try_from(input["samples"].as_u64().unwrap_or(200)).unwrap_or(0);
    let rows = labcore::with_run(|run| Ok(run.latencies()))?;
    for (name, values, timeouts) in rows {
        let mut sorted = values.clone();
        sorted.sort_by(f64::total_cmp);
        let p95 = sorted
            .get((sorted.len() * 95).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or(f64::MAX);
        evidence.check(
            &format!("latency_{name}"),
            values.len() >= samples && timeouts == 0 && p95 < limit,
            &format!(
                "samples={} timeouts={timeouts} p95_ms={p95:.1}",
                values.len()
            ),
        );
    }
    Ok(())
}

fn replica_identity(endpoint: &Endpoint) -> Option<String> {
    let text = fs::read_to_string(endpoint.home.join("credentials.json")).ok()?;
    let document: Value = serde_json::from_str(&text).ok()?;
    document["enrollment"]["replica_id"]
        .as_str()
        .map(str::to_owned)
}
fn dial_status(world: &World) -> Option<Vec<Value>> {
    let gateway = gateway_client(world).ok()?;
    let status = gateway.call("replica.status", &json!({}), 10).ok()??;
    status["replica"]["dial_replicas"].as_array().cloned()
}
fn http_status(stream: &mut core_frames::Tls, path: &str) -> Option<u16> {
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .ok()?;
    let mut head = [0_u8; 64];
    let mut filled = 0;
    while filled < 12 {
        let n = stream.read(&mut head[filled..]).ok()?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    std::str::from_utf8(&head[..filled])
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
fn unpaired_rejected(world: &World, endpoint: &Endpoint) -> bool {
    let Ok(pin) = core_frames::fingerprint(&endpoint.home) else {
        return false;
    };
    let Ok(proxy) = world.proxy_address() else {
        return false;
    };
    let live = core_frames::pinned_client(&proxy, &world.gateway_home, &pin)
        .ok()
        .and_then(|mut stream| http_status(&mut stream, "/v1/listener-probe"));
    if live != Some(404) {
        return false;
    }
    let home = world.directory.join("unpaired-dial");
    if replicas::cli(&home, &["token", "create", "--scope", "system:read"]).is_err() {
        return false;
    }
    let outcome = pinned_client(&proxy, &home, &pin).and_then(|stream| {
        tungstenite::client::client(format!("wss://{proxy}/v1/feed"), stream)
            .map(|_| ())
            .map_err(|e| Error::Invalid(e.to_string()))
    });
    let _ = fs::remove_dir_all(&home);
    outcome.is_err()
}

pub fn revoke(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    labcore::finish_hook(world, &Value::Null, evidence)?;
    let endpoint = replica()?;
    let before = replica_state(&endpoint);
    let revoked =
        replica_identity(&endpoint).is_some_and(|id| world.gateway_cli(&["revoke", &id]).is_ok());
    evidence.check(
        "revoke_gateway_dial_replica",
        revoked,
        "the gateway revoked the dialed replica",
    );
    evidence.check(
        "revoke_closes_feed",
        await_until(2, || replica_state(&endpoint)["state"] == "disconnected"),
        "revocation closes the feed",
    );
    let statuses = dial_status(world);
    evidence.check(
        "gateway_reports_revoked",
        statuses
            .as_ref()
            .is_some_and(|s| s.len() == 1 && s[0]["state"] == "revoked"),
        "the gateway reports the dial as revoked",
    );
    let attempts = statuses
        .as_ref()
        .and_then(|s| s.first())
        .and_then(|s| s["attempts"].as_u64());
    thread::sleep(Duration::from_secs(6));
    let later = dial_status(world);
    evidence.check(
        "revoked_gateway_stops_redial",
        later.as_ref().is_some_and(|s| {
            s.len() == 1 && s[0]["attempts"].as_u64() == attempts && s[0]["state"] == "revoked"
        }),
        "a revoked dial is not retried",
    );
    let after = replica_state(&endpoint);
    evidence.check(
        "revoked_no_reconnect",
        after["state"] == "disconnected" && before["resumes"] == after["resumes"],
        "the replica did not reconnect",
    );
    evidence.check(
        "unpaired_dial_certificate_rejected",
        unpaired_rejected(world, &endpoint),
        "an unpaired certificate cannot use the dial listener",
    );
    Ok(())
}
