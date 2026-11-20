use super::{
    Failure, Outcome, children, commands,
    failure::{Reason, Step},
    gateway,
    modes::Modes,
    tree::{DEFAULT_LINK_NAME, LinkArgs, LinkClient, LinkScope, LinkTarget, UnlinkArgs},
    ui,
};
use clap::ValueEnum;
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const PHASE_BUDGET: Duration = Duration::from_secs(30);
const SLOW_AFTER: Duration = Duration::from_secs(3);

pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
}

impl Launch {
    pub fn for_home(home: &Path) -> Self {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("bridge"));
        let launcher = exe
            .ancestors()
            .find(|path| path.extension().is_some_and(|e| e == "app"))
            .and_then(Path::parent)
            .map(|package| package.join("bin/bridge"))
            .filter(|launcher| launcher.is_file());
        let mut args = vec!["mcp".to_owned()];
        let absolute = std::path::absolute(home).unwrap_or_else(|_| home.to_owned());
        if crate::default_home().ok().as_deref() != Some(absolute.as_path()) {
            args.extend(["--home".to_owned(), absolute.to_string_lossy().into_owned()]);
        }
        Self {
            command: launcher.unwrap_or(exe).to_string_lossy().into_owned(),
            args,
        }
    }

    fn argv(&self) -> Vec<String> {
        let mut argv = vec![self.command.clone()];
        argv.extend(self.args.iter().cloned());
        argv
    }

    fn stdio(&self) -> Value {
        json!({"command": self.command, "args": self.args})
    }
}

struct Tool {
    program: &'static str,
    title: &'static str,
    install: &'static str,
    add: Vec<String>,
    remove: Vec<String>,
    get: Vec<String>,
}

struct File {
    title: &'static str,
    path: PathBuf,
    section: &'static str,
    entry: Value,
    after: &'static str,
}

enum Setup {
    Tool(Tool),
    File(File),
    Open {
        title: &'static str,
        url: String,
        after: &'static str,
    },
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn with(mut head: Vec<String>, tail: Vec<String>) -> Vec<String> {
    head.extend(tail);
    head
}

fn user_home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map_or_else(|| user_home().join(".config"), PathBuf::from)
}

pub fn encode_component(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn vscode_server(name: &str, launch: &Launch) -> Value {
    json!({"name": name, "type": "stdio", "command": launch.command, "args": launch.args})
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
    })
}

fn setup(client: LinkClient, name: &str, scope: LinkScope, launch: &Launch) -> Option<Setup> {
    let argv = launch.argv();
    Some(match client {
        LinkClient::Claude => Setup::Tool(Tool {
            program: "claude",
            title: "Claude Code",
            install: "https://code.claude.com",
            add: with(
                strings(&["mcp", "add", "--scope", scope.as_str(), name, "--"]),
                argv,
            ),
            remove: strings(&["mcp", "remove", "--scope", scope.as_str(), name]),
            get: strings(&["mcp", "get", name]),
        }),
        LinkClient::Codex => Setup::Tool(Tool {
            program: "codex",
            title: "Codex",
            install: "https://developers.openai.com/codex",
            add: with(strings(&["mcp", "add", name, "--"]), argv),
            remove: strings(&["mcp", "remove", name]),
            get: strings(&["mcp", "get", name]),
        }),
        LinkClient::Gemini => Setup::Tool(Tool {
            program: "gemini",
            title: "Gemini CLI",
            install: "https://github.com/google-gemini/gemini-cli",
            add: with(strings(&["mcp", "add", "--scope", "user", name]), argv),
            remove: strings(&["mcp", "remove", "--scope", "user", name]),
            get: Vec::new(),
        }),
        LinkClient::Vscode if on_path("code") => Setup::Tool(Tool {
            program: "code",
            title: "Visual Studio Code",
            install: "https://code.visualstudio.com",
            add: vec![
                "--add-mcp".to_owned(),
                vscode_server(name, launch).to_string(),
            ],
            remove: Vec::new(),
            get: Vec::new(),
        }),
        LinkClient::Vscode => Setup::Open {
            title: "Visual Studio Code",
            url: format!(
                "vscode:mcp/install?{}",
                encode_component(&vscode_server(name, launch).to_string())
            ),
            after: "Click Install in Visual Studio Code.",
        },
        LinkClient::Cursor => Setup::File(File {
            title: "Cursor",
            path: user_home().join(".cursor/mcp.json"),
            section: "mcpServers",
            entry: launch.stdio(),
            after: "Cursor loads it on its own. If it doesn't, open Cursor Settings, then MCP.",
        }),
        LinkClient::ClaudeDesktop => Setup::File(File {
            title: "the Claude app",
            path: user_home().join("Library/Application Support/Claude/claude_desktop_config.json"),
            section: "mcpServers",
            entry: launch.stdio(),
            after: "Quit and reopen the Claude app to load it.",
        }),
        LinkClient::Zed => Setup::File(File {
            title: "Zed",
            path: config_home().join("zed/settings.json"),
            section: "context_servers",
            entry: json!({"source": "custom", "command": launch.command, "args": launch.args, "env": {}}),
            after: "Zed loads it on its own.",
        }),
        LinkClient::Opencode => Setup::File(File {
            title: "OpenCode",
            path: config_home().join("opencode/opencode.json"),
            section: "mcp",
            entry: json!({"type": "local", "command": argv, "enabled": true}),
            after: "Start a new OpenCode session to load it.",
        }),
        LinkClient::Goose => {
            let mut url = format!(
                "goose://extension?cmd={}",
                encode_component(&launch.command)
            );
            for arg in &launch.args {
                url.push_str("&arg=");
                url.push_str(&encode_component(arg));
            }
            let _ = write!(
                url,
                "&id={}&name={}&description={}",
                encode_component(name),
                encode_component("Apple Bridge"),
                encode_component("Messages, Mail, Calendar and more from this Mac")
            );
            Setup::Open {
                title: "Goose",
                url,
                after: "Confirm the extension in Goose.",
            }
        }
        LinkClient::Other | LinkClient::GrokBot => return None,
    })
}

