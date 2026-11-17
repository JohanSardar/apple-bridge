use super::{
    Failure, Outcome, commands,
    failure::{Reason, Step},
    gateway,
    modes::Modes,
    tree::TailscaleArgs,
    ui,
};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const TAILSCALE_APP: &str = "/Applications/Tailscale.app";
const TAILSCALE_PACKAGE: &str = "https://pkgs.tailscale.com/stable/Tailscale-latest-macos.pkg";
const TAILSCALE_TEAM: &str = "(W5364U7YZB)";

fn quiet(program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn running(process: &str) -> bool {
    quiet("/usr/bin/pgrep", &["-x", process])
}

fn install_tailscale(home: &Path, keep_awake: bool) -> Outcome {
    let package = std::env::temp_dir().join(format!("Tailscale-{}.pkg", std::process::id()));
    let _ = std::fs::remove_file(&package);
    ui::phase(1, 3, "Downloading Tailscale from tailscale.com…");
    if !quiet(
        "/usr/bin/curl",
        &["-fsSL", "-o", &package.to_string_lossy(), TAILSCALE_PACKAGE],
    ) {
        return Err(Failure::new(
            Reason::ApiError,
            "Couldn't download Tailscale. Check your internet connection.",
        )
        .sentence("Or install it yourself from tailscale.com/download, then run this again."));
    }
    let checked = Command::new("/usr/sbin/pkgutil")
        .args(["--check-signature", &package.to_string_lossy()])
        .output()?;
    let signer = String::from_utf8_lossy(&checked.stdout);
    if !checked.status.success()
        || !signer.contains("Developer ID Installer: Tailscale Inc.")
        || !signer.contains(TAILSCALE_TEAM)
    {
        return Err(Failure::new(
            Reason::PermissionDenied,
            "The download wasn't signed by Tailscale, so it wasn't opened.",
        )
        .sentence("Install Tailscale from tailscale.com/download instead."));
    }
    ui::phase(
        2,
        3,
        "Installing Tailscale. macOS asks for your password or Touch ID…",
    );
    let mut arguments = vec![
        "--tailscale-package".to_owned(),
        package.to_string_lossy().into_owned(),
    ];
    if keep_awake {
        arguments.push("--keep-awake".to_owned());
    }
    privileged(home, &arguments);
    let _ = std::fs::remove_file(&package);
    if Path::new(TAILSCALE_APP).exists() {
        return Ok(());
    }
    Err(
        Failure::new(Reason::PermissionDenied, "Tailscale isn't installed.")
            .sentence("macOS didn't get your password or Touch ID.")
            .then("Try again", ["bridge", "tailscale"]),
    )
}

const TAILSCALE_CLI: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";

fn tailscale_state() -> String {
    Command::new(TAILSCALE_CLI)
        .args(["status", "--json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
        .and_then(|status| status["BackendState"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn sign_in_to_tailscale() -> Outcome {
    let state = tailscale_state();
    if state == "Running" || state.is_empty() {
        return Ok(());
    }
    if state != "NeedsLogin" {
        let _ = quiet(TAILSCALE_CLI, &["up"]);
        if tailscale_state() == "Running" {
            return Ok(());
        }
    }
    let mut login = Command::new(TAILSCALE_CLI)
        .arg("login")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (sender, receiver) = std::sync::mpsc::channel();
    for stream in [
        login
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        login
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let sender = sender.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if let Some(link) = line
                    .split_whitespace()
                    .find(|word| word.starts_with("https://login.tailscale.com/"))
                {
                    let _ = sender.send(link.to_owned());
                }
            }
        });
    }
    if let Ok(link) = receiver.recv_timeout(Duration::from_secs(20)) {
        ui::note(
            "Sign in to Tailscale in your browser. It's free, and you can use Google, GitHub, Apple or Microsoft.",
        );
        ui::row("Sign-in link", &link);
        let over_ssh =
            std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
        if !over_ssh {
            let _ = ab_remote::onboarding::open_link(&link);
        }
    }
    let deadline = Instant::now() + Duration::from_mins(10);
    while tailscale_state() != "Running" {
        if Instant::now() >= deadline {
            let _ = login.kill();
            return Err(
                Failure::new(Reason::Timeout, "Tailscale isn't signed in yet.")
                    .then("Run this again when it is", ["bridge", "tailscale"]),
            );
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let _ = login.wait();
    ui::done("Signed in", "to Tailscale");
    Ok(())
}

fn start_tailscale(home: &Path, keep_awake: bool) -> Outcome {
    if !cfg!(target_os = "macos") || std::env::var_os("APPLE_BRIDGE_TAILSCALE_SOCKET").is_some() {
        return Ok(());
    }
    if !Path::new(TAILSCALE_APP).exists() {
        if !Modes::get().interactive() && !Modes::get().yes {
            return Err(
                Failure::new(Reason::NotFound, "Tailscale isn't installed on this Mac.")
                    .sentence("Installing it asks for the owner's password or Touch ID.")
                    .then("Install it and turn it on", ["bridge", "tailscale"]),
            );
        }
        install_tailscale(home, keep_awake)?;
    }
    if !running("Tailscale") {
        ui::note("Starting Tailscale…");
        if !quiet("/usr/bin/open", &["-g", "-a", TAILSCALE_APP]) {
            return Err(Failure::new(
                Reason::SpawnFailed,
                "Couldn't start Tailscale.",
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while !running("Tailscale") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(300));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    sign_in_to_tailscale()
}

fn ensure_running(home: &Path) -> Outcome {
    if gateway::lock_held(home) {
        return Ok(());
    }
    ui::note("Starting Apple Bridge…");
    let started = std::env::current_exe().is_ok_and(|bridge| {
        Command::new(bridge)
            .arg("up")
            .arg("--home")
            .arg(home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    });
    if started && gateway::lock_held(home) {
        Ok(())
    } else {
        Err(Failure::not_running(home))
    }
}

pub(super) async fn turn_on(home: &Path, keep_awake: bool) -> Outcome<String> {
    ensure_running(home)?;
    commands::check_remote_files(home)?;
    start_tailscale(home, keep_awake)?;
    if let Err(link) = commands::onboard(home, "on", None, true).await? {
        return Err(
            Failure::new(Reason::Timeout, "Waiting for you to finish in the browser.")
                .row("Sign-in link", link)
                .then("Then run this again", ["bridge", "tailscale"]),
        );
    }
    let status = ab_remote::state::status(home).map_err(|error| {
        Failure::new(Reason::InvalidConfig, "Couldn't read the Tailscale state.")
            .remote("Tailscale", error.to_string())
    })?;
    status
        .routes
        .get(ab_remote::state::ROUTE)
        .filter(|found| found.state == "on" && found.url.starts_with("https://"))
        .map(|found| found.url.trim_end_matches('/').to_owned())
        .ok_or_else(|| {
            Failure::new(Reason::ApiError, "Tailscale didn't report an address yet.")
                .then("Check it", ["bridge", "status"])
        })
}

async fn turn_off(home: &Path) -> Outcome {
    commands::check_remote_files(home)?;
    if ab_remote::state::status(home)?.state == "off" {
        if Modes::get().json {
            ui::json_document(serde_json::json!({"outcome": "unchanged"}));
        } else {
            ui::receipt(
                "Already off",
                "Tailscale",
                "Tailscale access is already off.",
            );
        }
        return Ok(());
    }
    if let Err(link) = commands::onboard(home, "off", None, false).await? {
        return Err(
            Failure::new(Reason::Timeout, "Tailscale access isn't off yet.")
                .row("Sign-in link", link)
                .then("Check it", ["bridge", "status"]),
        );
    }
    if Modes::get().json {
        ui::json_document(serde_json::json!({"outcome": "off"}));
    } else {
        ui::receipt("Off", "Tailscale", "Tailscale access is off.");
    }
    Ok(())
}

fn sleep_disabled() -> bool {
    Command::new("/usr/bin/pmset")
        .arg("-g")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout).lines().any(|line| {
                let mut words = line.split_whitespace();
                words.next() == Some("SleepDisabled") && words.next() == Some("1")
            })
        })
}

fn privileged(home: &Path, arguments: &[String]) {
    let saved = crate::Config::load(home)
        .map(|config| config.helper_app)
        .unwrap_or_default();
    let helper = crate::bundle::helper_app(&saved, std::env::current_exe().ok().as_deref());
    let _ = Command::new("/usr/bin/open")
        .args(["-n", "-W", "-a", &helper, "--args", "privileged"])
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn keep_awake(home: &Path, on: bool) -> Outcome {
    if !cfg!(target_os = "macos") || sleep_disabled() == on {
        return Ok(());
    }
    privileged(
        home,
        &[if on { "--keep-awake" } else { "--let-sleep" }.to_owned()],
    );
    if sleep_disabled() == on {
        return Ok(());
    }
    let failure = Failure::new(
        Reason::PermissionDenied,
        if on {
            "The Mac can still sleep with the lid closed."
        } else {
            "The Mac still stays awake with the lid closed."
        },
    )
    .sentence("macOS didn't get your password or Touch ID.");
    Err(if on {
        failure.then("Try again", ["bridge", "tailscale", "--keep-awake"])
    } else {
        failure.then(
            "Try again",
            ["bridge", "tailscale", "--off", "--keep-awake"],
        )
    })
}

fn route_port(url: &str) -> u16 {
    url.trim_start_matches("https://")
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .unwrap_or(443)
}

fn ask_sentence(connect: &str) -> String {
    format!("Connect to Apple Bridge at {connect} and use it from now on.")
}

fn ssh_command(home: &Path, url: &str) -> Option<String> {
    let listening = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], ab_remote::reach::SSH_PORT)),
        Duration::from_millis(300),
    )
    .is_ok();
    let user = std::env::var("USER").ok().filter(|user| !user.is_empty())?;
    let host = url
        .trim_start_matches("https://")
        .split([':', '/'])
        .next()?;
    let launch = super::link::Launch::for_home(home);
    let mut remote = vec![launch.command];
    remote.extend(launch.args);
    listening.then(|| {
        ui::shell_line(&[
            "ssh".to_owned(),
            "-T".to_owned(),
            format!("{user}@{host}"),
            ui::shell_line(&remote),
        ])
    })
}

fn names(devices: &[&ab_remote::reach::Device]) -> String {
    let mut shown: Vec<String> = devices
        .iter()
        .take(8)
        .map(|device| {
            if device.online {
                device.name.clone()
            } else {
                format!("{} (offline)", device.name)
            }
        })
        .collect();
    if devices.len() > 8 {
        shown.push(format!("and {} more", devices.len() - 8));
    }
    shown.join(", ")
}

fn explain_reach(reach: &ab_remote::reach::Reach, ssh: Option<&str>) {
    let Some(rule) = reach.rule_text() else {
        return;
    };
    ui::blank();
    let blocked: Vec<_> = reach.blocked.iter().collect();
    let allowed: Vec<_> = reach.allowed.iter().collect();
    ui::warn(
        "blocked_by_tailscale_rules",
        &format!(
            "Your Tailscale access rules don't let {} reach this Mac on port {}. A connection from there waits and then fails without an answer.",
            names(&blocked),
            reach.port
        ),
    );
    if !allowed.is_empty() {
        ui::note(&format!("Allowed now: {}.", names(&allowed)));
    }
    ui::note(&format!(
        "To let them in, add this rule to \"acls\" in your Tailscale policy file at {}, then run bridge tailscale again to check:",
        ab_remote::reach::POLICY_FILE
    ));
    ui::err_line(&format!("    {rule}"));
    let over_ssh: Vec<_> = reach.blocked.iter().filter(|device| device.ssh).collect();
    if let (Some(ssh), false) = (ssh, over_ssh.is_empty()) {
        ui::note(&format!(
            "{} can already reach this Mac over SSH. An agent there can start Apple Bridge over SSH instead, with this as its MCP command:",
            names(&over_ssh)
        ));
        ui::err_line(&format!("    {ssh}"));
    }
}

pub async fn tailscale(home: &Path, args: &TailscaleArgs) -> Outcome {
    if args.off {
        turn_off(home).await?;
        if args.keep_awake {
            keep_awake(home, false)?;
        }
        return Ok(());
    }
    let url = turn_on(home, args.keep_awake).await?;
    if args.keep_awake {
        keep_awake(home, true)?;
        ui::done("Awake", "with the lid closed while Apple Bridge is on");
    }
    let mcp = format!("{url}/mcp");
    let connect = format!("{url}/connect");
    let reach = ab_remote::reach::reach(&ab_remote::localapi::LocalApi::Discover, route_port(&url))
        .await
        .ok();
    let ssh = ssh_command(home, &url);
    if Modes::get().json {
        ui::json_document(serde_json::json!({
            "outcome": "on",
            "mcp": mcp,
            "connect": connect,
            "ask": ask_sentence(&connect),
            "reach": reach.as_ref().map(|reach| serde_json::json!({
                "port": reach.port,
                "allowed": reach.allowed,
                "blocked": reach.blocked,
                "rule": reach.rule(),
                "policy": ab_remote::reach::POLICY_FILE,
            })),
            "ssh": ssh,
        }));
        return Ok(());
    }
    if !Modes::get().stdout_tty() {
        ui::outln(&connect);
    }
    let blocked = reach
        .as_ref()
        .is_some_and(|reach| !reach.blocked.is_empty());
    ui::receipt(
        "On",
        &connect,
        if blocked {
            "Apple Bridge is on your Tailscale network now, for the devices your Tailscale rules let in."
        } else {
            "Your devices on Tailscale can reach Apple Bridge now."
        },
    );
    ui::blank();
    ui::note("To connect an agent on another computer, a server or Grok Bot, tell it:");
    ui::err_line(&format!("    {}", ask_sentence(&connect)));
    ui::note(
        "It asks this Mac, you click Allow once, and it gets its own key. Nothing on its side needs a browser.",
    );
    if let Some(reach) = &reach {
        explain_reach(reach, ssh.as_deref());
    }
    ui::next_steps(&[Step::new("Turn it off", ["bridge", "tailscale", "--off"])]);
    Ok(())
}
