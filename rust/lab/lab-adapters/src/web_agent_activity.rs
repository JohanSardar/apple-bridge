#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    http::{Client, random_id},
    web_agents::{create_agent, owner_session, post, text},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LOCKER: &str = "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute(\"BEGIN IMMEDIATE\"); print(1,flush=True); sys.stdin.read(); db.close()";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

pub(crate) fn activity_rows(home: &Path, principal: &str, operation: &str) -> Result<Value> {
    let db = Connection::open_with_flags(
        home.join("state.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| Error::Invalid(e.to_string()))?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let mut statement = db
        .prepare(
            "SELECT id,principal,operation,kind,state,at,count,argument_bytes,argument_hash FROM agent_activity WHERE (?1='' OR principal=?1) AND (?2='' OR operation=?2) AND at>=?3 ORDER BY at DESC,id DESC LIMIT 1000",
        )
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let rows = statement
        .query_map(params![principal, operation, now() - 90 * 86400], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "principal": row.get::<_, String>(1)?,
                "operation": row.get::<_, String>(2)?,
                "kind": row.get::<_, String>(3)?,
                "state": row.get::<_, String>(4)?,
                "at": row.get::<_, i64>(5)?,
                "count": row.get::<_, i64>(6)?,
                "argument_bytes": row.get::<_, i64>(7)?,
                "argument_hash": row.get::<_, String>(8)?,
            }))
        })
        .map_err(|e| Error::Invalid(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(json!({"activity": rows}))
}

fn effect(world: &World, id: &str) -> u64 {
    world
        .control("/effects", &Value::Null)
        .map_or(u64::MAX, |effects| effects[id].as_u64().unwrap_or(0))
}

fn query_chats(world: &World, token: &str) -> Option<Vec<Value>> {
    let (status, body) = post(
        world,
        token,
        "/v1/op/records.query",
        &json!({"collection": "messages.chat"}),
    )
    .ok()?;
    (status == 200).then(|| body["records"].as_array().cloned().unwrap_or_default())
}

pub fn activity(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    if world.gateway_cli(&["agent", "list"]).is_err() {
        evidence.check("activity_authority", false, "agent list");
        return Ok(());
    }
    let Some((agent, token, _)) =
        create_agent(world, "Bridge Test activity", "read", "messages:write")
    else {
        evidence.check("activity_agent", false, "agent create");
        return Ok(());
    };
    if owner_session(world, None).is_err() {
        evidence.check("activity_owner", false, "owner enrollment");
        return Ok(());
    }
    for _ in 0..3 {
        let read = post(world, &token, "/v1/op/messages.chats", &json!({}));
        evidence.check(
            "activity_read",
            read.is_ok_and(|(status, _)| status == 200),
            "agent read",
        );
    }
    let ids = [random_id("cmd")?, random_id("cmd")?];
    let message = "[ab-test activity] private synthetic text";
    for id in &ids {
        let sent = post(
            world,
            &token,
            "/v1/op/messages.send",
            &json!({"command_id": id, "args": {"chat_id": "BridgeTest", "text": message}}),
        );
        evidence.check(
            "activity_denied_without_effect",
            sent.is_ok_and(|(status, _)| status == 401) && effect(world, id) == 0,
            "read agent send denied with no effect",
        );
    }
    let home = world.gateway_home.clone();
    let listed = activity_rows(&home, &agent, "");
    evidence.check("activity_available", listed.is_ok(), "activity readable");
    let Ok(listed) = listed else {
        return Ok(());
    };
    let raw = listed.to_string();
    let rows = listed["activity"].as_array().cloned().unwrap_or_default();
    let mut reads = 0;
    let mut buckets = 0;
    let mut commands = std::collections::BTreeMap::new();
    for row in &rows {
        if row["kind"] == "read" && row["operation"] == "messages.chats" {
            reads += row["count"].as_i64().unwrap_or(0);
            buckets += 1;
        }
        if row["kind"] == "command" {
            commands.insert(
                text(row, "id").to_owned(),
                row["count"] == 1 && !text(row, "argument_hash").is_empty(),
            );
        }
    }
    evidence.check(
        "activity_reads_aggregate_per_minute",
        reads == 3 && buckets <= 2,
        "reads aggregate per minute",
    );
    let logged = |id: &String| commands.get(&format!("{agent}:{id}")).copied() == Some(true);
    evidence.check(
        "activity_each_command_individually",
        logged(&ids[0]) && logged(&ids[1]),
        "each command is its own row",
    );
    evidence.check(
        "activity_arguments_redacted",
        !raw.contains(message) && !raw.contains(&token),
        "arguments and token not logged",
    );
    let filtered = activity_rows(&home, &agent, "messages.send").map(|v| v.to_string());
    evidence.check(
        "activity_operation_filter",
        filtered.is_ok_and(|f| f.contains(&ids[0]) && !f.contains("messages.chats")),
        "operation filter",
    );
    evidence.check(
        "activity_restart",
        world.restart_gateway().is_ok(),
        "gateway restarted",
    );
    let restarted = activity_rows(&home, &agent, "").map(|v| v.to_string());
    evidence.check(
        "activity_survives_restart",
        restarted.is_ok_and(|r| r.contains(&ids[0])),
        "activity persists across restart",
    );
    let aged = Connection::open(home.join("state.db")).and_then(|db| {
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute(
            "UPDATE agent_activity SET at=unixepoch()-91*86400 WHERE principal=?1",
            params![agent],
        )
    });
    let pruned = activity_rows(&home, &agent, "").map(|v| v.to_string());
    evidence.check(
        "activity_retains_90_days",
        aged.is_ok() && pruned.is_ok_and(|p| p.contains("\"activity\":[]")),
        "rows older than 90 days are not listed",
    );
    let client = Client::with_token(world, &token)?;
    let owner_only = client.request("GET", "/web/v1/activity", &[], None, 15);
    evidence.check(
        "activity_owner_only",
        owner_only.is_ok_and(|r| r.status == 404),
        "agents cannot read the owner activity route",
    );
    capture(world, evidence)?;
    rejection(world, evidence)
}

fn capture(world: &World, evidence: &mut Evidence) -> Result<()> {
    evidence.check(
        "activity_capture_fixture",
        world
            .control("/messages", &json!({"action": "seed", "count": 0}))
            .is_ok(),
        "message fixture",
    );
    let Some((_, token, _)) = create_agent(
        world,
        "Bridge Test concurrent activity",
        "full",
        "messages:write",
    ) else {
        evidence.check("activity_capture_agent", false, "agent create");
        return Ok(());
    };
    evidence.check("activity_capture_agent", true, "agent created");
    let mut chat = None;
    let found = await_until(5, || {
        chat = query_chats(world, &token).filter(|c| c.len() == 1);
        chat.is_some()
    });
    let Some(chat) = chat.filter(|_| found) else {
        evidence.check("activity_capture_chat", false, "chat visible");
        return Ok(());
    };
    let chat_id = text(&chat[0], "id").to_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let mut readers = vec![];
    for _ in 0..3 {
        let Some((_, read_token, _)) = create_agent(
            world,
            "Bridge Test concurrent reader",
            "read",
            "messages:read",
        ) else {
            stop.store(true, Ordering::SeqCst);
            evidence.check("activity_capture_read_agent", false, "reader create");
            return Ok(());
        };
        let client = Client::with_token(world, &read_token)?;
        let stop = Arc::clone(&stop);
        readers.push(thread::spawn(move || {
            let mut ok = true;
            while !stop.load(Ordering::SeqCst) {
                let reply = client.request(
                    "POST",
                    "/v1/op/messages.chats",
                    &[("Content-Type", "application/json")],
                    Some(b"{}"),
                    15,
                );
                if !stop.load(Ordering::SeqCst) && !reply.is_ok_and(|r| r.status == 200) {
                    ok = false;
                }
                thread::sleep(Duration::from_millis(10));
            }
            ok
        }));
    }
    let mut observed = true;
    for _ in 0..24 {
        let id = random_id("cmd")?;
        let sent = post(
            world,
            &token,
            "/v1/op/messages.send",
            &json!({"command_id": id, "op": "messages.send", "args": {"chat_id": chat_id, "text": "[ab-test activity] concurrent capture"}, "wait_ms": 15000}),
        );
        let good = sent.is_ok_and(|(status, row)| {
            status == 200
                && row["state"] == "observed"
                && row["commit_seq"].as_u64().unwrap_or(0) > 0
        });
        if !good || effect(world, &id) != 1 {
            observed = false;
            break;
        }
    }
    stop.store(true, Ordering::SeqCst);
    let read_ok = readers
        .into_iter()
        .map(|reader| reader.join().unwrap_or(false))
        .collect::<Vec<_>>()
        .into_iter()
        .all(|ok| ok);
    evidence.check(
        "activity_concurrent_reads_succeed",
        read_ok,
        "reads succeed",
    );
    evidence.check(
        "activity_concurrent_sends_observed_once",
        observed,
        "sends observed once",
    );
    Ok(())
}

fn source_state(world: &World) -> (String, String, u64) {
    let Ok(status) = world.query("sources.status", &json!({})) else {
        return (String::new(), String::new(), 0);
    };
    for record in status["records"].as_array().into_iter().flatten() {
        let data = &record["data"];
        if data["source"] == "messages" {
            let count = data["capture_rejected_count"].as_u64().unwrap_or(0);
            if data["state"] == "degraded" {
                return (
                    text(data, "state").to_owned(),
                    text(data, "reason").to_owned(),
                    count,
                );
            }
            return (
                text(data, "state").to_owned(),
                text(data, "last_capture_rejected_reason").to_owned(),
                count,
            );
        }
    }
    (String::new(), String::new(), 0)
}

fn rejection(world: &World, evidence: &mut Evidence) -> Result<()> {
    let Ok(mut locker) = Command::new("python3")
        .args(["-c", LOCKER])
        .arg(world.gateway_home.join("state.db"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    else {
        evidence.check("capture_rejection_lock_process", false, "python3");
        return Ok(());
    };
    let mut ready = String::new();
    if let Some(stdout) = locker.stdout.take() {
        let _ = BufReader::new(stdout).read_line(&mut ready);
    }
    if ready.trim() != "1" {
        evidence.check("capture_rejection_write_lock", false, "write lock");
        let _ = locker.kill();
        let _ = locker.wait();
        return Ok(());
    }
    evidence.check(
        "capture_rejection_fixture",
        world
            .control(
                "/messages",
                &json!({"action": "incoming", "key": random_id("busy")?, "wait": false}),
            )
            .is_ok(),
        "incoming message fixture",
    );
    let degraded = await_until(8, || {
        let (state, reason, count) = source_state(world);
        state == "degraded" && reason == "gateway_capture_rejected:sqlite_5" && count > 0
    });
    evidence.check(
        "capture_rejection_degraded_and_counted",
        degraded,
        "source degraded and counted",
    );
    let log = fs::read_to_string(world.directory.join("gateway.log"));
    evidence.check(
        "capture_rejection_logged",
        log.is_ok_and(|l| {
            l.contains("\"stage\":\"gateway_capture_rejected\"")
                && l.contains("\"error_code\":\"sqlite_5\"")
        }),
        "rejection logged",
    );
    drop(locker.stdin.take());
    let _ = locker.wait();
    let retained = await_until(5, || {
        let (state, reason, count) = source_state(world);
        state == "ready" && reason == "gateway_capture_rejected:sqlite_5" && count > 0
    });
    evidence.check(
        "capture_rejection_count_retained_after_reconnect",
        retained,
        "count retained after reconnect",
    );
    let raw = world.query("records.query", &json!({"collection": "system.source"}));
    let mut clean = raw.is_ok();
    let mut found = false;
    for record in raw.iter().filter_map(|r| r["records"].as_array()).flatten() {
        let data = &record["data"];
        if data["source"] != "messages" {
            continue;
        }
        found = true;
        clean = clean
            && data.get("capture_rejected_count").is_none()
            && data.get("last_capture_rejected_reason").is_none();
    }
    evidence.check(
        "raw_source_records_exclude_local_capture_diagnostics",
        clean && found,
        "diagnostics stay out of raw records",
    );
    Ok(())
}