fn shown(program: &str, arguments: &[String]) -> String {
    let quoted: Vec<String> = arguments
        .iter()
        .map(|argument| ui::shell_quote(argument))
        .collect();
    format!("{program} {}", quoted.join(" "))
}

fn piped_result(text: &str) {
    if !Modes::get().json && !Modes::get().stdout_tty() {
        ui::outln(text);
    }
}

fn snippet(section: &str, name: &str, entry: &Value) -> Value {
    json!({ section: { name: entry } })
}

fn print_other(name: &str, launch: &Launch) {
    let config = snippet("mcpServers", name, &launch.stdio());
    if Modes::get().json {
        ui::json_document(
            json!({"client": "other", "name": name, "command": launch.command, "args": launch.args, "config": config}),
        );
        return;
    }
    ui::outln(&serde_json::to_string_pretty(&config).unwrap_or_default());
    ui::note(&format!(
        "Any agent on this Mac can start Apple Bridge with this command: {}",
        shown(&ui::shell_quote(&launch.command), &launch.args)
    ));
    ui::note(
        "The first time an agent connects, click Allow in the Apple Bridge panel on your Mac.",
    );
}

fn print_grok_connection(answer: &Value, url: &str, name: &str) -> Outcome {
    let mcp = answer["mcp"].as_str().ok_or_else(|| {
        Failure::new(
            Reason::ApiError,
            "The Mac didn't return a Tailscale MCP address.",
        )
    })?;
    if mcp != format!("{url}/mcp") {
        return Err(Failure::new(
            Reason::Conflict,
            "The Tailscale address changed. Ask again with bridge link grok-bot.",
        ));
    }
    let token = answer["token"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::new(Reason::ApiError, "The Mac did not return a key."))?;
    let config = json!({"mcpServers": {name: {"type": "http", "url": mcp, "headers": {"Authorization": format!("Bearer {token}")}}}});
    let result = json!({
        "outcome": "allowed", "client": "Grok Bot", "name": name,
        "mcp": mcp, "token": answer["token"], "header": answer["header"], "config": config,
        "next": "Configure Grok Bot's connector on its Tailscale-connected cloud computer with this URL and header, then call apple_status (sources.status). Keep the key private. The Mac must stay awake.",
    });
    if Modes::get().json {
        ui::json_document(result);
    } else {
        ui::outln(&serde_json::to_string_pretty(&result).unwrap_or_default());
    }
    Ok(())
}

