use std::{
    io::Read,
    path::Path,
    process::{Command, Output, Stdio},
};

type Check = Result<(), Box<dyn std::error::Error>>;

fn bridge() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bridge"));
    command
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .env_remove("APPLE_BRIDGE_HOME")
        .env_remove("APPLE_BRIDGE_LEGACY_JSON")
        .env_remove("NO_COLOR")
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

fn error_document(output: &Output) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let stderr = text(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "{stderr}");
    Ok(serde_json::from_str(lines[0])?)
}

fn short_home() -> Result<tempfile::TempDir, std::io::Error> {
    tempfile::tempdir_in("/tmp")
}

fn seeded_home(home: &Path) -> Check {
    drop(ab_state::State::<ab_state::Gateway>::open(home)?);
    Ok(())
}

fn pty(arguments: &[&str], home: &Path, input: &[u8]) -> Result<Output, std::io::Error> {
    let script = "import os,pty,select,subprocess,sys,time\nmaster,slave=pty.openpty()\nproc=subprocess.Popen(sys.argv[2:],stdin=slave,stdout=slave,stderr=slave)\ntime.sleep(1.5)\nos.write(master,bytes.fromhex(sys.argv[1]))\ndata=b''\nend=time.time()+15\nwhile time.time()<end:\n  if proc.poll() is not None and not select.select([master],[],[],0.2)[0]: break\n  if select.select([master],[],[],0.2)[0]:\n    try: chunk=os.read(master,65536)\n    except OSError: break\n    data+=chunk\nproc.wait(timeout=5)\nsys.stdout.buffer.write(data)\nsys.exit(proc.returncode)";
    let mut command = Command::new("python3");
    command
        .args(["-c", script])
        .arg(input.iter().fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        }))
        .arg(env!("CARGO_BIN_EXE_bridge"))
        .args(arguments)
        .arg("--home")
        .arg(home)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .env("TERM", "xterm-256color")
        .env("COLUMNS", "80")
        .env_remove("CI");
    for variable in [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "AI_AGENT",
        "CODEX_SANDBOX",
        "CODEX_THREAD_ID",
    ] {
        command.env_remove(variable);
    }
    command.output()
}

#[test]
fn bare_bridge_and_groups_print_help_to_stdout() -> Check {
    for arguments in [
        &[][..],
        &["agents"][..],
        &["remote"][..],
        &["web", "sessions"][..],
    ] {
        let output = bridge().args(arguments).output()?;
        assert_eq!(output.status.code(), Some(0), "{arguments:?}");
        assert!(text(&output.stdout).starts_with("bridge"), "{arguments:?}");
        assert!(
            output.stderr.is_empty(),
            "{arguments:?}: {}",
            text(&output.stderr)
        );
    }
    let root = text(&bridge().arg("--help").output()?.stdout);
    for heading in [
        "Getting started:",
        "Reading data:",
        "Access:",
        "Remote and replicas:",
        "Maintenance:",
        "Global options:",
        "Learn more:",
    ] {
        assert!(root.contains(heading), "{heading}");
    }
    for hidden in [
        "install-credentials",
        "  start",
        "  connect ",
        "  help ",
        "For more information",
    ] {
        assert!(!root.contains(hidden), "{hidden}");
    }
    Ok(())
}

#[test]
fn help_wins_over_validation_and_is_available_as_json() -> Check {
    let output = bridge().args(["read", "--bad", "--help"]).output()?;
    assert!(output.status.success());
    assert!(text(&output.stdout).starts_with("bridge read <KIND>"));
    let output = bridge()
        .args(["agents", "revoke", "--help", "--json"])
        .output()?;
    let help: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(help["path"], "bridge agents revoke");
    assert!(
        help["examples"]
            .as_array()
            .is_some_and(|examples| !examples.is_empty())
    );
    let output = bridge().args(["help", "--all", "--json"]).output()?;
    let all: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(
        all["commands"]
            .as_array()
            .is_some_and(|commands| commands.len() > 40)
    );
    let output = bridge().args(["help", "search", "sign", "out"]).output()?;
    assert!(text(&output.stdout).contains("web sessions revoke"));
    let output = bridge().args(["completion", "bash"]).output()?;
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("complete -F"));
    Ok(())
}

