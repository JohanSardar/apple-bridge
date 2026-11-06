use std::{
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

type Check = Result<(), Box<dyn std::error::Error>>;

fn bridge() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bridge"));
    command
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .stdin(Stdio::null());
    command
}

fn run(home: &Path, arguments: &[&str]) -> Result<Output, std::io::Error> {
    bridge().args(arguments).arg("--home").arg(home).output()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn free_port() -> Result<String, std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.to_string())
}

struct Gateway<'a>(&'a Path);

impl Drop for Gateway<'_> {
    fn drop(&mut self) {
        let _ = run(self.0, &["down"]);
    }
}

fn up<'a>(
    home: &'a Path,
    api: &str,
) -> Result<(Gateway<'a>, serde_json::Value), Box<dyn std::error::Error>> {
    let output = run(
        home,
        &[
            "up",
            "--api-addr",
            api,
            "--peer-addr",
            "127.0.0.1:0",
            "--json",
        ],
    )?;
    let guard = Gateway(home);
    assert!(output.status.success(), "{}", text(&output.stderr));
    Ok((guard, serde_json::from_slice(&output.stdout)?))
}

fn fake_tool(directory: &Path, name: &str) -> Result<PathBuf, std::io::Error> {
    let log = directory.join(format!("{name}.log"));
    let path = directory.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$FAKE_EXISTS\" = 1 ] && [ \"$2\" = add ]; then echo 'MCP server apple-bridge already exists in user config' >&2; exit 1; fi\nif [ \"$2\" = get ] && {{ [ \"$FAKE_ADD_WAITS\" != 1 ] || grep -q '^mcp add ' '{}'; }}; then echo \"URL: $FAKE_URL\"; echo \"$FAKE_STATUS\"; fi\nif [ \"$2\" = add ] && [ \"$FAKE_ADD_WAITS\" = 1 ]; then sleep 60; fi\nexit 0\n",
            log.display(),
            log.display()
        ),
    )?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(log)
}

fn seed_sources(home: &Path) -> Check {
    use ab_trust::domain::{Coverage, Source};
    let coverage = || Coverage {
        pack_version: String::new(),
        operations: Vec::new(),
        fields: "complete".into(),
        missing: None,
    };
    let mut state = ab_state::State::<ab_state::Gateway>::open(home)?;
    state.source_state(Source::Contacts, "ready", coverage(), None, "")?;
    state.source_state(Source::Messages, "permission_missing", coverage(), None, "")?;
    Ok(())
}

#[test]
fn help_and_version_work_and_say_how_to_start() -> Check {
    for flag in ["--help", "-h"] {
        let output = bridge().arg(flag).output()?;
        assert!(output.status.success(), "{flag}");
        let help = text(&output.stdout);
        for word in ["bridge up", "bridge link claude", "remote", "--home"] {
            assert!(help.contains(word), "{flag} lacks {word}: {help}");
        }
        assert!(!text(&output.stderr).contains("invalid replica configuration"));
    }
    for flag in ["--version", "-V"] {
        let output = bridge().arg(flag).output()?;
        assert!(output.status.success());
        assert_eq!(
            text(&output.stdout).trim(),
            format!("bridge {}", env!("CARGO_PKG_VERSION"))
        );
    }
    let output = bridge().args(["remote", "--help"]).output()?;
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("--relay-url"));
    Ok(())
}

#[test]
fn every_failure_names_what_went_wrong() -> Check {
    let home = tempfile::tempdir()?;
    let cases: &[(&[&str], &str)] = &[
        (&["nonsense"], "unrecognized subcommand"),
        (&["status"], "has not been set up"),
        (&["doctor"], "has not been set up"),
        (&["stats"], "has not been set up"),
        (&["link", "claude"], "is not running"),
        (&["read", "messages"], "is not running"),
        (&["connector", "pending"], "is not running"),
        (&["permissions", "request", "fake"], "not a permission"),
        (&["agent", "revoke", "nope"], "not an agent id"),
        (
            &["agent", "create", "--name", "x", "--level", "root"],
            "not an agent level",
        ),
        (
            &["token", "create", "--scope", "made-up"],
            "not a known scope",
        ),
        (&["join", "not-a-code"], "not a valid pairing code"),
        (&["remote", "on", "--snippets"], "--snippets only works"),
        (&["skills", "get", "nope"], "no skill called"),
        (&["call", "nope.nothing"], "no operation called"),
        (&["call", "messages.send"], "only reads"),
        (&["start", "--api-addr", "8.8.8.8:80"], "loopback"),
    ];
    for (arguments, expected) in cases {
        let output = run(home.path(), arguments)?;
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        let error = text(&output.stderr);
        assert!(error.contains(expected), "{arguments:?}: {error}");
        assert!(
            !error.contains("invalid replica configuration"),
            "{arguments:?}: {error}"
        );
    }
    Ok(())
}