async fn grok_bot(
    home: &Path,
    name: &str,
    print: bool,
    timeout: Option<&str>,
    limit: Duration,
) -> Outcome {
    let mut retry = vec!["bridge", "link", "grok-bot", "--json", "--name", name];
    if let Some(timeout) = timeout {
        retry.extend(["--timeout", timeout]);
    }
    if print {
        let home = home.to_string_lossy();
        retry.extend(["--home", &home]);
        let command = retry
            .iter()
            .map(|part| ui::shell_quote(part))
            .collect::<Vec<_>>()
            .join(" ");
        if Modes::get().json {
            ui::json_document(json!({"client": "Grok Bot", "command": command}));
        } else {
            ui::outln(&command);
        }
        ui::note(
            "Run this on the Mac. It enables private Tailscale access, waits for your Allow, and returns the MCP URL and bearer header for Grok Bot's Tailscale-connected cloud computer.",
        );
        return Ok(());
    }
    Modes::get().require_native_interaction("the Grok Bot connection", &retry)?;
    let url = super::connect::turn_on(home, false).await?;
    let ticket = crate::connector::ask_for_tailnet_agent(home, "Grok Bot")
        .await
        .map_err(|_| {
            Failure::new(
                Reason::ApiError,
                "Couldn't ask the Mac to connect Grok Bot.",
            )
            .sentence("If Apple Bridge was updated while running, stop the older gateway with bridge down and start it with bridge up.")
            .then("Check Apple Bridge", ["bridge", "status"])
        })?;
    let id = ticket["id"].as_str().ok_or_else(|| {
        Failure::new(
            Reason::ApiError,
            "The Mac didn't return a connection request.",
        )
    })?;
    let secret = ticket["secret"].as_str().ok_or_else(|| {
        Failure::new(
            Reason::ApiError,
            "The Mac didn't return a connection secret.",
        )
    })?;
    ui::note(
        "Click Allow on your Mac. This command waits for you; the key is returned only after Allow.",
    );
    let deadline = Instant::now() + limit.min(Duration::from_mins(5));
    loop {
        let answer =
            crate::connector::tailnet_agent_answer(home, id, secret, &format!("{url}/mcp"))
                .await
                .map_err(|_| {
                    Failure::new(
                        Reason::ApiError,
                        "The connection request expired or the gateway restarted.",
                    )
                    .then("Ask again", ["bridge", "link", "grok-bot", "--json"])
                })?;
        match answer["state"].as_str() {
            Some("allowed") => {
                print_grok_connection(&answer, &url, name)?;
                return Ok(());
            }
            Some("denied") => {
                return Err(Failure::new(
                    Reason::PermissionDenied,
                    "The Mac declined Grok Bot. No connection was made.",
                ));
            }
            Some("waiting") => {}
            _ => {
                return Err(Failure::new(
                    Reason::ApiError,
                    "The Mac returned an unknown connection state.",
                ));
            }
        }
        if Instant::now() >= deadline {
            return Err(Failure::new(
                Reason::Timeout,
                "Stopped waiting for Allow. No key was collected.",
            )
            .then("Ask again", ["bridge", "link", "grok-bot", "--json"]));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[derive(Debug, Clone, Copy)]
struct Deadline {
    end: Option<Instant>,
    given: Option<Duration>,
}

impl Deadline {
    fn new(given: Option<Duration>) -> Self {
        Self {
            end: given.map(|given| Instant::now() + given),
            given,
        }
    }

    fn within(&self, phase: Duration) -> Instant {
        let phase_end = Instant::now() + phase;
        self.end.map_or(phase_end, |end| end.min(phase_end))
    }
}

struct Phase {
    what: String,
    confirmed: String,
}

fn spawn(tool: &Tool, arguments: &[String]) -> Outcome<std::process::Child> {
    let mut command = Command::new(tool.program);
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            missing(tool)
        } else {
            Failure::new(
                Reason::SpawnFailed,
                format!("Couldn't run {}.", tool.program),
            )
            .row("Detail", ui::clean(&error.to_string()))
        }
    })
}

fn drain(
    source: Option<impl std::io::Read + Send + 'static>,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut source) = source {
            let _ = source.read_to_end(&mut bytes);
        }
        let _ = sender.send(bytes);
    });
    receiver
}

