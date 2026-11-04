use crate::{Common, Error, Local, Result, Session, failed, new_id, records_digest};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};

pub const LIST: &str = "Bridge Test";
const NORMAL_DELETE_TTL_MS: u64 = 300_000;

#[must_use]
pub fn public_roots() -> rustls::RootCertStore {
    rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())
}

pub fn run<R: Read, W: Write>(action: &str, session: &mut Session<R, W>) -> Result<()> {
    run_with_roots(action, session, &public_roots())
}

pub fn run_with_roots<R: Read, W: Write>(
    action: &str,
    session: &mut Session<R, W>,
    roots: &rustls::RootCertStore,
) -> Result<()> {
    match action {
        "query" => query(session),
        "blob" => blob(session),
        "command" => command(session),
        "postburst" => postburst(session),
        "agents" => crate::agents::run(session),
        "live" => crate::live::run(session),
        "contacts-crud" => crate::contacts::run(session),
        "replay" => crate::replica::replay(session, roots),
        "revocation" => crate::replica::revocation(session, roots),
        _ => Err(Error::Invalid("probe action")),
    }
}

pub fn text(value: &Value, name: &str) -> String {
    value[name].as_str().unwrap_or_default().to_owned()
}
pub fn common(value: &Value) -> Result<Common> {
    Ok(serde_json::from_value(value.clone())?)
}
pub fn read_json(path: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn sleep(milliseconds: u64) {
    std::thread::sleep(Duration::from_millis(milliseconds));
}
pub fn single(local: &Local, collection: &str, title: &str) -> Result<Option<Value>> {
    let mut found = local.named(collection, title)?;
    Ok(if found.len() == 1 { found.pop() } else { None })
}
pub fn disposition(body: &Value) -> &str {
    body["disposition"].as_str().unwrap_or_default()
}

fn query<R: Read, W: Write>(session: &mut Session<R, W>) -> Result<()> {
    let input = common(&session.input::<Value>()?)?;
    let local = Local::new(&input)?;
    let (records, cursor) = local.snapshot(&input.collection)?;
    let status = local.op("sources.status", &json!({}))?;
    let credentials = read_json(&format!("{}/credentials.json", input.home))?;
    let route = read_json(&input.proxy_ready)?;
    session.emit(&json!({
        "digest": records_digest(&records),
        "count": records.len(),
        "cursor": cursor,
        "status": status["replica"],
        "enrollment": credentials["enrollment"],
        "public_connections": route["connections"],
    }))
}

fn blob<R: Read, W: Write>(session: &mut Session<R, W>) -> Result<()> {
    let value = session.input::<Value>()?;
    let local = Local::new(&common(&value)?)?;
    let (title, digest) = (text(&value, "Title"), text(&value, "Digest"));
    let size = value["Size"].as_u64().unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(record) = single(&local, "system.source", &title)? {
            let path = format!("/v1/blobs/{digest}?record={}", text(&record, "id"));
            let reply = local.fetch("GET", &path, &[], size + 16384)?;
            if reply.status == 200 {
                let matches = u64::try_from(reply.body.len()).is_ok_and(|n| n == size)
                    && hex::encode(Sha256::digest(&reply.body)) == digest;
                if !matches {
                    return failed("blob bytes differ");
                }
                return session.emit(&json!({"bytes": reply.body.len(), "digest_matches": true}));
            }
        }
        if Instant::now() >= deadline {
            return failed("context deadline exceeded");
        }
        sleep(20);
    }
}

fn command<R: Read, W: Write>(session: &mut Session<R, W>) -> Result<()> {
    let value = session.input::<Value>()?;
    let local = Local::new(&common(&value)?)?;
    let title = text(&value, "Title");
    let Some(list) = single(&local, "reminders.list", LIST)? else {
        return failed("Bridge Test list ambiguous or unavailable");
    };
    let created = local.op(
        "reminders.create",
        &json!({"command_id": new_id("cmd"), "args": {"list_id": list["id"], "title": title}, "wait_ms": 15000}),
    )?;
    if disposition(&created) != "locally_observed" {
        return failed("create unobserved");
    }
    let Some(record) = single(&local, "reminders.reminder", &title)? else {
        return failed("create read-your-write failed");
    };
    session.ready()?;
    session.signal()?;
    let deleted = local.op(
        "reminders.delete",
        &json!({"command_id": new_id("cmd"), "args": {"id": record["id"]}, "expected_version": record["version"], "wait_ms": 15000}),
    )?;
    if disposition(&deleted) != "locally_observed" {
        return failed("delete unobserved");
    }
    if !local.named("reminders.reminder", &title)?.is_empty() {
        return failed("delete read-your-write failed");
    }
    session.emit(&json!({
        "create_observed": true,
        "create_read_your_write": true,
        "delete_observed": true,
        "delete_read_your_write": true,
    }))
}