#[test]
fn up_starts_with_no_input_and_down_stops_it() -> Check {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("fresh home");
    let api = free_port()?;
    let (guard, answer) = up(&home, &api)?;
    assert_eq!(answer["state"], "on");
    assert_eq!(answer["started"], true);
    assert_eq!(answer["mcp"], format!("http://{api}/mcp"));
    assert_eq!(answer["remote"], "off");

    let again = run(&home, &["up"])?;
    assert!(again.status.success(), "{}", text(&again.stderr));
    let summary = text(&again.stdout);
    assert!(summary.contains("was already on"), "{summary}");
    assert!(summary.contains(&format!("http://{api}/mcp")), "{summary}");
    assert!(summary.contains("bridge link claude"), "{summary}");
    assert!(!summary.contains("Warning:"), "{summary}");
    let marker = home.join("run/gateway-build.json");
    let original = std::fs::read(&marker)?;
    let mut stale: serde_json::Value = serde_json::from_slice(&original)?;
    stale["inode"] = serde_json::json!(0);
    stale["version"] = serde_json::json!("older-fixture");
    std::fs::write(&marker, serde_json::to_vec(&stale)?)?;
    let updated = run(&home, &["up"])?;
    let warning = text(&updated.stdout);
    assert!(warning.contains("older-fixture"), "{warning}");
    assert!(warning.contains("bridge down, then bridge up"), "{warning}");
    let updated = run(&home, &["up", "--json"])?;
    let updated: serde_json::Value = serde_json::from_slice(&updated.stdout)?;
    assert_eq!(updated["restart_needed"], true);
    std::fs::write(marker, original)?;

    let doctor = run(&home, &["doctor"])?;
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout)?;
    assert_eq!(report["private_modes"], true, "{report}");

    let down = run(&home, &["down"])?;
    assert!(down.status.success(), "{}", text(&down.stderr));
    assert!(TcpListener::bind(&api).is_ok());
    drop(guard);
    let stopped = run(&home, &["down"])?;
    assert!(stopped.status.success());
    assert!(text(&stopped.stdout).contains("not running"));
    Ok(())
}

#[test]
fn uninstalling_a_running_executable_stops_its_gateway() -> Check {
    let directory = tempfile::tempdir_in("/tmp")?;
    let binary = directory.path().join("bridge");
    std::fs::copy(env!("CARGO_BIN_EXE_bridge"), &binary)?;
    let home = directory.path().join("home");
    let api = free_port()?;
    let _guard = Gateway(&home);
    let output = Command::new(&binary)
        .args([
            "up",
            "--json",
            "--api-addr",
            &api,
            "--peer-addr",
            "127.0.0.1:0",
            "--home",
        ])
        .arg(&home)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    std::fs::remove_file(binary)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(80);
    while TcpListener::bind(&api).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "removed gateway still running"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(std::fs::read_to_string(home.join("gateway.log"))?.contains("was removed; stopping"));
    Ok(())
}

#[test]
fn up_reports_a_busy_port_and_leaves_it_alone() -> Check {
    let home = tempfile::tempdir()?;
    let busy = TcpListener::bind("127.0.0.1:0")?;
    let address = busy.local_addr()?.to_string();
    let output = run(home.path(), &["up", "--api-addr", &address])?;
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("already in use"));
    Ok(())
}

#[test]
fn down_refuses_a_gateway_it_did_not_start() -> Check {
    let home = tempfile::tempdir()?;
    let api = free_port()?;
    let mut foreground = bridge()
        .args([
            "start",
            "--role",
            "gateway",
            "--peer-addr",
            "127.0.0.1:0",
            "--api-addr",
            &api,
            "--home",
        ])
        .arg(home.path())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !home.path().join("run/control.sock").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let output = run(home.path(), &["down"])?;
    let error = text(&output.stderr);
    foreground.kill()?;
    foreground.wait()?;
    assert_eq!(output.status.code(), Some(1), "{error}");
    assert!(error.contains("not started by `bridge up`"), "{error}");
    Ok(())
}