#[test]
fn version_has_every_spelling_and_json() -> Check {
    for flag in ["-v", "-V", "--version"] {
        let output = bridge().arg(flag).output()?;
        assert_eq!(
            text(&output.stdout),
            format!("bridge {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
    let output = bridge().args(["--version", "--json"]).output()?;
    let version: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));
    Ok(())
}

#[test]
fn usage_errors_exit_two_with_one_suggestion_and_one_json_line() -> Check {
    let home = short_home()?;
    for (typed, suggested) in [("stauts", "status"), ("sttas", "stats")] {
        let output = run(home.path(), &[typed])?;
        assert_eq!(output.status.code(), Some(2));
        let error = text(&output.stderr);
        assert!(
            error.contains(&format!("Did you mean {suggested}?")),
            "{error}"
        );
        assert_eq!(error.matches("Did you mean").count(), 1, "{error}");
    }
    let output = run(
        home.path(),
        &["read", "notes", "--limit", "0", "--json=false"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, [] as [u8; 0]);
    let output = run(home.path(), &["read", "notes", "--limit", "x", "--json"])?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, [] as [u8; 0]);
    let error = error_document(&output)?;
    assert_eq!(error["status"], "error");
    assert_eq!(error["reason"], "invalid_value");
    let output = run(home.path(), &["status", "--json"])?;
    assert_eq!(output.status.code(), Some(1));
    let error = error_document(&output)?;
    assert_eq!(error["status"], "action_required");
    assert_eq!(error["reason"], "not_set_up");
    let home_text = home.path().to_string_lossy().into_owned();
    assert_eq!(
        error["next"][0]["argv"],
        serde_json::json!(["bridge", "up", "--home", home_text])
    );
    Ok(())
}

#[test]
fn unknown_options_are_refused_the_same_wherever_globals_sit() -> Check {
    let home = short_home()?;
    let home_text = home.path().to_string_lossy().into_owned();
    let forms: [&[&str]; 2] = [
        &[
            "--home",
            &home_text,
            "agents",
            "revoke",
            "agent:01M7XXXXXXXXXXXXXXXXXXXXXX",
            "--private",
        ],
        &[
            "agents",
            "revoke",
            "agent:01M7XXXXXXXXXXXXXXXXXXXXXX",
            "--private",
            "--home",
            &home_text,
        ],
    ];
    for form in forms {
        let output = bridge().args(form).output()?;
        assert_eq!(output.status.code(), Some(2), "{form:?}");
        assert!(text(&output.stderr).contains("--private"), "{form:?}");
    }
    let forms: [&[&str]; 2] = [
        &[
            "--home", &home_text, "status", "--force", "--name", "zz", "--via", "relay",
        ],
        &[
            "status", "--force", "--name", "zz", "--via", "relay", "--home", &home_text,
        ],
    ];
    for form in forms {
        let output = bridge().args(form).output()?;
        let error = text(&output.stderr)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        for option in ["--force", "--name", "--via"] {
            assert!(
                error.contains(&format!(
                    "{option} is ignored by bridge status and will be refused in the next major version."
                )),
                "{error}"
            );
        }
        assert_eq!(output.status.code(), Some(1), "{error}");
    }
    Ok(())
}

#[test]
fn nothing_after_the_separator_is_rewritten_and_old_spellings_warn() -> Check {
    let home = short_home()?;
    let output = bridge()
        .arg("--home")
        .arg(home.path())
        .args(["skills", "get", "--", "--force=false"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("Unknown skill --force=false."));
    let output = bridge()
        .args(["-home"])
        .arg(home.path())
        .args(["web", "devices", "--non-interactive"])
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("-home is deprecated. Use --home."));
    let output = run(home.path(), &["web", "enroll", "--public=false", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let enrolled: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(enrolled.get("warnings").is_none(), "{enrolled}");
    assert_eq!(output.stderr, [] as [u8; 0]);
    Ok(())
}

#[test]
fn reads_and_invalid_input_create_nothing() -> Check {
    let parent = short_home()?;
    let home = parent.path().join("fresh");
    for arguments in [
        &["agents", "list"][..],
        &["agents", "list", "--json"][..],
        &["web", "devices"][..],
        &["status"][..],
        &["doctor"][..],
        &["remote", "status", "--json"][..],
        &["agents", "create", "--name", "x", "--level", "root"][..],
        &["agents", "create", "--name", "x", "--scope", "made:up"][..],
        &["tokens", "create", "--scope", "made-up"][..],
        &["join", "not-a-code"][..],
        &["pair", "--scope", "made-up"][..],
        &["up", "--api-addr", "not-an-address"][..],
        &["up", "--api-addr", "8.8.8.8:80"][..],
        &[
            "scope",
            "agent:01M7XXXXXXXXXXXXXXXXXXXXXX",
            "--scope",
            "made:up",
        ][..],
    ] {
        let _ = run(&home, arguments)?;
        assert!(!home.exists(), "{arguments:?} created {}", home.display());
    }
    let output = run(
        &home,
        &[
            "agents", "create", "--name", "", "--level", "root", "--scope", "made:up",
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("3 problems"),
        "{}",
        text(&output.stderr)
    );
    Ok(())
}

#[test]
fn a_too_long_home_is_refused_before_anything_starts() -> Check {
    let parent = short_home()?;
    let home = parent.path().join("x".repeat(90));
    let output = run(&home, &["up"])?;
    assert_eq!(output.status.code(), Some(2));
    let error = text(&output.stderr);
    assert!(error.contains("Its socket path is"), "{error}");
    let limit = if cfg!(target_os = "macos") {
        "macOS allows 103."
    } else {
        "this system allows 107."
    };
    assert!(error.contains(limit), "{error}");
    assert!(!error.contains("SUN_LEN"), "{error}");
    assert!(!home.exists());
    Ok(())
}

#[test]
fn the_home_comes_from_the_environment_and_must_be_valid() -> Check {
    let parent = short_home()?;
    let file = parent.path().join("file");
    std::fs::write(&file, b"x")?;
    let output = bridge()
        .env("APPLE_BRIDGE_HOME", &file)
        .args(["status", "--json"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "invalid_home");
    let output = bridge()
        .env("APPLE_BRIDGE_HOME", "")
        .args(["status"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let home = parent.path().join("env-home");
    let output = bridge()
        .env("APPLE_BRIDGE_HOME", &home)
        .args(["agents", "list", "--json"])
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, [] as [u8; 0]);
    let error = error_document(&output)?;
    assert_eq!(error["reason"], "not_set_up");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("Apple Bridge isn't set up in ")),
        "{error}"
    );
    std::fs::create_dir(&home)?;
    let output = bridge()
        .env("APPLE_BRIDGE_HOME", &home)
        .args(["agents", "list", "--json"])
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let listed: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        listed,
        serde_json::json!({"agents": [], "next_cursor": null})
    );
    Ok(())
}

#[test]
fn a_malformed_runtime_file_is_reported_and_left_alone() -> Check {
    let home = short_home()?;
    let path = home.path().join("runtime.json");
    let original = b"{\"role\": \"gateway\",\n \"home\": oops}";
    std::fs::write(&path, original)?;
    for arguments in [&["status"][..], &["up"][..], &["pair"][..]] {
        let output = run(home.path(), arguments)?;
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        let error = text(&output.stderr);
        assert!(error.contains("runtime.json:2:10"), "{error}");
        assert!(error.contains("left as it is"), "{error}");
        assert_eq!(std::fs::read(&path)?, original);
    }
    let output = run(home.path(), &["status", "--json"])?;
    assert_eq!(error_document(&output)?["reason"], "invalid_config");
    Ok(())
}

#[test]
fn backup_never_replaces_a_file_without_force() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let destination = home.path().join("existing.db");
    std::fs::write(&destination, b"keep me")?;
    let output = run(
        home.path(),
        &["backup", destination.to_str().ok_or("path")?],
    )?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(text(&output.stderr).contains("--force"));
    assert_eq!(std::fs::read(&destination)?, b"keep me");
    let output = run(
        home.path(),
        &["backup", destination.to_str().ok_or("path")?, "--json"],
    )?;
    assert_eq!(error_document(&output)?["reason"], "destination_exists");
    let output = run(
        home.path(),
        &[
            "backup",
            destination.to_str().ok_or("path")?,
            "--force",
            "--json",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(receipt["outcome"], "replaced");
    assert!(std::fs::read(&destination)?.starts_with(b"SQLite format 3\0"));
    let fresh = home.path().join("fresh.db");
    let output = run(home.path(), &["backup", fresh.to_str().ok_or("path")?])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout).trim(), fresh.to_string_lossy());
    assert!(text(&output.stderr).contains("Backed up"));
    let leftovers = std::fs::read_dir(home.path())?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0);
    Ok(())
}

#[test]
fn restore_needs_consent_and_reports_what_it_kept() -> Check {
    let source = short_home()?;
    seeded_home(source.path())?;
    let backup = source.path().join("backup.db");
    let output = run(source.path(), &["backup", backup.to_str().ok_or("path")?])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let backup_text = backup.to_str().ok_or("path")?;

    let target = short_home()?;
    seeded_home(target.path())?;
    let before = std::fs::read(target.path().join("state.db"))?;
    let output = run(target.path(), &["restore", backup_text])?;
    assert_eq!(output.status.code(), Some(2));
    let error = text(&output.stderr);
    for shown in ["Home ", "Backup ", "Replaces ", "Ledger check", "--yes"] {
        assert!(error.contains(shown), "{shown}: {error}");
    }
    assert_eq!(std::fs::read(target.path().join("state.db"))?, before);
    let output = run(target.path(), &["restore", backup_text, "--json"])?;
    let document = error_document(&output)?;
    assert_eq!(document["reason"], "confirmation_required");
    assert_eq!(document["status"], "action_required");

    let output = run(target.path(), &["restore", backup_text, "--yes"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let receipt = text(&output.stderr);
    assert!(receipt.contains("Restored"), "{receipt}");
    assert!(receipt.contains("Kept"), "{receipt}");

    let parent = short_home()?;
    let fresh = parent.path().join("new");
    let output = run(&fresh, &["restore", backup_text, "--yes"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("status checks fail until you start it"));
    let status = run(&fresh, &["status"])?;
    assert_eq!(status.status.code(), Some(1));
    assert!(text(&status.stderr).contains("after restoring into a new home"));

    let missing = run(&fresh, &["restore", "/nonexistent/backup.db", "--yes"])?;
    assert_eq!(missing.status.code(), Some(1));
    assert!(text(&missing.stderr).contains("Couldn't find the backup"));
    assert!(!text(&missing.stderr).contains("SQLite"));
    let none = run(&fresh, &["restore"])?;
    assert_eq!(none.status.code(), Some(2));
    assert!(!text(&none.stderr).contains("invalid input"));
    Ok(())
}

#[test]
fn revoking_shows_the_access_asks_and_says_already() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["agents", "create", "--name", "bot", "--json"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let created: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(created["outcome"], "created");
    let id = created["agent"]["id"].as_str().ok_or("id")?.to_owned();

    let output = run(home.path(), &["agents", "revoke", &id, "--json"])?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "confirmation_required");

    let output = bridge()
        .env("CLAUDECODE", "1")
        .args(["agent", "revoke", &id, "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let receipt = text(&output.stderr);
    assert!(receipt.contains("deprecated"), "{receipt}");
    assert!(receipt.contains("Revoked "), "{receipt}");
    assert!(receipt.contains(&id), "{receipt}");
    assert!(receipt.contains("reminders:read"), "{receipt}");

    let output = run(home.path(), &["tokens", "revoke", &id, "--yes"])?;
    assert!(output.status.success());
    assert!(text(&output.stderr).contains("is already revoked."));
    let output = run(home.path(), &["web", "lock"])?;
    assert!(output.status.success());
    assert!(text(&output.stderr).contains("No browsers are signed in."));
    Ok(())
}

#[test]
fn one_selector_changes_agent_and_token_access() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["agents", "create", "--name", "bot", "--json"],
    )?;
    let created: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let id = created["agent"]["id"].as_str().ok_or("id")?.to_owned();
    let output = run(
        home.path(),
        &["scope", &id, "--scope", "contacts:read", "--json"],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let changed: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(changed["outcome"], "updated");
    let output = run(
        home.path(),
        &[
            "tokens",
            "update",
            &id,
            "--scope",
            "contacts:read",
            "--json",
        ],
    )?;
    let unchanged: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(unchanged["outcome"], "unchanged");
    Ok(())
}

#[test]
fn piped_reads_keep_json_for_one_release_with_a_notice() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &[
            "agents",
            "create",
            "--name",
            "bot",
            "--scope",
            "reminders:read",
        ],
    )?;
    assert!(output.status.success());
    let legacy: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(legacy.get("outcome").is_none());
    assert!(
        legacy["token"]
            .as_str()
            .is_some_and(|token| token.starts_with("aba-"))
    );
    assert!(text(&output.stderr).contains("Pass --json to keep JSON."));
    let output = bridge()
        .env("APPLE_BRIDGE_LEGACY_JSON", "0")
        .args(["agents", "list", "--home"])
        .arg(home.path())
        .output()?;
    assert!(
        text(&output.stdout).starts_with("ID\tName\tLevel"),
        "{}",
        text(&output.stdout)
    );
    assert_eq!(output.stderr, [] as [u8; 0]);
    let output = bridge()
        .env("APPLE_BRIDGE_LEGACY_JSON", "0")
        .args(["agents", "create", "--name", "two", "--home"])
        .arg(home.path())
        .output()?;
    let token = text(&output.stdout);
    assert_eq!(token.lines().count(), 1, "{token}");
    assert!(token.starts_with("aba-"));
    assert!(text(&output.stderr).contains("Created"));
    Ok(())
}

#[test]
fn github_actions_masks_delivered_secrets() -> Check {
    let home = short_home()?;
    let output = bridge()
        .env("GITHUB_ACTIONS", "true")
        .args(["tokens", "create", "--home"])
        .arg(home.path())
        .output()?;
    assert!(output.status.success());
    let token = text(&output.stdout).trim().to_owned();
    assert!(text(&output.stderr).contains(&format!("::add-mask::{token}")));
    Ok(())
}

#[test]
fn permissions_never_prompt_without_a_terminal_and_can_be_named() -> Check {
    let directory = short_home()?;
    let fixture = directory.path().join("fixture.json");
    std::fs::write(
        &fixture,
        br#"{"statuses":{"contacts":"unknown","calendar":"unknown"},"grant_when_asked":["contacts","calendar"]}"#,
    )?;
    let fixture_text = fixture.to_str().ok_or("path")?;
    let home = directory.path().join("home");
    for environment in [None, Some(("CI", "1")), Some(("CLAUDECODE", "1"))] {
        let mut command = bridge();
        if let Some((key, value)) = environment {
            command.env(key, value);
        }
        let output = command
            .args(["permissions", "guide", "--fixture", fixture_text, "--home"])
            .arg(&home)
            .output()?;
        assert_eq!(output.status.code(), Some(2), "{environment:?}");
        assert!(
            text(&output.stderr).contains("Can't walk through permissions without a terminal.")
        );
    }
    let untouched: serde_json::Value = serde_json::from_slice(&std::fs::read(&fixture)?)?;
    assert!(untouched.get("asked").is_none());
    let output = run(
        &home,
        &[
            "permissions",
            "guide",
            "--only",
            "calendar",
            "--fixture",
            fixture_text,
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&fixture)?)?;
    assert_eq!(after["asked"], serde_json::json!(["calendar"]));
    let output = run(
        &home,
        &[
            "permissions",
            "guide",
            "--only",
            "calendar,nope",
            "--fixture",
            fixture_text,
        ],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("nope"));
    Ok(())
}

#[test]
fn end_of_input_at_the_first_prompt_requests_nothing() -> Check {
    let directory = short_home()?;
    let fixture = directory.path().join("fixture.json");
    std::fs::write(
        &fixture,
        br#"{"statuses":{"contacts":"unknown"},"grant_when_asked":["contacts"]}"#,
    )?;
    let home = directory.path().join("home");
    let output = pty(
        &[
            "permissions",
            "guide",
            "--fixture",
            fixture.to_str().ok_or("path")?,
        ],
        &home,
        b"\x04",
    )?;
    let shown = text(&output.stdout);
    assert_eq!(output.status.code(), Some(130), "{shown}");
    assert!(shown.contains("Press Enter to ask now"), "{shown}");
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&fixture)?)?;
    assert!(after.get("asked").is_none(), "{after}");
    Ok(())
}