struct Outcome {
    initial_http: u16,
    initial_ms: f64,
    record: Value,
    failure: Option<(u16, String)>,
}
fn attempt_delete(local: &Local, command: &Value, id: &str, ttl: Duration) -> (Outcome, Instant) {
    let started = Instant::now();
    let first = local.post("reminders.delete", command);
    let initial_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut outcome = match first {
        Ok((200, body)) => Outcome {
            initial_http: 200,
            initial_ms,
            record: body,
            failure: None,
        },
        Ok((status, body)) => Outcome {
            initial_http: status,
            initial_ms,
            record: Value::Null,
            failure: Some((status, text(&body["error"], "code"))),
        },
        Err(_) => Outcome {
            initial_http: 0,
            initial_ms,
            record: Value::Null,
            failure: Some((0, "transport_error".into())),
        },
    };
    let pending = |state: &str| matches!(state, "admitted" | "prepared" | "dispatched");
    while outcome.failure.is_none() && pending(outcome.record["state"].as_str().unwrap_or_default())
    {
        if started.elapsed() >= ttl {
            outcome.failure = Some((0, "transport_error".into()));
            break;
        }
        sleep(1000);
        match local.post("commands.status", &json!({"command_id": id})) {
            Ok((200, body)) => outcome.record = body,
            Ok((status, body)) => outcome.failure = Some((status, text(&body["error"], "code"))),
            Err(_) => outcome.failure = Some((0, "transport_error".into())),
        }
    }
    (outcome, started)
}

fn postburst<R: Read, W: Write>(session: &mut Session<R, W>) -> Result<()> {
    let value = session.input::<Value>()?;
    let input = common(&value)?;
    let (title, delete_id) = (text(&value, "Title"), text(&value, "DeleteID"));
    let token = crate::agents::create(&value, &input, "Bridge Test post-burst full", "full")?;
    let local = Local::new(&input)?.with_token(&token);
    let Some(list) = single(&local, "reminders.list", LIST)? else {
        return failed("post-burst list unavailable");
    };
    let created = local.op(
        "reminders.create",
        &json!({"command_id": new_id("cmd"), "op": "reminders.create", "args": {"list_id": list["id"], "title": title}, "wait_ms": 15000}),
    )?;
    if created["state"] != "observed" || single(&local, "reminders.reminder", &title)?.is_none() {
        return failed("post-burst fixture not observed");
    }
    session.ready()?;
    session.signal()?;
    let Some(target) = single(&local, "reminders.reminder", &title)? else {
        return failed("post-burst fixture unavailable before delete");
    };
    let ttl = Duration::from_millis(NORMAL_DELETE_TTL_MS);
    let command = json!({"command_id": delete_id, "op": "reminders.delete", "args": {"id": target["id"]}, "expected_version": target["version"], "wait_ms": 15000});
    let (outcome, started) = attempt_delete(&local, &command, &delete_id, ttl);
    let elapsed = started.elapsed();
    let state = text(&outcome.record, "state");
    let observed = outcome.failure.is_none() && state == "observed" && elapsed <= ttl;
    let (http, code) = match &outcome.failure {
        Some((status, code)) => (*status, code.clone()),
        None => (200, text(&outcome.record["error"], "code")),
    };
    let absent = observed && local.named("reminders.reminder", &title)?.is_empty();
    session.emit(&json!({
        "initial_http_status": outcome.initial_http,
        "http_status": http,
        "error_code": code,
        "state": state,
        "initial_response_ms": outcome.initial_ms,
        "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
        "normal_ttl_ms": NORMAL_DELETE_TTL_MS,
        "observed": observed,
        "local_absent": absent,
        "command_id": delete_id,
    }))
}