fn stop_group(leader: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{leader}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn collect_output(
    receiver: &std::sync::mpsc::Receiver<Vec<u8>>,
    until: Instant,
    leader: u32,
) -> Vec<u8> {
    let grace = until
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(500));
    receiver.recv_timeout(grace).unwrap_or_else(|_| {
        stop_group(leader);
        receiver
            .recv_timeout(Duration::from_millis(200))
            .unwrap_or_default()
    })
}

fn stopped(tool: &Tool, arguments: &[String], phase: &Phase, deadline: &Deadline) -> Failure {
    let shown_command = shown(tool.program, arguments);
    let headline = match deadline.given {
        Some(given) => format!(
            "Stopped {} after {}. --timeout passed.",
            phase.what,
            commands::friendly(given)
        ),
        None => format!(
            "Stopped {}. {} didn't answer in time.",
            phase.what, tool.title
        ),
    };
    Failure::new(Reason::Timeout, headline)
        .row("Command", &shown_command)
        .row("Confirmed", &phase.confirmed)
        .data("command", shown_command)
        .data("confirmed", phase.confirmed.clone())
        .sentence(format!(
            "Apple Bridge stopped {} and waited for it to exit.",
            tool.program
        ))
        .then(
            "Try again with more time",
            ["bridge", "link", tool.program, "--timeout", "2m"],
        )
}

fn run_bounded(
    tool: &Tool,
    arguments: &[String],
    deadline: &Deadline,
    phase: &Phase,
) -> Outcome<(bool, String)> {
    let mut child = spawn(tool, arguments)?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let mut child = children::Owned::new(child);
    let leader = child.0.id();
    let began = Instant::now();
    let until = deadline.within(PHASE_BUDGET);
    let mut announced = false;
    if Modes::get().accessible {
        ui::note(&format!("Now {}…", phase.what));
    }
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if Instant::now() >= until {
            child.stop();
            stop_group(leader);
            return Err(stopped(tool, arguments, phase, deadline));
        }
        if !announced && began.elapsed() >= SLOW_AFTER {
            announced = true;
            ui::note(&format!("Still waiting for {}…", phase.what));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut text = String::from_utf8_lossy(&collect_output(&stdout, until, leader)).into_owned();
    text.push_str(&String::from_utf8_lossy(&collect_output(
        &stderr, until, leader,
    )));
    Ok((status.success(), text))
}

fn missing(tool: &Tool) -> Failure {
    Failure::new(
        Reason::MissingCommand,
        format!(
            "Can't find {}. {} isn't on your PATH.",
            tool.title, tool.program
        ),
    )
    .sentence(format!(
        "Install it from {}, then run this again.",
        tool.install
    ))
    .then("Or connect another agent", ["bridge", "link", "other"])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Added {
    New,
    Updated,
    Already,
    Opened,
}

impl Added {
    fn outcome(self) -> &'static str {
        match self {
            Self::New => "added",
            Self::Updated => "updated",
            Self::Already => "already_added",
            Self::Opened => "opened",
        }
    }
}

fn ours(text: &str, launch: &Launch) -> bool {
    text.contains(&launch.command)
        || text.contains("/bin/bridge")
        || (text.contains("/mcp") && (text.contains("127.0.0.1:") || text.contains("localhost:")))
}

fn add_with_tool(tool: &Tool, name: &str, launch: &Launch, deadline: &Deadline) -> Outcome<Added> {
    let phase = |what: String| Phase {
        what,
        confirmed: format!("not known whether {} saved {name}", tool.title),
    };
    let adding = phase(format!("adding {name} to {}", tool.title));
    let (success, text) = run_bounded(tool, &tool.add, deadline, &adding)?;
    if success {
        return Ok(Added::New);
    }
    if !text.contains("already exists") || tool.get.is_empty() {
        return Err(Failure::new(
            Reason::ApiError,
            format!("{} didn't add the server.", tool.title),
        )
        .remote(tool.title, text.trim()));
    }
    let (_, details) = run_bounded(
        tool,
        &tool.get,
        deadline,
        &phase(format!("checking {name} in {}", tool.title)),
    )?;
    if details.contains(&launch.command) && launch.args.iter().all(|arg| details.contains(arg)) {
        return Ok(Added::Already);
    }
    if !ours(&details, launch) {
        return Err(Failure::new(
            Reason::Conflict,
            format!(
                "Can't add {name}. {} already has a different server with that name.",
                tool.title
            ),
        )
        .then(
            "Pick another name",
            ["bridge", "link", tool.program, "--name", "<NAME>"],
        ));
    }
    run_bounded(
        tool,
        &tool.remove,
        deadline,
        &phase(format!("replacing the old {name} in {}", tool.title)),
    )?;
    let (success, text) = run_bounded(tool, &tool.add, deadline, &adding)?;
    if success {
        Ok(Added::Updated)
    } else {
        Err(Failure::new(
            Reason::ApiError,
            format!("{} didn't add the server.", tool.title),
        )
        .remote(tool.title, text.trim()))
    }
}

struct Config {
    header: String,
    values: serde_json::Map<String, Value>,
}

fn without_trailing_commas(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if in_string {
            in_string = escaped || character != '"';
            escaped = !escaped && character == '\\';
        } else if character == '"' {
            in_string = true;
        } else if character == ',' && text[index + 1..].trim_start().starts_with(['}', ']']) {
            continue;
        }
        kept.push(character);
    }
    kept
}

