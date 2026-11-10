use super::{
    Failure, Outcome, access,
    failure::{Reason, Step},
    gateway,
    modes::Modes,
    permissions,
    tree::{
        BackupArgs, ConnectArgs, JoinArgs, PairArgs, RemoteDisableArgs, RemoteEnableArgs,
        RemoteKeyArgs, RemoteStatusArgs, RestoreArgs, SelftestArgs, StartArgs,
    },
    ui,
};
use crate::{Config, Error, product_cli};
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const REMOTE_WAIT: Duration = Duration::from_secs(600);

pub fn duration(option: &str, text: &str) -> Outcome<Duration> {
    let invalid = || {
        Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use {option} {}. Durations look like 30s, 5m or 2h.",
                ui::clean(text)
            ),
        )
    };
    let parsed = if let Ok(seconds) = text.parse::<u64>() {
        Duration::from_secs(seconds)
    } else {
        let nanos = crate::duration::parse(text).map_err(|_| invalid())?;
        Duration::from_nanos(u64::try_from(nanos).map_err(|_| invalid())?)
    };
    if parsed.is_zero() {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use {option} {}. It must be longer than zero.",
                ui::clean(text)
            ),
        ));
    }
    Ok(parsed)
}

pub fn friendly(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 90 {
        format!("{seconds}s")
    } else if seconds < 5400 {
        format!("{}m", seconds.div_ceil(60))
    } else {
        format!("{}h", seconds / 3600)
    }
}

pub fn address(option: &str, text: &str) -> Outcome<String> {
    let valid = text.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty()
            && port.parse::<u16>().is_ok()
            && host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c))
    });
    if valid {
        Ok(text.to_owned())
    } else {
        Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use {option} {}. Addresses look like 127.0.0.1:7780.",
                ui::clean(text)
            ),
        ))
    }
}

pub fn load_config(home: &Path) -> Outcome<Option<Config>> {
    let path = home.join("runtime.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Failure::from(error)),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        let location = if error.line() > 0 {
            format!("{}:{}:{}", path.display(), error.line(), error.column())
        } else {
            path.display().to_string()
        };
        let problem = match error.classify() {
            serde_json::error::Category::Data => "doesn't match the settings format",
            serde_json::error::Category::Eof => "ends early",
            _ => "isn't valid JSON",
        };
        Failure::new(Reason::InvalidConfig, format!("{location} {problem}."))
            .row("Detail", ui::clean(&error.to_string()))
            .sentence("The file was left as it is. Fix it, or move it away and run bridge up to write new settings.")
    })
}

pub async fn permission_request(home: &Path, name: &str, helper_app: Option<&str>) -> Outcome {
    if !product_cli::PERMISSIONS.contains(&name) {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't request {}. It isn't a permission name.",
                ui::clean(name)
            ),
        )
        .row("Accepted", product_cli::PERMISSIONS.join(", "))
        .sentence("For Full Disk Access, run bridge permissions guide."));
    }
    let report = product_cli::permissions_request(
        Path::new("open"),
        &product_cli::default_report_dir(),
        &crate::bundle::helper_app(
            helper_app.unwrap_or_default(),
            std::env::current_exe().ok().as_deref(),
        ),
        name,
        None,
    )
    .map_err(|error| helper_failure(&error, name))?;
    let outcome = report.get("outcome").cloned().unwrap_or_default();
    if Modes::get().json {
        ui::json_document(
            json!({"permission": name, "outcome": if outcome.is_empty() { "requested".to_owned() } else { outcome.clone() }}),
        );
    }
    if outcome == "denied" {
        ui::warn("permission_denied", &permissions::declined(name));
        return Ok(());
    }
    ui::done("Requested", name);
    permissions::after_request(home, name, helper_app).await
}

fn helper_failure(error: &Error, name: &str) -> Failure {
    let text = error.to_string();
    if text.contains("timed out") || text.contains("nobody answered") {
        return Failure::new(
            Reason::Timeout,
            format!(
                "Can't confirm the {name} permission. Nobody answered the macOS prompt in time."
            ),
        )
        .then("Ask again", ["bridge", "permissions", "request", name]);
    }
    Failure::new(
        Reason::HelperFailed,
        format!("Failed to ask macOS for {name}. The Helper app didn't report back."),
    )
    .remote("Helper", text)
    .sentence("Check that the Apple Bridge app is installed, or pass --helper-app.")
}

pub fn selftest(args: &SelftestArgs) -> Outcome {
    let mut app = args.helper_app.clone().unwrap_or_default();
    if app.is_empty()
        && let Some(bundle) = product_cli::detect_helper_app()
    {
        app = bundle.to_string_lossy().into_owned();
    }
    ui::note("Running the Messages self-test. It sends real messages to your own chat.");
    let result = product_cli::messages_selftest(
        Path::new("open"),
        &product_cli::default_report_dir(),
        &app,
        args.force,
        !args.no_ui,
        None,
    )
    .map_err(|error| {
        Failure::new(
            Reason::HelperFailed,
            "Failed to run the Messages self-test.",
        )
        .remote("Helper", error.to_string())
        .sentence("Check that the Apple Bridge app is installed, or pass --helper-app.")
    })?;
    ui::outln(result.data.trim_end());
    if !result.proven() {
        return Err(Failure::new(
            Reason::Unproven,
            format!(
                "Messages self-test ended {}. Failed operations stay unproven.",
                ui::clean(&result.state)
            ),
        )
        .then(
            "Run it again",
            ["bridge", "selftest", "messages", "--force"],
        ));
    }
    ui::done("Proven", "Messages");
    Ok(())
}

