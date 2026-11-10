#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::Duration,
};
use support::*;

fn json_out(output: &Output) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn created_agent(home: &Path, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let output = run(
        home,
        &[
            "agents",
            "create",
            "--name",
            name,
            "--level",
            "read",
            "--expires-in",
            "1d",
            "--json",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    Ok(json_out(&output)?["agent"]["id"]
        .as_str()
        .ok_or("id")?
        .to_owned())
}

fn remote_status(home: &Path, status: &Value) -> Check {
    std::fs::create_dir_all(home.join("remote"))?;
    std::fs::write(home.join("remote/status.json"), status.to_string())?;
    Ok(())
}

fn fail_remote_requests(home: PathBuf, cause: &'static str) {
    std::thread::spawn(move || {
        for _ in 0..600 {
            std::thread::sleep(Duration::from_millis(20));
            let Ok(config) = ab_remote::state::read::<ab_remote::state::Config>(&home, "config")
            else {
                continue;
            };
            if config.id.is_empty() {
                continue;
            }
            let status = json!({
                "id": config.id,
                "state": "error",
                "last_error": format!("Remote setup failed: {cause}. Turn Remote off and try again."),
            });
            let _ = std::fs::write(home.join("remote/status.json"), status.to_string());
            return;
        }
    });
}

fn opener_bin() -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
    let bin = short_home()?;
    let opened = bin.path().join("opened");
    for launcher in ["open", "xdg-open"] {
        fake_program(
            bin.path(),
            launcher,
            &format!("echo \"$1\" >> {}", opened.display()),
        )?;
    }
    Ok((bin, opened))
}

fn local_bridge(bin: &Path) -> Command {
    let mut command = bridge();
    command
        .env("PATH", with_path(bin))
        .env_remove("SSH_CONNECTION")
        .env_remove("SSH_TTY");
    command
}

#[test]
fn options_written_before_the_remote_action_still_apply() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["remote", "--relay-url", "https://b.example.test", "key"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let key = text(&output.stdout);
    assert_eq!(key.trim_end().len(), 43, "{key}");
    assert_eq!(key.lines().count(), 1);
    let config: Value =
        serde_json::from_slice(&std::fs::read(home.path().join("remote/config.json"))?)?;
    assert_eq!(config["relay_url"], "https://b.example.test");

    remote_status(
        home.path(),
        &json!({"id": "r1", "state": "on", "backend": "relay", "url": "https://mac.example.test", "last_error": ""}),
    )?;
    let output = run(home.path(), &["remote", "--snippets", "status"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).starts_with(
            "Claude Code: claude mcp add --transport http apple-bridge https://mac.example.test/mcp"
        ),
        "{}",
        text(&output.stdout)
    );
    let output = run(home.path(), &["remote", "status", "--snippets", "--json"])?;
    assert!(output.status.success());
    let document = json_out(&output)?;
    assert!(
        document["snippets"]
            .as_str()
            .is_some_and(|snippets| snippets.contains("https://mac.example.test/mcp")),
        "{document}"
    );
    assert_eq!(output.stderr, [] as [u8; 0]);
    Ok(())
}

#[test]
fn a_relay_address_with_a_password_is_refused_before_anything_is_saved() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["remote", "--relay-url", "https://b.example.test", "key"],
    )?;
    assert!(output.status.success());
    let before = std::fs::read(home.path().join("remote/config.json"))?;
    for arguments in [
        &[
            "remote",
            "--relay-url",
            "https://me:hunter2@c.example.test",
            "key",
        ][..],
        &[
            "remote",
            "enable",
            "--relay-url",
            "https://me:hunter2@c.example.test",
        ][..],
        &[
            "remote",
            "on",
            "--relay-url",
            "https://me:hunter2@c.example.test",
        ][..],
    ] {
        let output = bridge()
            .env("COLUMNS", "400")
            .args(arguments)
            .arg("--home")
            .arg(home.path())
            .output()?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(output.stdout, [] as [u8; 0]);
        let stderr = text(&output.stderr);
        assert!(!stderr.contains("hunter2"), "{stderr}");
        assert!(!stderr.contains("me:"), "{stderr}");
        assert!(
            stderr.contains("Relay addresses can't include a user name or password."),
            "{stderr}"
        );
        assert!(stderr.contains("Nothing was saved."), "{stderr}");
    }
    let output = run(
        home.path(),
        &[
            "remote",
            "enable",
            "--relay-url",
            "https://me:hunter2@c.example.test",
            "--json",
            "--debug",
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    assert!(!stderr.contains("hunter2"), "{stderr}");
    assert_eq!(error_document(&output)?["reason"], "invalid_value");
    assert_eq!(
        std::fs::read(home.path().join("remote/config.json"))?,
        before
    );
    Ok(())
}

#[test]
fn explicit_input_wins_and_errors_do_not_depend_on_order() -> Check {
    let home = short_home()?;
    let missing = home.path().join("missing");
    let missing = missing.to_str().ok_or("path")?;
    let output = bridge()
        .args(["status", "--json", "--json=false", "--home", missing])
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).starts_with("Error: "),
        "{}",
        text(&output.stderr)
    );
    let output = bridge()
        .args(["status", "--json=false", "--json", "--home", missing])
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(error_document(&output)?["reason"], "not_set_up");

    let first = run(home.path(), &["--json", "status", "--wat"])?;
    let last = run(home.path(), &["status", "--wat", "--json"])?;
    assert_eq!(first.status.code(), Some(2));
    assert_eq!(first.stderr, last.stderr);
    assert_eq!(error_document(&first)?["reason"], "unknown_option");

    for arguments in [
        &["typo", "--help"][..],
        &["help", "typo"][..],
        &["agents", "typo", "--help"][..],
        &["help", "agents", "typo"][..],
    ] {
        let output = bridge().args(arguments).output()?;
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert_eq!(output.stdout, [] as [u8; 0], "{arguments:?}");
        assert!(
            text(&output.stderr).starts_with("Error: Unknown command typo for bridge"),
            "{arguments:?}: {}",
            text(&output.stderr)
        );
    }

    let output = run(
        home.path(),
        &["agents", "create", "--name", "a", "--name", "b"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).starts_with("Error: --name can be given once."));
    assert!(!home.path().join("credentials.json").exists());
    Ok(())
}