#[test]
fn a_closed_stdout_ends_a_read_quietly() -> Check {
    let mut child = bridge()
        .args(["help", "--all"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    drop(child.stdout.take());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .ok_or("stderr")?
        .read_to_string(&mut stderr)?;
    let status = child.wait()?;
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    let mut child = bridge()
        .args(["call", "--list"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    drop(child.stdout.take());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .ok_or("stderr")?
        .read_to_string(&mut stderr)?;
    assert_eq!(child.wait()?.code(), Some(0), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
    Ok(())
}

#[test]
fn signals_stop_waiting_with_their_codes() -> Check {
    let home = short_home()?;
    for (signal, code) in [("-INT", 130), ("-TERM", 143)] {
        let child = bridge()
            .args(["stats", "--follow", "--home"])
            .arg(home.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        std::thread::sleep(std::time::Duration::from_millis(1500));
        Command::new("/bin/kill")
            .args([signal, &child.id().to_string()])
            .status()?;
        let output = child.wait_with_output()?;
        assert_eq!(output.status.code(), Some(code), "{signal}");
        assert_eq!(output.stdout, [] as [u8; 0]);
    }
    let output = run(home.path(), &["stats", "--follow", "--timeout", "1s"])?;
    assert!(output.status.success());
    assert!(text(&output.stderr).contains("Stopped following after"));
    let output = run(home.path(), &["stats", "--watch", "--timeout", "1s"])?;
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("Stopped watching"));
    Ok(())
}

#[test]
fn cursors_and_limits_are_checked_before_reading() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["read", "messages", "--next", "garbage", "--json"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "invalid_value");
    let output = run(home.path(), &["read", "messages", "--next", "x", "--all"])?;
    assert_eq!(output.status.code(), Some(2));
    let output = run(home.path(), &["read", "notes", "--limit", "500", "--json"])?;
    let document = error_document(&output)?;
    assert_eq!(document["warnings"][0]["reason"], "limit_clamped");
    let output = run(home.path(), &["read", "notes", "--query", "x"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("--query works only with bridge read messages."));
    Ok(())
}

#[test]
fn call_takes_arguments_from_data_a_file_or_stdin() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &["call", "sources.status", "--data", "{", "--json"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(error_document(&output)?["reason"], "invalid_value");
    let output = run(
        home.path(),
        &["call", "sources.status", "--data", "{}", "--file", "x"],
    )?;
    assert_eq!(output.status.code(), Some(2));
    let output = run(home.path(), &["call", "sources.status", "--args", "{}"])?;
    assert!(text(&output.stderr).contains("--args is deprecated. Use --data."));
    let mut child = bridge()
        .args(["call", "sources.status", "--data", "-", "--home"])
        .arg(home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    drop(child.stdin.take());
    let output = child.wait_with_output()?;
    assert!(
        text(&output.stderr).contains("isn't valid JSON"),
        "{}",
        text(&output.stderr)
    );
    Ok(())
}

#[test]
fn piped_up_prints_only_the_address_and_down_stays_quiet_on_stdout() -> Check {
    let home = short_home()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let api = listener.local_addr()?.to_string();
    drop(listener);
    let started = run(
        home.path(),
        &["up", "--api-addr", &api, "--peer-addr", "127.0.0.1:0"],
    )?;
    let stop = || run(home.path(), &["down"]);
    if !started.status.success() {
        drop(stop());
        return Err(text(&started.stderr).into());
    }
    let outcome = (|| -> Check {
        assert_eq!(text(&started.stdout), format!("http://{api}/mcp\n"));
        let summary = text(&started.stderr);
        assert!(summary.contains("is on"), "{summary}");
        assert!(
            summary.contains("Warning: No Helper app was found"),
            "{summary}"
        );
        let wrong = run(home.path(), &["up", "--api-addr", "not-an-address"])?;
        assert_eq!(wrong.status.code(), Some(2));
        let json = run(home.path(), &["up", "--json"])?;
        let answer: serde_json::Value = serde_json::from_slice(&json.stdout)?;
        for key in [
            "state",
            "started",
            "mcp",
            "home",
            "helper",
            "remote",
            "seconds",
            "restart_needed",
            "outcome",
            "next",
        ] {
            assert!(answer.get(key).is_some(), "{key}");
        }
        Ok(())
    })();
    let stopped = stop()?;
    outcome?;
    assert!(stopped.status.success());
    assert_eq!(stopped.stdout, [] as [u8; 0]);
    let stopped_text = text(&stopped.stderr);
    assert!(
        stopped_text
            .lines()
            .any(|line| line == "Apple Bridge is stopped."),
        "{stopped_text}"
    );
    Ok(())
}

#[test]
fn yes_never_stands_in_for_force_on_backup() -> Check {
    let home = short_home()?;
    seeded_home(home.path())?;
    let destination = home.path().join("existing.db");
    std::fs::write(&destination, b"keep me")?;
    let path = destination.to_str().ok_or("path")?;
    let output = run(home.path(), &["backup", path, "--yes", "--json"])?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(error_document(&output)?["reason"], "destination_exists");
    assert_eq!(std::fs::read(&destination)?, b"keep me");
    let output = run(home.path(), &["backup", path, "--yes"])?;
    assert!(text(&output.stderr).contains("only --force does"));
    assert_eq!(std::fs::read(&destination)?, b"keep me");
    Ok(())
}

#[test]
fn only_the_old_revoke_spellings_skip_consent_for_one_release() -> Check {
    let home = short_home()?;
    let mut ids = Vec::new();
    for name in ["one", "two"] {
        let output = run(home.path(), &["agents", "create", "--name", name, "--json"])?;
        assert!(output.status.success(), "{}", text(&output.stderr));
        let created: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        ids.push(created["agent"]["id"].as_str().ok_or("id")?.to_owned());
    }
    for words in [["tokens", "revoke"], ["agents", "revoke"]] {
        let output = run(home.path(), &[words[0], words[1], &ids[0]])?;
        assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
        assert!(text(&output.stderr).contains("--yes"));
    }
    let output = run(
        home.path(),
        &["tokens", "revoke", &ids[0], "--yes", "--json"],
    )?;
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(receipt["outcome"], "revoked", "{receipt}");
    let output = run(home.path(), &["revoke", &ids[1]])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("deprecated"));
    let third = run(
        home.path(),
        &["agents", "create", "--name", "three", "--json"],
    )?;
    let created: serde_json::Value = serde_json::from_slice(&third.stdout)?;
    let id = created["agent"]["id"].as_str().ok_or("id")?;
    let output = run(home.path(), &["agent", "revoke", id])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("deprecated"));
    Ok(())
}

#[test]
fn flags_old_commands_refused_are_still_refused() -> Check {
    let home = short_home()?;
    let output = run(
        home.path(),
        &[
            "remote",
            "on",
            "--snippets",
            "--timeout",
            "1ms",
            "--no-open",
        ],
    )?;
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
    assert!(!home.path().join("remote").exists());
    Ok(())
}

#[test]
fn malformed_addresses_and_long_names_create_nothing() -> Check {
    for words in [
        &["start", "--peer-addr", "::::1", "--api-addr", "127.0.0.1:0"][..],
        &[
            "start",
            "--peer-addr",
            "no such host.invalid:1",
            "--api-addr",
            "127.0.0.1:0",
        ][..],
        &[
            "start",
            "--peer-addr",
            "nowhere.invalid:7781",
            "--api-addr",
            "127.0.0.1:0",
        ][..],
        &["pair", "--peer-addr", "::::1"][..],
    ] {
        let home = short_home()?;
        let output = run(home.path(), words)?;
        assert_eq!(
            output.status.code(),
            Some(2),
            "{words:?}: {}",
            text(&output.stderr)
        );
        assert_eq!(std::fs::read_dir(home.path())?.count(), 0, "{words:?}");
    }
    let home = short_home()?;
    let long = "é".repeat(100);
    let output = run(
        home.path(),
        &["agents", "create", "--name", &long, "--json"],
    )?;
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
    assert_eq!(error_document(&output)?["reason"], "invalid_value");
    assert_eq!(std::fs::read_dir(home.path())?.count(), 0);
    Ok(())
}

fn fake_gateway(
    home: &Path,
) -> Result<
    (
        std::fs::File,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ),
    Box<dyn std::error::Error>,
> {
    use std::io::Write;
    use std::sync::atomic::Ordering;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let posts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = posts.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = stream.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&request).into_owned();
            let respond = |stream: &mut std::net::TcpStream, status: &str, body: &str| {
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            };
            if head.starts_with("GET /v1/health") {
                respond(&mut stream, "200 OK", "{}");
            } else if head.starts_with("POST /v1/op/photos.fetch") {
                counter.fetch_add(1, Ordering::SeqCst);
            } else {
                let authorization = head
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                    .map(|line| line[14..].trim().to_owned())
                    .unwrap_or_default();
                let body = serde_json::json!({"error": {"code": "unauthorized", "message": format!("rejected {authorization}")}}).to_string();
                respond(&mut stream, "401 Unauthorized", &body);
            }
        }
    });
    std::fs::write(
        home.join("runtime.json"),
        serde_json::json!({"role": "gateway", "home": home, "peer_address": "127.0.0.1:1", "api_address": address, "socket": ""}).to_string(),
    )?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join("lock"))?;
    lock.try_lock()?;
    Ok((lock, posts))
}

#[test]
fn transfers_are_sent_once_and_reflected_tokens_are_hidden() -> Check {
    let home = short_home()?;
    let (_lock, posts) = fake_gateway(home.path())?;
    let output = bridge()
        .env("REVIEW_TOKEN", "aba-REVIEW-SECRET")
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
    assert_eq!(posts.load(std::sync::atomic::Ordering::SeqCst), 1);
    let output = bridge()
        .env("REVIEW_TOKEN", "aba-REVIEW-SECRET")
        .args([
            "call",
            "sources.status",
            "--token-env",
            "REVIEW_TOKEN",
            "--json",
            "--home",
        ])
        .arg(home.path())
        .output()?;
    let stderr = text(&output.stderr);
    assert!(!stderr.contains("REVIEW-SECRET"), "{stderr}");
    assert!(stderr.contains("[redacted]"), "{stderr}");
    Ok(())
}