fn inspection_failure(home: &Path, error: &Error) -> Failure {
    if home.join("state.db").exists() {
        Failure::new(
            Reason::UnhealthyHome,
            format!("Couldn't read {}. Its data isn't readable.", home.display()),
        )
        .row("Detail", ui::clean(&error.to_string()))
        .then("Check it", ["bridge", "doctor"])
    } else {
        Failure::not_set_up(home)
    }
}

pub fn status(home: &Path) -> Outcome {
    let config = load_config(home)?;
    if config.is_none() {
        if home.join("state.db").exists() {
            return Err(Failure::new(
                Reason::NotSetUp,
                format!(
                    "Can't show status. {} has data but no gateway settings yet.",
                    home.display()
                ),
            )
            .sentence("This happens after restoring into a new home.")
            .then("Start it to write its settings", ["bridge", "up"]));
        }
        return Err(Failure::not_set_up(home));
    }
    let report =
        crate::inspection::status(home).map_err(|error| inspection_failure(home, &error))?;
    let modes = Modes::get();
    if modes.json || modes.legacy_json() {
        if !modes.json {
            legacy("bridge status");
        }
        if modes.json {
            ui::json_document(report);
        } else {
            ui::exact_json(&report);
        }
        return Ok(());
    }
    let cursor = &report["cursor"];
    let rows = [
        (
            "Role",
            report["role"].as_str().unwrap_or_default().to_owned(),
        ),
        ("Home", home.display().to_string()),
        (
            "Epoch",
            cursor["epoch"].as_str().unwrap_or_default().to_owned(),
        ),
        ("Sequence", cursor["sequence"].to_string()),
        ("Subscription", cursor["subscription_revision"].to_string()),
    ];
    for (label, value) in rows {
        ui::outln(&format!("{label:<16}{}", ui::clean(&value)));
    }
    Ok(())
}

fn legacy(command: &str) {
    ui::warn(
        "deprecated",
        &format!(
            "Piped {command} prints JSON for one more release, then a table. Pass --json to keep JSON."
        ),
    );
}

pub fn doctor(home: &Path) -> Outcome {
    let report =
        crate::inspection::doctor(home).map_err(|error| inspection_failure(home, &error))?;
    let ok = report["ok"].as_bool().unwrap_or(false);
    let modes = Modes::get();
    let failed = || {
        let mut failure = Failure::new(
            Reason::UnhealthyHome,
            format!("Doctor found problems in {}.", home.display()),
        );
        if let Some(remedy) = report["credentials"]["last_error"]["remedy"].as_str() {
            failure = failure.sentence(ui::clean(remedy));
        }
        failure
    };
    if modes.json || modes.legacy_json() {
        if !modes.json {
            legacy("bridge doctor");
        }
        if modes.json {
            ui::json_document(report.clone());
        } else {
            ui::exact_json(&report);
        }
        return if ok {
            Ok(())
        } else {
            Err(failed().quiet_in_json())
        };
    }
    let text = |value: &Value| {
        value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_owned)
    };
    let mut rows = vec![
        (
            "Healthy",
            if ok {
                "yes".to_owned()
            } else {
                "no".to_owned()
            },
        ),
        ("SQLite", text(&report["sqlite"])),
        ("Schema", text(&report["schema"])),
        ("Private files", text(&report["private_modes"])),
        ("Disk encryption", text(&report["disk_encryption"])),
        ("Credentials", text(&report["credentials"]["store"])),
        ("Exactly once", text(&report["exactly_once"])),
    ];
    rows.retain(|(_, value)| value != "null");
    for (label, value) in rows {
        ui::outln(&format!("{label:<16}{}", ui::clean(&value)));
    }
    if let Some(warnings) = report["warnings"].as_array() {
        for warning in warnings.iter().filter_map(Value::as_str) {
            ui::warn("doctor", warning);
        }
    }
    if ok { Ok(()) } else { Err(failed()) }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

fn modified(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| jiff::Timestamp::try_from(time).ok())
        .map(|time| ui::age(&time.to_string()))
        .unwrap_or_default()
}

