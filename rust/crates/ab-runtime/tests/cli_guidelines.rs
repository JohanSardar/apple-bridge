#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type Check = Result<(), Box<dyn std::error::Error>>;
type Log = Arc<Mutex<Vec<(String, Value)>>>;
type Reply = (u16, Vec<(&'static str, String)>, Value);

fn bridge() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bridge"));
    command
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .env_remove("APPLE_BRIDGE_HOME")
        .env_remove("APPLE_BRIDGE_LEGACY_JSON")
        .env_remove("NO_COLOR")
        .env_remove("FORCE_COLOR")
        .stdin(Stdio::null());
    for variable in [
        "AI_AGENT",
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CODEX_SANDBOX",
        "CODEX_THREAD_ID",
        "CURSOR_AGENT",
        "GEMINI_CLI",
        "CLINE_ACTIVE",
        "OPENCODE",
        "AMP_AGENT",
    ] {
        command.env_remove(variable);
    }
    command
}

fn run(home: &Path, arguments: &[&str]) -> Result<Output, std::io::Error> {
    bridge().args(arguments).arg("--home").arg(home).output()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn error_document(output: &Output) -> Result<Value, Box<dyn std::error::Error>> {
    let stderr = text(&output.stderr);
    let line = stderr
        .lines()
        .rev()
        .find(|line| line.starts_with('{'))
        .ok_or_else(|| format!("no JSON error on stderr: {stderr}"))?;
    Ok(serde_json::from_str(line)?)
}

fn short_home() -> Result<tempfile::TempDir, std::io::Error> {
    tempfile::tempdir_in("/tmp")
}

fn seeded_home(home: &Path) -> Check {
    drop(ab_state::State::<ab_state::Gateway>::open(home)?);
    Ok(())
}

fn gateway_home(home: &Path) -> Check {
    seeded_home(home)?;
    let path = home.join("runtime.json");
    std::fs::write(
        &path,
        json!({"role":"gateway","home":home,"peer_address":"","api_address":"","socket":""})
            .to_string(),
    )?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn record(collection: &str, id: &str) -> Value {
    json!({"collection": collection, "id": id, "data": {"title": format!("item {id}"), "date": "2026-11-01T00:00:00Z"}})
}

#[allow(clippy::too_many_lines)]
fn scripted_gateway(
    home: &Path,
    handler: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static,
) -> Result<(std::fs::File, Log), Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let log: Log = Arc::default();
    let seen = log.clone();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let seen = seen.clone();
            let handler = handler.clone();
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 8192];
                let mut body_start = None;
                while body_start.is_none() {
                    let Ok(read) = stream.read(&mut buffer) else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    body_start = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|at| at + 4);
                }
                let start = body_start.unwrap_or(request.len());
                let head = String::from_utf8_lossy(&request[..start]).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while request.len() < start + length {
                    let Ok(read) = stream.read(&mut buffer) else {
                        return;
                    };
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let respond = |stream: &mut std::net::TcpStream,
                               status: u16,
                               headers: &[(&str, String)],
                               body: &str| {
                    let mut extra = String::new();
                    for (name, value) in headers {
                        extra.push_str(name);
                        extra.push_str(": ");
                        extra.push_str(value);
                        extra.push_str("\r\n");
                    }
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                };
                if head.starts_with("GET /v1/health") {
                    respond(&mut stream, 200, &[], "{}");
                    return;
                }
                let operation = head
                    .split_whitespace()
                    .nth(1)
                    .and_then(|path| path.strip_prefix("/v1/op/"))
                    .unwrap_or_default()
                    .to_owned();
                let body: Value = serde_json::from_slice(&request[start..]).unwrap_or(Value::Null);
                if let Ok(mut log) = seen.lock() {
                    log.push((operation.clone(), body.clone()));
                }
                let (status, headers, reply) = handler(&operation, &body);
                if status == 0 {
                    return;
                }
                let body = match &reply {
                    Value::String(raw) if raw.starts_with("raw:") => raw[4..].to_owned(),
                    other => other.to_string(),
                };
                respond(&mut stream, status, &headers, &body);
            });
        }
    });
    std::fs::write(
        home.join("runtime.json"),
        json!({"role": "gateway", "home": home, "peer_address": "127.0.0.1:1", "api_address": address, "socket": ""}).to_string(),
    )?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join("lock"))?;
    lock.try_lock()?;
    Ok((lock, log))
}

