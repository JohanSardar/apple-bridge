use super::{
    Failure, Outcome, commands, gateway,
    failure::{Reason, Step},
    modes::Modes,
    tree::TailscaleArgs,
    ui,
};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const TAILSCALE_APP: &str = "/Applications/Tailscale.app";
const TAILSCALE_PACKAGE: &str = "https://pkgs.tailscale.com/stable/Tailscale-latest-macos.pkg";
const TAILSCALE_TEAM: &str = "(W5364U7YZB)";
const INSTALL_WAIT: Duration = Duration::from_secs(900);
const CLAUDE_CONNECTORS: &str = "https://claude.ai/new?modal=add-custom-connector#customize/connectors";

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

fn install_tailscale() -> Outcome {
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
    ui::phase(2, 3, "Opening the Tailscale installer. Follow it to the end…");
    if !quiet("/usr/bin/open", &["-W", &package.to_string_lossy()]) {
        return Err(Failure::new(
            Reason::SpawnFailed,
            "Couldn't open the Tailscale installer.",
        ));
    }
    let deadline = Instant::now() + INSTALL_WAIT;
    while !Path::new(TAILSCALE_APP).exists() {
        if Instant::now() >= deadline {
            return Err(Failure::new(
                Reason::Timeout,
                "Tailscale isn't installed yet.",
            )
            .then("Run this again when it is", ["bridge", "tailscale"]));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn start_tailscale() -> Outcome {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    if !Path::new(TAILSCALE_APP).exists() {
        if !Modes::get().interactive() && !Modes::get().yes {
            return Err(Failure::new(
                Reason::NotFound,
                "Tailscale isn't installed on this Mac.",
            )
            .sentence("Installing it needs a person to click through the installer.")
            .then("Install it and turn it on", ["bridge", "tailscale"]));
        }
        install_tailscale()?;
    }
    if !running("Tailscale") {
        ui::note("Starting Tailscale…");
        if !quiet("/usr/bin/open", &["-g", "-a", TAILSCALE_APP]) {
            return Err(Failure::new(Reason::SpawnFailed, "Couldn't start Tailscale."));
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while !running("Tailscale") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(300));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    Ok(())
}

fn ensure_running(home: &Path) -> Outcome {
    if gateway::lock_held(home) {
        return Ok(());
    }
    ui::note("Starting Apple Bridge…");
    let started = std::env::current_exe().ok().is_some_and(|bridge| {
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

async fn turn_on(home: &Path, route: &str) -> Outcome<String> {
    ensure_running(home)?;
    commands::check_remote_files(home)?;
    start_tailscale()?;
    let voice = commands::Voice::Modern {
        open: true,
        no_open: false,
    };
    match commands::onboard(home, "on", route, None, voice).await? {
        Ok(_) => {}
        Err(link) => {
            return Err(Failure::new(
                Reason::Timeout,
                "Waiting for you to finish in the browser.",
            )
            .row("Sign-in link", link)
            .then("Then run this again", ["bridge", "tailscale"]));
        }
    }
    let status = ab_remote::state::status(home).map_err(|error| {
        Failure::new(Reason::InvalidConfig, "Couldn't read the remote access state.")
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

pub async fn tailscale(home: &Path, args: &TailscaleArgs) -> Outcome {
    if args.off {
        let disable = super::tree::RemoteDisableArgs {
            via: Some("tailnet".into()),
            private: false,
            name: None,
            timeout: None,
        };
        return commands::remote_disable(home, &disable, commands::MODERN).await;
    }
    let url = turn_on(home, "tailnet").await?;
    let mcp = format!("{url}/mcp");
    let connect = format!("{url}/connect");
    if Modes::get().json {
        ui::json_document(serde_json::json!({"outcome": "on", "mcp": mcp, "connect": connect}));
        return Ok(());
    }
    if !Modes::get().stdout_tty() {
        ui::outln(&mcp);
    }
    let copied = copy(&connect);
    ui::receipt(
        "On",
        &mcp,
        "Your devices on Tailscale can reach Apple Bridge now.",
    );
    ui::row("Address", &mcp);
    ui::blank();
    ui::note(&format!(
        "Cloud agents such as Grok Bot: tell them \"connect to Apple Bridge at {connect}\".{}",
        if copied { " It's copied." } else { "" }
    ));
    ui::note("They ask on this Mac first, and you click Allow. Only devices on your Tailscale network can reach it.");
    ui::next_steps(&[Step::new("Turn it off", ["bridge", "tailscale", "--off"])]);
    Ok(())
}

pub async fn claude_app(home: &Path) -> Outcome {
    let url = turn_on(home, "cloud").await?;
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
            Failure::new(Reason::InvalidValue, format!("Can't use --ip {}.", ui::clean(ip)))
        });
    }
    let config = Command::new("ssh").args(["-G", server]).output()?;
    let host = String::from_utf8_lossy(&config.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("hostname ").map(str::to_owned))
        .unwrap_or_else(|| server.rsplit('@').next().unwrap_or(server).to_owned());
    use std::net::ToSocketAddrs;
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
    dig("NS", domain).contains(&format!("ns1.{domain}.")) && dig("A", &format!("ns1.{domain}")).contains(ip)
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

pub async fn relay_setup(home: &Path, args: &super::tree::RelaySetupArgs) -> Outcome {
    let domain = args.domain.trim().trim_end_matches('.').to_lowercase();
    if !commands::valid_domain(&domain) {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!("Can't use --domain {}. Use a name such as bridge.example.com.", ui::clean(&args.domain)),
        ));
    }
    ensure_running(home)?;
    commands::check_remote_files(home)?;
    let relay = format!("https://{domain}");
    ui::phase(1, 5, &format!("Checking {}…", ui::clean(&args.server)));
    let ip = server_address(&args.server, args.ip.as_deref())?.to_string();
    if !quiet("ssh", &["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", &args.server, "true"]) {
        return Err(Failure::new(
            Reason::ApiError,
            format!("Can't sign in to {} with ssh.", ui::clean(&args.server)),
        )
        .sentence("Check that `ssh <server>` works without a password prompt, then run this again."));
    }
    ab_remote::state::set_relay(home, &relay).map_err(|error| {
        Failure::new(Reason::InvalidConfig, "Couldn't save the relay address.").remote("Remote", error.to_string())
    })?;
    let key = ab_remote::relay::public_key(home, &relay).map_err(|error| {
        Failure::new(Reason::ApiError, "Couldn't read this Mac's relay key.").remote("Remote", ui::redact_urls(&error.to_string()))
    })?;
    ui::phase(2, 5, "Checking your domain…");
    if !dns_ready(&domain, &ip) {
        ui::blank();
        ui::note("Add these two records where your domain's DNS lives, then leave this running:");
        ui::row(&format!("ns1.{domain}"), &format!("A    {ip}"));
        ui::row(&domain, &format!("NS   ns1.{domain}"));
        ui::note("Also open ports 443 and 53 (TCP and UDP) on the server's firewall.");
        let deadline = Instant::now() + Duration::from_secs(3600);
        while !dns_ready(&domain, &ip) {
            if Instant::now() >= deadline {
                return Err(Failure::new(Reason::Timeout, "The DNS records aren't visible yet.")
                    .then("Run this again later", ["bridge", "relay", "setup", &args.server, "--domain", &domain]));
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        ui::done("Found", "your DNS records");
    }
    ui::phase(3, 5, "Installing the relay on your server…");
    let install = format!(
        "set -e; curl -fsSL https://raw.githubusercontent.com/JohanSardar/apple-bridge/main/deploy/selfhost/install.sh -o /tmp/ab-relay-install.sh; if [ \"$(id -u)\" = 0 ]; then bash /tmp/ab-relay-install.sh --domain {domain} --owner-key {key}; else sudo bash /tmp/ab-relay-install.sh --domain {domain} --owner-key {key}; fi"
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
    let deadline = Instant::now() + Duration::from_secs(900);
    while !relay_answers(&domain).await {
        if Instant::now() >= deadline {
            return Err(Failure::new(Reason::Timeout, "The relay isn't answering on HTTPS yet.")
                .sentence("Let's Encrypt can take a few minutes after DNS changes.")
                .then("Run this again later", ["bridge", "relay", "setup", &args.server, "--domain", &domain]));
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    ui::phase(5, 5, "Connecting this Mac…");
    let voice = commands::Voice::Modern { open: true, no_open: false };
    if let Err(link) = commands::onboard(home, "on", "relay", None, voice).await? {
        return Err(Failure::new(Reason::Timeout, "Waiting for you to finish in the browser.").row("Sign-in link", link));
    }
    let status = ab_remote::state::status(home).map_err(|error| {
        Failure::new(Reason::InvalidConfig, "Couldn't read the remote access state.").remote("Remote", error.to_string())
    })?;
    let mcp = format!("{}/mcp", status.url.trim_end_matches('/'));
    let copied = copy(&mcp);
    ui::receipt("Ready", &mcp, "Your relay is running and this Mac is connected.");
    ui::row("Address", &mcp);
    if copied {
        ui::note("The address is copied. Add it as a custom connector in Claude or ChatGPT, then click Allow on this Mac.");
    }
    Ok(())
}
