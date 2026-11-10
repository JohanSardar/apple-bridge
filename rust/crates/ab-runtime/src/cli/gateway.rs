use super::{
    Failure, Outcome,
    commands::{address, load_config},
    failure::{Reason, Step},
    modes::Modes,
    stats,
    tree::UpArgs,
    ui,
};
use crate::{Config, bundle};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Seek, SeekFrom},
    net::{SocketAddr, TcpListener},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const DEFAULT_API: &str = "127.0.0.1:7780";
const DEFAULT_PEER: &str = "127.0.0.1:7443";
const ANY_PEER: &str = "127.0.0.1:0";
const START_BUDGET: Duration = Duration::from_secs(60);
const STOP_BUDGET: Duration = Duration::from_secs(20);

pub fn lock_held(home: &Path) -> bool {
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .open(home.join("lock"))
    else {
        return false;
    };
    file.try_lock().is_err()
}

fn pid_file(home: &Path) -> PathBuf {
    home.join("run/gateway.pid")
}

fn log_file(home: &Path) -> PathBuf {
    home.join("gateway.log")
}

pub async fn healthy(address: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
    else {
        return false;
    };
    client
        .get(format!("http://{address}/v1/health"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

pub async fn running_address(home: &Path) -> Outcome<String> {
    let address = load_config(home)?
        .map(|config| config.api_address)
        .ok_or_else(|| Failure::not_running(home))?;
    if lock_held(home) && healthy(&address).await {
        Ok(address)
    } else {
        Err(Failure::not_running(home))
    }
}

pub fn mcp_address(api_address: &str) -> String {
    format!("http://{api_address}/mcp")
}

fn prepare_home(home: &Path) -> Outcome {
    let cannot = |error: std::io::Error| {
        Failure::new(
            Reason::PermissionDenied,
            format!("Can't create {}.", home.display()),
        )
        .row("Detail", ui::clean(&error.to_string()))
        .then(
            "Choose a folder you can write to",
            ["bridge", "up", "--home", "<DIR>"],
        )
    };
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(home.join("run"))
        .map_err(cannot)?;
    fs::set_permissions(home, fs::Permissions::from_mode(0o700)).map_err(cannot)?;
    fs::set_permissions(home.join("run"), fs::Permissions::from_mode(0o700)).map_err(cannot)?;
    Ok(())
}

pub fn loopback(text: &str) -> Outcome<SocketAddr> {
    let parsed = text.parse::<SocketAddr>().map_err(|_| {
        Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --api-addr {}. Addresses look like 127.0.0.1:7780.",
                ui::clean(text)
            ),
        )
    })?;
    if !parsed.ip().is_loopback() {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --api-addr {}. It must be a loopback address such as 127.0.0.1:PORT.",
                ui::clean(text)
            ),
        )
        .sentence("For access from other devices, turn on remote access instead.")
        .then("Turn on remote access", ["bridge", "remote", "enable"]));
    }
    Ok(parsed)
}

fn free(address: SocketAddr) -> std::io::Result<()> {
    TcpListener::bind(address).map(drop)
}

