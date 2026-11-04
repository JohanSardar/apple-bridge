use crate::{
    Common, Local, Result, Session,
    actions::{LIST, common, single, sleep, text},
    failed, new_id,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

const SCOPE: &str = "reminders:write";

fn bridge(value: &Value) -> PathBuf {
    let named = text(value, "Bridge");
    if !named.is_empty() {
        return PathBuf::from(named);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("bridge-linux-amd64")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("bridge"))
}

pub fn create(value: &Value, input: &Common, name: &str, level: &str) -> Result<String> {
    let output = Command::new(bridge(value))
        .args(["agent", "create", "--name", name, "--level", level])
        .args(["--scope", SCOPE, "--home", &input.home])
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .output()?;
    if !output.status.success() {
        return failed("agent creation refused");
    }
    let created: Value = serde_json::from_slice(&output.stdout)?;
    match created["token"].as_str() {
        Some(token) if !token.is_empty() => Ok(token.to_owned()),
        _ => failed("agent token missing"),
    }
}

fn denied(local: &Local, command: &Value) -> bool {
    local
        .post("reminders.create", command)
        .is_ok_and(|(status, _)| status == 401)
}

fn create_command(id: &str, list: &Value, title: &str) -> Value {
    json!({"command_id": id, "op": "reminders.create", "args": {"list_id": list["id"], "title": title}, "wait_ms": 15000})
}

fn wait_for_list(local: &Local) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(list)) = single(local, "reminders.list", LIST) {
            return Ok(list);
        }
        if Instant::now() >= deadline {
            return failed("Bridge Test list unavailable");
        }
        sleep(20);
    }
}

fn wait_for_approved(local: &Local, id: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(75);
    loop {
        let record = local.op("commands.status", &json!({"command_id": id}))?;
        match record["state"].as_str().unwrap_or_default() {
            "observed" => return Ok(()),
            "admitted" | "prepared" | "dispatched" | "awaiting_approval" => {}
            _ => return failed("approved command did not complete"),
        }
        if Instant::now() >= deadline {
            return failed("context deadline exceeded");
        }
        sleep(20);
    }
}

fn cleanup(local: &Local, prefix: &str) -> Result<()> {
    for suffix in [" full", " approve"] {
        let title = format!("{prefix}{suffix}");
        let Some(record) = single(local, "reminders.reminder", &title)? else {
            return failed("agent cleanup fixture unavailable");
        };
        let removed = local.op(
            "reminders.delete",
            &json!({"command_id": new_id("cmd"), "op": "reminders.delete", "args": {"id": record["id"]}, "expected_version": record["version"], "wait_ms": 15000}),
        )?;
        if removed["state"] != "observed" || !local.named("reminders.reminder", &title)?.is_empty()
        {
            return failed("agent cleanup not observed");
        }
    }
    Ok(())
}

pub fn run<R: Read, W: Write>(session: &mut Session<R, W>) -> Result<()> {
    let value = session.input::<Value>()?;
    let input = common(&value)?;
    let (prefix, full_id, approve_id) = (
        text(&value, "Prefix"),
        text(&value, "FullID"),
        text(&value, "ApproveID"),
    );
    let mut tokens = Vec::new();
    for level in ["read", "full", "approve"] {
        let name = format!("Bridge Test relay {level}");
        tokens.push(create(&value, &input, &name, level)?);
    }
    let base = Local::new(&input)?;
    let list = wait_for_list(&base)?;
    let (read, full, approve) = (
        base.with_token(&tokens[0]),
        base.with_token(&tokens[1]),
        base.with_token(&tokens[2]),
    );
    if !denied(
        &read,
        &create_command(&new_id("cmd"), &list, &format!("{prefix} read")),
    ) {
        return failed("read agent was not denied");
    }
    let made = full.op(
        "reminders.create",
        &create_command(&full_id, &list, &format!("{prefix} full")),
    )?;
    let full_title = format!("{prefix} full");
    if made["state"] != "observed" || full.named("reminders.reminder", &full_title)?.len() != 1 {
        return failed("full agent create not observed with read-your-write");
    }
    let approve_title = format!("{prefix} approve");
    let pending = approve.op(
        "reminders.create",
        &create_command(&approve_id, &list, &approve_title),
    )?;
    if pending["state"] != "awaiting_approval"
        || !approve
            .named("reminders.reminder", &approve_title)?
            .is_empty()
    {
        return failed("approve agent did not wait without an effect");
    }
    session.ready()?;
    session.signal()?;
    wait_for_approved(&approve, &approve_id)?;
    if approve.named("reminders.reminder", &approve_title)?.len() != 1 {
        return failed("approved agent read-your-write missing");
    }
    cleanup(&full, &prefix)?;
    session.emit(&json!({
        "read_denied": true,
        "full_observed": true,
        "approve_queued": true,
        "approved_observed": true,
        "read_your_write": true,
        "cleanup_observed": true,
    }))
}