fn read_config(file: &File, name: &str) -> Outcome<Config> {
    let text = match std::fs::read_to_string(&file.path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(Failure::new(
                Reason::PermissionDenied,
                format!("Can't read {}.", file.path.display()),
            )
            .row("Detail", ui::clean(&error.to_string())));
        }
    };
    let header: String = text
        .split_inclusive('\n')
        .take_while(|line| line.trim().is_empty() || line.trim_start().starts_with("//"))
        .collect();
    let body = &text[header.len()..];
    if body.trim().is_empty() {
        return Ok(Config {
            header,
            values: serde_json::Map::new(),
        });
    }
    match serde_json::from_str::<Value>(&without_trailing_commas(body)) {
        Ok(Value::Object(values)) => Ok(Config { header, values }),
        _ => Err(Failure::new(
            Reason::InvalidConfig,
            format!(
                "Can't safely edit {}. It isn't plain JSON, so Apple Bridge left it alone.",
                file.path.display()
            ),
        )
        .sentence(format!(
            "Add this to it yourself: {}",
            serde_json::to_string(&snippet(file.section, name, &file.entry)).unwrap_or_default()
        ))),
    }
}

fn write_config(path: &Path, config: &Config) -> Outcome {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let mode = std::fs::metadata(path).map_or(0o600, |meta| meta.permissions().mode() & 0o777);
    let temporary = directory.join(format!(
        ".{}.bridge-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let mut text = config.header.clone();
    text.push_str(&serde_json::to_string_pretty(&config.values)?);
    text.push('\n');
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temporary)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
        .and_then(|()| std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode)))
        .and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(Failure::new(
            Reason::PermissionDenied,
            format!("Can't save {}.", path.display()),
        )
        .row("Detail", ui::clean(&error.to_string())));
    }
    Ok(())
}

fn add_to_file(file: &File, name: &str, launch: &Launch) -> Outcome<Added> {
    let mut config = read_config(file, name)?;
    let section = config
        .values
        .entry(file.section)
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Value::Object(servers) = section else {
        return Err(Failure::new(
            Reason::InvalidConfig,
            format!(
                "Can't safely edit {}. Its {} isn't an object.",
                file.path.display(),
                file.section
            ),
        ));
    };
    let added = match servers.get(name) {
        Some(existing) if existing == &file.entry => return Ok(Added::Already),
        Some(existing) if !ours(&existing.to_string(), launch) => {
            return Err(Failure::new(
                Reason::Conflict,
                format!(
                    "Can't add {name}. {} already has a different server with that name.",
                    file.title
                ),
            )
            .row("File", file.path.display().to_string())
            .then(
                "Pick another name",
                ["bridge", "link", "<AGENT>", "--name", "<NAME>"],
            ));
        }
        Some(_) => Added::Updated,
        None => Added::New,
    };
    servers.insert(name.to_owned(), file.entry.clone());
    write_config(&file.path, &config)?;
    Ok(added)
}

fn open_url(url: &str, title: &str) -> Outcome {
    let status = Command::new("/usr/bin/open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if status.is_ok_and(|status| status.success()) {
        return Ok(());
    }
    Err(Failure::new(
        Reason::MissingCommand,
        format!("Can't open {title}. It doesn't seem to be installed."),
    )
    .row("Link", url)
    .then("Or connect another agent", ["bridge", "link", "other"]))
}