#[test]
fn limit_zero_with_json_prints_one_envelope_as_before() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), |_, _| {
        ok(json!({"records": [record("notes.note", "n1")], "cursor": null, "next": "more"}))
    })?;
    let output = run(home.path(), &["read", "notes", "--limit", "0", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let document = json_out(&output)?;
    assert_eq!(document["records"], json!([]), "{document}");
    assert_eq!(document["next_cursor"], Value::Null);
    let output = run(home.path(), &["search", "item", "--limit", "0", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let document = json_out(&output)?;
    assert_eq!(document["records"].as_array().map(Vec::len), Some(1));
    assert_eq!(document["next_cursor"], Value::Null);
    let calls = log.lock().map(|log| log.len()).unwrap_or_default();
    assert_eq!(calls, 2);
    let output = run(home.path(), &["read", "notes", "--limit", "0"])?;
    assert_eq!(output.status.code(), Some(2));
    Ok(())
}

fn photo_pages(operation: &str, body: &Value) -> Reply {
    if operation != "call" && operation != "photos.search" {
        return ok(json!({}));
    }
    let arguments = if body["args"].is_object() {
        &body["args"]
    } else {
        &body["arguments"]["args"]
    };
    let Some(cursor) = arguments["cursor"].as_str() else {
        return ok(
            json!({"result_json": {"assets": [], "next_cursor": "o0", "examined_count": 0}}),
        );
    };
    let offset: usize = cursor.trim_start_matches('o').parse().unwrap_or(0);
    let asked = arguments["limit"]
        .as_u64()
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(10);
    let end = (offset + asked.min(2)).min(5);
    let assets: Vec<Value> = (offset..end)
        .map(|index| json!({"id": format!("a{}", index + 1)}))
        .collect();
    let next = if end < 5 {
        json!(format!("o{end}"))
    } else {
        Value::Null
    };
    ok(
        json!({"result_json": {"assets": assets, "next_cursor": next, "examined_count": assets.len()}}),
    )
}

#[test]
fn photos_follow_cursors_to_the_total_limit_and_offer_next() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, log) = scripted_gateway(home.path(), photo_pages)?;
    let output = run(home.path(), &["read", "photos", "--limit", "3", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let document = json_out(&output)?;
    let ids: Vec<&str> = document["result_json"]["assets"]
        .as_array()
        .ok_or("assets")?
        .iter()
        .filter_map(|asset| asset["id"].as_str())
        .collect();
    assert_eq!(ids, ["a1", "a2", "a3"], "{document}");
    let cursor = document["next_cursor"].as_str().ok_or("cursor")?.to_owned();
    assert!(log.lock().map(|log| log.len()).unwrap_or_default() >= 3);

    let output = run(
        home.path(),
        &["read", "photos", "--next", &cursor, "--json"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let document = json_out(&output)?;
    let ids: Vec<&str> = document["result_json"]["assets"]
        .as_array()
        .ok_or("assets")?
        .iter()
        .filter_map(|asset| asset["id"].as_str())
        .collect();
    assert_eq!(ids, ["a4", "a5"], "{document}");
    assert_eq!(document["next_cursor"], Value::Null);

    let output = run(home.path(), &["read", "photos", "--limit", "1"])?;
    assert!(output.status.success());
    let stderr = text(&output.stderr);
    assert!(stderr.contains("More photos match."), "{stderr}");
    assert!(stderr.contains("bridge read photos --next "), "{stderr}");
    Ok(())
}

fn seed_states(home: &Path, states: &[(ab_trust::domain::Source, &str)]) -> Check {
    use ab_trust::domain::Coverage;
    let coverage = || Coverage {
        permissions: std::collections::BTreeMap::new(),
        pack_version: String::new(),
        operations: Vec::new(),
        fields: "complete".into(),
        missing: None,
    };
    let mut state = ab_state::State::<ab_state::Gateway>::open(home)?;
    for (source, value) in states {
        state.source_state(*source, value, coverage(), None, "")?;
    }
    Ok(())
}

#[test]
fn stats_wait_fails_on_timeout_and_on_a_source_that_needs_attention() -> Check {
    use ab_trust::domain::Source;
    let home = short_home()?;
    seed_states(home.path(), &[(Source::Contacts, "ready")])?;
    let output = run(
        home.path(),
        &[
            "stats",
            "--wait",
            "--settle",
            "30s",
            "--timeout",
            "1s",
            "--json",
        ],
    )?;
    assert_eq!(output.status.code(), Some(1));
    let error = error_document(&output)?;
    assert_eq!(error["reason"], "timeout");
    assert_eq!(error["next"][0]["argv"][1], "stats");
    let home_text = home.path().to_string_lossy().into_owned();
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains(&home_text)),
        "{error}"
    );
    let stderr = text(&output.stderr);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        1,
        "{stderr}"
    );

    let output = run(
        home.path(),
        &["stats", "--wait", "--settle", "30s", "--timeout", "1s"],
    )?;
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr
            .lines()
            .any(|line| line.trim() == format!("bridge stats --home {home_text}")),
        "{stderr}"
    );
    assert!(!stderr.contains("bridge down"), "{stderr}");

    let other = short_home()?;
    seed_states(
        other.path(),
        &[
            (Source::Contacts, "ready"),
            (Source::Messages, "permission_missing"),
        ],
    )?;
    let output = run(
        other.path(),
        &[
            "stats",
            "--wait",
            "--settle",
            "0",
            "--timeout",
            "5s",
            "--json",
        ],
    )?;
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let last: Value =
        serde_json::from_str(text(&output.stdout).lines().last().ok_or("no events")?)?;
    assert_eq!(last["event"], "all_finished");
    assert_eq!(last["outcome"], "partial");
    assert_eq!(error_document(&output)?["reason"], "permission_missing");
    Ok(())
}

#[test]
fn never_expiring_access_warns_without_a_terminal() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["agents", "create", "--name", "bot", "--level", "read"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("Warning: bot's token never expires. Pass --expires-in to limit it."),
        "{stderr}"
    );
    let output = run(
        home.path(),
        &[
            "agents",
            "create",
            "--name",
            "brief",
            "--level",
            "read",
            "--expires-in",
            "30d",
        ],
    )?;
    assert!(output.status.success());
    assert!(!text(&output.stderr).contains("never expires"));

    let output = run(
        home.path(),
        &[
            "agents", "create", "--name", "j", "--level", "read", "--json",
        ],
    )?;
    assert!(output.status.success());
    assert_eq!(output.stderr, [] as [u8; 0]);
    let document = json_out(&output)?;
    assert_eq!(document["warnings"][0]["reason"], "never_expires");
    assert!(document.get("why").is_none());

    let output = run(
        home.path(),
        &["tokens", "create", "--scope", "reminders:read"],
    )?;
    assert!(output.status.success());
    let token = text(&output.stdout);
    assert!(
        token.starts_with("abt-") && token.lines().count() == 1,
        "{token}"
    );
    assert!(text(&output.stderr).contains("never expires"));
    Ok(())
}

