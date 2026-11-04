#![allow(clippy::unwrap_used, clippy::too_many_lines)]
use lab_adapters::replicas;
use lab_probe::{Common, Local, new_id, records_digest};
use lab_world::{World, WorldConfig};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SCOPE: &str = "reminders:read,reminders:write,contacts:read,contacts:write,system:read";

struct Probe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}
impl Probe {
    fn spawn(binary: &PathBuf, action: &str, input: &Value) -> Self {
        let mut child = Command::new(binary)
            .arg(action)
            .env("APPLE_BRIDGE_SECRET_STORE", "file")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut probe = Self {
            child,
            stdin,
            stdout,
        };
        probe.send(input);
        probe
    }
    fn send(&mut self, value: &Value) {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        assert!(value["error"].is_null(), "probe failed: {value}");
        value
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn snapshot(local: &Local, collection: &str) -> (usize, String) {
    let (records, _) = local.snapshot(collection).unwrap();
    (records.len(), records_digest(&records))
}
fn wait<F: FnMut() -> bool>(seconds: u64, mut done: F) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !done() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(50));
    }
}
fn path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} is required")))
}

#[test]
#[ignore = "needs AB_LAB_GATEWAY, AB_LAB_SIMAPPLE and AB_LAB_PROBE_BIN"]
fn probe_actions_work_against_a_real_gateway_and_replica() {
    let (gateway, simapple, probe) = (
        path("AB_LAB_GATEWAY"),
        path("AB_LAB_SIMAPPLE"),
        path("AB_LAB_PROBE_BIN"),
    );
    let directory = std::env::temp_dir().join(format!("probe-gateway-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let world = World::start(WorldConfig {
        gateway: gateway.clone(),
        simapple,
        directory: directory.clone(),
        scope: SCOPE.into(),
    })
    .unwrap();
    replicas::configure(&gateway);
    replicas::start_with(&world, 1, SCOPE, 0).unwrap();
    let endpoint = replicas::endpoints().remove(0);
    let replica = Common {
        url: format!("http://{}", endpoint.address),
        host: endpoint.address.clone(),
        token: endpoint.token.clone(),
        home: endpoint.home.to_string_lossy().into_owned(),
        collection: "reminders.reminder".into(),
        proxy_ready: directory.join("ready.json").to_string_lossy().into_owned(),
    };
    let common = |extra: Value| {
        let mut base = json!({"URL": replica.url, "Host": replica.host, "Token": replica.token, "Home": replica.home,
            "Collection": replica.collection, "ProxyReady": replica.proxy_ready, "Bridge": gateway.to_string_lossy()});
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        base
    };
    std::fs::write(
        &replica.proxy_ready,
        json!({"url": "http://127.0.0.1:1", "connections": 2}).to_string(),
    )
    .unwrap();
    let replica_api = Local::new(&replica).unwrap();
    let gateway_token = world
        .gateway_cli(&["token", "create", "--scope", SCOPE])
        .unwrap();
    let gateway_address = world.runtime["api_address"].as_str().unwrap().to_owned();
    let gateway_api = Local::new(&Common {
        url: format!("http://{gateway_address}"),
        host: gateway_address,
        token: gateway_token.trim().to_owned(),
        ..Common::default()
    })
    .unwrap();

    if replica_api
        .named("reminders.list", "Bridge Test")
        .unwrap()
        .is_empty()
    {
        replica_api
            .op("reminders.create_list", &json!({"command_id": new_id("cmd"), "args": {"title": "Bridge Test"}, "wait_ms": 15000}))
            .unwrap();
    }
    wait(20, || {
        replica_api
            .named("reminders.list", "Bridge Test")
            .unwrap()
            .len()
            == 1
    });

    let mut query = Probe::spawn(&probe, "query", &common(json!({})));
    let queried = query.read();
    let (count, digest) = snapshot(&gateway_api, "reminders.reminder");
    assert_eq!(queried["count"], count, "{queried}");
    assert_eq!(queried["digest"], digest);
    assert_eq!(queried["status"]["state"], "connected");
    assert!(queried["enrollment"]["replica_id"].as_str().is_some());
    assert_eq!(queried["public_connections"], 2);
    println!("query ok: {count} records, digest {digest}");

    let title = format!("[ab-test {}] forwarded reminder", new_id(""));
    let mut command = Probe::spawn(&probe, "command", &common(json!({"Title": title})));
    assert_eq!(command.read(), json!({"ready": true}));
    wait(10, || {
        gateway_api
            .named("reminders.reminder", &title)
            .unwrap()
            .len()
            == 1
    });
    command.send(&json!({"truth_verified": true}));
    let done = command.read();
    assert_eq!(done["delete_read_your_write"], true, "{done}");
    wait(10, || {
        gateway_api
            .named("reminders.reminder", &title)
            .unwrap()
            .is_empty()
    });
    println!("command ok");

    let postburst_title = format!("[ab-test {}] post-burst delete", new_id(""));
    let delete_id = new_id("cmd");
    let mut burst = Probe::spawn(
        &probe,
        "postburst",
        &common(json!({"Title": postburst_title, "DeleteID": delete_id})),
    );
    assert_eq!(burst.read(), json!({"ready": true}));
    burst.send(&json!({"burst_finished": true}));
    let measured = burst.read();
    assert_eq!(measured["observed"], true, "{measured}");
    assert_eq!(measured["local_absent"], true, "{measured}");
    println!("postburst ok: {measured}");

    let list = replica_api
        .named("reminders.list", "Bridge Test")
        .unwrap()
        .remove(0);
    let agent = |level: &str| {
        let output = Command::new(&gateway)
            .args([
                "agent",
                "create",
                "--name",
                &format!("probe {level}"),
                "--level",
                level,
                "--scope",
                "reminders:write",
                "--home",
            ])
            .arg(&endpoint.home)
            .env("APPLE_BRIDGE_SECRET_STORE", "file")
            .output()
            .unwrap();
        assert!(output.status.success());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let create = |token: &str, suffix: &str| {
        replica_api.with_token(token).post(
            "reminders.create",
            &json!({"command_id": new_id("cmd"), "op": "reminders.create", "args": {"list_id": list["id"], "title": format!("[ab-test agent] {suffix}")}, "wait_ms": 15000}),
        )
    };
    let (read_status, _) = create(&agent("read"), "read").unwrap();
    assert_eq!(read_status, 401);
    let (full_status, full) = create(&agent("full"), "full").unwrap();
    assert_eq!(
        (full_status, full["state"].as_str()),
        (200, Some("observed")),
        "{full}"
    );
    let (approve_status, approve) = create(&agent("approve"), "approve").unwrap();
    assert_eq!(
        (approve_status, approve["state"].as_str()),
        (200, Some("awaiting_approval")),
        "{approve}"
    );
    println!("agent levels ok");

    let prefix = format!("[ab-test live {}]", new_id(""));
    let mut live = Probe::spawn(&probe, "live", &common(json!({"Prefix": prefix})));
    assert_eq!(live.read(), json!({"ready": true}));
    let mut effects = Vec::new();
    for index in 0..3 {
        let title = format!("{prefix} {index}");
        let wall = jiff::Timestamp::now().as_nanosecond();
        gateway_api
            .op("reminders.create", &json!({"command_id": new_id("cmd"), "args": {"list_id": list["id"], "title": title}, "wait_ms": 15000}))
            .unwrap();
        effects.push(json!({"title": title, "wall_ns": i64::try_from(wall).unwrap()}));
    }
    live.send(&Value::Array(effects));
    let latencies = live.read();
    assert_eq!(
        latencies["latencies_ms"].as_array().unwrap().len(),
        3,
        "{latencies}"
    );
    println!("live ok: {latencies}");

    let tag = format!("[ab-test {}]", new_id(""));
    let mut contacts = Probe::spawn(&probe, "contacts-crud", &common(json!({"Tag": tag})));
    let crud = contacts.read();
    assert_eq!(crud["values"].as_array().unwrap().len(), 6, "{crud}");
    println!("contacts-crud ok: {crud}");
    replicas::stop_all();
    let _ = std::fs::remove_dir_all(&directory);
}
