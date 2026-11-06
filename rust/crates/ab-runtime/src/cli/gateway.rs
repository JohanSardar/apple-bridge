use super::{Failure, Outcome, stats, tree::UpArgs};
use crate::{Config, bundle};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Seek, SeekFrom},
    net::{SocketAddr, TcpListener},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
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
const FIRST_SYNC_NOTE: &str = "Ctrl-C stops watching. Syncing continues in the background.";

fn lock_held(home: &Path) -> bool {
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
    let address = Config::load(home)
        .map(|config| config.api_address)
        .map_err(|_| Failure::not_running(home))?;
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
        Failure::new(format!("Could not create {}: {error}.", home.display()))
            .next("Choose a folder you can write to with --home.")
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

fn loopback(address: &str) -> Outcome<SocketAddr> {
    address
        .parse::<SocketAddr>()
        .ok()
        .filter(|parsed| parsed.ip().is_loopback())
        .ok_or_else(|| {
            Failure::new(format!("`{address}` is not a local address."))
                .next("Local use only needs 127.0.0.1:PORT. Remote use is `bridge remote on`.")
        })
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
        Err(error) if error.kind() == ErrorKind::AddrInUse => Err(Failure::new(format!(
            "{chosen} is already in use by another program."
        ))
        .next("Choose a free one with --api-addr 127.0.0.1:PORT.")),
        Err(error) => Err(Failure::new(format!("Could not use {chosen}: {error}."))),
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
    let executable = std::env::current_exe()?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log_file(launch.home))?;
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
    let child = command.spawn()?;
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
            return Err(Failure::new(format!(
                "The gateway stopped right after starting ({status}). Last log lines:\n{tail}"
            ))
            .next(format!("Full log: {}", log_file(home).display())));
        }
        if healthy(address).await {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                Failure::new("The gateway is taking too long to start.").next(format!(
                    "Check {} and run `bridge up` again.",
                    log_file(home).display()
                )),
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn start_gateway(home: &Path, args: &UpArgs, previous: Option<&Config>) -> Outcome<String> {
    prepare_home(home)?;
    ab_trust::Authority::open(home).map_err(crate::Error::from)?;
    let api = api_address(args, previous)?;
    let peer = peer_address(args, previous);
    let helper = helper_app(args, previous);
    let mut child = spawn_gateway(&Launch {
        home,
        api: &api,
        peer: &peer,
        helper: &helper,
    })?;
    wait_until_healthy(home, &api, &mut child).await?;
    Ok(Config::load(home)?.api_address)
}

fn remote_state(home: &Path) -> String {
    ab_remote::state::read::<serde_json::Value>(home, "status")
        .ok()
        .and_then(|status| status["state"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "off".to_owned())
}

fn print_summary(home: &Path, address: &str, started: bool, helper: bool, elapsed: Duration) {
    let verb = if started { "is on" } else { "was already on" };
    println!(
        "Apple Bridge {verb} ({:.1} s). No account, server or domain needed.",
        elapsed.as_secs_f32()
    );
    println!("  For agents on this Mac   {}", mcp_address(address));
    println!("  Data folder              {}", home.display());
    let remote = remote_state(home);
    if remote == "off" {
        println!("  Remote access            off (optional: bridge remote on)");
    } else {
        println!("  Remote access            {remote}");
    }
    if !helper {
        println!("  Mac data                 no Helper app found, so nothing new will sync");
        println!("                           Install Apple Bridge, or pass --helper-app.");
    }
}

fn print_next(home_is_default: bool, home: &Path) {
    let suffix = if home_is_default {
        String::new()
    } else {
        format!(" --home {}", shell_quote(&home.to_string_lossy()))
    };
    println!();
    println!("Next:");
    println!("  bridge link claude{suffix}        connect Claude Code (or: bridge link codex)");
    println!("  bridge permissions guide{suffix}  allow the Mac permissions you want");
    println!("  bridge stats --watch{suffix}      follow the first sync");
}

pub fn shell_quote(text: &str) -> String {
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-:".contains(c))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

pub async fn up(home: &Path, args: &UpArgs) -> Outcome {
    let began = Instant::now();
    let previous = Config::load(home).ok();
    if previous
        .as_ref()
        .is_some_and(|config| config.role != "gateway")
    {
        return Err(Failure::new(format!(
            "{} is a replica of another gateway.",
            home.display()
        ))
        .next("Run it with `bridge start --role replica`, or choose another --home."));
    }
    let first_run = !home.join("state.db").exists();
    let running = lock_held(home);
    let address = if running {
        running_address(home).await.map_err(|_| {
            Failure::new("A gateway holds this home but is not answering yet.")
                .next("Wait a moment and run `bridge up` again.")
        })?
    } else {
        start_gateway(home, args, previous.as_ref()).await?
    };
    let helper = !Config::load(home)?.helper_app.is_empty();
    let older = running
        .then(|| crate::gateway_lifetime::older(home))
        .flatten();
    let elapsed = began.elapsed();
    if args.json {
        println!(
            "{}",
            json!({
                "state": "on",
                "started": !running,
                "mcp": mcp_address(&address),
                "home": home,
                "helper": helper,
                "remote": remote_state(home),
                "seconds": elapsed.as_secs_f64(),
                "restart_needed": older.is_some(),
            })
        );
        return Ok(());
    }
    print_summary(home, &address, !running, helper, elapsed);
    if let Some(version) = older {
        println!(
            "Warning: the running gateway ({version}) differs from this installed executable."
        );
        println!(
            "  Run bridge down, then bridge up to finish the update. If the app started it, restart the app instead."
        );
    } else if running && !home.join("run/gateway-build.json").exists() {
        println!(
            "Warning: this gateway predates update detection. If you updated, run bridge down, then bridge up."
        );
    }
    println!();
    stats::print_once(home)?;
    print_next(
        crate::default_home().is_ok_and(|default| default == home),
        home,
    );
    if first_run && helper && !args.no_watch && super::interactive() {
        println!();
        println!("{FIRST_SYNC_NOTE}");
        stats::watch_first_sync(home).await?;
    }
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
        println!("Apple Bridge is not running from {}.", home.display());
        let _ = fs::remove_file(pid_file(home));
        return Ok(());
    }
    let foreign = || {
        Failure::new("This gateway was not started by `bridge up`, so it was left running.")
            .next("Stop it the way it was started, such as from the Apple Bridge app.")
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
        return Err(Failure::new(format!("Could not stop process {pid}.")));
    }
    let deadline = Instant::now() + STOP_BUDGET;
    while lock_held(home) {
        if Instant::now() >= deadline {
            return Err(Failure::new("The gateway did not stop in time.")
                .next(format!("It may still be finishing. Process {pid}.")));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = fs::remove_file(pid_file(home));
    println!("Apple Bridge stopped.");
    Ok(())
}