fn requests(log: &Log, operation: &str) -> Vec<Value> {
    log.lock()
        .map(|log| {
            log.iter()
                .filter(|(name, _)| name == operation)
                .map(|(_, body)| body.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn ok(body: Value) -> Reply {
    (200, Vec::new(), body)
}

fn search_pages(page_size: usize, pages: usize) -> impl Fn(&str, &Value) -> Reply {
    move |operation, body| {
        if operation != "search" {
            return ok(json!({}));
        }
        let page: usize = body["after"]
            .as_str()
            .and_then(|token| token.strip_prefix('p'))
            .and_then(|number| number.parse().ok())
            .unwrap_or(0);
        let asked = body["limit"]
            .as_u64()
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(10);
        let records: Vec<Value> = (0..asked.min(page_size))
            .map(|index| record("notes.note", &format!("{page}-{index}")))
            .collect();
        let next = (page + 1 < pages).then(|| format!("p{}", page + 1));
        ok(json!({"records": records, "cursor": null, "next": next}))
    }
}

#[test]
fn limit_is_a_total_and_only_the_remaining_rows_are_asked_for() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), search_pages(2, 10))?;
    let output = run(home.path(), &["search", "item", "--limit", "5", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let document: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(document["records"].as_array().map(Vec::len), Some(5));
    let asked: Vec<u64> = requests(&log, "search")
        .iter()
        .filter_map(|body| body["limit"].as_u64())
        .collect();
    assert_eq!(asked, [5, 3, 1]);
    assert!(document["next_cursor"].is_string(), "{document}");
    Ok(())
}

#[test]
fn piped_search_is_tab_separated_with_a_header_even_when_empty() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), search_pages(0, 1))?;
    let output = run(home.path(), &["search", "nothing"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "When\tTitle\tID\n");
    assert!(text(&output.stderr).contains("No results"));
    let other = short_home()?;
    seeded_home(other.path())?;
    let (_lock, _log) = scripted_gateway(other.path(), search_pages(2, 1))?;
    let output = run(other.path(), &["search", "item"])?;
    let lines: Vec<String> = text(&output.stdout).lines().map(str::to_owned).collect();
    assert_eq!(lines[0], "When\tTitle\tID");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[1].split('\t').count(), 3);
    Ok(())
}

#[test]
fn a_repeated_page_cursor_stops_with_pagination_stalled() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), |_, _| {
        ok(json!({"records": [record("notes.note", "same")], "cursor": null, "next": "again"}))
    })?;
    let output = run(home.path(), &["search", "item", "--all", "--json"])?;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "pagination_stalled");
    assert_eq!(document["data"]["rowsFetched"], 2);
    assert!(requests(&log, "search").len() <= 3);
    Ok(())
}

#[test]
fn empty_pages_that_claim_more_stop_instead_of_spinning() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), |_, body| {
        let page = body["after"].as_str().map_or(0, str::len);
        ok(json!({"records": [], "cursor": null, "next": "x".repeat(page + 1)}))
    })?;
    let output = run(home.path(), &["search", "item", "--json"])?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(error_document(&output)?["reason"], "pagination_stalled");
    assert_eq!(requests(&log, "search").len(), 3);
    Ok(())
}

#[test]
fn a_failed_later_page_reports_rows_fetched_and_where_to_resume() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), |_, body| {
        if body["after"].as_str() == Some("p1") {
            return (
                503,
                Vec::new(),
                json!({"error": {"code": "unavailable", "message": "busy", "retryable": false}}),
            );
        }
        ok(
            json!({"records": [record("notes.note", "a"), record("notes.note", "b")], "cursor": null, "next": "p1"}),
        )
    })?;
    let output = run(home.path(), &["search", "item", "--limit", "6", "--json"])?;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let document = error_document(&output)?;
    assert_eq!(document["data"]["rowsFetched"], 2, "{document}");
    assert_eq!(document["data"]["pages"], 1);
    assert!(document["data"]["cursor"].is_null());
    let resume = document["data"]["resume"]
        .as_str()
        .ok_or("resume")?
        .to_owned();
    assert_eq!(output.stdout, [] as [u8; 0]);
    let before = requests(&log, "search").len();
    let output = run(home.path(), &["search", "--next", &resume, "--json"])?;
    assert_eq!(output.status.code(), Some(1));
    let again = requests(&log, "search");
    assert_eq!(again[before]["after"], "p1");
    assert_eq!(again[before]["text"], "item");
    assert_eq!(error_document(&output)?["data"]["cursor"], resume.as_str());
    Ok(())
}