#[allow(clippy::too_many_lines)]
pub fn backup(home: &Path, args: &BackupArgs) -> Outcome {
    let destination = &args.path;
    if destination.as_os_str().is_empty() {
        return Err(Failure::new(
            Reason::MissingArgument,
            "Can't back up without a file name.",
        ));
    }
    if destination.is_dir() {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't back up to {}. It's a folder; name a file inside it.",
                destination.display()
            ),
        ));
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_owned();
    if !parent.is_dir() {
        return Err(Failure::new(
            Reason::NotFound,
            format!("Couldn't find the folder {}.", parent.display()),
        ));
    }
    if !home.join("state.db").is_file() {
        return Err(Failure::not_set_up(home));
    }
    let existed = destination.exists();
    if existed && !args.force {
        let mut words = vec![
            "bridge".to_owned(),
            "backup".to_owned(),
            destination.to_string_lossy().into_owned(),
            "--force".to_owned(),
        ];
        if Modes::get().interactive() || Modes::get().yes {
            words.pop();
        }
        ui::confirm(
            &[format!(
                "{} exists ({}, changed {}). It will be replaced; the old file can't be recovered.",
                destination.display(),
                ui::size(file_size(destination)),
                modified(destination)
            )],
            &format!("Replace {}?", destination.display()),
            &Step::new("Replace it", words),
            "The existing file was kept.",
        )
        .map_err(|failure| {
            if failure.reason == Reason::ConfirmationRequired {
                Failure::new(
                    Reason::DestinationExists,
                    format!(
                        "Can't back up to {}. The file exists.",
                        destination.display()
                    ),
                )
                .needs_action()
                .then(
                    "Replace it, after checking you don't need it",
                    [
                        "bridge".to_owned(),
                        "backup".to_owned(),
                        destination.to_string_lossy().into_owned(),
                        "--force".to_owned(),
                    ],
                )
            } else {
                failure
            }
        })?;
    }
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let staged = parent.join(format!(".{name}.bridge-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&staged);
    crate::maintenance::backup(home, &staged).map_err(|error| {
        let _ = std::fs::remove_file(&staged);
        match error {
            Error::Invalid => Failure::not_set_up(home),
            other => Failure::new(
                Reason::UnhealthyHome,
                format!("Failed to back up {}.", home.display()),
            )
            .row("Detail", ui::clean(&other.to_string()))
            .then("Check the home", ["bridge", "doctor"]),
        }
    })?;
    let placed = if existed {
        std::fs::rename(&staged, destination)
    } else {
        ab_sys::rename_exclusive(&staged, destination)
    };
    if let Err(error) = placed {
        let _ = std::fs::remove_file(&staged);
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(Failure::new(
                Reason::DestinationExists,
                format!(
                    "Can't back up to {}. A file appeared there while backing up.",
                    destination.display()
                ),
            )
            .then(
                "Replace it",
                [
                    "bridge".to_owned(),
                    "backup".to_owned(),
                    destination.to_string_lossy().into_owned(),
                    "--force".to_owned(),
                ],
            ));
        }
        return Err(Failure::from(error));
    }
    if let Ok(directory) = std::fs::File::open(&parent) {
        let _ = directory.sync_all();
    }
    let bytes = file_size(destination);
    let outcome = if existed { "replaced" } else { "created" };
    if Modes::get().json {
        ui::json_document(json!({"path": destination, "bytes": bytes, "outcome": outcome}));
        return Ok(());
    }
    if !Modes::get().stdout_tty() {
        ui::outln(&destination.display().to_string());
    }
    ui::done(
        "Backed up",
        &format!("{} ({})", destination.display(), ui::size(bytes)),
    );
    if existed {
        ui::row("Replaced", "the previous file");
    }
    Ok(())
}

fn sqlite_file(path: &Path) -> bool {
    let mut header = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok_and(|()| &header == b"SQLite format 3\0")
}

pub fn restore(home: &Path, args: RestoreArgs) -> Outcome {
    let backup = match (args.path, args.backup) {
        (Some(positional), Some(named)) if positional != named => {
            return Err(Failure::new(
                Reason::ConflictingOptions,
                "Can't restore two backups. Pass the file once, as FILE or --backup.",
            ));
        }
        (positional, named) => positional.or(named),
    };
    let Some(backup) = backup.filter(|path| !path.as_os_str().is_empty()) else {
        if !args.accept_ledger_loss {
            return Err(Failure::new(
                Reason::MissingArgument,
                "Can't restore without a backup file.",
            )
            .then("Restore a backup", ["bridge", "restore", "<FILE>"]));
        }
        return rebuild_in_place(home);
    };
    if !backup.is_file() {
        return Err(Failure::new(
            Reason::NotFound,
            format!("Couldn't find the backup {}.", backup.display()),
        )
        .then("Make a backup first", ["bridge", "backup", "<FILE>"]));
    }
    if !sqlite_file(&backup) {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't restore {}. It isn't an Apple Bridge backup.",
                backup.display()
            ),
        ));
    }
    if gateway::lock_held(home) {
        return Err(Failure::new(
            Reason::GatewayBusy,
            format!(
                "Can't restore while Apple Bridge is running from {}.",
                home.display()
            ),
        )
        .then("Stop it first", ["bridge", "down"]));
    }
    let current = home.join("state.db");
    let had_data = current.exists();
    let mut preview = vec![
        format!("Home: {}", home.display()),
        format!(
            "Backup: {} ({}, changed {})",
            backup.display(),
            ui::size(file_size(&backup)),
            modified(&backup)
        ),
    ];
    if current.exists() {
        preview.push(format!(
            "Replaces: {} ({}). Its synced data is lost; credentials, settings and remote setup are kept.",
            current.display(),
            ui::size(file_size(&current))
        ));
    } else {
        preview.push("Creates this home's data; it has none yet.".to_owned());
    }
    preview.push(if args.accept_ledger_loss {
        "Skips the command ledger check. Commands recorded since this backup may run again. This cannot be undone.".to_owned()
    } else {
        "Checks the command ledger first and stops if it doesn't match. Sync restarts from the backup.".to_owned()
    });
    let mut words = vec![
        "bridge".to_owned(),
        "restore".to_owned(),
        backup.to_string_lossy().into_owned(),
    ];
    if args.accept_ledger_loss {
        words.push("--accept-ledger-loss".to_owned());
    }
    words.push("--yes".to_owned());
    ui::confirm(
        &preview,
        &format!("Restore {} into {}?", backup.display(), home.display()),
        &Step::new("Restore it", words),
        "Nothing was changed.",
    )?;
    crate::maintenance::restore(home, &backup, args.accept_ledger_loss)
        .map_err(|error| restore_failure(home, &backup, &error))?;
    report_restore(home, Some(&backup), had_data)
}

