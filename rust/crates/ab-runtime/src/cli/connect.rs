use super::{
    Failure, Outcome, commands,
    failure::{Reason, Step},
    gateway,
    modes::Modes,
    tree::TailscaleArgs,
    ui,
};
use std::{
    io::{BufRead, BufReader, Write},
    net::ToSocketAddrs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const TAILSCALE_APP: &str = "/Applications/Tailscale.app";
const TAILSCALE_PACKAGE: &str = "https://pkgs.tailscale.com/stable/Tailscale-latest-macos.pkg";
const TAILSCALE_TEAM: &str = "(W5364U7YZB)";
const CLAUDE_CONNECTORS: &str =
    "https://claude.ai/new?modal=add-custom-connector#customize/connectors";

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

fn copy(text: &str) -> bool {
    let Ok(mut child) = Command::new("/usr/bin/pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    child.wait().is_ok_and(|status| status.success()) && wrote
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
    if !cfg!(target_os = "macos") {
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

async fn turn_on(home: &Path, route: &str, keep_awake: bool) -> Outcome<String> {
    ensure_running(home)?;
    commands::check_remote_files(home)?;
    start_tailscale(home, keep_awake)?;
    let voice = commands::Voice::Modern {
        open: true,
        no_open: false,
    };
    match commands::onboard(home, "on", route, None, voice).await? {
        Ok(_) => {}
        Err(link) => {
            return Err(
                Failure::new(Reason::Timeout, "Waiting for you to finish in the browser.")
                    .row("Sign-in link", link)
                    .then("Then run this again", ["bridge", "tailscale"]),
            );
        }
    }
    let status = ab_remote::state::status(home).map_err(|error| {
        Failure::new(
            Reason::InvalidConfig,
            "Couldn't read the remote access state.",
        )
        .remote("Remote", error.to_string())
    })?;
    let key = if route == "cloud" { "system" } else { route };
    status
        .routes
        .get(key)
        .filter(|found| found.state == "on" && found.url.starts_with("https://"))
        .map(|found| found.url.trim_end_matches('/').to_owned())
        .ok_or_else(|| {
            Failure::new(Reason::ApiError, "Tailscale didn't report an address yet.")
                .then("Check it", ["bridge", "remote", "status"])
        })
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

pub async fn tailscale(home: &Path, args: &TailscaleArgs) -> Outcome {
    if args.off {
        let disable = super::tree::RemoteDisableArgs {
            via: Some("tailnet".into()),
            private: false,
            name: None,
            timeout: None,
        };
        commands::remote_disable(home, &disable, commands::MODERN).await?;
        if args.keep_awake {
            keep_awake(home, false)?;
        }
        return Ok(());
    }
    let url = turn_on(home, "tailnet", args.keep_awake).await?;
    if args.keep_awake {
        keep_awake(home, true)?;
        ui::done("Awake", "with the lid closed while Apple Bridge is on");
    }
    let mcp = format!("{url}/mcp");
    let connect = format!("{url}/connect");
    if Modes::get().json {
        ui::json_document(serde_json::json!({"outcome": "on", "mcp": mcp, "connect": connect}));
        return Ok(());
    }
    if !Modes::get().stdout_tty() {
        ui::outln(&mcp);
    }
    ui::receipt(
        "On",
        &mcp,
        "Your devices on Tailscale can reach Apple Bridge now.",
    );
    ui::blank();
    ui::note(&format!(
        "Cloud agents such as Grok Bot find this Mac on your Tailscale network at {connect}. They ask on this Mac first, and you click Allow."
    ));
    ui::next_steps(&[Step::new("Turn it off", ["bridge", "tailscale", "--off"])]);
    Ok(())
}

pub async fn claude_app(home: &Path) -> Outcome {
    let url = turn_on(home, "cloud", false).await?;
    let mcp = format!("{url}/mcp");
    let armed = home.join(ab_frontdoor::web_connector::ARMED_FILE);
    let until = jiff::Timestamp::now().as_second()
        + i64::try_from(ab_frontdoor::web_connector::ARMED_FOR.as_secs()).unwrap_or(900);
    std::fs::write(&armed, until.to_string())?;
    let copied = copy(&mcp);
    if Modes::get().json {
        ui::json_document(serde_json::json!({"outcome": "ready", "url": mcp, "copied": copied}));
        return Ok(());
    }
    if !Modes::get().stdout_tty() {
        ui::outln(&mcp);
    }
    ui::receipt(
        if copied { "Copied" } else { "Ready" },
        &mcp,
        "Your Claude connector address is ready.",
    );
    ui::row("Address", &mcp);
    ui::blank();
    ui::note("In Claude: Customize > Connectors > Add custom connector.");
    ui::note(if copied {
        "Name it Apple Bridge, paste the address (Command-V) into the URL box, click Add, then Connect."
    } else {
        "Name it Apple Bridge, put the address above in the URL box, click Add, then Connect."
    });
    ui::note("Then click Allow in the Apple Bridge window on this Mac.");
    if Modes::get().browser_allowed() {
        let opened = Path::new("/Applications/Claude.app").exists()
            && quiet("/usr/bin/open", &["-a", "Claude"]);
        if !opened {
            let _ = ab_remote::onboarding::open_link(CLAUDE_CONNECTORS);
        }
    }
    Ok(())
}

fn server_address(server: &str, given: Option<&str>) -> Outcome<std::net::IpAddr> {
    if let Some(ip) = given {
        return ip.parse().map_err(|_| {
            Failure::new(
                Reason::InvalidValue,
                format!("Can't use --ip {}.", ui::clean(ip)),
            )
        });
    }
    let config = Command::new("ssh").args(["-G", server]).output()?;
    let host = String::from_utf8_lossy(&config.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("hostname ").map(str::to_owned))
        .unwrap_or_else(|| server.rsplit('@').next().unwrap_or(server).to_owned());
    (host.as_str(), 22)
        .to_socket_addrs()
        .ok()
        .and_then(|mut found| found.find(std::net::SocketAddr::is_ipv4))
        .map(|address| address.ip())
        .ok_or_else(|| {
            Failure::new(
                Reason::InvalidValue,
                format!("Couldn't find {}'s public address.", ui::clean(server)),
            )
            .then(
                "Name it yourself",
                ["bridge", "relay", "setup", server, "--ip", "<IP>"],
            )
        })
}

fn dig(kind: &str, name: &str) -> String {
    Command::new("/usr/bin/dig")
        .args(["+short", kind, name, "@1.1.1.1"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_lowercase())
        .unwrap_or_default()
}

fn dns_ready(domain: &str, ip: &str) -> bool {
    dig("NS", domain).contains(&format!("ns1.{domain}."))
        && dig("A", &format!("ns1.{domain}")).contains(ip)
}

async fn relay_answers(domain: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    else {
        return false;
    };
    client
        .get(format!("https://{domain}/info"))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

#[allow(clippy::too_many_lines)]
pub async fn relay_setup(home: &Path, args: &super::tree::RelaySetupArgs) -> Outcome {
    let domain = args.domain.trim().trim_end_matches('.').to_lowercase();
    if !commands::valid_domain(&domain) {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --domain {}. Use a name such as bridge.example.com.",
                ui::clean(&args.domain)
            ),
        ));
    }
    ensure_running(home)?;
    commands::check_remote_files(home)?;
    let relay = format!("https://{domain}");
    ui::phase(1, 5, &format!("Checking {}…", ui::clean(&args.server)));
    let ip = server_address(&args.server, args.ip.as_deref())?.to_string();
    if !quiet(
        "ssh",
        &[
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            &args.server,
            "true",
        ],
    ) {
        return Err(Failure::new(
            Reason::ApiError,
            format!("Can't sign in to {} with ssh.", ui::clean(&args.server)),
        )
        .sentence(
            "Check that `ssh <server>` works without a password prompt, then run this again.",
        ));
    }
    ab_remote::state::set_relay(home, &relay).map_err(|error| {
        Failure::new(Reason::InvalidConfig, "Couldn't save the relay address.")
            .remote("Remote", error.to_string())
    })?;
    let key = ab_remote::relay::public_key(home, &relay).map_err(|error| {
        Failure::new(Reason::ApiError, "Couldn't read this Mac's relay key.")
            .remote("Remote", ui::redact_urls(&error.to_string()))
    })?;
    ui::phase(2, 5, "Checking your domain…");
    if !dns_ready(&domain, &ip) {
        ui::blank();
        ui::note("Add these two records where your domain's DNS lives, then leave this running:");
        ui::row(&format!("ns1.{domain}"), &format!("A    {ip}"));
        ui::row(&domain, &format!("NS   ns1.{domain}"));
        ui::note("Also open ports 443 and 53 (TCP and UDP) on the server's firewall.");
        let deadline = Instant::now() + Duration::from_hours(1);
        while !dns_ready(&domain, &ip) {
            if Instant::now() >= deadline {
                return Err(
                    Failure::new(Reason::Timeout, "The DNS records aren't visible yet.").then(
                        "Run this again later",
                        [
                            "bridge",
                            "relay",
                            "setup",
                            &args.server,
                            "--domain",
                            &domain,
                        ],
                    ),
                );
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        ui::done("Found", "your DNS records");
    }
    ui::phase(3, 5, "Installing the relay on your server…");
    let install = format!(
        "set -e; curl -fsSL https://eilmer.si/applebridge/selfhost/install.sh -o /tmp/ab-relay-install.sh; if [ \"$(id -u)\" = 0 ]; then bash /tmp/ab-relay-install.sh --domain {domain} --owner-key {key}; else sudo bash /tmp/ab-relay-install.sh --domain {domain} --owner-key {key}; fi"
    );
    let installed = Command::new("ssh")
        .args(["-t", &args.server, &install])
        .status()
        .is_ok_and(|status| status.success());
    if !installed {
        return Err(Failure::new(Reason::ApiError, "The relay didn't install.")
            .sentence("The lines above say why. Fix it, then run this again; it picks up where it stopped."));
    }
    ui::phase(4, 5, "Waiting for the relay's certificate…");
    let deadline = Instant::now() + Duration::from_mins(15);
    while !relay_answers(&domain).await {
        if Instant::now() >= deadline {
            return Err(
                Failure::new(Reason::Timeout, "The relay isn't answering on HTTPS yet.")
                    .sentence("Let's Encrypt can take a few minutes after DNS changes.")
                    .then(
                        "Run this again later",
                        [
                            "bridge",
                            "relay",
                            "setup",
                            &args.server,
                            "--domain",
                            &domain,
                        ],
                    ),
            );
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    ui::phase(5, 5, "Connecting this Mac…");
    let voice = commands::Voice::Modern {
        open: true,
        no_open: false,
    };
    if let Err(link) = commands::onboard(home, "on", "relay", None, voice).await? {
        return Err(
            Failure::new(Reason::Timeout, "Waiting for you to finish in the browser.")
                .row("Sign-in link", link),
        );
    }
    let status = ab_remote::state::status(home).map_err(|error| {
        Failure::new(
            Reason::InvalidConfig,
            "Couldn't read the remote access state.",
        )
        .remote("Remote", error.to_string())
    })?;
    let mcp = format!("{}/mcp", status.url.trim_end_matches('/'));
    let copied = copy(&mcp);
    ui::receipt(
        "Ready",
        &mcp,
        "Your relay is running and this Mac is connected.",
    );
    ui::row("Address", &mcp);
    if copied {
        ui::note(
            "The address is copied. Add it as a custom connector in Claude or ChatGPT, then click Allow on this Mac.",
        );
    }
    Ok(())
}