#[test]
fn link_registers_each_agent_without_a_terminal_and_never_approves() -> Check {
    let tools = tempfile::tempdir()?;
    let claude_log = fake_tool(tools.path(), "claude")?;
    let codex_log = fake_tool(tools.path(), "codex")?;
    let home = tempfile::tempdir()?;
    let api = free_port()?;
    let (_guard, _) = up(home.path(), &api)?;
    let url = format!("http://{api}/mcp");
    let path = format!(
        "{}:{}",
        tools.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let link = |arguments: &[&str]| {
        bridge()
            .env("PATH", &path)
            .env("FAKE_URL", &url)
            .args(arguments)
            .arg("--home")
            .arg(home.path())
            .output()
    };
    let claude = link(&["link", "claude"])?;
    assert!(claude.status.success(), "{}", text(&claude.stderr));
    let calls = std::fs::read_to_string(&claude_log)?;
    assert_eq!(
        calls.lines().next(),
        Some(format!("mcp add --transport http --scope user apple-bridge {url}").as_str())
    );
    assert!(!calls.contains("mcp login"));
    assert!(text(&claude.stdout).contains("claude mcp login apple-bridge"));

    let project = link(&["link", "claude-code", "--scope", "project", "--name", "ab"])?;
    assert!(project.status.success());
    assert!(std::fs::read_to_string(&claude_log)?.contains(&format!("--scope project ab {url}")));

    let codex = bridge()
        .env("PATH", &path)
        .env("FAKE_URL", &url)
        .env("FAKE_ADD_WAITS", "1")
        .args(["link", "codex", "--home"])
        .arg(home.path())
        .output()?;
    assert!(codex.status.success(), "{}", text(&codex.stderr));
    assert!(text(&codex.stdout).contains("codex mcp login apple-bridge"));
    let expected_add = format!("mcp add apple-bridge --url {url}");
    assert!(
        std::fs::read_to_string(&codex_log)?
            .lines()
            .any(|line| line == expected_add)
    );

    let print = link(&["link", "codex", "--print"])?;
    assert!(text(&print.stdout).contains(&format!("codex mcp add apple-bridge --url {url}")));

    let other = link(&["link", "other"])?;
    assert!(text(&other.stdout).contains(&url));

    let unlink = bridge()
        .env("PATH", &path)
        .args(["unlink", "claude"])
        .output()?;
    assert!(unlink.status.success(), "{}", text(&unlink.stderr));
    let calls = std::fs::read_to_string(&claude_log)?;
    assert!(calls.contains("mcp logout apple-bridge"));
    assert!(calls.contains("mcp remove apple-bridge --scope user"));
    Ok(())
}

#[test]
fn link_accepts_the_same_server_and_refuses_a_different_one() -> Check {
    let tools = tempfile::tempdir()?;
    fake_tool(tools.path(), "claude")?;
    let home = tempfile::tempdir()?;
    let api = free_port()?;
    let (_guard, _) = up(home.path(), &api)?;
    let path = format!(
        "{}:{}",
        tools.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let link = |existing_url: String| {
        bridge()
            .env("PATH", &path)
            .env("FAKE_EXISTS", "1")
            .env("FAKE_URL", existing_url)
            .args(["link", "claude", "--no-login", "--home"])
            .arg(home.path())
            .output()
    };
    let same = link(format!("http://{api}/mcp"))?;
    assert!(same.status.success(), "{}", text(&same.stderr));
    assert!(text(&same.stdout).contains("already has"));
    let different = link("http://elsewhere.example/mcp".to_owned())?;
    assert_eq!(different.status.code(), Some(1));
    assert!(text(&different.stderr).contains("different server"));
    Ok(())
}

#[test]
fn link_waits_for_claude_code_to_approve_a_project_server() -> Check {
    let tools = tempfile::tempdir()?;
    let log = fake_tool(tools.path(), "claude")?;
    let home = tempfile::tempdir()?;
    let api = free_port()?;
    let (_guard, _) = up(home.path(), &api)?;
    let path = format!(
        "{}:{}",
        tools.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = bridge()
        .env("PATH", &path)
        .env("FAKE_STATUS", "Status: ⏸ Pending approval")
        .args([
            "link",
            "claude",
            "--scope",
            "project",
            "--no-browser",
            "--home",
        ])
        .arg(home.path())
        .output()?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("approve project servers first"));
    assert!(!std::fs::read_to_string(&log)?.contains("mcp login"));
    Ok(())
}

#[test]
fn link_explains_a_missing_agent() -> Check {
    let home = tempfile::tempdir()?;
    let api = free_port()?;
    let (_guard, _) = up(home.path(), &api)?;
    let empty = tempfile::tempdir()?;
    let output = bridge()
        .env("PATH", empty.path())
        .args(["link", "claude", "--home"])
        .arg(home.path())
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let error = text(&output.stderr);
    assert!(error.contains("Claude Code is not installed"), "{error}");
    assert!(error.contains("next:"), "{error}");
    Ok(())
}

#[test]
fn stats_shows_counts_and_state_per_source() -> Check {
    let home = tempfile::tempdir()?;
    seed_sources(home.path())?;
    let output = run(home.path(), &["stats"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let shown = text(&output.stdout);
    assert!(shown.contains("Contacts"), "{shown}");
    assert!(shown.contains("needs a Mac permission"), "{shown}");
    let output = run(home.path(), &["stats", "--json"])?;
    let answer: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let sources = answer["sources"].as_array().ok_or("sources")?;
    let messages = sources
        .iter()
        .find(|source| source["source"] == "messages")
        .ok_or("messages")?;
    assert_eq!(messages["state"], "permission_missing");
    assert_eq!(messages["total"], 0);
    Ok(())
}

#[test]
fn stats_watch_reports_each_source_then_finishes() -> Check {
    let home = tempfile::tempdir()?;
    seed_sources(home.path())?;
    let output = run(
        home.path(),
        &[
            "stats",
            "--watch",
            "--settle",
            "1",
            "--timeout",
            "20",
            "--json",
        ],
    )?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let events: Vec<serde_json::Value> = text(&output.stdout)
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let kind = |event: &str, source: &str| {
        events
            .iter()
            .any(|line| line["event"] == event && line["source"] == source)
    };
    assert!(kind("syncing", "contacts"), "{events:?}");
    assert!(kind("finished", "contacts"), "{events:?}");
    assert!(kind("attention", "messages"), "{events:?}");
    assert_eq!(
        events.last().ok_or("events")?["event"],
        "all_finished",
        "{events:?}"
    );
    Ok(())
}

#[test]
fn stats_watch_stops_at_its_time_limit_on_an_empty_home() -> Check {
    let home = tempfile::tempdir()?;
    let output = run(home.path(), &["stats", "--watch", "--timeout", "1"])?;
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("Still syncing"));
    Ok(())
}

#[test]
fn read_commands_use_the_gateway_and_leave_no_credential_behind() -> Check {
    let home = tempfile::tempdir()?;
    seed_sources(home.path())?;
    let api = free_port()?;
    let (_guard, _) = up(home.path(), &api)?;
    let principals = |home: &Path| -> Result<usize, Box<dyn std::error::Error>> {
        let raw = std::fs::read(home.join("credentials.json"))?;
        let value: serde_json::Value = serde_json::from_slice(&raw)?;
        Ok(value["principals"].as_array().map_or(0, Vec::len))
    };
    let before = principals(home.path())?;
    for arguments in [
        &["read", "messages", "--json"][..],
        &["read", "messages", "--text", "hello", "--json"][..],
        &["read", "chats", "--json"][..],
        &["read", "contacts", "--json"][..],
        &["read", "calendar", "--json"][..],
        &["read", "reminders", "--json"][..],
        &["read", "notes", "--json"][..],
        &["read", "mail", "--json"][..],
        &["search", "hello", "--json"][..],
        &["call", "sources.status"][..],
    ] {
        let output = run(home.path(), arguments)?;
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            text(&output.stderr)
        );
        let _: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    }
    let human = run(home.path(), &["read", "contacts"])?;
    assert!(text(&human.stdout).contains("Nothing to show"));
    let listed = run(home.path(), &["call", "--list"])?;
    let listed = text(&listed.stdout);
    assert!(listed.contains("sources.status"));
    assert!(!listed.contains("messages.send"));
    assert_eq!(principals(home.path())?, before);
    Ok(())
}

#[test]
fn skills_are_in_the_binary_and_name_only_real_operations() -> Check {
    let list = bridge().args(["skills", "list"]).output()?;
    assert!(text(&list.stdout).contains("core"));
    let operations: Vec<String> = ab_catalog::operations()?
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect();
    for skill in ["core", "setup"] {
        let output = bridge().args(["skills", "get", skill]).output()?;
        assert!(output.status.success());
        let body = text(&output.stdout);
        assert!(body.len() > 200);
        for word in body.split('`') {
            let looks_like_operation = word.contains('.')
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
                && !word.starts_with("messages.message")
                && !word.starts_with("calendar.event")
                && !word.starts_with("contacts.contact");
            assert!(
                !looks_like_operation || operations.iter().any(|name| name == word),
                "{skill} names {word}"
            );
        }
    }
    let core = text(&bridge().args(["skills", "get", "core"]).output()?.stdout);
    for name in [
        "sources.status",
        "records.query",
        "messages.chats",
        "search",
    ] {
        assert!(core.contains(name), "{name}");
    }
    Ok(())
}

#[test]
fn permission_guide_uses_only_the_fixture_and_walks_the_steps() -> Check {
    let directory = tempfile::tempdir()?;
    let fixture = directory.path().join("fixture.json");
    std::fs::write(
        &fixture,
        serde_json::to_vec(&serde_json::json!({
            "statuses": {"full_disk_access":"denied","contacts":"not_determined","calendar":"granted","reminders":"not_determined"},
            "grant_when_opened": ["full_disk_access"],
            "grant_when_asked": ["contacts", "reminders"],
        }))?,
    )?;
    let fixture_arg = fixture.to_string_lossy().into_owned();
    let home = directory.path().join("home");
    let status = run(
        &home,
        &["permissions", "status", "--json", "--fixture", &fixture_arg],
    )?;
    let statuses: serde_json::Value = serde_json::from_slice(&status.stdout)?;
    assert_eq!(statuses["full_disk_access"], "denied");
    assert_eq!(statuses["calendar"], "granted");

    let no_terminal = run(&home, &["permissions", "guide", "--fixture", &fixture_arg])?;
    assert!(no_terminal.status.success());
    assert!(text(&no_terminal.stdout).contains("Nothing was changed or requested"));
    let untouched: serde_json::Value = serde_json::from_slice(&std::fs::read(&fixture)?)?;
    assert!(untouched.get("asked").is_none(), "{untouched}");
    assert!(untouched.get("opened").is_none(), "{untouched}");

    let walked = run(
        &home,
        &["permissions", "guide", "--yes", "--fixture", &fixture_arg],
    )?;
    assert!(walked.status.success(), "{}", text(&walked.stderr));
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&fixture)?)?;
    assert_eq!(after["asked"], serde_json::json!(["contacts", "reminders"]));
    assert_eq!(after["opened"], serde_json::json!(["full_disk_access"]));
    for permission in ["full_disk_access", "contacts", "calendar", "reminders"] {
        assert_eq!(after["statuses"][permission], "granted", "{permission}");
    }
    assert!(text(&walked.stdout).contains("Full Disk Access allowed"));
    assert!(text(&walked.stdout).contains("Waiting up to 10 minutes"));
    assert!(text(&walked.stdout).contains("Mission Control"));
    assert!(text(&walked.stdout).contains("existing grants are kept"));
    Ok(())
}