fn rebuild_in_place(home: &Path) -> Outcome {
    let current = home.join("state.db");
    if !current.is_file() {
        return Err(Failure::not_set_up(home));
    }
    if gateway::lock_held(home) {
        return Err(Failure::new(
            Reason::GatewayBusy,
            format!(
                "Can't rebuild while Apple Bridge is running from {}.",
                home.display()
            ),
        )
        .then("Stop it first", ["bridge", "down"]));
    }
    ui::confirm(
        &[
            format!(
                "Rebuilds {} without the command ledger check.",
                current.display()
            ),
            "Commands recorded before the ledger was lost may run again. This cannot be undone."
                .to_owned(),
        ],
        &format!("Rebuild {}?", home.display()),
        &Step::new(
            "Rebuild it",
            ["bridge", "restore", "--accept-ledger-loss", "--yes"],
        ),
        "Nothing was changed.",
    )?;
    crate::maintenance::restore(home, Path::new(""), true)
        .map_err(|error| restore_failure(home, &current, &error))?;
    report_restore(home, None, true)
}

fn restore_failure(home: &Path, backup: &Path, error: &Error) -> Failure {
    let text = error.to_string();
    match error {
        Error::LedgerMissing | Error::State(ab_state::Error::LedgerMissing) => Failure::new(
            Reason::LedgerMissing,
            format!(
                "Can't restore into {}. Its command ledger is missing or doesn't match the backup.",
                home.display()
            ),
        )
        .sentence("Restoring anyway can run commands again that already ran.")
        .then(
            "Only if you accept that, run",
            [
                "bridge".to_owned(),
                "restore".to_owned(),
                backup.to_string_lossy().into_owned(),
                "--accept-ledger-loss".to_owned(),
            ],
        ),
        Error::Locked => Failure::new(
            Reason::GatewayBusy,
            format!(
                "Can't restore while Apple Bridge is running from {}.",
                home.display()
            ),
        )
        .then("Stop it first", ["bridge", "down"]),
        _ if text.contains("corrupt")
            || text.contains("malformed")
            || text.contains("not a database") =>
        {
            Failure::new(
                Reason::InvalidValue,
                format!("Can't restore {}. The backup is damaged.", backup.display()),
            )
        }
        _ => Failure::new(
            Reason::UnhealthyHome,
            format!("Failed to restore {}.", backup.display()),
        )
        .row("Detail", ui::clean(&text))
        .sentence("Nothing was replaced."),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn report_restore(home: &Path, backup: Option<&PathBuf>, replaced: bool) -> Outcome {
    let settings = home.join("runtime.json").exists();
    let modes = Modes::get();
    if modes.json {
        ui::json_document(json!({
            "home": home,
            "backup": backup,
            "outcome": "restored",
            "replaced": replaced,
            "kept": if replaced { json!(["credentials", "settings", "remote", "ledger"]) } else { json!([]) },
            "settings_present": settings,
        }));
        return Ok(());
    }
    ui::done("Restored", &home.display().to_string());
    if let Some(backup) = backup {
        ui::row("From", &backup.display().to_string());
    }
    ui::row(
        "Replaced",
        if replaced {
            "synced data"
        } else {
            "nothing; this home had no data"
        },
    );
    ui::row(
        "Kept",
        "credentials, settings, remote setup and the command ledger",
    );
    if settings {
        ui::next_steps(&[Step::new("Start it again", ["bridge", "up"])]);
    } else {
        ui::warn(
            "settings_missing",
            "This home has no gateway settings yet, so bridge status fails until you start it.",
        );
        ui::next_steps(&[Step::new(
            "Start it to write its settings",
            ["bridge", "up"],
        )]);
    }
    Ok(())
}

fn route(via: Option<&str>, private: bool, domain: Option<&str>) -> Outcome<String> {
    if private && (via.is_some() || domain.is_some()) {
        let other = if via.is_some() { "--via" } else { "--domain" };
        return Err(Failure::new(
            Reason::ConflictingOptions,
            format!("Can't use --private with {other}."),
        ));
    }
    let route = if private {
        "private"
    } else {
        via.unwrap_or("relay")
    };
    if !matches!(route, "relay" | "private" | "tailscale-app" | "both") {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --via {}. Accepted: relay, private, tailscale-app, both.",
                ui::clean(route)
            ),
        ));
    }
    Ok(route.to_owned())
}