fn api_address(args: &UpArgs, previous: Option<&Config>) -> Outcome<String> {
    let chosen = args
        .api_addr
        .clone()
        .or_else(|| {
            previous
                .map(|config| config.api_address.clone())
                .filter(|address| !address.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_API.to_owned());
    match free(loopback(&chosen)?) {
        Err(error) if error.kind() == ErrorKind::AddrInUse => Err(Failure::new(
            Reason::PortInUse,
            format!("Can't start on {chosen}. It's already in use by another program."),
        )
        .then(
            "Choose a free address",
            ["bridge", "up", "--api-addr", "127.0.0.1:<PORT>"],
        )),
        Err(error) => Err(
            Failure::new(Reason::PortInUse, format!("Can't use {chosen}."))
                .row("Detail", ui::clean(&error.to_string())),
        ),
        Ok(()) => Ok(chosen),
    }
}

fn peer_address(args: &UpArgs, previous: Option<&Config>) -> String {
    if let Some(explicit) = &args.peer_addr {
        return explicit.clone();
    }
    let wanted = previous
        .map(|config| config.peer_address.clone())
        .filter(|address| !address.is_empty())
        .unwrap_or_else(|| DEFAULT_PEER.to_owned());
    let available = wanted
        .parse::<SocketAddr>()
        .is_ok_and(|address| free(address).is_ok());
    if available {
        wanted
    } else {
        ANY_PEER.to_owned()
    }
}

fn helper_app(args: &UpArgs, previous: Option<&Config>) -> String {
    let detected = bundle::helper_app(
        args.helper_app.as_deref().unwrap_or_default(),
        std::env::current_exe().ok().as_deref(),
    );
    if detected.is_empty() {
        previous
            .map(|config| config.helper_app.clone())
            .unwrap_or_default()
    } else {
        detected
    }
}

fn log_tail(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let size = file.metadata().map_or(0, |meta| meta.len());
    if file
        .seek(SeekFrom::Start(size.saturating_sub(800)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

struct Launch<'a> {
    home: &'a Path,
    api: &'a str,
    peer: &'a str,
    helper: &'a str,
}

fn spawn_gateway(launch: &Launch<'_>) -> Outcome<std::process::Child> {
    let spawn_failed = |error: std::io::Error| {
        Failure::new(Reason::SpawnFailed, "Failed to start the gateway process.")
            .row("Detail", ui::clean(&error.to_string()))
    };
    let executable = std::env::current_exe().map_err(spawn_failed)?;
    let log = crate::gateway_lifetime::open_log(&log_file(launch.home))?;
    let mut command = Command::new("/usr/bin/nohup");
    command
        .arg(executable)
        .args(["start", "--role", "gateway", "--home"])
        .arg(launch.home)
        .args(["--api-addr", launch.api, "--peer-addr", launch.peer])
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0);
    if !launch.helper.is_empty() {
        command.args(["--helper-app", launch.helper]);
    }
    let child = command.spawn().map_err(spawn_failed)?;
    ab_trust::atomic_write(&pid_file(launch.home), child.id().to_string().as_bytes())?;
    Ok(child)
}

async fn wait_until_healthy(
    home: &Path,
    address: &str,
    child: &mut std::process::Child,
) -> Outcome {
    let deadline = Instant::now() + START_BUDGET;
    loop {
        if let Some(status) = child.try_wait()? {
            let tail = log_tail(&log_file(home));
            let mut failure = Failure::new(
                Reason::SpawnFailed,
                format!("The gateway stopped right after starting ({status})."),
            )
            .row("Log", log_file(home).display().to_string());
            if !tail.is_empty() {
                failure = failure.remote("The gateway log", tail);
            }
            return Err(failure.then("Try again after fixing the cause", ["bridge", "up"]));
        }
        if healthy(address).await {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Failure::new(
                Reason::Timeout,
                "Stopped waiting after 60s. The gateway hasn't answered yet.",
            )
            .row("Log", log_file(home).display().to_string())
            .sentence("It may still be starting; nothing was stopped.")
            .then("Check again", ["bridge", "up"]));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn start_gateway(home: &Path, args: &UpArgs, previous: Option<&Config>) -> Outcome<String> {
    let api = api_address(args, previous)?;
    prepare_home(home)?;
    super::access::open(home)?;
    let peer = peer_address(args, previous);
    let helper = helper_app(args, previous);
    if !Modes::get().json {
        ui::note("Starting Apple Bridge…");
    }
    let mut child = spawn_gateway(&Launch {
        home,
        api: &api,
        peer: &peer,
        helper: &helper,
    })?;
    wait_until_healthy(home, &api, &mut child).await?;
    Ok(load_config(home)?.map_or(api, |config| config.api_address))
}

fn remote_state(home: &Path) -> String {
    ab_remote::state::read::<serde_json::Value>(home, "status")
        .ok()
        .and_then(|status| status["state"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "off".to_owned())
}

fn print_summary(home: &Path, address: &str, started: bool, helper: bool, elapsed: Duration) {
    let verb = if started { "is on" } else { "was already on" };
    ui::done(
        &format!("Apple Bridge {verb}"),
        &format!(
            "({:.1} s). No account, server or domain needed.",
            elapsed.as_secs_f32()
        ),
    );
    ui::row("MCP address", &mcp_address(address));
    ui::row("Data folder", &home.display().to_string());
    let remote = remote_state(home);
    ui::row(
        "Remote access",
        if remote == "off" {
            "off (optional)"
        } else {
            &remote
        },
    );
    if !helper {
        ui::row("Mac data", "no Helper app found, so nothing new will sync");
        ui::note("Install Apple Bridge, or pass --helper-app.");
    }
}

fn validate_up(args: &UpArgs) -> Outcome {
    let mut failures = Vec::new();
    if let Some(api) = &args.api_addr
        && let Err(failure) = loopback(api)
    {
        failures.push(failure);
    }
    if let Some(peer) = &args.peer_addr
        && let Err(failure) = address("--peer-addr", peer)
    {
        failures.push(failure);
    }
    if let Some(helper) = &args.helper_app
        && !Path::new(helper).exists()
    {
        failures.push(Failure::new(
            Reason::NotFound,
            format!(
                "Can't use --helper-app {}. Nothing is there.",
                ui::clean(helper)
            ),
        ));
    }
    let mut failures = failures.into_iter();
    let Some(first) = failures.next() else {
        return Ok(());
    };
    let rest: Vec<Failure> = failures.collect();
    if rest.is_empty() {
        return Err(first);
    }
    let mut combined = Failure::new(
        Reason::InvalidValue,
        format!("Can't start. {} options have problems.", rest.len() + 1),
    );
    for failure in std::iter::once(first).chain(rest) {
        combined = combined.issue(failure.headline.clone());
    }
    Err(combined)
}

pub async fn up(home: &Path, args: &UpArgs) -> Outcome {
    let began = Instant::now();
    validate_up(args)?;
    super::check_socket_length(home)?;
    let previous = load_config(home)?;
    if previous
        .as_ref()
        .is_some_and(|config| config.role != "gateway")
    {
        return Err(Failure::new(
            Reason::Conflict,
            format!(
                "Can't start a gateway in {}. It's a replica of another gateway.",
                home.display()
            ),
        )
        .sentence("Choose another --home for a gateway on this Mac."));
    }
    let running = lock_held(home);
    let address = if running {
        let address = running_address(home).await.map_err(|_| {
            Failure::new(
                Reason::GatewayBusy,
                "A gateway holds this home but isn't answering yet.",
            )
            .then("Wait a moment, then try again", ["bridge", "up"])
        })?;
        if let Some(wanted) = &args.api_addr
            && wanted != &address
        {
            return Err(Failure::new(
                Reason::Conflict,
                format!("Can't move Apple Bridge to {wanted}. It's already running on {address}."),
            )
            .then(
                "Stop it, then start it on the new address",
                ["bridge", "down"],
            ));
        }
        address
    } else {
        start_gateway(home, args, previous.as_ref()).await?
    };
    let helper = load_config(home)?.is_some_and(|config| !config.helper_app.is_empty());
    let older = running
        .then(|| crate::gateway_lifetime::older(home))
        .flatten();
    let elapsed = began.elapsed();
    if Modes::get().json {
        ui::outln(
            &json!({
                "state": "on",
                "started": !running,
                "mcp": mcp_address(&address),
                "home": home,
                "helper": helper,
                "remote": remote_state(home),
                "seconds": elapsed.as_secs_f64(),
                "restart_needed": older.is_some(),
            })
            .to_string(),
        );
        return Ok(());
    }
    if !Modes::get().stdout_tty() {
        ui::outln(&mcp_address(&address));
    }
    print_summary(home, &address, !running, helper, elapsed);
    if let Some(version) = older {
        ui::warn(
            "restart_needed",
            &format!(
                "The running gateway ({version}) differs from this installed executable. Run bridge down, then bridge up to finish the update. If the app started it, restart the app instead."
            ),
        );
    } else if running && !home.join("run/gateway-build.json").exists() {
        ui::warn(
            "restart_needed",
            "This gateway predates update detection. If you updated, run bridge down, then bridge up.",
        );
    }
    stats::print_once_stderr(home)?;
    ui::next_steps(&[
        Step::new(
            "Connect Claude Code (or: bridge link codex)",
            ["bridge", "link", "claude"],
        ),
        Step::new(
            "Allow the Mac permissions you want",
            ["bridge", "permissions", "guide"],
        ),
        Step::new("Follow the first sync", ["bridge", "stats", "--follow"]),
    ]);
    Ok(())
}

fn started_by_up(pid: &str, home: &Path) -> bool {
    Command::new("ps")
        .args(["-p", pid, "-o", "command="])
        .output()
        .is_ok_and(|output| {
            let command = String::from_utf8_lossy(&output.stdout);
            command.contains(" start ") && command.contains(&*home.to_string_lossy())
        })
}

pub fn down(home: &Path) -> Outcome {
    if !lock_held(home) {
        let _ = fs::remove_file(pid_file(home));
        if Modes::get().json {
            ui::json_document(json!({"home": home, "outcome": "unchanged"}));
        } else {
            ui::done("Already stopped", &home.display().to_string());
        }
        return Ok(());
    }
    let foreign = || {
        Failure::new(
            Reason::NotOwned,
            "Can't stop this gateway. It was not started by bridge up, so it was left running.",
        )
        .sentence("Stop it the way it was started, such as from the Apple Bridge app.")
    };
    let pid = fs::read_to_string(pid_file(home)).map_err(|_| foreign())?;
    let pid = pid.trim();
    if !started_by_up(pid, home) {
        return Err(foreign());
    }
    let stopped = Command::new("/bin/kill")
        .args(["-TERM", pid])
        .status()?
        .success();
    if !stopped {
        return Err(
            Failure::new(Reason::Conflict, format!("Couldn't stop process {pid}."))
                .sentence("It may have exited already.")
                .then("Check again", ["bridge", "down"]),
        );
    }
    let deadline = Instant::now() + STOP_BUDGET;
    while lock_held(home) {
        if Instant::now() >= deadline {
            return Err(Failure::new(
                Reason::Timeout,
                format!("Stopped waiting after 20s. Process {pid} is still finishing."),
            )
            .then("Check again", ["bridge", "down"]));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = fs::remove_file(pid_file(home));
    if Modes::get().json {
        ui::json_document(json!({"home": home, "outcome": "stopped"}));
    } else {
        ui::done("Apple Bridge stopped", &home.display().to_string());
    }
    Ok(())
}
