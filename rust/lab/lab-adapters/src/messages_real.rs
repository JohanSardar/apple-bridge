#![allow(clippy::too_many_lines)]
use crate::{
    assertions::await_until,
    http::{Client, random_id},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet, env, fs, path::PathBuf, process::Command, thread, time::Duration,
};

const SCOPE: &str = "messages:read,messages:write,system:read";

fn chat_db() -> Option<Connection> {
    let path = PathBuf::from(env::var_os("HOME")?).join("Library/Messages/chat.db");
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

fn own_handles(db: &Connection) -> BTreeSet<String> {
    let mut handles = BTreeSet::new();
    for column in ["destination_caller_id", "account_login"] {
        let sql = format!(
            "SELECT DISTINCT {column} FROM message WHERE is_from_me = 1 AND {column} IS NOT NULL"
        );
        let Ok(mut statement) = db.prepare(&sql) else {
            continue;
        };
        let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else {
            continue;
        };
        handles.extend(rows.flatten().map(|value| value.to_lowercase()));
    }
    handles
}

fn self_chat(db: &Connection, own: &BTreeSet<String>) -> Option<(String, String)> {
    let mut statement = db
        .prepare(
            "SELECT c.guid, MIN(h.id) FROM chat c \
             JOIN chat_handle_join j ON j.chat_id = c.ROWID \
             JOIN handle h ON h.ROWID = j.handle_id \
             WHERE c.style = 45 GROUP BY c.ROWID HAVING COUNT(*) = 1",
        )
        .ok()?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    rows.flatten()
        .find(|(_, handle)| own.contains(&handle.to_lowercase()))
}

fn live_rows(db: &Connection, guid: &str) -> Vec<(String, String)> {
    let Ok(mut statement) = db.prepare(
        "SELECT m.guid, c.guid FROM message m \
         JOIN chat_message_join j ON j.message_id = m.ROWID \
         JOIN chat c ON c.ROWID = j.chat_id \
         WHERE m.is_from_me = 1 AND m.guid = ?1 \
         AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id = m.ROWID AND r.chat_id = j.chat_id)",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([guid], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

fn cleanup(run: &str, tag: &str, guid: &str, baseline: u64) -> bool {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    let Some(app) = env::var_os("REAL_HELPER_APP") else {
        return false;
    };
    let base = home.join("Library/Application Support/AppleBridgeDev");
    let ledger = base.join("messages-command-cleanup");
    let registration = json!({
        "scenario": "MSG-SEND-SELF-TEXT",
        "run_id": run,
        "baseline_max_row_id": baseline,
        "created_message_ids": [guid],
        "cleanup_state": "pending_native_cleanup",
    });
    let written = fs::create_dir_all(&ledger).is_ok()
        && serde_json::to_vec(&registration)
            .is_ok_and(|bytes| fs::write(ledger.join(format!("{run}.json")), bytes).is_ok());
    if !written {
        return false;
    }
    let report = base.join(format!("cleanup-{run}.json"));
    let _ = fs::remove_file(&report);
    Command::new("open")
        .args(["-n", "-W", "-a"])
        .arg(app)
        .args([
            "--args",
            "probe",
            "cleanup-tagged-self",
            "--tag",
            tag,
            "--owned-only",
            "--report",
        ])
        .arg(&report)
        .status()
        .is_ok_and(|status| status.success())
        && fs::read(&report)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|value| value["passed"] == true)
}

fn landed(reply: &Value) -> bool {
    reply["error"].is_null()
        && ["observed", "delivered"].contains(&reply["state"].as_str().unwrap_or_default())
        && reply["commit_seq"].as_u64().unwrap_or(0) > 0
}

fn command(op: &str, args: &Value, version: u64) -> Result<Value> {
    let mut value = json!({
        "command_id": random_id("cmd")?,
        "op": format!("messages.{op}"),
        "args": args,
        "wait_ms": 15000,
    });
    if version != 0 {
        value["expected_version"] = json!(version);
    }
    Ok(value)
}

fn message_record(client: &Client, id: &str) -> Value {
    client
        .call(
            "records.get",
            &json!({"collection": "messages.message", "id": id}),
            40,
        )
        .ok()
        .flatten()
        .and_then(|page| {
            page["records"]
                .as_array()
                .and_then(|rows| rows.first().cloned())
        })
        .unwrap_or(Value::Null)
}

pub fn self_text(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let Some(db) = chat_db() else {
        evidence.check("chat_db_readable", false, "chat.db could not be opened");
        return Ok(());
    };
    let own = own_handles(&db);
    evidence.check(
        "own_handles_known",
        !own.is_empty(),
        "no own handle in chat.db",
    );
    let Some((chat, handle)) = self_chat(&db, &own) else {
        evidence.check(
            "existing_self_chat",
            false,
            "no existing one-participant chat with an own handle; nothing was sent",
        );
        return Ok(());
    };
    evidence.check("existing_self_chat", true, "");
    let client = Client::new(world, SCOPE)?;
    let run = random_id("run")?;
    let tag = format!("[ab-test {run}]");
    let ready = await_until(240, || {
        client
            .call(
                "records.query",
                &json!({"collection": "system.source", "limit": 100}),
                40,
            )
            .ok()
            .flatten()
            .and_then(|page| page["records"].as_array().cloned())
            .is_some_and(|records| {
                records.iter().any(|record| {
                    record["data"]["source"] == "messages" && record["data"]["state"] == "ready"
                })
            })
    });
    evidence.check(
        "messages_source_ready",
        ready,
        "the Helper never reported messages ready",
    );
    if !ready {
        return Ok(());
    }
    let send = command(
        "send",
        &json!({"handles": [handle], "service": "iMessage", "text": format!("{tag} real self text")}),
        0,
    )?;
    let mut reply = Value::Null;
    for _ in 0..12 {
        reply = client
            .call_raw("messages.send", &send, 40)
            .ok()
            .flatten()
            .map_or(Value::Null, |(_, value)| value);
        let waiting = ["admitted", "prepared", "dispatched"]
            .contains(&reply["state"].as_str().unwrap_or_default());
        if !waiting && reply["error"]["message"] != "peer unavailable" {
            break;
        }
        thread::sleep(Duration::from_secs(5));
    }
    evidence.check(
        "send_landed_with_evidence",
        landed(&reply),
        &reply.to_string(),
    );
    let guid = reply["evidence"]["native_key"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let baseline = reply["evidence"]["baseline"]["max_row_id"]
        .as_u64()
        .unwrap_or(0);
    let rows = live_rows(&db, &guid);
    evidence.check(
        "exactly_one_row_in_the_self_chat",
        !guid.is_empty() && rows.len() == 1 && rows[0].1 == chat,
        &format!("{rows:?}"),
    );
    let id = reply["observed_record_ids"][0]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let record = message_record(&client, &id);
    evidence.check(
        "gateway_record_carries_the_tagged_text",
        record["data"]["is_from_me"] == true
            && record["data"]["text"]
                .as_str()
                .is_some_and(|text| text.contains(&tag)),
        &record.to_string(),
    );
    if guid.is_empty() || baseline == 0 {
        evidence.check("cleanup_target_known", false, "clean up by hand");
        return Ok(());
    }
    evidence.check(
        "cleanup_removed_the_row",
        cleanup(&run, &tag, &guid, baseline),
        "",
    );
    evidence.check(
        "cleanup_row_gone",
        await_until(30, || live_rows(&db, &guid).is_empty()),
        &format!("{:?}", live_rows(&db, &guid)),
    );
    Ok(())
}