fn valid_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

fn relay_url(url: &str) -> Outcome<String> {
    let parsed = reqwest::Url::parse(url)
        .ok()
        .filter(|parsed| parsed.scheme() == "https" && parsed.host_str().is_some());
    if parsed.is_none() {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --relay-url {}. Relays use https://HOST.",
                ui::clean(url)
            ),
        ));
    }
    Ok(url.to_owned())
}

fn device_name(name: &str) -> Outcome {
    if name.trim().is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(Failure::new(
            Reason::InvalidValue,
            "Can't use that --name. Names are 1 to 64 printable characters.",
        ));
    }
    Ok(())
}

struct Stderr;

impl std::io::Write for Stderr {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if !Modes::get().json {
            let text = String::from_utf8_lossy(buffer);
            let mut stderr = std::io::stderr().lock();
            for line in text.split_inclusive('\n') {
                std::io::Write::write_all(
                    &mut stderr,
                    format!("  {}", ui::clean_block(line)).as_bytes(),
                )?;
            }
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut std::io::stderr())
    }
}

async fn onboard(
    home: &Path,
    action: &str,
    route: &str,
    timeout: Option<&str>,
    no_open: bool,
) -> Outcome<ab_remote::state::Status> {
    let limit = timeout
        .map(|text| duration("--timeout", text))
        .transpose()?
        .unwrap_or(REMOTE_WAIT);
    if !gateway::lock_held(home) {
        return Err(Failure::not_running(home));
    }
    super::own_interrupt();
    let verb = if action == "on" {
        "Turning on"
    } else {
        "Turning off"
    };
    ui::note(&format!("{verb} remote access ({route})…"));
    let open = !no_open && Modes::get().browser_allowed();
    let opener: ab_remote::onboarding::Opener =
        Arc::new(move |link: &str| open && ab_remote::onboarding::open_link(link));
    let mut out = Stderr;
    let waited = tokio::time::timeout(
        limit,
        ab_remote::onboarding::settle(
            home,
            action,
            route,
            &mut out,
            opener,
            Duration::from_millis(100),
        ),
    )
    .await;
    let verb_past = if action == "on" { "on" } else { "off" };
    match waited {
        Err(_) => Err(Failure::new(
            Reason::Timeout,
            format!(
                "Stopped waiting after {}. Remote access isn't confirmed {verb_past} yet.",
                friendly(limit)
            ),
        )
        .sentence("The gateway keeps working on the request; nothing was canceled.")
        .then("Check it", ["bridge", "remote", "status"])),
        Ok(Err(ab_remote::Error::Remote(message))) if message == "interrupted" => {
            Err(Failure::canceled(format!(
                "Canceled. The request to turn remote access {verb_past} was withdrawn."
            )))
        }
        Ok(Err(ab_remote::Error::Remote(message))) if message.contains("superseded") => {
            Err(Failure::new(
                Reason::Conflict,
                "Stopped waiting. Another remote access request replaced this one.",
            )
            .then("Check it", ["bridge", "remote", "status"]))
        }
        Ok(Err(error)) => Err(Failure::new(
            Reason::ApiError,
            format!("Failed to turn remote access {verb_past}."),
        )
        .remote("Gateway", error.to_string())
        .then("Check it", ["bridge", "remote", "status"])),
        Ok(Ok(status)) => Ok(status),
    }
}

#[allow(clippy::unnecessary_wraps)]
fn report_remote(status: &ab_remote::state::Status, on: bool) -> Outcome {
    if Modes::get().json {
        ui::json_document(
            json!({"remote": status, "outcome": if on { "enabled" } else { "disabled" }}),
        );
        return Ok(());
    }
    if on {
        if !Modes::get().stdout_tty() && !status.url.is_empty() {
            ui::outln(&status.url);
        }
        ui::done("Remote access on", &status.url);
        if !status.url.is_empty() {
            ui::row("MCP address", &format!("{}/mcp", status.url));
        }
        ui::row("Route", &status.backend);
    } else {
        ui::done("Remote access off", "");
    }
    Ok(())
}

