use crate::assertions::await_until;
use lab_runner::Evidence;
use lab_world::{Error, Result};
use serde_json::Value;
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const REAL: [&str; 2] = ["PKG-INSTALL", "PKG-BACKGROUND"];

struct Install {
    installed: PathBuf,
    home: PathBuf,
    root: PathBuf,
    report: PathBuf,
    helper: PathBuf,
    domain: String,
    registered: bool,
    foreground: Option<String>,
}

fn run(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<Output> {
    Ok(Command::new(program).args(args).output()?)
}

fn succeeded(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> bool {
    run(program, args).is_ok_and(|output| output.status.success())
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

impl Install {
    fn agent_pid(&self) -> u32 {
        let Ok(output) = run("launchctl", &["print", &self.domain]) else {
            return 0;
        };
        let listing = text(&output);
        if !output.status.success() || !listing.contains("state = running") {
            return 0;
        }
        listing
            .lines()
            .find_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                (fields.len() == 3 && fields[0] == "pid" && fields[1] == "=")
                    .then(|| fields[2].parse().unwrap_or(0))
            })
            .unwrap_or(0)
    }

    fn healthy(&self) -> bool {
        let Some(address) = fs::read(self.home.join("runtime.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .and_then(|config| config["api_address"].as_str().map(str::to_owned))
            .filter(|address| !address.is_empty())
        else {
            return false;
        };
        run(
            "curl",
            &[
                "--silent",
                "--max-time",
                "1",
                "-w",
                "%{http_code}",
                &format!("http://{address}/v1/health"),
            ],
        )
        .is_ok_and(|output| {
            let body = text(&output);
            body.contains("alive") && body.ends_with("200")
        })
    }

    fn cleanup(&mut self, evidence: &mut Evidence) {
        if let Some(pid) = self.foreground.take() {
            let _ = run("kill", &["-TERM", &pid]);
        }
        if self.registered {
            let helper = self.helper.to_string_lossy().into_owned();
            evidence.check(
                "smappservice_unregister",
                succeeded(&helper, &["uninstall"]),
                "",
            );
            let gone = await_until(15, || !succeeded("launchctl", &["print", &self.domain]));
            evidence.check("agent_removed_from_login_session", gone, "");
        }
        let keychain = self.home.join("Apple Bridge.keychain-db");
        if keychain.exists() {
            let removed = succeeded(
                "security",
                &["delete-keychain", &keychain.to_string_lossy()],
            );
            evidence.check("test_keychain_removed", removed, "");
        }
        evidence.check(
            "test_app_removed",
            fs::remove_dir_all(&self.installed).is_ok() || !self.installed.exists(),
            "",
        );
        evidence.check(
            "test_state_removed",
            fs::remove_dir_all(&self.root).is_ok(),
            "",
        );
        let _ = fs::remove_file(&self.report);
    }
}

fn unique_root() -> Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Invalid(e.to_string()))?
        .as_nanos();
    let root =
        PathBuf::from("/tmp/ablab").join(format!("packaging-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    Ok(root)
}

fn steps(install: &mut Install, evidence: &mut Evidence, background: bool) -> Result<()> {
    let build = run(
        "packaging/build-app.sh",
        &[
            &install.installed.to_string_lossy(),
            "--home",
            &install.home.to_string_lossy(),
            "--api-addr",
            "127.0.0.1:0",
            "--peer-addr",
            "127.0.0.1:0",
        ],
    )?;
    evidence.check("build_signed_installable_app", build.status.success(), "");
    if !build.status.success() {
        eprintln!("{}", text(&build));
        return Ok(());
    }
    let signed = succeeded(
        "codesign",
        &[
            "--verify",
            "--deep",
            "--strict",
            &install.installed.to_string_lossy(),
        ],
    );
    evidence.check("bundle_signature", signed, "");
    if !signed {
        return Ok(());
    }
    let status = run(
        "open",
        &[
            "-n",
            "-W",
            "-g",
            "-a",
            &install.installed.to_string_lossy(),
            "--args",
            "status",
            "--report",
            &install.report.to_string_lossy(),
        ],
    );
    let permissions: Option<serde_json::Map<String, Value>> = fs::read(&install.report)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok());
    let onboarded =
        status.is_ok_and(|o| o.status.success()) && permissions.is_some_and(|map| map.len() >= 7);
    evidence.check("onboarding_status_uses_existing_permissions", onboarded, "");
    if !onboarded {
        return Ok(());
    }
    install.registered = true;
    let helper = install.helper.to_string_lossy().into_owned();
    let registered = run(&helper, &["install"]).is_ok_and(|o| o.status.success());
    evidence.check("smappservice_registered", registered, "");
    if !registered {
        return Ok(());
    }
    let running = await_until(40, || install.agent_pid() > 0 && install.healthy());
    evidence.check("launchctl_running_and_gateway_health", running, "");
    if !running {
        return Ok(());
    }
    let private = fs::metadata(install.home.join("keychain-password"))
        .is_ok_and(|meta| meta.permissions().mode() & 0o777 == 0o600);
    evidence.check("dedicated_keychain_password_private", private, "");
    let created = install.home.join("Apple Bridge.keychain-db").exists();
    if !private {
        return Ok(());
    }
    evidence.check("dedicated_keychain_created", created, "");
    if !created || !background {
        return Ok(());
    }
    background_steps(install, evidence, &helper)
}

fn background_steps(install: &mut Install, evidence: &mut Evidence, helper: &str) -> Result<()> {
    let launched = succeeded(
        "open",
        &["-n", "-g", "-a", &install.installed.to_string_lossy()],
    );
    evidence.check("foreground_app_launched", launched, "");
    if !launched {
        return Ok(());
    }
    thread::sleep(Duration::from_secs(1));
    let found = run("pgrep", &["-f", &format!("^{helper}$")])?;
    let listing = text(&found);
    let pids: Vec<&str> = listing.split_whitespace().collect();
    let identified = found.status.success() && pids.len() == 1;
    evidence.check("foreground_app_identified", identified, "");
    if !identified {
        return Ok(());
    }
    let foreground = pids[0].to_owned();
    install.foreground = Some(foreground.clone());
    let quit = succeeded("kill", &["-TERM", &foreground]);
    evidence.check("quit_foreground_app", quit, "");
    if !quit {
        return Ok(());
    }
    let survives = await_until(40, || {
        !succeeded("kill", &["-0", &foreground]) && install.healthy() && install.agent_pid() > 0
    });
    evidence.check("gateway_survives_app_quit", survives, "");
    if !survives {
        return Ok(());
    }
    let previous = install.agent_pid();
    let locked = succeeded(
        "security",
        &[
            "lock-keychain",
            &install
                .home
                .join("Apple Bridge.keychain-db")
                .to_string_lossy(),
        ],
    );
    evidence.check("lock_dedicated_keychain_before_restart", locked, "");
    if !locked {
        return Ok(());
    }
    let killed = succeeded("kill", &["-KILL", &previous.to_string()]);
    evidence.check("kill_gateway", killed, "");
    if !killed {
        return Ok(());
    }
    let restarted = await_until(40, || {
        let pid = install.agent_pid();
        pid > 0 && pid != previous && install.healthy()
    });
    evidence.check(
        "launchd_restarts_gateway_and_unlocks_credentials",
        restarted,
        "",
    );
    Ok(())
}

pub fn real(evidence: &mut Evidence) -> Result<()> {
    let macos = cfg!(target_os = "macos");
    evidence.check("real_macos", macos, "");
    if !macos {
        return Ok(());
    }
    let home = env::var("HOME").map_err(|_| Error::Invalid("HOME".into()))?;
    let installed = Path::new(&home).join("Applications/Apple Bridge.app");
    let unoccupied = fs::symlink_metadata(&installed)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
    evidence.check("test_install_path_unoccupied", unoccupied, "");
    if !unoccupied {
        return Ok(());
    }
    let uid = String::from_utf8_lossy(&run("id", &["-u"])?.stdout)
        .trim()
        .to_owned();
    let domain = format!("gui/{uid}/dev.applebridge.gateway");
    let registered = succeeded("launchctl", &["print", &domain]);
    evidence.check("test_agent_unregistered", !registered, "");
    if registered {
        return Ok(());
    }
    let root = unique_root();
    evidence.check("isolated_home", root.is_ok(), "");
    let root = root?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut install = Install {
        helper: installed.join("Contents/MacOS/AppleBridgeHelper"),
        report: Path::new(&home)
            .join("Library/Application Support/AppleBridgeDev")
            .join(format!("{name}-permissions.json")),
        home: root.join("home"),
        installed,
        root,
        domain,
        registered: false,
        foreground: None,
    };
    let background = evidence.scenario == "PKG-BACKGROUND";
    let result = steps(&mut install, evidence, background);
    install.cleanup(evidence);
    result
}