#[test]
fn a_cursor_keeps_the_search_and_refuses_a_different_one() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), search_pages(2, 5))?;
    let output = run(home.path(), &["search", "item", "--limit", "2", "--json"])?;
    let document: Value = serde_json::from_slice(&output.stdout)?;
    let cursor = document["next_cursor"].as_str().ok_or("cursor")?.to_owned();
    let output = run(home.path(), &["search", "--next", &cursor, "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let last = requests(&log, "search").pop().ok_or("request")?;
    assert_eq!(last["text"], "item");
    assert_eq!(last["limit"], 2);
    assert_eq!(last["after"], "p1");
    let output = run(
        home.path(),
        &["search", "other", "--next", &cursor, "--json"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "conflicting_options");
    let elsewhere = short_home()?;
    seeded_home(elsewhere.path())?;
    let output = run(elsewhere.path(), &["search", "--next", &cursor, "--json"])?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "invalid_value");
    Ok(())
}

#[test]
fn rate_limits_are_their_own_error_and_keep_retry_after() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), |_, _| {
        (
            429,
            vec![("Retry-After", "30".to_owned())],
            json!({"error": {"code": "rate_limited", "message": "slow down"}}),
        )
    })?;
    let output = run(home.path(), &["search", "item", "--json"])?;
    assert_eq!(output.status.code(), Some(1));
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "rate_limited");
    assert_eq!(document["data"]["retryAfterSeconds"], 30);
    assert_eq!(requests(&log, "search").len(), 1);
    Ok(())
}

#[test]
fn transient_reads_retry_and_then_succeed() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let calls = Arc::new(Mutex::new(0_u32));
    let counter = calls.clone();
    let (_lock, _log) = scripted_gateway(home.path(), move |_, _| {
        let mut count = counter.lock().map(|mut count| {
            *count += 1;
            *count
        });
        if count.as_mut().is_ok_and(|count| *count == 1) {
            return (
                429,
                vec![("Retry-After", "0".to_owned())],
                json!({"error": {"code": "rate_limited"}}),
            );
        }
        ok(json!({"records": [record("notes.note", "a")], "cursor": null, "next": null}))
    })?;
    let output = run(home.path(), &["search", "item", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(*calls.lock().map_err(|_| "lock")?, 2);
    Ok(())
}

#[test]
fn a_missing_chat_is_not_found() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| {
        (
            404,
            Vec::new(),
            json!({"error": {"code": "not_found", "message": "no such chat", "retryable": false}}),
        )
    })?;
    let output = run(home.path(), &["read", "messages", "--chat", "c1"])?;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    assert!(
        text(&output.stderr).contains("Couldn't find chat c1."),
        "{}",
        text(&output.stderr)
    );
    let output = run(home.path(), &["read", "messages", "--chat", "c1", "--json"])?;
    assert_eq!(error_document(&output)?["reason"], "not_found");
    Ok(())
}

#[test]
fn calendar_reads_accept_a_long_window() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), |_, _| {
        ok(json!({"records": [record("calendar.event", "e1")]}))
    })?;
    let output = run(
        home.path(),
        &["read", "calendar", "--days", "400", "--json"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let asked = requests(&log, "calendar.agenda").pop().ok_or("request")?;
    let start: jiff::Timestamp = asked["start"].as_str().ok_or("start")?.parse()?;
    let end: jiff::Timestamp = asked["end"].as_str().ok_or("end")?.parse()?;
    assert_eq!(end.duration_since(start).as_hours(), 400 * 24);
    Ok(())
}

#[test]
fn an_unconfirmed_write_is_unknown_outcome_and_never_replayed() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), |_, _| {
        (
            400,
            Vec::new(),
            json!({"error": {"code": "indeterminate", "message": "started", "retryable": false}}),
        )
    })?;
    let output = bridge()
        .env("REVIEW_TOKEN", "aba-REVIEW")
        .args([
            "call",
            "photos.fetch",
            "--data",
            r#"{"args":{"asset_id":"fixture"}}"#,
            "--token-env",
            "REVIEW_TOKEN",
            "--json",
            "--home",
        ])
        .arg(home.path())
        .output()?;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "unknown_outcome");
    assert!(document["data"]["confirmed"].is_string(), "{document}");
    assert_ne!(document["next"].as_array().ok_or("next")?.len(), 0);
    assert_eq!(requests(&log, "photos.fetch").len(), 1);
    Ok(())
}

