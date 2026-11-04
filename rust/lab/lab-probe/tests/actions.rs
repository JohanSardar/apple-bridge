#![allow(
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::assigning_clones,
    clippy::assert_is_empty,
    clippy::cloned_ref_to_slice_refs
)]
use lab_probe::{Session, run_action};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

struct Request {
    path: String,
    body: Value,
    authorization: String,
}
enum Reply {
    Json(u16, Value),
    Raw(String),
    Stream(String),
}
type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

fn read_request(stream: &mut TcpStream) -> Option<(String, Request)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_owned();
    let (mut length, mut authorization) = (0, String::new());
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        if header == "\r\n" {
            break;
        }
        let lower = header.to_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().ok()?;
        }
        if lower.starts_with("authorization:") {
            authorization = header["authorization:".len()..].trim().to_owned();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some((
        line,
        Request {
            path,
            body: serde_json::from_slice(&body).unwrap_or(Value::Null),
            authorization,
        },
    ))
}
fn serve(handler: Arc<Handler>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            thread::spawn(move || {
                let mut stream = stream;
                let Some((_, request)) = read_request(&mut stream) else {
                    return;
                };
                match handler(&request) {
                    Reply::Json(status, body) => {
                        let text = body.to_string();
                        let _ = write!(
                            stream,
                            "HTTP/1.0 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}",
                            text.len()
                        );
                    }
                    Reply::Raw(text) => {
                        let _ = stream.write_all(text.as_bytes());
                    }
                    Reply::Stream(text) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.0 200 OK\r\nContent-Type: text/event-stream\r\n\r\n{text}"
                        );
                        let _ = stream.flush();
                        thread::sleep(Duration::from_secs(30));
                    }
                }
            });
        }
    });
    address
}
fn ok(value: Value) -> Reply {
    Reply::Json(200, value)
}
fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("lab-probe-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn lines(inputs: &[Value]) -> Vec<u8> {
    inputs
        .iter()
        .flat_map(|value| format!("{value}\n").into_bytes())
        .collect()
}
fn run(action: &str, inputs: &[Value]) -> (Result<(), String>, Vec<Value>) {
    let mut session = Session::new(Cursor::new(lines(inputs)), Vec::new());
    let result = run_action(action, &mut session).map_err(|error| error.to_string());
    let output = String::from_utf8(session.into_output()).unwrap();
    let values = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (result, values)
}
fn base(address: &str, home: &Path) -> Value {
    json!({
        "URL": format!("http://{address}"),
        "Host": address,
        "Token": "replica-token",
        "Home": home.to_string_lossy(),
        "ProxyReady": home.join("ready.json").to_string_lossy(),
    })
}
fn cursor(sequence: u64) -> Value {
    json!({"epoch": "epoch-1", "subscription_revision": 1, "sequence": sequence})
}
fn operation(request: &Request) -> &str {
    request.path.strip_prefix("/v1/op/").unwrap_or_default()
}
fn record(id: &str, title: &str, version: u64) -> Value {
    json!({"collection": "reminders.reminder", "id": id, "version": version, "deleted": false, "data": {"title": title}})
}