#[test]
#[cfg(target_os = "macos")]
fn a_terminal_asks_before_creating_access_that_never_expires() -> Check {
    let home = short_home()?;
    let output = Command::new("/usr/bin/script")
        .args(["-q", "/dev/null"])
        .arg(env!("CARGO_BIN_EXE_bridge"))
        .args([
            "agents", "create", "--name", "bot", "--level", "read", "--home",
        ])
        .arg(home.path())
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .env("NO_COLOR", "1")
        .env("COLUMNS", "80")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .env_remove("AI_AGENT")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env_remove("CODEX_SANDBOX")
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CURSOR_AGENT")
        .env_remove("GEMINI_CLI")
        .env_remove("CLINE_ACTIVE")
        .env_remove("OPENCODE")
        .env_remove("AMP_AGENT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(b"n\n");
            }
            child.wait_with_output()
        })?;
    let screen = text(&output.stdout);
    assert!(
        screen.contains("Create agent bot with access that never expires? (y/N)"),
        "{screen}"
    );
    assert!(screen.contains("Nothing was created."), "{screen}");
    assert!(!screen.contains("aba-"), "{screen}");
    for row in ["Level", "Access", "Expires"] {
        assert!(screen.contains(row), "{row}: {screen}");
    }
    let listed = run(home.path(), &["agents", "list", "--json"])?;
    assert_eq!(json_out(&listed)?["agents"], json!([]));
    Ok(())
}