pub async fn remote_enable(home: &Path, args: &RemoteEnableArgs) -> Outcome {
    let route = route(args.via.as_deref(), args.private, args.domain.as_deref())?;
    if let Some(domain) = &args.domain
        && (route != "relay" || !valid_domain(domain))
    {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --domain {}. It needs a domain name such as mac.example.com and the relay route.",
                ui::clean(domain)
            ),
        ));
    }
    if let Some(url) = &args.relay_url {
        relay_url(url)?;
        if route != "relay" {
            return Err(Failure::new(
                Reason::ConflictingOptions,
                "Can't use --relay-url with a route other than relay.",
            ));
        }
    }
    if let Some(name) = &args.name {
        device_name(name)?;
    }
    if let Some(timeout) = &args.timeout {
        duration("--timeout", timeout)?;
    }
    if !gateway::lock_held(home) {
        return Err(Failure::not_running(home));
    }
    if let Some(url) = &args.relay_url {
        ab_remote::state::set_relay(home, url)?;
    }
    if route == "relay" {
        ab_remote::state::set_domain(home, args.domain.as_deref().unwrap_or_default())?;
    }
    if let Some(name) = &args.name {
        ab_remote::state::annotate(home, name)?;
    }
    let status = onboard(home, "on", &route, args.timeout.as_deref(), args.no_open).await?;
    report_remote(&status, true)
}

pub async fn remote_disable(home: &Path, args: &RemoteDisableArgs) -> Outcome {
    let route = route(args.via.as_deref(), args.private, None)?;
    if let Some(name) = &args.name {
        device_name(name)?;
    }
    if let Some(timeout) = &args.timeout {
        duration("--timeout", timeout)?;
    }
    let current = ab_remote::state::status(home)?;
    if current.state == "off" {
        if Modes::get().json {
            ui::json_document(json!({"remote": current, "outcome": "unchanged"}));
        } else {
            ui::done("Already off", "remote access");
        }
        return Ok(());
    }
    if let Some(name) = &args.name {
        ab_remote::state::annotate(home, name)?;
    }
    let status = onboard(home, "off", &route, args.timeout.as_deref(), true).await?;
    report_remote(&status, false)
}

pub fn remote_status(home: &Path, args: &RemoteStatusArgs) -> Outcome {
    if args.snippets {
        let snippets = ab_remote::state::snippets(home).map_err(|_| {
            Failure::new(
                Reason::Conflict,
                "Can't print snippets. Remote access isn't on.",
            )
            .then("Turn it on", ["bridge", "remote", "enable"])
        })?;
        ui::out(&snippets);
        return Ok(());
    }
    let status = ab_remote::state::status(home)?;
    let modes = Modes::get();
    if modes.json || modes.legacy_json() {
        if !modes.json {
            legacy("bridge remote status");
        }
        let value = serde_json::to_value(&status)?;
        if modes.json {
            ui::json_document(value);
        } else {
            ui::exact_json(&value);
        }
        return Ok(());
    }
    let mut rows = vec![("State", status.state.clone())];
    for (label, value) in [
        ("Route", &status.backend),
        ("URL", &status.url),
        ("TLS ends", &status.tls_termination),
        ("Pending link", &status.pending_link),
        ("Last error", &status.last_error),
    ] {
        if !value.is_empty() {
            rows.push((label, value.clone()));
        }
    }
    if !status.certificate_expiry.starts_with("0001") {
        rows.push((
            "Certificate",
            format!("expires {}", status.certificate_expiry),
        ));
    }
    for (label, value) in rows {
        ui::outln(&format!("{label:<16}{}", ui::clean(&value)));
    }
    if !status.ct_warning.is_empty() {
        ui::warn("certificate", &status.ct_warning);
    }
    if status.state == "off" {
        ui::next_steps(&[Step::new("Turn it on", ["bridge", "remote", "enable"])]);
    }
    Ok(())
}

pub fn remote_key(home: &Path, args: &RemoteKeyArgs) -> Outcome {
    if let Some(url) = &args.relay_url {
        relay_url(url)?;
        ab_remote::state::set_relay(home, url)?;
    }
    let config =
        ab_remote::state::read::<ab_remote::state::Config>(home, "config").unwrap_or_default();
    let key = ab_remote::relay::public_key(home, &config.relay_url).map_err(|error| {
        Failure::new(Reason::ApiError, "Failed to read this Mac's relay key.")
            .remote("Remote", error.to_string())
    })?;
    if Modes::get().json {
        ui::json_document(json!({"key": key}));
    } else {
        ui::outln(&key);
    }
    Ok(())
}

pub fn certificate_accept(home: &Path, fingerprint: Option<&str>) -> Outcome {
    let Some(fingerprint) = fingerprint else {
        return Err(Failure::new(
            Reason::MissingArgument,
            "Can't accept a certificate without its fingerprint.",
        )
        .then(
            "See the certificate warning",
            ["bridge", "remote", "status"],
        ));
    };
    let normalized = fingerprint.replace(':', "").to_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use {} as a fingerprint. Fingerprints are 64 hexadecimal characters.",
                ui::clean(fingerprint)
            ),
        ));
    }
    ab_remote::ct::accept(home, &normalized)?;
    if Modes::get().json {
        ui::json_document(json!({"fingerprint": normalized, "outcome": "accepted"}));
    } else {
        ui::done("Accepted", &normalized);
    }
    Ok(())
}

