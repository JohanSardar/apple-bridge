use super::{
    Failure, Outcome, access,
    failure::{Reason, Step},
    gateway,
    modes::Modes,
    permissions,
    tree::{BackupArgs, JoinArgs, PairArgs, RestoreArgs, SelftestArgs, StartArgs},
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

pub fn duration_or_zero(option: &str, text: &str) -> Outcome<Duration> {
    let invalid = || {
        Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use {option} {}. Durations look like 30s, 5m, 2h or 7d.",
                ui::clean(text)
            ),
        )
    };
    let trimmed = text.trim();
    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Ok(Duration::from_secs(seconds));
    }
    for (suffix, hours) in [("d", 24_u64), ("w", 168)] {
        if let Some(count) = trimmed.strip_suffix(suffix)
            && let Ok(count) = count.parse::<u64>()
        {
            return count
                .checked_mul(hours * 3600)
                .filter(|seconds| *seconds <= 100 * 365 * 86_400)
                .map(Duration::from_secs)
                .ok_or_else(invalid);
        }
    }
    let nanos = crate::duration::parse(trimmed).map_err(|_| invalid())?;
    Ok(Duration::from_nanos(
        u64::try_from(nanos).map_err(|_| invalid())?,
    ))
}

pub fn duration(option: &str, text: &str) -> Outcome<Duration> {
    let parsed = duration_or_zero(option, text)?;
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
    } else if seconds < 2 * 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

pub fn address(option: &str, text: &str) -> Outcome<String> {
    let hostname = |host: &str| {
        host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
    };
    let valid = text.parse::<std::net::SocketAddr>().is_ok()
        || text.rsplit_once(':').is_some_and(|(host, port)| {
            port.parse::<u16>().is_ok()
                && host.parse::<std::net::IpAddr>().is_err()
                && !host.chars().all(|c| c.is_ascii_digit() || c == '.')
                && hostname(host)
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
            .sentence("The file was left as it is. Fix it, or move it away and write new settings.")
            .then("Write new settings", ["bridge", "up"])
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
        .then("For Full Disk Access", ["bridge", "permissions", "guide"]));
    }
    let mut retry = vec!["bridge", "permissions", "request", name];
    if let Some(helper) = helper_app {
        retry.extend(["--helper-app", helper]);
    }
    Modes::get().require_native_interaction("the permission request", &retry)?;
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
    if outcome == "denied" {
        ui::warn("permission_denied", &permissions::declined(name));
    } else {
        permissions::after_request(home, name, helper_app).await?;
    }
    if Modes::get().json {
        let outcome = if outcome.is_empty() {
            "requested"
        } else {
            &outcome
        };
        ui::json_document(json!({"permission": name, "outcome": outcome}));
    } else if outcome != "denied" {
        ui::done("Requested", name);
    }
    Ok(())
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
    if args.dry_run {
        dry_run("selftest messages", "Send real messages to your own chat.");
        return Ok(());
    }
    if !Modes::get().yes {
        return Err(Failure::new(
            Reason::ConfirmationRequired,
            "The Messages self-test sends real messages to your own chat.",
        )
        .then(
            "Confirm the send",
            ["bridge", "selftest", "messages", "--yes"],
        ));
    }
    Modes::get().require_native_interaction(
        "the Messages self-test",
        &["bridge", "selftest", "messages", "--yes"],
    )?;
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
            ["bridge", "selftest", "messages", "--force", "--yes"],
        ));
    }
    ui::receipt("Proven", "Messages", "Messages is proven.");
    Ok(())
}

pub fn replica_start(home: &Path) -> Option<Step> {
    let config_replica = load_config(home)
        .ok()
        .flatten()
        .is_some_and(|config| config.role == "replica");
    let enrollment = home
        .join("credentials.json")
        .is_file()
        .then(|| access::open(home).ok())
        .flatten()
        .and_then(|authority| authority.enrollment().ok().flatten());
    if !config_replica && enrollment.is_none() {
        return None;
    }
    let mut argv = vec![
        "bridge".to_owned(),
        "start".to_owned(),
        "--role".to_owned(),
        "replica".to_owned(),
    ];
    if let Some(dial) = enrollment
        .as_ref()
        .map(|enrollment| enrollment.dial.clone())
        .filter(|dial| !dial.is_empty())
    {
        argv.extend(["--listen".to_owned(), dial]);
    }
    Some(Step::new("Start the replica", argv))
}

