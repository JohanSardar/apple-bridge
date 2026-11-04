#![allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
use crate::http::Client;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 1] = ["MSG-DELETE-VISIBILITY"];
const SCOPE: &str = "messages:read,system:read";

pub fn sim(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let output = Command::new("packaging/dev/check-messages-capture.sh").output();
    let (stdout, stderr) = output.as_ref().map_or_else(
        |_| (String::new(), String::new()),
        |o| {
            (
                String::from_utf8_lossy(&o.stdout).into_owned(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
            )
        },
    );
    let combined = format!("{stdout}{stderr}");
    let passed = output.as_ref().is_ok_and(|o| o.status.success())
        && combined.contains("all Messages capture checks passed");
    let mut detail = combined
        .lines()
        .find_map(|line| line.strip_prefix("Messages capture checks failed: "))
        .unwrap_or_default()
        .to_owned();
    if !passed && detail.is_empty() {
        let tail: String = combined
            .trim()
            .chars()
            .rev()
            .take(2048)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        detail = match &output {
            Ok(o) => format!("{}: {tail}", o.status),
            Err(error) => format!("{error}: {tail}"),
        };
    }
    evidence.check("native_capture_visibility_and_restore", passed, &detail);
    Ok(())
}

fn valid_run(run: &str) -> bool {
    !run.is_empty()
        && run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn truth_visibility(guid: &str, text: &str) -> Result<Value> {
    let root = env::temp_dir().join(format!("ab-delete-truth-{}", std::process::id()));
    fs::create_dir_all(&root)?;
    let app = root.join("AppleTruth.app");
    let built = Command::new("packaging/dev/build-truth.sh")
        .arg(&app)
        .output()?;
    if !built.status.success() {
        return Err(Error::Invalid("Truth build failed".into()));
    }
    let request = root.join("request.json");
    let report = root.join("report.json");
    fs::write(
        &request,
        serde_json::to_vec(&json!({"source": "messages", "guid": guid, "expected_text": text}))?,
    )?;
    Command::new("open")
        .args(["-n", "-W", "-g", "-a"])
        .arg(&app)
        .args(["--args", "observe", "--request-file"])
        .arg(&request)
        .arg("--report")
        .arg(&report)
        .status()?;
    let value = serde_json::from_slice(&fs::read(&report)?)?;
    let _ = fs::remove_dir_all(&root);
    Ok(value)
}

fn manifest(run: &str) -> Option<Value> {
    let home = PathBuf::from(env::var_os("HOME")?);
    let path = home
        .join("Library/Application Support/AppleBridgeDev/messages-command-cleanup")
        .join(format!("{run}.json"));
    let value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let text = format!("Apple Bridge interactive self-check {run}");
    (value["run_id"] == run
        && value["scenario"] == "MSG-INTERACTIVE-GATE"
        && value["native_key"].as_str().is_some_and(|k| !k.is_empty())
        && value["interactive_text"] == text.as_str())
    .then_some(value)
}

fn read_only(path: &Path) -> Option<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

pub fn real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let run = env::var("APPLE_BRIDGE_DELETE_PROOF_RUN").unwrap_or_default();
    if !valid_run(&run) {
        evidence.unverified(
            "registered_deleted_test_fixture",
            "Set APPLE_BRIDGE_DELETE_PROOF_RUN to a cleaned interactive self-test run; this proof never dispatches a write",
        );
        return Ok(());
    }
    let Some(manifest) = manifest(&run) else {
        evidence.check("registered_deleted_test_fixture", false, "");
        return Ok(());
    };
    let key = manifest["native_key"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let text = manifest["interactive_text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let proof = truth_visibility(&key, &text).unwrap_or(Value::Null);
    let retained = proof["records"].as_array().is_some_and(|rows| {
        rows.iter().any(|row| {
            row["guid"] == key.as_str()
                && row["is_from_me"] == true
                && row["visible"] == false
                && row["expected_text_present"] == true
        })
    });
    evidence.check(
        "truth_retains_deleted_test_message",
        proof["status"] == "verified" && retained,
        "",
    );
    let client = Client::new(world, SCOPE)?;
    let state = read_only(&world.gateway_home.join("state.db"));
    let lookup = |db: &Connection| -> Option<String> {
        db.query_row(
            "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=?1 LIMIT 1",
            [&key],
            |row| row.get(0),
        )
        .ok()
    };
    let started = Instant::now();
    let mut record_id = None;
    while started.elapsed() < Duration::from_secs(120) {
        record_id = state.as_ref().and_then(lookup);
        if record_id.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    evidence.check("retained_target_captured", record_id.is_some(), "");
    let record = record_id.as_ref().and_then(|id| {
        client
            .call(
                "records.get",
                &json!({"collection": "messages.message", "id": id}),
                30,
            )
            .ok()
            .flatten()
    });
    let rows = record
        .as_ref()
        .and_then(|reply| reply["records"].as_array().cloned())
        .unwrap_or_default();
    let deleted_at = rows
        .first()
        .and_then(|row| row["data"]["deleted_at"].as_str())
        .unwrap_or_default()
        .to_owned();
    evidence.check(
        "copy_retains_recoverable_record",
        rows.len() == 1 && rows[0]["deleted"] != true && !deleted_at.is_empty(),
        "",
    );
    let native = env::var_os("HOME").and_then(|home| {
        read_only(&PathBuf::from(home).join("Library/Messages/chat.db")).and_then(|db| {
            db.query_row(
                "SELECT MAX(r.delete_date) FROM chat_recoverable_message_join r JOIN message m ON m.ROWID=r.message_id WHERE m.guid=?1",
                [&key],
                |row| row.get::<_, f64>(0),
            )
            .ok()
        })
    });
    let matches = native.is_some_and(|raw| {
        deleted_at.parse::<jiff::Timestamp>().is_ok_and(|actual| {
            let expected_ms = 978_307_200_000 + (raw / 1_000_000.0) as i64;
            (actual.as_millisecond() - expected_ms).abs() < 1000
        })
    });
    evidence.check("native_deleted_timestamp_matches_copy", matches, "");
    let guard = world.gateway_home.join("helper/dispatch-guard.log");
    evidence.check(
        "no_command_dispatched",
        fs::metadata(guard).is_ok_and(|m| m.len() == 0),
        "",
    );
    Ok(())
}