pub async fn connect(args: &ConnectArgs) -> Outcome {
    address("--listen", &args.listen)?;
    ab_remote::private::connect(&args.code, &args.listen).await?;
    Ok(())
}

pub async fn shortcuts(pack: PathBuf) -> Outcome {
    if !pack.is_dir() {
        return Err(Failure::new(
            Reason::NotFound,
            format!("Couldn't find the Shortcuts pack at {}.", pack.display()),
        )
        .then(
            "Point to it",
            ["bridge", "shortcuts", "install", "--pack", "<DIR>"],
        ));
    }
    let shown = pack.display().to_string();
    tokio::task::spawn_blocking(move || ab_shortcuts::cli::open_pack(&pack))
        .await
        .map_err(|error| Failure::internal(&error))?
        .map_err(|error| {
            Failure::new(Reason::SpawnFailed, "Failed to open the Shortcuts pack.")
                .remote("Shortcuts", error.to_string())
        })?;
    ui::done("Opened", &shown);
    ui::note("Import each shortcut in the Shortcuts app.");
    Ok(())
}

pub async fn join(home: &Path, args: &JoinArgs) -> Outcome {
    let code = ab_trust::PairCode::decode(&args.code).map_err(|_| {
        Failure::new(
            Reason::InvalidValue,
            "Can't use that pairing code. Copy the whole code bridge pair printed.",
        )
    })?;
    if let Some(listen) = &args.listen {
        address("--listen", listen)?;
    }
    let authority = Arc::new(access::open(home)?);
    let result = if code.remote {
        ab_replication::relay::join(&authority, &args.code)
            .await
            .map_err(|_| ab_trust::Error::Unauthorized)
    } else if let Some(listen) = &args.listen {
        authority.join_listen(&args.code, listen).await
    } else {
        authority.join(&args.code).await
    };
    result.map(drop).map_err(|error| {
        if matches!(error, ab_trust::Error::Unauthorized) {
            Failure::new(
                Reason::PermissionDenied,
                "Can't join. The gateway refused this pairing code.",
            )
            .sentence("Pairing codes expire and work once.")
            .then("Create a new one on the gateway", ["bridge", "pair"])
        } else {
            Failure::new(
                Reason::NetworkError,
                "Couldn't reach the gateway in the pairing code.",
            )
            .sentence("Check that it's running and its address is reachable from here.")
        }
    })?;
    if Modes::get().json {
        ui::json_document(json!({"home": home, "outcome": "joined"}));
    } else {
        ui::done("Joined", "the gateway as a replica");
        ui::row("Home", &home.display().to_string());
    }
    Ok(())
}

pub fn install_credentials(home: &Path) -> Outcome {
    access::open(home)?;
    if Modes::get().stdout_tty() {
        ui::done("Credentials ready", &home.display().to_string());
    }
    Ok(())
}

pub async fn start(home: &Path, args: StartArgs) -> Outcome {
    let retention_age = crate::duration::parse(&args.retention_age).map_err(|_| {
        Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --retention-age {}. Durations look like 90s, 10m or 168h.",
                ui::clean(&args.retention_age)
            ),
        )
    })?;
    if !matches!(args.role.as_str(), "gateway" | "replica") {
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --role {}. Accepted: gateway, replica.",
                ui::clean(&args.role)
            ),
        ));
    }
    gateway::loopback(&args.api_addr)?;
    for (option, value) in [("--peer-addr", &args.peer_addr), ("--listen", &args.listen)] {
        if !value.is_empty() {
            address(option, value)?;
        }
    }
    super::check_socket_length(home)?;
    load_config(home)?;
    let authority = Arc::new(access::open(home)?);
    let config = Config {
        role: args.role,
        home: home.to_owned(),
        api_address: args.api_addr,
        peer_address: args.peer_addr,
        listen: args.listen,
        socket: args.socket,
        helper_app: crate::bundle::helper_app(
            &args.helper_app,
            std::env::current_exe().ok().as_deref(),
        ),
        helper_lab_commands: args.helper_lab_commands,
        retention_rows: args.retention_rows,
        retention_age,
        blob_cap: args.blob_cap,
        blob_grace_ms: args.blob_grace_ms,
    };
    crate::start(config, authority).await.map_err(Failure::from)
}

fn remote_url(home: &Path) -> Outcome<String> {
    let off = || {
        Failure::new(
            Reason::Conflict,
            "Can't pair through remote access. It's off.",
        )
        .then("Turn it on", ["bridge", "remote", "enable"])
    };
    let status: Value = ab_remote::state::read(home, "status").map_err(|_| off())?;
    if status["state"] != "on" {
        return Err(off());
    }
    status["url"].as_str().map(str::to_owned).ok_or_else(off)
}

fn remote_address(home: &Path) -> Outcome<String> {
    #[cfg(feature = "lab-harness")]
    if let Some(address) = std::env::var_os("APPLE_BRIDGE_LAB_PAIR_URL") {
        return address.into_string().map_err(|_| {
            Failure::new(
                Reason::InvalidValue,
                "APPLE_BRIDGE_LAB_PAIR_URL isn't text.",
            )
        });
    }
    remote_url(home)
}