#[test]
fn permission_status_reads_source_state_and_prompts_for_nothing() -> Check {
    let home = tempfile::tempdir()?;
    seed_sources(home.path())?;
    let output = run(home.path(), &["permissions", "status", "--json"])?;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let statuses: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(statuses["full_disk_access"], "denied");
    assert_eq!(statuses["contacts"], "granted");
    assert_eq!(statuses["calendar"], "unknown");
    Ok(())
}

#[allow(clippy::too_many_lines)]
#[test]
fn every_form_that_scripts_and_the_helper_use_still_parses() -> Check {
    let forms: &[&[&str]] = &[
        &[
            "start",
            "--role",
            "gateway",
            "--home",
            "/h",
            "--api-addr",
            "127.0.0.1:7780",
            "--peer-addr",
            "127.0.0.1:7443",
        ],
        &[
            "start",
            "--role",
            "replica",
            "--home",
            "/h",
            "--api-addr",
            "127.0.0.1:0",
            "--listen",
            "127.0.0.1:0",
        ],
        &[
            "start",
            "--role",
            "gateway",
            "--home",
            "/h",
            "--helper-app",
            "/a.app",
            "--helper-lab-commands",
            "--socket",
            "/s",
            "--blob-cap",
            "1",
            "--blob-grace-ms",
            "1",
            "--retention-rows",
            "1",
            "--retention-age",
            "1h",
        ],
        &["token", "create", "--scope", "admin", "--home", "/h"],
        &[
            "token",
            "create",
            "--scope",
            "reminders:read",
            "--remote",
            "--home",
            "/h",
        ],
        &[
            "agent",
            "create",
            "--name",
            "n",
            "--level",
            "read",
            "--scope",
            "reminders:read",
            "--home",
            "/h",
        ],
        &["agent", "list", "--home", "/h"],
        &["agent", "revoke", "agent:1", "--home", "/h"],
        &[
            "pair",
            "--home",
            "/h",
            "--scope",
            "a:read",
            "--expires-in",
            "1m",
            "--peer-addr",
            "x:1",
            "--dial-replica",
            "y:2",
        ],
        &[
            "pair",
            "--peer-addr",
            "x:1",
            "--name",
            "replica",
            "--scope",
            "a:read",
            "--home",
            "/h",
        ],
        &["pair", "--iphone", "--via-remote", "--home", "/h"],
        &["pair", "--via-remote", "--home", "/h"],
        &["join", "CODE", "--listen", "127.0.0.1:0", "--home", "/h"],
        &["connect", "CODE", "--listen", "127.0.0.1:0", "--home", "/h"],
        &["connector", "pending", "--home", "/h"],
        &[
            "connector",
            "allow",
            "--request",
            "r",
            "--code",
            "1234",
            "--home",
            "/h",
        ],
        &[
            "connector",
            "deny",
            "--request",
            "r",
            "--code",
            "1234",
            "--home",
            "/h",
        ],
        &["remote", "on", "--home", "/h"],
        &[
            "remote",
            "on",
            "--relay-url",
            "https://r.example",
            "--home",
            "/h",
        ],
        &["remote", "on", "--via", "tailscale-app", "--home", "/h"],
        &["remote", "on", "--private", "--home", "/h"],
        &[
            "remote",
            "on",
            "--domain",
            "d.example",
            "--name",
            "m",
            "--home",
            "/h",
        ],
        &["remote", "off", "--home", "/h"],
        &["remote", "status", "--snippets", "--home", "/h"],
        &["remote", "key", "--relay-url", "https://r.example"],
        &["remote", "certificate-accept", "abc", "--home", "/h"],
        &[
            "web",
            "enroll",
            "--host",
            "http://localhost:1",
            "--home",
            "/h",
        ],
        &["web", "enroll", "--public", "--home", "/h"],
        &["web", "lock", "--home", "/h"],
        &["web", "devices", "--home", "/h"],
        &["revoke", "id", "--home", "/h"],
        &["scope", "id", "--scope", "a:read", "--home", "/h"],
        &["status", "--home", "/h"],
        &["doctor", "--home", "/h"],
        &["backup", "/b", "--home", "/h"],
        &["restore", "/b", "--home", "/h"],
        &[
            "restore",
            "--backup",
            "/b",
            "--accept-ledger-loss",
            "--home",
            "/h",
        ],
        &["install-credentials", "--home", "/h"],
        &["permissions", "request", "camera", "--helper-app", "/a.app"],
        &[
            "selftest",
            "messages",
            "--helper-app",
            "/a.app",
            "--force",
            "--no-ui",
        ],
        &["setup", "--helper-app", "/a.app"],
        &["shortcuts", "install-pack", "--pack", "/p"],
        &["-home", "/h", "status"],
        &["status", "-home=/h"],
        &["pair", "--iphone=true", "--via-remote", "--home=/h"],
    ];
    for form in forms {
        let arguments = std::iter::once("bridge").chain(form.iter().copied());
        ab_runtime::cli::parse(arguments.map(std::ffi::OsString::from))
            .map_err(|error| format!("{form:?}: {error}"))?;
    }
    Ok(())
}