#[test]
fn plural_spellings_key_results_by_noun_and_old_ones_keep_their_shape() -> Check {
    let home = short_home()?;
    let id = created_agent(home.path(), "bot")?;
    let output = run(home.path(), &["agents", "list", "--json"])?;
    assert!(output.status.success());
    let listed = json_out(&output)?;
    assert_eq!(listed["agents"][0]["id"], id.as_str());
    assert_eq!(listed["next_cursor"], Value::Null);
    let output = run(home.path(), &["agent", "list", "--json"])?;
    assert!(json_out(&output)?.is_array());
    let output = run(home.path(), &["agents", "list"])?;
    assert!(json_out(&output)?.is_array());

    let output = run(
        home.path(),
        &[
            "tokens",
            "create",
            "--scope",
            "reminders:read",
            "--expires-in",
            "1h",
            "--json",
        ],
    )?;
    let token = json_out(&output)?;
    assert_eq!(token["outcome"], "created");
    assert!(
        token["secret"]
            .as_str()
            .is_some_and(|secret| secret.starts_with("abt-"))
    );
    assert!(
        token["token"]["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("token:"))
    );
    assert_eq!(token["token"]["remote"], false);
    assert!(token["token"]["expiresAt"].is_string());
    let output = run(
        home.path(),
        &[
            "tokens",
            "create",
            "--scope",
            "reminders:read",
            "--expires-in",
            "1h",
            "--json",
            "--fields",
            "id",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let picked = json_out(&output)?;
    assert!(picked["token"]["id"].is_string(), "{picked}");
    assert!(picked.get("secret").is_none(), "{picked}");

    let output = run(home.path(), &["agents", "revoke", &id, "--yes", "--json"])?;
    assert!(output.status.success());
    assert_eq!(output.stderr, [] as [u8; 0]);
    let revoked = json_out(&output)?;
    assert_eq!(revoked["outcome"], "revoked");
    assert_eq!(revoked["agent"]["id"], id.as_str());
    assert!(revoked["next"].is_array());
    let output = run(home.path(), &["agents", "revoke", &id, "--yes", "--json"])?;
    assert_eq!(json_out(&output)?["outcome"], "unchanged");

    let other = created_agent(home.path(), "old")?;
    let output = run(home.path(), &["agent", "revoke", &other, "--yes", "--json"])?;
    let flat = json_out(&output)?;
    assert_eq!(flat["id"], other.as_str());
    assert_eq!(flat["outcome"], "revoked");

    let output = run(home.path(), &["web", "sessions", "list", "--json"])?;
    assert_eq!(
        json_out(&output)?,
        json!({"sessions": [], "next_cursor": null})
    );
    let output = run(home.path(), &["web", "devices"])?;
    assert_eq!(text(&output.stdout).trim(), "[]");
    Ok(())
}

#[test]
fn admin_lists_are_bounded_and_continue_with_next() -> Check {
    let home = short_home()?;
    let ids: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|name| created_agent(home.path(), name))
        .collect::<Result<_, _>>()?;
    let output = run(home.path(), &["agents", "list", "--limit", "2", "--json"])?;
    let page = json_out(&output)?;
    let shown: Vec<&str> = page["agents"]
        .as_array()
        .ok_or("agents")?
        .iter()
        .filter_map(|agent| agent["id"].as_str())
        .collect();
    assert_eq!(shown, [ids[0].as_str(), ids[1].as_str()]);
    let cursor = page["next_cursor"].as_str().ok_or("cursor")?.to_owned();
    let output = run(
        home.path(),
        &["agents", "list", "--next", &cursor, "--json"],
    )?;
    let page = json_out(&output)?;
    assert_eq!(page["agents"][0]["id"], ids[2].as_str());
    assert_eq!(page["agents"].as_array().map(Vec::len), Some(1));
    assert_eq!(page["next_cursor"], Value::Null);

    let output = run(home.path(), &["agents", "list", "--limit", "201"])?;
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let output = run(home.path(), &["agents", "list", "--limit", "nope"])?;
    assert_eq!(output.status.code(), Some(2));
    let output = run(home.path(), &["web", "sessions", "list", "--next", &cursor])?;
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
    Ok(())
}

#[test]
fn a_missing_home_is_an_error_for_lists() -> Check {
    let parent = short_home()?;
    let missing = parent.path().join("nowhere");
    for arguments in [
        &["agents", "list"][..],
        &["agents", "list", "--json"][..],
        &["connectors", "list"][..],
        &["web", "sessions", "list"][..],
    ] {
        let output = run(&missing, arguments)?;
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert_eq!(output.stdout, [] as [u8; 0], "{arguments:?}");
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains("Apple Bridge isn't set up in"),
            "{arguments:?}: {stderr}"
        );
        assert!(
            stderr.contains("bridge up --home "),
            "{arguments:?}: {stderr}"
        );
    }
    assert!(!missing.exists());
    Ok(())
}

#[test]
fn one_update_reports_every_bad_value() -> Check {
    let home = short_home()?;
    let id = created_agent(home.path(), "bot")?;
    let output = run(
        home.path(),
        &[
            "agents", "update", &id, "--level", "root", "--scope", "made-up",
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("Error: Can't update that agent."),
        "{stderr}"
    );
    assert!(stderr.contains("root"), "{stderr}");
    assert!(stderr.contains("made-up"), "{stderr}");
    Ok(())
}

#[test]
fn remote_json_returns_a_pending_link_and_no_open_never_opens() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
    let (bin, opened) = opener_bin()?;
    answer_remote_requests(
        home.path().to_path_buf(),
        "https://relay.example.test/sign-in",
        "https://mac.example.test",
    );
    let output = local_bridge(bin.path())
        .args(["remote", "enable", "--json", "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let pending = json_out(&output)?;
    assert_eq!(pending["outcome"], "pending");
    assert_eq!(pending["userActionRequired"], true);
    assert_eq!(
        pending["remote"]["signInUrl"],
        "https://relay.example.test/sign-in"
    );
    assert!(pending["next"].is_array());
    assert_eq!(output.stderr, [] as [u8; 0]);
    assert!(!opened.exists());

    std::thread::sleep(Duration::from_millis(500));
    let _ = std::fs::remove_file(home.path().join("remote/status.json"));
    let output = local_bridge(bin.path())
        .args(["remote", "on", "--no-open", "--home"])
        .arg(home.path())
        .stderr(Stdio::null())
        .output()?;
    assert!(output.status.success());
    assert!(text(&output.stdout).starts_with("https://relay.example.test/sign-in\n"));
    assert!(!opened.exists(), "--no-open still opened a browser");

    let _ = std::fs::remove_file(home.path().join("remote/status.json"));
    let output = local_bridge(bin.path())
        .args(["remote", "on", "--json", "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let document = json_out(&output)?;
    assert!(document.is_object(), "{}", text(&output.stdout));
    Ok(())
}

#[test]
fn a_failed_remote_attempt_names_the_host_and_the_cause() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
    let (bin, _opened) = opener_bin()?;
    fail_remote_requests(home.path().to_path_buf(), "connection refused");
    let output = local_bridge(bin.path())
        .args([
            "remote",
            "enable",
            "--relay-url",
            "https://relay.example.test",
            "--no-open",
            "--home",
        ])
        .arg(home.path())
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("Couldn't reach relay.example.test. Connection refused."),
        "{stderr}"
    );
    assert!(stderr.contains("bridge remote status"), "{stderr}");
    Ok(())
}

#[test]
fn remote_status_off_is_not_a_warning() -> Check {
    let home = short_home()?;
    let output = bridge()
        .env("TERM", "dumb")
        .args(["remote", "status", "--json=false", "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success());
    let stderr = text(&output.stderr);
    assert!(!stderr.contains('!'), "{stderr}");
    assert!(!stderr.contains("Warning: Remote"), "{stderr}");
    Ok(())
}

#[test]
fn the_working_folder_option_resolves_relative_paths_and_help_stays_offline() -> Check {
    let parent = short_home()?;
    std::fs::create_dir(parent.path().join("rel"))?;
    let output = bridge()
        .arg("--cwd")
        .arg(parent.path())
        .args([
            "agents",
            "create",
            "--name",
            "x",
            "--level",
            "read",
            "--expires-in",
            "1d",
            "--home",
            "rel",
        ])
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(parent.path().join("rel/credentials.json").is_file());
    for arguments in [
        &["--help"][..],
        &["agents", "--help"][..],
        &["help", "up"][..],
    ] {
        let output = bridge()
            .args(["--cwd", "/does/not/exist"])
            .args(arguments)
            .output()?;
        assert!(output.status.success(), "{arguments:?}");
        assert_ne!(output.stdout, [] as [u8; 0]);
    }
    let output = bridge()
        .args(["--cwd", "/does/not/exist", "status"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    Ok(())
}

#[test]
fn help_lists_result_schemas_and_safe_examples() -> Check {
    let output = bridge().args(["help", "--all", "--json"]).output()?;
    assert!(output.status.success());
    let all: Value = serde_json::from_slice(&output.stdout)?;
    let commands = all["commands"].as_array().ok_or("commands")?;
    let schema = |path: &str| -> Vec<String> {
        commands
            .iter()
            .find(|command| command["path"] == format!("bridge {path}").as_str())
            .and_then(|command| command["result_schema"]["fields"].as_array())
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|field| field["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    for (path, field) in [
        ("link", "outcome"),
        ("down", "outcome"),
        ("connectors list", "connectors"),
        ("remote enable", "userActionRequired"),
        ("tokens create", "secret"),
        ("agents list", "next_cursor"),
        ("web sessions list", "sessions"),
    ] {
        assert!(
            schema(path).iter().any(|name| name == field),
            "{path} lacks {field}: {:?}",
            schema(path)
        );
    }
    let help = text(
        &bridge()
            .args(["agents", "create", "--help"])
            .output()?
            .stdout,
    );
    assert!(help.contains("--level read"), "{help}");
    assert!(help.contains("--expires-in 30d"), "{help}");
    let help = text(&bridge().args(["stats", "--help"]).output()?.stdout);
    assert!(
        help.contains("bridge stats --follow --timeout 5m --json"),
        "{help}"
    );
    assert!(help.contains("(default: "), "{help}");
    assert!(!help.contains("[default: "), "{help}");
    Ok(())
}

#[test]
fn accessible_mode_numbers_choices_and_names_phases() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
    let (bin, _opened) = opener_bin()?;
    answer_remote_requests(
        home.path().to_path_buf(),
        "https://relay.example.test/sign-in",
        "https://mac.example.test",
    );
    let output = local_bridge(bin.path())
        .env("APPLE_BRIDGE_ACCESSIBLE", "1")
        .args(["remote", "enable", "--no-open", "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("Step 1 of 2: "), "{stderr}");
    assert!(!stderr.contains('\u{1b}'), "{stderr}");
    let help = text(&bridge().args(["--help"]).output()?.stdout);
    assert!(help.contains("--accessible"), "{help}");
    Ok(())
}

#[test]
fn piped_receipts_are_sentences_at_column_zero() -> Check {
    let home = short_home()?;
    let id = created_agent(home.path(), "bot")?;
    let output = run(
        home.path(),
        &["web", "sessions", "revoke", "--all", "--yes"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stderr), "No browsers are signed in.\n");
    let output = run(home.path(), &["agents", "revoke", &id, "--yes"])?;
    let stderr = text(&output.stderr);
    assert!(
        stderr.lines().next() == Some(&format!("Revoked agent bot ({id}).")[..]),
        "{stderr}"
    );
    Ok(())
}

fn on_a_terminal(arguments: &[&str], color: bool) -> Result<String, Box<dyn std::error::Error>> {
    let mut command = Command::new("/usr/bin/script");
    command
        .args(["-q", "/dev/null"])
        .arg(env!("CARGO_BIN_EXE_bridge"))
        .args(arguments)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .env("COLUMNS", "80")
        .env("TERM", "xterm-256color")
        .env_remove("FORCE_COLOR")
        .stdin(Stdio::null());
    if color {
        command.env_remove("NO_COLOR");
    } else {
        command.env("NO_COLOR", "1");
    }
    Ok(text(&command.output()?.stdout).replace("\r\n", "\n"))
}

#[test]
fn start_stopped_by_sigterm_exits_143_without_a_word() -> Check {
    let home = short_home()?;
    let api = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .to_string();
    let child = bridge()
        .args([
            "start",
            "--role",
            "gateway",
            "--api-addr",
            &api,
            "--peer-addr",
            "127.0.0.1:0",
            "--home",
        ])
        .arg(home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let began = std::time::Instant::now();
    while std::net::TcpStream::connect(&api).is_err() && began.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(100));
    }
    Command::new("/bin/kill")
        .args(["-TERM", &child.id().to_string()])
        .status()?;
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code(), Some(143));
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(
        !text(&output.stderr).contains("Error"),
        "{}",
        text(&output.stderr)
    );
    Ok(())
}

#[test]
#[cfg(target_os = "macos")]
fn a_terminal_shows_permissions_as_one_table_and_colored_headings() -> Check {
    let home = short_home()?;
    let fixture = home.path().join("fixture.json");
    std::fs::write(
        &fixture,
        json!({"statuses": {"contacts": "granted"}}).to_string(),
    )?;
    let fixture = fixture.to_string_lossy().into_owned();
    let home_text = home.path().to_string_lossy().into_owned();
    let screen = on_a_terminal(
        &[
            "permissions",
            "status",
            "--fixture",
            &fixture,
            "--home",
            &home_text,
        ],
        false,
    )?;
    let header = screen
        .lines()
        .find(|line| line.contains("Permission"))
        .ok_or("no header")?;
    assert!(
        header.contains("Status") && header.contains("Used for"),
        "{screen}"
    );
    assert_eq!(screen.matches("Permission ").count(), 1, "{screen}");
    assert!(
        screen
            .lines()
            .any(|line| line.contains("Contacts") && line.contains("allowed")),
        "{screen}"
    );
    assert!(
        screen.lines().all(|line| line.chars().count() <= 80),
        "{screen}"
    );

    let help = on_a_terminal(&["up", "--help"], true)?;
    assert!(help.contains("\u{1b}[1;35mOptions:\u{1b}[0m"), "{help:?}");
    let narrow = Command::new(env!("CARGO_BIN_EXE_bridge"))
        .env("COLUMNS", "40")
        .arg("--help")
        .output()?;
    let narrow = text(&narrow.stdout);
    assert!(
        narrow
            .lines()
            .all(|line| line.chars().count() <= 40 || line.contains("https://")),
        "{narrow}"
    );
    Ok(())
}