#[test]
fn query_pages_a_stable_snapshot_and_reports_replica_state() {
    let home = temp("query");
    std::fs::write(
        home.join("credentials.json"),
        json!({"enrollment": {"remote": true, "address": "https://relay.example", "replica_id": "replica:1"}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        home.join("ready.json"),
        json!({"url": "http://127.0.0.1:1", "connections": 3}).to_string(),
    )
    .unwrap();
    let drift = Arc::new(Mutex::new(false));
    let flag = drift.clone();
    let address = serve(Arc::new(move |request| match operation(request) {
        "records.query" if request.body["after"].is_null() => {
            ok(json!({"records": [record("a", "A", 1)], "cursor": cursor(5), "next": "p2"}))
        }
        "records.query" => {
            let moved = if *flag.lock().unwrap() { 6 } else { 5 };
            ok(json!({"records": [record("b", "B", 1)], "cursor": cursor(moved)}))
        }
        "sources.status" => {
            ok(json!({"replica": {"state": "connected", "snapshots": 1, "snapshot_pages": 2}}))
        }
        _ => Reply::Json(404, json!({})),
    }));
    let mut input = base(&address, &home);
    input["Collection"] = json!("reminders.reminder");
    let (result, output) = run("query", &[input.clone()]);
    result.unwrap();
    assert_eq!(output[0]["count"], 2);
    assert_eq!(output[0]["cursor"]["sequence"], 5);
    assert_eq!(output[0]["status"]["state"], "connected");
    assert_eq!(output[0]["enrollment"]["address"], "https://relay.example");
    assert_eq!(output[0]["public_connections"], 3);
    let expected = lab_probe::records_digest(&[record("a", "A", 1), record("b", "B", 1)]);
    assert_eq!(output[0]["digest"], expected);
    *drift.lock().unwrap() = true;
    let (result, output) = run("query", &[input]);
    assert_eq!(result.unwrap_err(), "snapshot changed during pagination");
    assert!(output.is_empty());
}

#[test]
fn command_creates_and_deletes_through_the_forwarding_replica() {
    let home = temp("command");
    let titles = Arc::new(Mutex::new(Vec::<(String, u64)>::new()));
    let state = titles.clone();
    let address = serve(Arc::new(move |request| {
        let mut titles = state.lock().unwrap();
        match operation(request) {
            "records.query" => {
                let collection = request.body["collection"].as_str().unwrap();
                let records: Vec<Value> = if collection == "reminders.list" {
                    vec![
                        json!({"collection": collection, "id": "list-1", "version": 1, "data": {"title": "Bridge Test"}}),
                    ]
                } else {
                    titles
                        .iter()
                        .map(|(title, version)| record("r-1", title, *version))
                        .collect()
                };
                ok(json!({"records": records, "cursor": cursor(1)}))
            }
            "reminders.create" => {
                assert_eq!(request.body["args"]["list_id"], "list-1");
                titles.push((
                    request.body["args"]["title"].as_str().unwrap().to_owned(),
                    7,
                ));
                ok(json!({"disposition": "locally_observed"}))
            }
            "reminders.delete" => {
                assert_eq!(request.body["expected_version"], 7);
                assert_eq!(request.body["args"]["id"], "r-1");
                titles.clear();
                ok(json!({"disposition": "locally_observed"}))
            }
            _ => Reply::Json(404, json!({})),
        }
    }));
    let mut input = base(&address, &home);
    input["Title"] = json!("forwarded reminder");
    let (result, output) = run("command", &[input, json!({"truth_verified": true})]);
    result.unwrap();
    assert_eq!(output[0], json!({"ready": true}));
    assert_eq!(
        output[1],
        json!({"create_observed": true, "create_read_your_write": true, "delete_observed": true, "delete_read_your_write": true})
    );
    assert!(titles.lock().unwrap().is_empty());
}

#[test]
fn blob_waits_for_the_source_then_checks_size_and_digest() {
    use sha2::{Digest, Sha256};
    let home = temp("blob");
    let bytes = b"blob bytes for the probe".to_vec();
    let digest = hex::encode(Sha256::digest(&bytes));
    let calls = Arc::new(Mutex::new(0));
    let (body, counter, wanted) = (bytes.clone(), calls.clone(), digest.clone());
    let address = serve(Arc::new(move |request| {
        if operation(request) == "records.query" {
            let mut calls = counter.lock().unwrap();
            *calls += 1;
            let records = if *calls < 3 {
                vec![]
            } else {
                vec![
                    json!({"collection": "system.source", "id": "src-1", "version": 1, "data": {"title": "stage"}}),
                ]
            };
            return ok(json!({"records": records, "cursor": cursor(1)}));
        }
        if request.path == format!("/v1/blobs/{wanted}?record=src-1") {
            let head = format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            return Reply::Raw(format!("{head}{}", String::from_utf8_lossy(&body)));
        }
        Reply::Json(404, json!({}))
    }));
    let mut input = base(&address, &home);
    input["Title"] = json!("stage");
    input["Digest"] = json!(digest);
    input["Size"] = json!(bytes.len());
    let (result, output) = run("blob", &[input.clone()]);
    result.unwrap();
    assert_eq!(
        output[0],
        json!({"bytes": bytes.len(), "digest_matches": true})
    );
    input["Size"] = json!(bytes.len() + 1);
    let (result, _) = run("blob", &[input]);
    assert_eq!(result.unwrap_err(), "blob bytes differ");
}

fn fake_bridge(directory: &Path) -> PathBuf {
    let script = directory.join("bridge");
    std::fs::write(
        &script,
        "#!/bin/sh\nlevel=''\nwhile [ $# -gt 0 ]; do [ \"$1\" = --level ] && level=$2; shift; done\nprintf '{\"agent\":{},\"token\":\"token-%s\"}' \"$level\"\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("755")
        .arg(&script)
        .status()
        .unwrap();
    script
}

#[test]
fn agents_enforce_the_three_levels_and_clean_up() {
    let home = temp("agents");
    let bridge = fake_bridge(&home);
    let reminders = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let approved = Arc::new(Mutex::new(false));
    let (state, gate) = (reminders.clone(), approved.clone());
    let address = serve(Arc::new(move |request| {
        let mut reminders = state.lock().unwrap();
        let token = request.authorization.clone();
        match operation(request) {
            "records.query" => {
                let collection = request.body["collection"].as_str().unwrap();
                let text = request.body["text"].as_str().unwrap_or_default();
                let records: Vec<Value> = if collection == "reminders.list" {
                    vec![
                        json!({"collection": collection, "id": "list-1", "version": 1, "data": {"title": "Bridge Test"}}),
                    ]
                } else {
                    reminders
                        .iter()
                        .filter(|(title, _)| title == text)
                        .map(|(title, id)| record(id, title, 2))
                        .collect()
                };
                ok(json!({"records": records, "cursor": cursor(1)}))
            }
            "reminders.create" if token == "Bearer token-read" => {
                Reply::Json(401, json!({"error": {"code": "unauthorized"}}))
            }
            "reminders.create" if token == "Bearer token-full" => {
                reminders.push((
                    request.body["args"]["title"].as_str().unwrap().to_owned(),
                    "id-full".into(),
                ));
                ok(json!({"state": "observed"}))
            }
            "reminders.create" => ok(json!({"state": "awaiting_approval"})),
            "commands.status" => {
                let mut approved = gate.lock().unwrap();
                if !*approved {
                    *approved = true;
                    reminders.push((format!("{} approve", "[ab-test x]"), "id-approve".into()));
                    return ok(json!({"state": "dispatched"}));
                }
                ok(json!({"state": "observed"}))
            }
            "reminders.delete" => {
                let id = request.body["args"]["id"].as_str().unwrap().to_owned();
                reminders.retain(|(_, existing)| *existing != id);
                ok(json!({"state": "observed"}))
            }
            _ => Reply::Json(404, json!({})),
        }
    }));
    let mut input = base(&address, &home);
    input["Prefix"] = json!("[ab-test x]");
    input["FullID"] = json!("cmd-full");
    input["ApproveID"] = json!("cmd-approve");
    input["Bridge"] = json!(bridge.to_string_lossy());
    let (result, output) = run("agents", &[input, json!({"consent_verified": true})]);
    result.unwrap();
    assert_eq!(output[0], json!({"ready": true}));
    assert_eq!(output[1]["read_denied"], true);
    assert_eq!(output[1]["full_observed"], true);
    assert_eq!(output[1]["approve_queued"], true);
    assert_eq!(output[1]["approved_observed"], true);
    assert_eq!(output[1]["cleanup_observed"], true);
    assert!(reminders.lock().unwrap().is_empty());
}

#[test]
fn postburst_measures_the_forwarded_delete_against_the_normal_ttl() {
    let home = temp("postburst");
    let bridge = fake_bridge(&home);
    let present = Arc::new(Mutex::new(false));
    let state = present.clone();
    let address = serve(Arc::new(move |request| {
        let mut present = state.lock().unwrap();
        match operation(request) {
            "records.query" => {
                let collection = request.body["collection"].as_str().unwrap();
                let records = if collection == "reminders.list" {
                    vec![
                        json!({"collection": collection, "id": "list-1", "version": 1, "data": {"title": "Bridge Test"}}),
                    ]
                } else if *present {
                    vec![record("r-1", "burst title", 3)]
                } else {
                    vec![]
                };
                ok(json!({"records": records, "cursor": cursor(1)}))
            }
            "reminders.create" => {
                *present = true;
                ok(json!({"state": "observed"}))
            }
            "reminders.delete" if request.body["command_id"] == "cmd-delete" => {
                *present = false;
                ok(json!({"state": "observed"}))
            }
            _ => Reply::Json(404, json!({})),
        }
    }));
    let mut input = base(&address, &home);
    input["Title"] = json!("burst title");
    input["DeleteID"] = json!("cmd-delete");
    input["Bridge"] = json!(bridge.to_string_lossy());
    let (result, output) = run("postburst", &[input, json!({"burst_finished": true})]);
    result.unwrap();
    let measured = &output[1];
    assert_eq!(measured["http_status"], 200);
    assert_eq!(measured["state"], "observed");
    assert_eq!(measured["observed"], true);
    assert_eq!(measured["local_absent"], true);
    assert_eq!(measured["normal_ttl_ms"], 300_000);
    assert_eq!(measured["command_id"], "cmd-delete");
}

#[test]
fn postburst_reports_an_unavailable_delete_without_failing_the_probe() {
    let home = temp("postburst-503");
    let bridge = fake_bridge(&home);
    let address = serve(Arc::new(move |request| match operation(request) {
        "records.query" => {
            let collection = request.body["collection"].as_str().unwrap();
            let records = if collection == "reminders.list" {
                vec![
                    json!({"collection": collection, "id": "list-1", "version": 1, "data": {"title": "Bridge Test"}}),
                ]
            } else {
                vec![record("r-1", "burst title", 3)]
            };
            ok(json!({"records": records, "cursor": cursor(1)}))
        }
        "reminders.create" => ok(json!({"state": "observed"})),
        _ => Reply::Json(503, json!({"error": {"code": "unavailable"}})),
    }));
    let mut input = base(&address, &home);
    input["Title"] = json!("burst title");
    input["DeleteID"] = json!("cmd-delete");
    input["Bridge"] = json!(bridge.to_string_lossy());
    let (result, output) = run("postburst", &[input, json!({})]);
    result.unwrap();
    assert_eq!(output[1]["initial_http_status"], 503);
    assert_eq!(output[1]["http_status"], 503);
    assert_eq!(output[1]["error_code"], "unavailable");
    assert_eq!(output[1]["observed"], false);
}

fn clock(home: &Path) {
    std::fs::write(
        home.join("gateway-clock.json"),
        json!({
            "t0": "2026-01-01T00:00:00.000Z", "t1": "2026-01-01T00:00:00.110Z",
            "t2": "2026-01-01T00:00:00.112Z", "t3": "2026-01-01T00:00:00.022Z",
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn live_converges_when_every_effect_title_arrives_on_the_feed() {
    let home = temp("live");
    clock(&home);
    let now = jiff::Timestamp::now().as_nanosecond();
    let address = serve(Arc::new(move |request| {
        if request.path.starts_with("/v1/changes?after=") {
            let batch = json!({"epoch": "epoch-1", "changes": [
                {"record": {"collection": "reminders.reminder", "data": {"title": "[ab-test] one"}}},
                {"record": {"collection": "reminders.reminder", "data": {"title": "[ab-test] two"}}},
                {"record": {"collection": "other", "data": {"title": "[ab-test] ignored"}}}]});
            return Reply::Stream(format!(": open\n\ndata: {batch}\n\n"));
        }
        ok(json!({"records": [], "cursor": cursor(4)}))
    }));
    let mut input = base(&address, &home);
    input["Collection"] = json!("reminders.reminder");
    input["Prefix"] = json!("[ab-test]");
    let wall = i64::try_from(now - 40_000_000).unwrap();
    let effects = json!([{"title": "[ab-test] one", "wall_ns": wall}, {"title": "[ab-test] two", "wall_ns": wall}]);
    let (result, output) = run("live", &[input.clone(), effects]);
    result.unwrap();
    let latencies = output[1]["latencies_ms"].as_array().unwrap();
    assert_eq!(latencies.len(), 2);
    assert!(latencies.iter().all(|v| v.as_f64().unwrap() >= 0.0));
    assert!((output[1]["clock_uncertainty_ms"].as_f64().unwrap() - 10.0).abs() < 1e-6);
    let (result, _) = run("live", &[input, json!([])]);
    assert_eq!(result.unwrap_err(), "no effects");
}

#[test]
fn contacts_crud_runs_rest_and_mcp_and_matches_each_write_to_the_feed() {
    let home = temp("contacts");
    clock(&home);
    let version = Arc::new(Mutex::new(0_u64));
    let state = Arc::new(Mutex::new(json!({})));
    let (counter, stored) = (version.clone(), state.clone());
    let address = serve(Arc::new(move |request| {
        let tag_name = |args: &Value| args["given_name"].clone();
        if request.path.starts_with("/v1/changes?after=") {
            let mut changes = Vec::new();
            for (id, job, deleted) in [
                ("c-rest", "Testjob", false),
                ("c-rest", "Updated Testjob", false),
                ("c-rest", "Updated Testjob", true),
                ("c-mcp", "Testjob", false),
                ("c-mcp", "Updated Testjob", false),
                ("c-mcp", "Updated Testjob", true),
            ] {
                let given = if id == "c-rest" {
                    "[ab-test] rest"
                } else {
                    "[ab-test] mcp"
                };
                changes.push(json!({"record": {"collection": "contacts.contact", "id": id, "deleted": deleted,
                    "data": {"given_name": given, "job_title": job, "organization": "Bridge Test"}}}));
            }
            return Reply::Stream(format!("data: {}\n\n", json!({"changes": changes})));
        }
        let command = |name: &str, args: &Value, version: u64| -> Value {
            let mut stored = stored.lock().unwrap();
            let id = if args["id"].is_string() {
                args["id"].as_str().unwrap().to_owned()
            } else if stored["next"] == "mcp" {
                "c-mcp".into()
            } else {
                "c-rest".into()
            };
            let mut data = stored[&id].clone();
            if data.is_null() {
                data = json!({"given_name": tag_name(args), "job_title": "Testjob", "organization": "Bridge Test"});
            }
            let mut deleted = false;
            match name {
                "contacts.update" => data["job_title"] = args["job_title"].clone(),
                "contacts.delete" => deleted = true,
                _ => {}
            }
            stored[&id] = data.clone();
            stored["last"] =
                json!({"id": id, "version": version, "deleted": deleted, "data": data});
            json!({"disposition": "locally_observed", "observed_record_ids": [id]})
        };
        let mut counter = counter.lock().unwrap();
        match operation(request) {
            "records.query" => ok(json!({"records": [], "cursor": cursor(1)})),
            "records.get" => {
                let stored = stored.lock().unwrap();
                let id = request.body["id"].as_str().unwrap();
                ok(
                    json!({"records": [{"collection": "contacts.contact", "id": id, "version": *counter,
                    "deleted": stored["last"]["deleted"], "data": stored[id]}]}),
                )
            }
            name if name.starts_with("contacts.") => {
                *counter += 1;
                ok(command(name, &request.body["args"], *counter))
            }
            _ if request.path == "/mcp" => {
                *counter += 1;
                let name = request.body["params"]["name"]
                    .as_str()
                    .unwrap()
                    .replace('_', ".");
                stored.lock().unwrap()["next"] = json!("mcp");
                let result = command(
                    &name,
                    &request.body["params"]["arguments"]["args"],
                    *counter,
                );
                let envelope =
                    json!({"jsonrpc": "2.0", "id": 1, "result": {"structuredContent": result}});
                Reply::Raw(format!(
                    "HTTP/1.0 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: message\ndata: {envelope}\n\n"
                ))
            }
            _ => Reply::Json(404, json!({})),
        }
    }));
    let mut input = base(&address, &home);
    input["Tag"] = json!("[ab-test]");
    let (result, output) = run("contacts-crud", &[input]);
    result.unwrap();
    let values = output[0]["values"].as_array().unwrap();
    assert_eq!(values.len(), 6);
    let operations: Vec<&str> = values
        .iter()
        .map(|v| v["operation"].as_str().unwrap())
        .collect();
    assert_eq!(
        operations,
        [
            "contacts.create",
            "contacts.update",
            "contacts.delete",
            "contacts.create",
            "contacts.update",
            "contacts.delete"
        ]
    );
    assert!(
        values
            .iter()
            .all(|v| v["wall_ns"].as_i64().unwrap() > 0 && v["read_wall_ns"].as_i64().unwrap() > 0)
    );
    assert!((output[0]["uncertainty_ms"].as_f64().unwrap() - 10.0).abs() < 1e-6);
}