#[test]
fn a_corrupt_remote_config_is_reported_and_creates_no_keys() -> Check {
    let home = short_home()?;
    std::fs::create_dir_all(home.path().join("remote"))?;
    std::fs::write(home.path().join("remote/config.json"), "{broken")?;
    let output = run(home.path(), &["remote", "key", "--json"])?;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "invalid_config");
    assert!(
        document["message"]
            .as_str()
            .is_some_and(|message| message.contains("config.json"))
    );
    let names: Vec<String> = std::fs::read_dir(home.path().join("remote"))?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["config.json"]);
    let plain = run(home.path(), &["remote", "key"])?;
    let stderr = text(&plain.stderr);
    assert!(stderr.contains("line 1"), "{stderr}");
    Ok(())
}

#[test]
fn global_options_mean_the_same_in_any_position() -> Check {
    let home = short_home()?;
    let pairs: [(&[&str], &[&str]); 4] = [
        (
            &["help", "agents", "--home", "/tmp/x"],
            &["--home", "/tmp/x", "help", "agents"],
        ),
        (
            &["help", "search", "revoke", "--json"],
            &["--json", "help", "search", "revoke"],
        ),
        (&["help", "read", "--json"], &["--json", "help", "read"]),
        (&["agents", "list", "--json"], &["--json", "agents", "list"]),
    ];
    for (after, before) in pairs {
        let late = bridge()
            .args(after)
            .env("APPLE_BRIDGE_HOME", home.path())
            .output()?;
        let early = bridge()
            .args(before)
            .env("APPLE_BRIDGE_HOME", home.path())
            .output()?;
        assert_eq!(late.status.code(), early.status.code(), "{after:?}");
        assert_eq!(text(&late.stdout), text(&early.stdout), "{after:?}");
        assert!(!late.stdout.is_empty(), "{after:?}");
    }
    let output = bridge().args(["--json=true", "--version"]).output()?;
    let version: Value = serde_json::from_slice(&output.stdout)?;
    assert!(version["version"].is_string());
    let output = bridge()
        .args(["--json", "--json=false", "--version"])
        .output()?;
    assert!(text(&output.stdout).starts_with("bridge "));
    assert_eq!(output.stderr, [] as [u8; 0]);
    Ok(())
}

#[test]
fn help_search_without_matches_answers_on_stdout() -> Check {
    let output = bridge().args(["help", "search", "zzzzqqqq"]).output()?;
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("No commands match"));
    Ok(())
}

#[test]
fn enumerated_values_are_listed_for_help_and_completion() -> Check {
    let output = bridge()
        .args(["agents", "create", "--help", "--json"])
        .output()?;
    let document: Value = serde_json::from_slice(&output.stdout)?;
    let level = document["options"]
        .as_array()
        .ok_or("options")?
        .iter()
        .find(|option| option["name"] == "--level" || option["long"] == "level")
        .cloned()
        .unwrap_or_else(|| document.clone());
    assert!(level.to_string().contains("approve"), "{level}");
    let output = bridge().args(["completion", "bash"]).output()?;
    let script = text(&output.stdout);
    assert!(script.contains("\"agents create|--level\") words=\"full approve read\""));
    let output = bridge().args(["agents", "ls", "--help"]).output()?;
    assert!(output.status.success());
    Ok(())
}