fn print_code(code: &str, qr: bool, ttl: Duration) -> Outcome {
    ui::mask_secret(code);
    if Modes::get().json {
        ui::json_document(
            json!({"code": code, "expires_in_seconds": ttl.as_secs(), "outcome": "created"}),
        );
        return Ok(());
    }
    ui::outln(code);
    if Modes::get().stdout_tty() {
        if qr {
            ui::out(&product_cli::terminal_qr(code)?);
        }
        ui::note(&format!(
            "The code is a secret. It works once, for {}.",
            friendly(ttl)
        ));
    }
    Ok(())
}

pub fn pair(home: &Path, args: &PairArgs) -> Outcome {
    let nanos = crate::duration::parse(&args.expires_in);
    let ttl = nanos
        .ok()
        .and_then(|nanos| u64::try_from(nanos).ok())
        .filter(|nanos| *nanos > 0)
        .map(Duration::from_nanos)
        .ok_or_else(|| {
            Failure::new(
                Reason::InvalidValue,
                format!(
                    "Can't use --expires-in {}. Durations look like 90s, 10m or 168h.",
                    ui::clean(&args.expires_in)
                ),
            )
        })?;
    if args.iphone && (!args.via_remote || args.peer_addr.is_some() || args.dial_replica.is_some())
    {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't pair an iPhone that way. --iphone needs --via-remote and can't use --peer-addr or --dial-replica.",
        ));
    }
    if args.via_remote && (args.peer_addr.is_some() || args.dial_replica.is_some()) {
        return Err(Failure::new(
            Reason::ConflictingOptions,
            "Can't use --via-remote with --peer-addr or --dial-replica.",
        ));
    }
    let grants = if args.iphone {
        Vec::new()
    } else {
        access::parse_grants(&args.scope, false)?
    };
    if let Some(peer) = &args.peer_addr {
        address("--peer-addr", peer)?;
    }
    if let Some(dial) = &args.dial_replica {
        address("--dial-replica", dial)?;
    }
    let config = load_config(home)?.ok_or_else(|| Failure::not_set_up(home))?;
    if config.role != "gateway" {
        return Err(Failure::new(
            Reason::Conflict,
            "Can't create a pairing code on a replica. Only a gateway can.",
        ));
    }
    if args.iphone {
        let url = remote_url(home)?;
        let authority = access::open(home)?;
        let warning = ab_remote::ct::owner_warning(home, ab_remote::ct::now());
        if !warning.is_empty() {
            ui::warn("certificate", &warning);
        }
        let code = authority.pair_iphone_code(&url, true, ttl)?;
        return print_code(&code, true, ttl);
    }
    if args.via_remote {
        let address = remote_address(home)?;
        let authority = access::open(home)?;
        let code = authority.pair_remote(&address, grants, ttl)?;
        return print_code(&code, false, ttl);
    }
    let authority = access::open(home)?;
    let code = authority.pair_with_ttl(
        args.peer_addr.as_deref().unwrap_or(&config.peer_address),
        args.dial_replica.as_deref().unwrap_or_default(),
        grants,
        ttl,
    )?;
    print_code(&code, false, ttl)
}

#[cfg(feature = "lab-harness")]
pub fn lab_operation_catalog() -> Outcome {
    let mut descriptors = ab_catalog::operations().map_err(|error| Failure::internal(&error))?;
    for name in ["test.echo", "test.sleep", "test.blob"] {
        descriptors.push(ab_operations::descriptor_for(name).map_err(Error::from)?);
    }
    let catalog = descriptors
        .into_iter()
        .map(|descriptor| {
            (
                descriptor.name,
                json!({"scope":descriptor.scope,"schema":descriptor.input}),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    ui::outln(&serde_json::to_string(&catalog)?);
    Ok(())
}

#[cfg(feature = "lab-harness")]
pub fn lab_credential_lifecycle(home: &Path) -> Outcome {
    access::open(home)?;
    crate::inspection::credential_lifecycle(home)?;
    Ok(())
}

#[cfg(feature = "lab-harness")]
pub fn lab_session(home: &Path) -> Outcome {
    let (session, _) = access::open(home)?.create_device_session("trusted")?;
    ui::outln(&json!({"name":"ab_session_local","value":session}).to_string());
    Ok(())
}

#[cfg(feature = "lab-harness")]
pub fn lab_principal(home: &Path, args: &super::tree::LabPrincipalArgs) -> Outcome {
    let authority = access::open(home)?;
    let principal = match (&args.id, &args.fingerprint) {
        (Some(id), None) => authority.principal(id)?,
        (None, Some(fingerprint)) => authority.principal_by_fingerprint(fingerprint)?,
        _ => {
            return Err(Failure::usage(
                "Can't look up a principal. Pass exactly one of --id or --fingerprint.",
            ));
        }
    };
    ui::outln(&json!({"id":principal.id,"kind":principal.kind,"fingerprint":principal.fingerprint,"grants":principal.grants,"revision":principal.revision,"relay2":principal.relay2()}).to_string());
    Ok(())
}