fn try_it(title: &str) {
    ui::note(&format!(
        "The first time {title} starts Apple Bridge, click Allow in the Apple Bridge panel on your Mac. After that it just works."
    ));
    ui::note(
        "Try it: ask your agent \"Use Apple Bridge to show my reminder lists and how much has synced.\"",
    );
}

fn print_setup(setup: &Setup, name: &str) {
    let json = Modes::get().json;
    match setup {
        Setup::Tool(tool) => {
            if json {
                let mut argv = vec![tool.program.to_owned()];
                argv.extend(tool.add.iter().cloned());
                ui::json_document(
                    json!({"client": tool.program, "name": name, "argv": argv, "command": shown(tool.program, &tool.add)}),
                );
            } else {
                ui::outln(&shown(tool.program, &tool.add));
            }
        }
        Setup::File(file) => {
            let config = snippet(file.section, name, &file.entry);
            if json {
                ui::json_document(json!({"name": name, "file": file.path, "config": config}));
            } else {
                ui::outln(&serde_json::to_string_pretty(&config).unwrap_or_default());
                ui::note(&format!("Add this to {}.", file.path.display()));
            }
        }
        Setup::Open { url, .. } => {
            if json {
                ui::json_document(json!({"name": name, "url": url}));
            } else {
                ui::outln(url);
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
pub async fn link(home: &Path, args: &LinkArgs) -> Outcome {
    use LinkTarget as Target;
    let (client, named, tool, scope) = match &args.client {
        Target::Claude(args) => (LinkClient::Claude, None, Some(&args.tool), args.scope),
        Target::Codex(args) => (LinkClient::Codex, None, Some(args), LinkScope::User),
        Target::Gemini(args) => (LinkClient::Gemini, None, Some(args), LinkScope::User),
        Target::Vscode(args) => (LinkClient::Vscode, None, Some(args), LinkScope::User),
        Target::GrokBot(args) => (LinkClient::GrokBot, None, Some(args), LinkScope::User),
        Target::Cursor(args) => (LinkClient::Cursor, Some(args), None, LinkScope::User),
        Target::Zed(args) => (LinkClient::Zed, Some(args), None, LinkScope::User),
        Target::Goose(args) => (LinkClient::Goose, Some(args), None, LinkScope::User),
        Target::Opencode(args) => (LinkClient::Opencode, Some(args), None, LinkScope::User),
        Target::Other(args) => (LinkClient::Other, Some(args), None, LinkScope::User),
        Target::ClaudeApp(_) => (LinkClient::ClaudeDesktop, None, None, LinkScope::User),
    };
    let named = named.or(tool.map(|args| &args.named));
    let name = named.map_or(DEFAULT_LINK_NAME, |args| args.name.as_str());
    let print = match &args.client {
        Target::ClaudeApp(args) => args.print,
        _ => named.is_some_and(|args| args.output.print),
    };
    let timeout = tool.and_then(|args| args.timeout.as_deref());
    let given = timeout
        .map(|text| commands::duration("--timeout", text))
        .transpose()?;
    let deadline = Deadline::new(given);
    if client == LinkClient::GrokBot {
        return grok_bot(
            home,
            name,
            print,
            timeout,
            given.unwrap_or(Duration::from_mins(5)),
        )
        .await;
    }
    let launch = Launch::for_home(home);
    let Some(setup) = setup(client, name, scope, &launch) else {
        print_other(name, &launch);
        return Ok(());
    };
    if print {
        print_setup(&setup, name);
        return Ok(());
    }
    if client == LinkClient::Claude || matches!(setup, Setup::Open { .. }) {
        let value = client.to_possible_value();
        let client_name = value
            .as_ref()
            .map_or("", clap::builder::PossibleValue::get_name);
        let mut retry = vec!["bridge", "link", client_name, "--name", name];
        if client == LinkClient::Claude {
            retry.extend(["--scope", scope.as_str()]);
        }
        if let Some(timeout) = timeout {
            retry.extend(["--timeout", timeout]);
        }
        Modes::get().require_native_interaction("the agent connection", &retry)?;
    }
    let running = gateway::running_address(home).await.is_ok();
    let (title, added, after, place) = match &setup {
        Setup::Tool(tool) => (
            tool.title,
            add_with_tool(tool, name, &launch, &deadline)?,
            "",
            shown(tool.program, &tool.add),
        ),
        Setup::File(file) => (
            file.title,
            add_to_file(file, name, &launch)?,
            file.after,
            file.path.display().to_string(),
        ),
        Setup::Open { title, url, after } => {
            open_url(url, title)?;
            (*title, Added::Opened, *after, url.clone())
        }
    };
    if Modes::get().json {
        ui::json_document(json!({
            "outcome": added.outcome(),
            "client": title,
            "name": name,
            "command": launch.command,
            "args": launch.args,
            "where": place,
            "running": running,
        }));
        return Ok(());
    }
    piped_result(name);
    match added {
        Added::Already => ui::receipt(
            "Already added",
            &format!("{name} to {title}"),
            &format!("{name} is already in {title}."),
        ),
        Added::Updated => ui::done("Updated", &format!("{name} in {title}")),
        Added::New => ui::done("Added", &format!("{name} to {title}")),
        Added::Opened => ui::done("Opened", title),
    }
    if !after.is_empty() {
        ui::note(after);
    }
    if client == LinkClient::Claude && scope == LinkScope::Project {
        ui::note(
            "Claude Code asks you to approve project servers the first time it starts in this folder.",
        );
    }
    if !running {
        ui::warn(
            "not_running",
            "Apple Bridge isn't running yet. Start it with bridge up before you use the agent.",
        );
    }
    try_it(title);
    Ok(())
}

pub fn unlink(home: &Path, args: &UnlinkArgs) -> Outcome {
    let given = args
        .timeout
        .as_deref()
        .map(|text| commands::duration("--timeout", text))
        .transpose()?;
    let deadline = Deadline::new(given);
    let launch = Launch::for_home(home);
    let find = Step::new(
        "To end its access on this Mac too, find it",
        ["bridge", "agents", "list"],
    );
    let report = |outcome: &str, title: &str, next: &[Step]| {
        if Modes::get().json {
            ui::json_document(json!({
                "outcome": outcome,
                "client": title,
                "name": args.name,
                "next": next.iter().map(Step::to_json).collect::<Vec<_>>(),
            }));
        } else {
            piped_result(&args.name);
            let (label, sentence) = if outcome == "removed" {
                ("Removed", format!("Removed {} from {title}.", args.name))
            } else {
                (
                    "Already removed",
                    format!("{} isn't in {title}.", args.name),
                )
            };
            ui::receipt(label, &format!("{} from {title}", args.name), &sentence);
            ui::next_steps(next);
        }
    };
    match setup(args.client, &args.name, args.scope, &launch) {
        Some(Setup::Tool(tool)) if !tool.remove.is_empty() => {
            if matches!(args.client, LinkClient::Claude | LinkClient::Codex) {
                let _ = run_bounded(
                    &tool,
                    &strings(&["mcp", "logout", &args.name]),
                    &deadline,
                    &Phase {
                        what: "signing out".to_owned(),
                        confirmed: format!("{} is still registered in {}", args.name, tool.title),
                    },
                );
            }
            let (success, text) = run_bounded(
                &tool,
                &tool.remove,
                &deadline,
                &Phase {
                    what: "removing the server".to_owned(),
                    confirmed: format!("not known whether {} removed {}", tool.title, args.name),
                },
            )?;
            if success {
                report("removed", tool.title, &[find]);
                return Ok(());
            }
            if text.contains("not found")
                || text.contains("No MCP server")
                || text.contains("does not exist")
            {
                report("already_removed", tool.title, &[]);
                return Ok(());
            }
            Err(Failure::new(
                Reason::ApiError,
                format!("{} couldn't remove {}.", tool.title, args.name),
            )
            .remote(tool.title, text.trim()))
        }
        Some(Setup::File(file)) => {
            let mut config = read_config(&file, &args.name)?;
            let removed = config
                .values
                .get_mut(file.section)
                .and_then(Value::as_object_mut)
                .and_then(|servers| servers.remove(&args.name))
                .is_some();
            if !removed {
                report("already_removed", file.title, &[]);
                return Ok(());
            }
            write_config(&file.path, &config)?;
            report("removed", file.title, &[find]);
            Ok(())
        }
        _ => Err(Failure::new(
            Reason::InvalidValue,
            "Can't unlink this agent from here. Remove Apple Bridge in the agent's own settings.",
        )
        .then(
            "To end its access on this Mac, find it",
            ["bridge", "agents", "list"],
        )),
    }
}