#[test]
fn widening_access_needs_consent_and_narrowing_does_not() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &[
            "agents",
            "create",
            "--name",
            "bot",
            "--level",
            "approve",
            "--scope",
            "notes:read",
            "--json",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let created: Value = serde_json::from_slice(&output.stdout)?;
    let id = created["agent"]["id"].as_str().ok_or("id")?.to_owned();
    let output = run(home.path(), &["agents", "update", &id, "--level", "full"])?;
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    let flat = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("without --yes in non-interactive mode."),
        "{stderr}"
    );
    for shown in ["Agent", "Level", "approve → full"] {
        assert!(stderr.contains(shown), "{shown}: {stderr}");
    }
    let output = run(
        home.path(),
        &["agents", "update", &id, "--level", "full", "--json"],
    )?;
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "confirmation_required");
    assert!(document.get("issues").is_none());
    let output = run(
        home.path(),
        &[
            "agents", "update", &id, "--level", "full", "--yes", "--json",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let updated: Value = serde_json::from_slice(&output.stdout)?;
    assert!(
        updated["next"]
            .as_array()
            .is_some_and(|next| !next.is_empty())
    );
    let output = run(
        home.path(),
        &["agents", "update", &id, "--level", "approve"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    Ok(())
}

#[test]
fn access_can_be_issued_with_an_expiry() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &[
            "agents",
            "create",
            "--name",
            "short",
            "--expires-in",
            "1h",
            "--json",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let created: Value = serde_json::from_slice(&output.stdout)?;
    let expires: jiff::Timestamp = created["agent"]["expires_at"]
        .as_str()
        .ok_or("expires_at")?
        .parse()?;
    let left = expires.duration_since(jiff::Timestamp::now()).as_secs();
    assert!((3500..=3600).contains(&left), "{left}");
    let output = run(
        home.path(),
        &["agents", "create", "--name", "forever", "--json"],
    )?;
    let created: Value = serde_json::from_slice(&output.stdout)?;
    assert!(created["agent"]["expires_at"].is_null());
    let output = run(
        home.path(),
        &[
            "tokens",
            "create",
            "--scope",
            "notes:read",
            "--expires-in",
            "2h",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).trim().starts_with("ab"),
        "{}",
        text(&output.stdout)
    );
    assert_eq!(text(&output.stdout).lines().count(), 1);
    let output = run(
        home.path(),
        &[
            "tokens",
            "create",
            "--scope",
            "notes:read",
            "--expires-in",
            "0",
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    Ok(())
}

#[test]
fn connectors_allow_reports_every_missing_input() -> Check {
    let home = short_home()?;
    let output = run(home.path(), &["connectors", "allow"])?;
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    let flat = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("Can't allow a connector. 2 inputs are missing: --request, --code."),
        "{stderr}"
    );
    assert!(!stderr.contains("Or then"), "{stderr}");
    Ok(())
}

#[test]
fn stats_modes_follow_wait_and_watch_as_documented() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let output = run(
        home.path(),
        &[
            "stats",
            "--watch",
            "--settle",
            "0",
            "--timeout",
            "0",
            "--json",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout).lines().last(),
        Some(r#"{"event":"timeout","total":0}"#)
    );
    let output = run(home.path(), &["stats", "--wait", "--timeout", "0"])?;
    assert!(output.status.success());
    let output = bridge()
        .env("CI", "1")
        .args(["stats", "--follow", "--json", "--home"])
        .arg(home.path())
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "missing_timeout");
    assert_eq!(document["message"], "Can't follow without --timeout in CI.");
    let output = run(home.path(), &["stats"])?;
    assert!(output.status.success());
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(text(&output.stderr).contains("Nothing has synced yet"));
    let output = run(home.path(), &["stats", "--json", "--fields", "total"])?;
    let document: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(document, json!({"total": 0}));
    Ok(())
}

#[test]
fn doctor_keeps_exit_zero_for_a_report_and_check_mode_fails() -> Check {
    let home = short_home()?;
    gateway_home(home.path())?;
    let report = run(home.path(), &["doctor", "--json"])?;
    assert_eq!(report.status.code(), Some(0), "{}", text(&report.stderr));
    let document: Value = serde_json::from_slice(&report.stdout)?;
    let check = run(home.path(), &["doctor", "--check", "--json"])?;
    if document["ok"] == true {
        assert_eq!(check.status.code(), Some(0));
    } else {
        assert_eq!(check.status.code(), Some(1));
        assert_eq!(error_document(&check)?["reason"], "unhealthy_home");
    }
    let plain = bridge()
        .env("APPLE_BRIDGE_LEGACY_JSON", "0")
        .args(["doctor", "--home"])
        .arg(home.path())
        .output()?;
    assert_eq!(plain.status.code(), Some(0));
    let shown = text(&plain.stdout);
    assert!(
        !shown.contains("true") && !shown.contains("false"),
        "{shown}"
    );
    Ok(())
}

#[test]
fn piped_permissions_guide_lists_the_statuses_then_refuses() -> Check {
    let home = short_home()?;
    let fixture = home.path().join("fixture.json");
    std::fs::write(
        &fixture,
        json!({"statuses": {"contacts": "allowed"}}).to_string(),
    )?;
    let output = bridge()
        .args(["permissions", "guide", "--fixture"])
        .arg(&fixture)
        .arg("--home")
        .arg(home.path())
        .output()?;
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
    let listed = text(&output.stdout);
    assert!(
        listed.starts_with("Permission\tStatus\tUsed for\n"),
        "{listed}"
    );
    assert!(listed.lines().count() > 5, "{listed}");
    assert!(
        listed
            .lines()
            .nth(1)
            .is_some_and(|row| row.split('\t').count() == 3),
        "{listed}"
    );
    assert!(text(&output.stderr).contains("without a terminal"));
    Ok(())
}

#[test]
fn bad_paths_and_homes_are_usage_errors() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["up", "--helper-app", "/nonexistent/Helper.app", "--json"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "invalid_value");
    let output = bridge()
        .env("APPLE_BRIDGE_HOME", "")
        .args(["status", "--json"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "invalid_home");
    Ok(())
}

#[test]
fn down_reports_the_home_on_its_own_row() -> Check {
    let home = short_home()?;
    let output = run(home.path(), &["down"])?;
    assert!(output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("Already stopped  Apple Bridge"), "{stderr}");
    assert!(
        stderr.contains(&format!("Home            {}", home.path().display())),
        "{stderr}"
    );
    Ok(())
}

fn fake_program(directory: &Path, name: &str, script: &str) -> Result<PathBuf, std::io::Error> {
    let path = directory.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

fn with_path(directory: &Path) -> String {
    format!(
        "{}:{}",
        directory.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

#[test]
fn link_stops_a_slow_client_at_the_deadline_and_reaps_it() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
    let bin = short_home()?;
    let pid_file = bin.path().join("pid");
    fake_program(
        bin.path(),
        "claude",
        &format!("echo $$ > {}\nexec sleep 30", pid_file.display()),
    )?;
    let began = Instant::now();
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .args([
            "link",
            "claude",
            "--no-login",
            "--timeout",
            "1s",
            "--json",
            "--home",
        ])
        .arg(home.path())
        .output()?;
    assert!(
        began.elapsed() < Duration::from_secs(8),
        "{:?}",
        began.elapsed()
    );
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "timeout");
    assert!(document["data"]["confirmed"].is_string(), "{document}");
    if let Ok(pid) = std::fs::read_to_string(&pid_file) {
        let pid = pid.trim();
        let alive = Command::new("/bin/kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()?
            .success();
        assert!(!alive, "the fake client {pid} is still running");
    }
    Ok(())
}

#[test]
fn link_prints_commands_on_their_own_lines_and_a_json_document() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
    let runtime: Value = serde_json::from_slice(&std::fs::read(home.path().join("runtime.json"))?)?;
    let url = format!(
        "http://{}/mcp",
        runtime["api_address"].as_str().ok_or("address")?
    );
    let bin = short_home()?;
    fake_program(
        bin.path(),
        "claude",
        &format!("if [ \"$2\" = get ]; then echo \"apple-bridge: {url}\"; fi\nexit 0"),
    )?;
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .env("COLUMNS", "40")
        .args(["link", "claude", "--no-login", "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stderr = text(&output.stderr);
    assert!(
        stderr
            .lines()
            .any(|line| line.starts_with("    bridge skills get core")),
        "{stderr}"
    );
    assert!(
        stderr
            .lines()
            .any(|line| line == "    claude mcp login apple-bridge"),
        "{stderr}"
    );
    assert!(!stderr.contains("--no-browser"), "{stderr}");
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .args(["link", "claude", "--no-login", "--json", "--home"])
        .arg(home.path())
        .output()?;
    let document: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(document["outcome"], "registered");
    assert_eq!(document["url"], url.as_str());
    let printed = run(home.path(), &["link", "claude", "--print"])?;
    assert_eq!(text(&printed.stdout).lines().count(), 2);
    let printed = run(home.path(), &["link", "claude", "--print", "--json"])?;
    let document: Value = serde_json::from_slice(&printed.stdout)?;
    assert_eq!(document["login"]["argv"][0], "claude");
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .args(["unlink", "claude", "--json", "--home"])
        .arg(home.path())
        .output()?;
    let document: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(document["outcome"], "removed");
    let output = run(home.path(), &["install-credentials", "--json"])?;
    let document: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(document["outcome"], "ready");
    Ok(())
}

#[test]
fn result_schemas_name_the_fields_commands_return() -> Check {
    let home = short_home()?;
    gateway_home(home.path())?;
    let fixture = home.path().join("fixture.json");
    std::fs::write(&fixture, json!({"statuses": {}}).to_string())?;
    let fixture = fixture.to_string_lossy().into_owned();
    let cases: [(&[&str], &[&str]); 3] = [
        (&["stats", "--json"], &["stats"]),
        (
            &["permissions", "status", "--fixture", &fixture, "--json"],
            &["permissions", "status"],
        ),
        (&["doctor", "--json"], &["doctor"]),
    ];
    for (command, path) in cases {
        let output = run(home.path(), command)?;
        assert!(
            output.status.success(),
            "{command:?}: {}",
            text(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout)?;
        let mut help = path.to_vec();
        help.extend(["--help", "--json"]);
        let described: Value = serde_json::from_slice(&bridge().args(&help).output()?.stdout)?;
        let fields: Vec<&str> = described["result_schema"]["fields"]
            .as_array()
            .ok_or("fields")?
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect();
        for key in result.as_object().ok_or("object")?.keys() {
            assert!(
                fields.contains(&key.as_str()),
                "{command:?} returned {key}, schema {fields:?}"
            );
        }
    }
    let described: Value =
        serde_json::from_slice(&bridge().args(["call", "--help", "--json"]).output()?.stdout)?;
    assert_eq!(
        described["result_schema"]["see"],
        "bridge call --list --json"
    );
    Ok(())
}

fn answer_remote_requests(home: PathBuf, link: &'static str, url: &'static str) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut answered = String::new();
        for _ in 0..600 {
            std::thread::sleep(Duration::from_millis(20));
            let Ok(config) = ab_remote::state::read::<ab_remote::state::Config>(&home, "config")
            else {
                continue;
            };
            if config.id.is_empty() || config.id == answered {
                continue;
            }
            std::thread::sleep(Duration::from_millis(100));
            let _ = runtime.block_on(ab_remote::onboarding::send_link(&home, &config.id, link));
            std::thread::sleep(Duration::from_millis(200));
            let status = json!({"id": config.id, "state": "on", "backend": "relay", "url": url, "last_error": ""});
            let _ = std::fs::write(home.join("remote/status.json"), status.to_string());
            answered = config.id;
        }
    });
}

#[test]
fn remote_on_and_enable_open_the_sign_in_link_without_a_terminal() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
    let bin = short_home()?;
    let opened = bin.path().join("opened");
    fake_program(
        bin.path(),
        "open",
        &format!("echo \"$1\" >> {}", opened.display()),
    )?;
    answer_remote_requests(
        home.path().to_path_buf(),
        "https://relay.example.test/sign-in",
        "https://mac.example.test",
    );
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_TTY")
        .args(["remote", "on", "--home"])
        .arg(home.path())
        .stderr(Stdio::null())
        .output()?;
    assert!(output.status.success());
    let lines: Vec<String> = text(&output.stdout).lines().map(str::to_owned).collect();
    assert_eq!(
        lines,
        [
            "https://relay.example.test/sign-in",
            "https://mac.example.test",
            "https://mac.example.test/mcp"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(&opened)?,
        "https://relay.example.test/sign-in\n"
    );
    std::fs::remove_file(home.path().join("remote/status.json"))?;
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_TTY")
        .args(["remote", "enable", "--open", "--home"])
        .arg(home.path())
        .stderr(Stdio::null())
        .output()?;
    assert!(output.status.success());
    let lines: Vec<String> = text(&output.stdout).lines().map(str::to_owned).collect();
    assert_eq!(
        lines,
        [
            "https://relay.example.test/sign-in",
            "https://mac.example.test"
        ]
    );
    assert_eq!(std::fs::read_to_string(&opened)?.lines().count(), 2);
    std::fs::remove_file(home.path().join("remote/status.json"))?;
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .env("SSH_CONNECTION", "10.0.0.1 1 10.0.0.2 22")
        .args(["remote", "enable", "--open", "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        std::fs::read_to_string(&opened)?.lines().count(),
        2,
        "opened over SSH"
    );
    assert!(text(&output.stderr).contains("Open the link above"));
    Ok(())
}

#[test]
fn array_results_from_old_spellings_still_warn_on_stderr() -> Check {
    let home = short_home()?;
    let output = run(home.path(), &["web", "devices", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout).trim(), "[]");
    let warning = error_document(&output)?;
    assert_eq!(warning["status"], "warning");
    assert_eq!(warning["reason"], "deprecated");
    Ok(())
}

#[test]
fn the_system_route_is_accepted_again() -> Check {
    let home = short_home()?;
    for spelling in [
        &["remote", "off", "--via", "system"][..],
        &["remote", "disable", "--via", "system"][..],
    ] {
        let output = run(home.path(), spelling)?;
        assert!(
            output.status.success(),
            "{spelling:?}: {}",
            text(&output.stderr)
        );
    }
    let output = run(home.path(), &["remote", "off", "--via", "system"])?;
    assert_eq!(text(&output.stdout), "off\n");
    Ok(())
}

#[test]
fn fields_pick_record_data_in_reads() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| {
        ok(json!({"records": [record("notes.note", "n1")], "cursor": null, "next": null}))
    })?;
    let output = run(
        home.path(),
        &["read", "notes", "--json", "--fields", "title"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let document: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        document["records"][0]["data"],
        json!({"title": "item n1"}),
        "{document}"
    );
    let output = run(
        home.path(),
        &["read", "mail", "--json", "--fields", "nonsense"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "invalid_value");
    Ok(())
}

#[test]
fn the_singular_agent_spelling_still_needs_consent_to_widen() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &[
            "agents", "create", "--name", "bot", "--level", "approve", "--json",
        ],
    )?;
    let created: Value = serde_json::from_slice(&output.stdout)?;
    let id = created["agent"]["id"].as_str().ok_or("id")?.to_owned();
    let output = run(
        home.path(),
        &["agent", "update", &id, "--level", "full", "--json"],
    )?;
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stdout));
    assert_eq!(error_document(&output)?["reason"], "confirmation_required");
    let output = run(home.path(), &["agent", "level", &id, "full", "--json"])?;
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stdout));
    Ok(())
}