pub fn not_set_up(home: &Path) -> Failure {
    match replica_start(home) {
        Some(step) => Failure::new(
            Reason::NotSetUp,
            format!(
                "The replica in {} hasn't started yet, so it has no data.",
                home.display()
            ),
        )
        .step(step),
        None => Failure::not_set_up(home),
    }
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
        not_set_up(home)
    }
}

pub fn status(home: &Path) -> Outcome {
    let config = load_config(home)?;
    if config.is_none() {
        if let Some(step) = replica_start(home) {
            return Err(Failure::new(
                Reason::NotSetUp,
                format!(
                    "Can't show status. The replica in {} hasn't started yet.",
                    home.display()
                ),
            )
            .step(step));
        }
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
        return Err(not_set_up(home));
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
    let role = report["role"].as_str().unwrap_or_default();
    let mut rows = vec![
        (label_case(role), absolute(home).display().to_string()),
        (
            "Epoch".to_owned(),
            cursor["epoch"].as_str().unwrap_or_default().to_owned(),
        ),
        ("Sequence".to_owned(), cursor["sequence"].to_string()),
        (
            "Subscription".to_owned(),
            cursor["subscription_revision"].to_string(),
        ),
    ];
    rows.retain(|(_, value)| value != "null");
    ui::block(ui::Mark::Good, &rows);
    Ok(())
}

pub fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

fn label_case(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

fn legacy(command: &str) {
    ui::warn(
        "deprecated",
        &format!(
            "Piped {command} prints JSON until the next major version, then a table. Pass --json to keep JSON."
        ),
    );
}

fn yes_no(value: &Value) -> String {
    match value {
        Value::Bool(true) => "yes".to_owned(),
        Value::Bool(false) => "no".to_owned(),
        Value::String(text) => match text.as_str() {
            "ok" | "enabled" | "complete" => "yes".to_owned(),
            other => other.replace('_', " "),
        },
        Value::Null => "null".to_owned(),
        other => other.to_string(),
    }
}

pub fn doctor(home: &Path, args: &super::tree::DoctorArgs) -> Outcome {
    let report =
        crate::inspection::doctor(home).map_err(|error| inspection_failure(home, &error))?;
    let ok = report["ok"].as_bool().unwrap_or(false);
    let modes = Modes::get();
    let failed = || {
        let mut failure = Failure::new(
            Reason::UnhealthyHome,
            format!("Doctor found problems in {}.", home.display()),
        )
        .data("report", report.clone());
        if let Some(remedy) = report["credentials"]["last_error"]["remedy"].as_str() {
            failure = failure.sentence(ui::clean(remedy));
        }
        failure
    };
    let verdict = || {
        if ok || !args.check {
            Ok(())
        } else {
            Err(failed())
        }
    };
    if modes.json || modes.legacy_json() {
        verdict()?;
        if !modes.json {
            legacy("bridge doctor");
        }
        if modes.json {
            ui::json_document(report.clone());
        } else {
            ui::exact_json(&report);
        }
        return Ok(());
    }
    let mut rows = vec![
        ("Healthy".to_owned(), yes_no(&json!(ok))),
        ("Home".to_owned(), absolute(home).display().to_string()),
        ("SQLite intact".to_owned(), yes_no(&report["sqlite"])),
        ("Schema".to_owned(), yes_no(&report["schema"])),
        ("Private files".to_owned(), yes_no(&report["private_modes"])),
        (
            "Disk encrypted".to_owned(),
            yes_no(&report["disk_encryption"]),
        ),
        (
            "Credentials".to_owned(),
            yes_no(&report["credentials"]["store"]),
        ),
        ("Exactly once".to_owned(), yes_no(&report["exactly_once"])),
    ];
    rows.retain(|(_, value)| value != "null");
    ui::block(if ok { ui::Mark::Good } else { ui::Mark::Bad }, &rows);
    if let Some(warnings) = report["warnings"].as_array() {
        for warning in warnings.iter().filter_map(Value::as_str) {
            let (sentence, step) = doctor_warning(warning);
            ui::warn("doctor", &sentence);
            if let Some(step) = step {
                ui::next_step(&step);
            }
        }
    }
    if !ok && !args.check {
        ui::warn(
            "unhealthy_home",
            "Doctor found problems. Pass --check to make that exit 1.",
        );
        if let Some(remedy) = report["credentials"]["last_error"]["remedy"].as_str() {
            ui::note(&ui::clean(remedy));
        }
    }
    verdict()
}

fn doctor_warning(text: &str) -> (String, Option<Step>) {
    if let Some(source) = text.strip_suffix(
        " document source is not ready; start the current Helper and check sources.status",
    ) {
        return (
            format!(
                "The {} document source isn't ready. Start the current Apple Bridge app, then check the source.",
                label_case(&ui::clean(source))
            ),
            Some(Step::new("Check it", ["bridge", "call", "sources.status"])),
        );
    }
    let text = ui::clean(text);
    let mut characters = text.chars();
    let sentence = characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect::<String>()
    });
    if sentence.ends_with('.') {
        (sentence, None)
    } else {
        (format!("{sentence}."), None)
    }
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
        return Err(not_set_up(home));
    }
    let existed = destination.exists();
    if args.dry_run {
        let action = if existed { "Replace" } else { "Create" };
        dry_run(
            "backup",
            &format!("{action} the backup at {}.", destination.display()),
        );
        return Ok(());
    }
    if existed && !args.force {
        let refused = || {
            let failure = Failure::new(
                Reason::DestinationExists,
                format!(
                    "Can't back up to {}. The file exists.",
                    destination.display()
                ),
            )
            .needs_action();
            let failure = if Modes::get().yes {
                failure.sentence("--yes doesn't replace files; only --force does.")
            } else {
                failure
            };
            failure.then(
                "Replace it, after checking you don't need it",
                [
                    "bridge".to_owned(),
                    "backup".to_owned(),
                    destination.to_string_lossy().into_owned(),
                    "--force".to_owned(),
                ],
            )
        };
        if Modes::get().yes || !Modes::get().interactive() {
            return Err(refused());
        }
        ui::confirm(&ui::Consent {
            action: format!("replace {}", destination.display()),
            rows: vec![
                ("File".to_owned(), destination.display().to_string()),
                ("Size".to_owned(), ui::size(file_size(destination))),
                ("Changed".to_owned(), modified(destination)),
            ],
            consequence: "It will be replaced; the old file can't be recovered.".to_owned(),
            question: format!("Replace {}?", destination.display()),
            with_yes: Step::new(
                "Replace it",
                [
                    "bridge".to_owned(),
                    "backup".to_owned(),
                    destination.to_string_lossy().into_owned(),
                    "--force".to_owned(),
                ],
            ),
            nothing_changed: "The existing file was kept.",
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
            Error::Invalid => not_set_up(home),
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
    ui::receipt(
        "Backed up",
        &format!("{} ({})", destination.display(), ui::size(bytes)),
        &format!(
            "Backed up to {} ({}).",
            destination.display(),
            ui::size(bytes)
        ),
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

pub(super) fn dry_run(operation: &str, description: &str) {
    if Modes::get().json {
        ui::json_document(
            json!({"outcome": "dry_run", "operation": operation, "description": description}),
        );
    } else {
        ui::outln(description);
    }
}

#[allow(clippy::too_many_lines)]
pub fn restore(home: &Path, args: RestoreArgs) -> Outcome {
    if [&args.path, &args.backup]
        .into_iter()
        .flatten()
        .any(|path| path.as_os_str().is_empty())
    {
        return Err(Failure::invalid("Can't restore an empty backup path."));
    }
    let backup = match (args.path, args.backup) {
        (Some(positional), Some(named)) if positional != named => {
            return Err(Failure::new(
                Reason::ConflictingOptions,
                "Can't restore two backups. Pass the file once, as FILE or --backup.",
            ));
        }
        (positional, named) => positional.or(named),
    };
    let Some(backup) = backup else {
        if !args.accept_ledger_loss {
            return Err(Failure::new(
                Reason::MissingArgument,
                "Can't restore without a backup file.",
            )
            .then("Restore a backup", ["bridge", "restore", "<FILE>"]));
        }
        if args.dry_run {
            dry_run(
                "restore",
                "Rebuild the local state and discard the operation ledger.",
            );
            return Ok(());
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
    if args.dry_run {
        dry_run(
            "restore",
            &format!("Restore {} into {}.", backup.display(), home.display()),
        );
        return Ok(());
    }
    let current = home.join("state.db");
    let had_data = current.exists();
    let mut rows = vec![
        ("Home".to_owned(), home.display().to_string()),
        (
            "Backup".to_owned(),
            format!(
                "{} ({}, changed {})",
                backup.display(),
                ui::size(file_size(&backup)),
                modified(&backup)
            ),
        ),
    ];
    if current.exists() {
        rows.push((
            "Replaces".to_owned(),
            format!("{} ({})", current.display(), ui::size(file_size(&current))),
        ));
        rows.push((
            "Kept".to_owned(),
            "credentials, settings and Tailscale setup".to_owned(),
        ));
    } else {
        rows.push((
            "Replaces".to_owned(),
            "nothing; this home has no data yet".to_owned(),
        ));
    }
    rows.push((
        "Ledger check".to_owned(),
        if args.accept_ledger_loss {
            "skipped".to_owned()
        } else {
            "checked first; stops if it doesn't match".to_owned()
        },
    ));
    let consequence = if args.accept_ledger_loss {
        "The synced data is replaced and commands recorded since this backup may run again. This cannot be undone."
    } else if current.exists() {
        "This home's synced data is replaced by the backup and sync restarts from it."
    } else {
        "This home gets its data from the backup."
    };
    let mut words = vec![
        "bridge".to_owned(),
        "restore".to_owned(),
        backup.to_string_lossy().into_owned(),
    ];
    if args.accept_ledger_loss {
        words.push("--accept-ledger-loss".to_owned());
    }
    words.push("--yes".to_owned());
    ui::confirm(&ui::Consent {
        action: format!("restore {} into {}", backup.display(), home.display()),
        rows,
        consequence: consequence.to_owned(),
        question: format!("Restore {} into {}?", backup.display(), home.display()),
        with_yes: Step::new("Restore it", words),
        nothing_changed: "Nothing was changed.",
    })?;
    crate::maintenance::restore(home, &backup, args.accept_ledger_loss)
        .map_err(|error| restore_failure(home, &backup, &error))?;
    report_restore(home, Some(&backup), had_data)
}

fn rebuild_in_place(home: &Path) -> Outcome {
    let current = home.join("state.db");
    if !current.is_file() {
        return Err(not_set_up(home));
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
    ui::confirm(&ui::Consent {
        action: format!("rebuild {}", home.display()),
        rows: vec![
            ("Home".to_owned(), home.display().to_string()),
            ("Data".to_owned(), current.display().to_string()),
            ("Ledger check".to_owned(), "skipped".to_owned()),
        ],
        consequence:
            "Commands recorded before the ledger was lost may run again. This cannot be undone."
                .to_owned(),
        question: format!("Rebuild {}?", home.display()),
        with_yes: Step::new(
            "Rebuild it",
            ["bridge", "restore", "--accept-ledger-loss", "--yes"],
        ),
        nothing_changed: "Nothing was changed.",
    })?;
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
    ui::receipt(
        "Restored",
        &home.display().to_string(),
        &format!("Restored {}.", home.display()),
    );
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
        "credentials, settings, Tailscale setup and the command ledger",
    );
    if settings {
        ui::next_steps(&[Step::new("Start it again", ["bridge", "up"])]);
    } else {
        ui::warn(
            "settings_missing",
            "This home has no gateway settings yet, so status checks fail until you start it.",
        );
        ui::next_steps(&[Step::new(
            "Start it to write its settings",
            ["bridge", "up"],
        )]);
    }
    Ok(())
}

fn json_problem(path: &Path, error: &serde_json::Error) -> Failure {
    let location = if error.line() > 0 {
        format!("{}:{}:{}", path.display(), error.line(), error.column())
    } else {
        path.display().to_string()
    };
    let problem = match error.classify() {
        serde_json::error::Category::Data => "doesn't match the expected format",
        serde_json::error::Category::Eof => "ends early",
        _ => "isn't valid JSON",
    };
    Failure::new(Reason::InvalidConfig, format!("{location} {problem}."))
        .data("file", path.display().to_string())
        .data("line", error.line())
        .data("column", error.column())
        .row("Detail", ui::clean(&error.to_string()))
}

pub fn remote_file<T: serde::de::DeserializeOwned + Default>(
    home: &Path,
    name: &str,
) -> Outcome<T> {
    let path = home.join("remote").join(format!("{name}.json"));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => return Err(Failure::from(error)),
    };
    serde_json::from_slice(&bytes).map_err(|error| {
        json_problem(&path, &error).sentence(
            "Nothing was changed and no keys were created. Fix the file, or move it away to start Tailscale setup again.",
        )
    })
}

pub(super) fn check_remote_files(home: &Path) -> Outcome {
    remote_file::<ab_remote::state::Config>(home, "config")?;
    remote_file::<Value>(home, "status")?;
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

fn sentence_case(text: &str) -> String {
    let text = text.trim().trim_end_matches('.');
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        format!("{}{}.", first.to_uppercase(), characters.as_str())
    })
}

fn remote_failed(verb_past: &str, error: &ab_remote::Error) -> Failure {
    let text = error.to_string();
    let detail = text.strip_prefix("remote: ").unwrap_or(&text).to_owned();
    let cause = ab_remote::failure_cause(&detail).map(str::to_owned);
    let headline = match &cause {
        Some(cause) => format!(
            "Couldn't turn Tailscale access {verb_past}. {}",
            sentence_case(&ui::redact_urls(cause))
        ),
        None => format!("Failed to turn Tailscale access {verb_past}."),
    };
    let mut failure = Failure::new(
        if cause.is_some() {
            Reason::NetworkError
        } else {
            Reason::ApiError
        },
        headline,
    );
    if cause.is_none() {
        failure = failure.remote("Gateway", detail);
    }
    failure.then("Check it", ["bridge", "status"]).then(
        "Turn it off before trying again",
        ["bridge", "tailscale", "--off"],
    )
}

struct Pending {
    link: std::sync::Mutex<Option<String>>,
    seen: tokio::sync::Notify,
}

#[allow(clippy::too_many_lines)]
pub(super) async fn onboard(
    home: &Path,
    action: &str,
    timeout: Option<&str>,
    open: bool,
) -> Outcome<Result<ab_remote::state::Status, String>> {
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
    let modes = Modes::get();
    let machine = modes.json;
    ui::phase(1, 2, &format!("{verb} Tailscale access…"));
    let pending = Arc::new(Pending {
        link: std::sync::Mutex::new(None),
        seen: tokio::sync::Notify::new(),
    });
    let recorder = pending.clone();
    let record = move |link: &str| {
        if let Ok(mut slot) = recorder.link.lock() {
            *slot = Some(link.to_owned());
        }
        recorder.seen.notify_one();
    };
    let launch = open && modes.browser_allowed();
    let echo = !modes.stdout_tty() && !modes.json;
    let opener: ab_remote::onboarding::Opener = Arc::new(move |link: &str| {
        record(link);
        if echo {
            ui::outln(link);
        }
        if launch {
            ui::phase(2, 2, "Opening the sign-in page in your browser…");
        }
        launch && ab_remote::onboarding::open_link(link)
    });
    let mut out = Stderr;
    let settling =
        ab_remote::onboarding::settle(home, action, &mut out, opener, Duration::from_millis(100));
    let link_needed = async {
        loop {
            pending.seen.notified().await;
            if let Some(link) = pending.link.lock().ok().and_then(|slot| slot.clone()) {
                return link;
            }
        }
    };
    let waited = tokio::time::timeout(limit, async {
        if machine && action == "on" {
            tokio::select! {
                settled = settling => Ok(settled),
                link = link_needed => Err(link),
            }
        } else {
            Ok(settling.await)
        }
    })
    .await;
    let verb_past = if action == "on" { "on" } else { "off" };
    match waited {
        Err(_) => {
            let link = pending.link.lock().ok().and_then(|slot| slot.clone());
            let mut failure = Failure::new(
                Reason::Timeout,
                format!(
                    "Stopped waiting after {}. Tailscale access isn't confirmed {verb_past} yet.",
                    friendly(limit)
                ),
            )
            .sentence("The gateway keeps working on the request; nothing was canceled.");
            if let Some(link) = link {
                failure = failure
                    .row("Sign-in link", link.clone())
                    .data("signInUrl", link);
            }
            Err(failure.then("Check it", ["bridge", "status"]))
        }
        Ok(Err(link)) => Ok(Err(link)),
        Ok(Ok(Err(ab_remote::Error::Remote(message)))) if message == "interrupted" => {
            Err(Failure::canceled(format!(
                "Canceled. The request to turn Tailscale access {verb_past} was withdrawn."
            )))
        }
        Ok(Ok(Err(ab_remote::Error::Remote(message)))) if message.contains("superseded") => {
            Err(Failure::new(
                Reason::Conflict,
                "Stopped waiting. Another Tailscale request replaced this one.",
            )
            .then("Check it", ["bridge", "status"]))
        }
        Ok(Ok(Err(error))) => Err(remote_failed(verb_past, &error)),
        Ok(Ok(Ok(status))) => Ok(Ok(status)),
    }
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
    Modes::get().require_native_interaction(
        "Shortcuts installation",
        &["bridge", "shortcuts", "install", "--pack", &shown],
    )?;
    tokio::task::spawn_blocking(move || ab_shortcuts::cli::open_pack(&pack))
        .await
        .map_err(|error| Failure::internal(&error))?
        .map_err(|error| {
            Failure::new(Reason::SpawnFailed, "Failed to open the Shortcuts pack.")
                .remote("Shortcuts", error.to_string())
        })?;
    ui::receipt(
        "Opened",
        &shown,
        &format!("Opened the Shortcuts pack at {shown}."),
    );
    ui::note("Import each shortcut in the Shortcuts app.");
    Ok(())
}

pub async fn join(home: &Path, args: &JoinArgs) -> Outcome {
    ab_trust::PairCode::decode(&args.code).map_err(|_| {
        Failure::new(
            Reason::InvalidValue,
            "Can't use that pairing code. Copy the whole code bridge pair printed.",
        )
    })?;
    if let Some(listen) = &args.listen {
        address("--listen", listen)?;
    }
    let authority = Arc::new(access::open(home)?);
    let result = if let Some(listen) = &args.listen {
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
        if !Modes::get().stdout_tty() {
            ui::outln(&absolute(home).display().to_string());
        }
        ui::receipt(
            "Joined",
            "the gateway as a replica",
            &format!(
                "Joined the gateway as a replica in {}.",
                absolute(home).display()
            ),
        );
        ui::row("Home", &absolute(home).display().to_string());
    }
    Ok(())
}

pub fn install_credentials(home: &Path) -> Outcome {
    access::open(home)?;
    if Modes::get().json {
        ui::json_document(json!({
            "outcome": "ready",
            "home": absolute(home),
            "next": [Step::new("Check this home", ["bridge", "status"]).to_json()],
        }));
    } else if Modes::get().stdout_tty() {
        ui::receipt(
            "Credentials ready",
            &absolute(home).display().to_string(),
            &format!("Credentials are ready in {}.", absolute(home).display()),
        );
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
            let resolves = std::net::ToSocketAddrs::to_socket_addrs(value.as_str())
                .is_ok_and(|mut found| found.next().is_some());
            if !resolves {
                return Err(Failure::new(
                    Reason::InvalidValue,
                    format!(
                        "Can't use {option} {}. That host doesn't resolve on this Mac.",
                        ui::clean(value)
                    ),
                ));
            }
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
            "Can't pair an iPhone. Tailscale access is off.",
        )
        .then("Turn it on", ["bridge", "tailscale"])
    };
    let status: Value = ab_remote::state::read(home, "status").map_err(|_| off())?;
    if status["state"] != "on" {
        return Err(off());
    }
    status["url"].as_str().map(str::to_owned).ok_or_else(off)
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
        let code = authority.pair_iphone_code(&url, true, ttl)?;
        return print_code(&code, true, ttl);
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
    let missing = |what: &str| {
        Failure::new(
            Reason::NotFound,
            format!("Couldn't find an active principal with {what}."),
        )
        .sentence("It may be revoked or expired, or it never existed in this home.")
        .then("See the agents with access", ["bridge", "agents", "list"])
    };
    let principal = match (&args.id, &args.fingerprint) {
        (Some(id), None) => authority
            .principal(id)
            .map_err(|_| missing(&format!("id {}", ui::clean(id))))?,
        (None, Some(fingerprint)) => authority
            .principal_by_fingerprint(fingerprint)
            .map_err(|_| missing(&format!("fingerprint {}", ui::clean(fingerprint))))?,
        _ => {
            return Err(Failure::usage(
                "Can't look up a principal. Pass exactly one of --id or --fingerprint.",
            ));
        }
    };
    ui::outln(&json!({"id":principal.id,"kind":principal.kind,"fingerprint":principal.fingerprint,"grants":principal.grants,"revision":principal.revision}).to_string());
    Ok(())
}