#[test]
fn a_client_whose_children_keep_its_output_open_still_meets_the_deadline() -> Check {
    let bin = short_home()?;
    fake_program(bin.path(), "claude", "sleep 20 &\nexit 0")?;
    let home = short_home()?;
    let began = Instant::now();
    let output = bridge()
        .env("PATH", with_path(bin.path()))
        .args(["unlink", "claude", "--timeout", "1s", "--json", "--home"])
        .arg(home.path())
        .output()?;
    assert!(
        began.elapsed() < Duration::from_secs(4),
        "{:?}",
        began.elapsed()
    );
    assert!(output.status.code().is_some(), "{}", text(&output.stderr));
    Ok(())
}

#[test]
fn the_read_timeout_bounds_a_slow_page() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| {
        std::thread::sleep(Duration::from_secs(3));
        ok(json!({"records": [], "cursor": null, "next": null}))
    })?;
    let began = Instant::now();
    let output = run(
        home.path(),
        &["search", "item", "--timeout", "300ms", "--json"],
    )?;
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "{:?}",
        began.elapsed()
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(error_document(&output)?["reason"], "timeout");
    Ok(())
}

#[test]
fn an_unreadable_answer_to_a_write_is_unknown_outcome() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) =
        scripted_gateway(home.path(), |_, _| (200, Vec::new(), json!("raw:broken")))?;
    let output = bridge()
        .env("REVIEW_TOKEN", "aba-REVIEW")
        .args([
            "call",
            "photos.fetch",
            "--data",
            r#"{"args":{"asset_id":"fixture"}}"#,
            "--token-env",
            "REVIEW_TOKEN",
            "--json",
            "--home",
        ])
        .arg(home.path())
        .output()?;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    assert_eq!(error_document(&output)?["reason"], "unknown_outcome");
    assert_eq!(requests(&log, "photos.fetch").len(), 1);
    Ok(())
}
