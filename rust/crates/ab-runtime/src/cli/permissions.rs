use super::{
    Failure, Outcome,
    failure::{Reason, Step},
    modes::Modes,
    stats,
    tree::{GuideArgs, GuideStatusArgs},
    ui,
};
use crate::{Config, product_cli};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const WAIT_FOR_GRANT: Duration = Duration::from_secs(600);
const POLL: Duration = Duration::from_secs(1);
const NEW_GRANT_SETTLES: Duration = Duration::from_secs(30);
const SETTINGS: &str = "x-apple.systempreferences:com.apple.preference.security?";

struct Permission {
    id: &'static str,
    title: &'static str,
    why: &'static str,
    source: &'static str,
    pane: &'static str,
    asks: bool,
}

const LEVEL_ONE: &[Permission] = &[
    Permission {
        id: "full_disk_access",
        title: "Full Disk Access",
        why: "Read Messages, Notes and Mail",
        source: "messages",
        pane: "Privacy_AllFiles",
        asks: false,
    },
    Permission {
        id: "contacts",
        title: "Contacts",
        why: "Show names instead of phone numbers",
        source: "contacts",
        pane: "Privacy_Contacts",
        asks: true,
    },
    Permission {
        id: "calendar",
        title: "Calendar",
        why: "Read your events",
        source: "calendar",
        pane: "Privacy_Calendars",
        asks: true,
    },
    Permission {
        id: "reminders",
        title: "Reminders",
        why: "Read your lists (needs Calendar too)",
        source: "reminders",
        pane: "Privacy_Reminders",
        asks: true,
    },
];

const OPTIONAL: &[Permission] = &[
    Permission {
        id: "photos",
        title: "Photos",
        why: "Read Photos live and change albums and favorites when asked (optional; no library copy)",
        source: "photos",
        pane: "Privacy_Photos",
        asks: true,
    },
    Permission {
        id: "screen_recording",
        title: "Screen & System Audio Recording",
        why: "Take screenshots and record the screen (optional)",
        source: "capture",
        pane: "Privacy_ScreenCapture",
        asks: false,
    },
    Permission {
        id: "camera",
        title: "Camera",
        why: "Take a camera photo (optional; needs a camera)",
        source: "capture",
        pane: "Privacy_Camera",
        asks: true,
    },
    Permission {
        id: "microphone",
        title: "Microphone",
        why: "Record audio (optional; needs a microphone)",
        source: "capture",
        pane: "Privacy_Microphone",
        asks: true,
    },
    Permission {
        id: "location",
        title: "Location Services",
        why: "Get the current location (optional; needs a location fix)",
        source: "location",
        pane: "Privacy_LocationServices",
        asks: true,
    },
];

fn permissions() -> impl Iterator<Item = &'static Permission> {
    LEVEL_ONE.iter().chain(OPTIONAL)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Fixture {
    statuses: BTreeMap<String, String>,
    #[serde(default)]
    grant_when_opened: Vec<String>,
    #[serde(default)]
    grant_when_asked: Vec<String>,
    #[serde(default)]
    deny_when_asked: Vec<String>,
    #[serde(default)]
    opened: Vec<String>,
    #[serde(default)]
    asked: Vec<String>,
}

enum Backend<'a> {
    Mac { home: &'a Path, helper: String },
    Fixture(PathBuf),
}

impl Backend<'_> {
    fn fixture(path: &Path) -> Outcome<Fixture> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    fn save(path: &Path, fixture: &Fixture) -> Outcome {
        std::fs::write(path, serde_json::to_vec(fixture)?)?;
        Ok(())
    }

    fn helper(&self) -> &str {
        match self {
            Self::Mac { helper, .. } => helper,
            Self::Fixture(_) => "",
        }
    }

    fn status(&self, permission: &Permission) -> Outcome<String> {
        match self {
            Self::Fixture(path) => Ok(Self::fixture(path)?
                .statuses
                .get(permission.id)
                .cloned()
                .unwrap_or_else(|| "unknown".to_owned())),
            Self::Mac { home, .. } => source_status(home, permission),
        }
    }

    fn open_settings(&self, permission: &Permission) -> Outcome {
        match self {
            Self::Fixture(path) => {
                let mut fixture = Self::fixture(path)?;
                fixture.opened.push(permission.id.to_owned());
                if fixture
                    .grant_when_opened
                    .iter()
                    .any(|id| id == permission.id)
                {
                    fixture
                        .statuses
                        .insert(permission.id.to_owned(), "granted".to_owned());
                }
                Self::save(path, &fixture)
            }
            Self::Mac { .. } => {
                let succeeded = Command::new("open")
                    .arg(format!("{SETTINGS}{}", permission.pane))
                    .stdin(Stdio::null())
                    .status()?
                    .success();
                if succeeded {
                    Ok(())
                } else {
                    Err(
                        Failure::new(Reason::SpawnFailed, "Couldn't open System Settings.")
                            .sentence(format!(
                                "Open Privacy & Security > {} yourself.",
                                permission.title
                            )),
                    )
                }
            }
        }
    }

    async fn restart_session(&self) -> Outcome<SessionRestart> {
        let Self::Mac { home, .. } = self else {
            return Ok(SessionRestart::NotNeeded);
        };
        if !home.join("run/control.sock").exists() {
            return Ok(SessionRestart::NotRunning);
        }
        Ok(match crate::control::restart_helper(home).await {
            Ok(fence) if fence > 0 => SessionRestart::Stopped(fence),
            _ => SessionRestart::CouldNotStop,
        })
    }

    async fn session_replaced(&self, previous: u64) -> bool {
        let Self::Mac { home, .. } = self else {
            return true;
        };
        crate::control::helper_fence(home)
            .await
            .is_ok_and(|fence| fence > previous)
    }

    fn ask_macos(&self, permission: &Permission) -> Outcome<bool> {
        match self {
            Self::Fixture(path) => {
                let mut fixture = Self::fixture(path)?;
                fixture.asked.push(permission.id.to_owned());
                if fixture
                    .grant_when_asked
                    .iter()
                    .any(|id| id == permission.id)
                {
                    fixture
                        .statuses
                        .insert(permission.id.to_owned(), "granted".to_owned());
                }
                let denied = fixture.deny_when_asked.iter().any(|id| id == permission.id);
                if denied {
                    fixture
                        .statuses
                        .insert(permission.id.to_owned(), "denied".to_owned());
                }
                Self::save(path, &fixture)?;
                Ok(denied)
            }
            Self::Mac { helper, .. } => {
                if helper.is_empty() {
                    return Err(Failure::new(
                        Reason::HelperFailed,
                        "Can't ask macOS. No Helper app was found.",
                    )
                    .sentence("Install Apple Bridge, or pass --helper-app."));
                }
                let report = product_cli::permissions_request(
                    Path::new("open"),
                    &product_cli::default_report_dir(),
                    helper,
                    permission.id,
                    None,
                )?;
                Ok(report
                    .get("outcome")
                    .is_some_and(|outcome| outcome == "denied"))
            }
        }
    }
}

fn source_status(home: &Path, permission: &Permission) -> Outcome<String> {
    if !home.join("state.db").exists() {
        return Ok("unknown".to_owned());
    }
    let connection = rusqlite::Connection::open_with_flags(
        home.join("state.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| Failure::not_set_up(home))?;
    if permission.source == "capture" {
        let mut statement = connection.prepare(
            "SELECT json_extract(CAST(payload AS TEXT),'$.data.coverage.permissions') FROM records WHERE collection='system.source' AND deleted=0 AND json_extract(CAST(payload AS TEXT),'$.data.source')='capture'",
        ).map_err(|_| Failure::not_set_up(home))?;
        let rows = statement
            .query_map([], |row| row.get::<_, Option<String>>(0))
            .map_err(|_| Failure::not_set_up(home))?;
        for row in rows {
            if let Some(value) = row.map_err(|_| Failure::not_set_up(home))? {
                let states: Option<BTreeMap<String, bool>> = serde_json::from_str(&value)?;
                if let Some(allowed) = states.as_ref().and_then(|states| states.get(permission.id))
                {
                    return Ok(if *allowed {
                        "granted"
                    } else {
                        "needs_permission"
                    }
                    .to_owned());
                }
            }
        }
        return Ok("unknown".to_owned());
    }
    let snapshot = stats::read_states(&connection)?;
    Ok(match snapshot.get(permission.source).map(String::as_str) {
        Some("ready" | "degraded") => "granted",
        Some("permission_missing") => "needs_permission",
        _ => "unknown",
    }
    .to_owned())
}

pub(super) fn declined(name: &str) -> String {
    let title = permissions()
        .find(|permission| permission.id == name)
        .map_or_else(
            || match name {
                "screen_recording" => "Screen & System Audio Recording".to_owned(),
                "speech_recognition" => "Speech Recognition".to_owned(),
                name if name.starts_with("automation_") => "Automation".to_owned(),
                _ => name.replace('_', " "),
            },
            |permission| permission.title.to_owned(),
        );
    let settings = format!("System Settings > Privacy & Security > {title}");
    format!(
        "{title} permission was declined. Other sources still work. To turn it on, open {settings}, enable Apple Bridge, then run bridge down and bridge up."
    )
}

fn label(status: &str) -> &'static str {
    match status {
        "granted" => "allowed",
        "denied" | "needs_permission" => "needs permission",
        _ => "not known yet",
    }
}

fn checklist(backend: &Backend<'_>) -> Outcome<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    for permission in permissions() {
        let status = backend.status(permission)?;
        rows.push(vec![
            permission.title.to_owned(),
            label(&status).to_owned(),
            permission.why.to_owned(),
        ]);
    }
    Ok(rows)
}

fn print_checklist(backend: &Backend<'_>) -> Outcome {
    ui::wrapped_table(&["Permission", "Status", "Used for"], &checklist(backend)?);
    Ok(())
}

fn print_checklist_stderr(backend: &Backend<'_>) -> Outcome {
    for row in checklist(backend)? {
        let mark = match (row[1].as_str(), Modes::get().rich()) {
            ("allowed", true) => "✓",
            ("allowed", false) => "ok",
            ("needs permission", true) => "!",
            ("needs permission", false) => "!!",
            _ => "-",
        };
        ui::err_line(&format!("{mark} {:<32} {}", row[0], row[1]));
    }
    Ok(())
}

fn backend_for<'a>(home: &'a Path, fixture: Option<&PathBuf>, helper: Option<&str>) -> Backend<'a> {
    if let Some(path) = fixture {
        return Backend::Fixture(path.clone());
    }
    let saved = Config::load(home)
        .map(|config| config.helper_app)
        .unwrap_or_default();
    let helper = helper.map_or(saved, str::to_owned);
    let helper = crate::bundle::helper_app(&helper, std::env::current_exe().ok().as_deref());
    Backend::Mac { home, helper }
}

pub fn status(home: &Path, args: &GuideStatusArgs) -> Outcome {
    let backend = backend_for(home, args.fixture.as_ref(), None);
    if Modes::get().json {
        let mut statuses = BTreeMap::new();
        for permission in permissions() {
            statuses.insert(permission.id, backend.status(permission)?);
        }
        ui::json_document(serde_json::to_value(&statuses)?);
    } else {
        print_checklist(&backend)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Consent<'a> {
    Ask,
    Everything,
    Only(&'a [&'static str]),
}

impl Consent<'_> {
    fn covers(self, permission: &Permission) -> Option<bool> {
        match self {
            Self::Ask => None,
            Self::Everything => Some(true),
            Self::Only(names) => Some(names.contains(&permission.id)),
        }
    }
}

fn pause(prompt: &str, consent: Consent<'_>, permission: &Permission) -> Outcome<bool> {
    if let Some(decided) = consent.covers(permission) {
        return Ok(decided);
    }
    if Modes::get().accessible {
        let choice = ui::choose(
            &format!("{}: what next?", permission.title),
            &[("", "Continue"), ("s", "Skip")],
        )?;
        return match choice {
            Some(index) => Ok(index == 0),
            None => Err(Failure::canceled(
                "Stopped: no answer (end of input). Nothing more was requested; grants already made are kept.",
            )),
        };
    }
    let Some(answer) = ui::ask(prompt)? else {
        return Err(Failure::canceled(
            "Stopped: no answer (end of input). Nothing more was requested; grants already made are kept.",
        ));
    };
    Ok(!matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "skip" | "n" | "no"
    ))
}

async fn wait_for(backend: &Backend<'_>, permission: &Permission) -> Outcome<bool> {
    let deadline = Instant::now() + WAIT_FOR_GRANT;
    while Instant::now() < deadline {
        if backend.status(permission)? == "granted" {
            return Ok(true);
        }
        if matches!(backend, Backend::Fixture(_)) {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
    Ok(false)
}

fn not_listed_hint(permission: &Permission, helper: &str) -> Option<String> {
    (!permission.asks && !helper.is_empty())
        .then(|| format!("  Not in the list? Click +, press Command-Shift-G and paste: {helper}"))
}

async fn settings_step(
    backend: &Backend<'_>,
    permission: &Permission,
    consent: Consent<'_>,
) -> Outcome {
    if backend.status(permission)? == "granted" || consent.covers(permission) == Some(false) {
        return Ok(());
    }
    ui::blank();
    ui::note(&format!("{}: {}.", permission.title, permission.why));
    ui::note(&format!(
        "System Settings > Privacy & Security > {}: switch on Apple Bridge.",
        permission.title
    ));
    if let Some(hint) = not_listed_hint(permission, backend.helper()) {
        ui::note(hint.trim());
    }
    if !Modes::get().interactive() && !matches!(backend, Backend::Fixture(_)) {
        ui::warn(
            "needs_person",
            &format!(
                "{} can only be switched on by a person in System Settings.",
                permission.title
            ),
        );
        return Ok(());
    }
    if !pause(
        "Press Enter to open it, or type s to skip:",
        consent,
        permission,
    )? {
        ui::note("Skipped. Other sources still work.");
        return Ok(());
    }
    backend.open_settings(permission)?;
    ui::note(
        "If Settings isn't visible, click System Settings in the Dock or use Mission Control to find its Space.",
    );
    ui::note("Waiting up to 10 minutes. Press Ctrl-C to stop; grants already made are kept.");
    if wait_for(backend, permission).await? {
        ui::done("Allowed", permission.title);
    } else {
        ui::warn(
            "not_confirmed",
            &format!(
                "{} isn't confirmed yet. If you allowed it, restart Apple Bridge so it takes effect.",
                permission.title
            ),
        );
        ui::next_steps(&[
            Step::new("Continue with the rest", ["bridge", "permissions", "guide"]),
            Step::new("Or restart Apple Bridge", ["bridge", "down"]),
            Step::new("Then start it again", ["bridge", "up"]),
        ]);
    }
    Ok(())
}

#[derive(Default)]
struct Answers {
    asked: Vec<&'static Permission>,
    skipped: Vec<&'static str>,
    failure: Option<Failure>,
}

fn ask_each(backend: &Backend<'_>, mut wants: impl FnMut(&Permission) -> Outcome<bool>) -> Answers {
    let mut answers = Answers::default();
    for permission in permissions().filter(|permission| permission.asks) {
        if let Err(failure) = ask_one(backend, permission, &mut wants, &mut answers) {
            answers.failure = Some(failure);
            break;
        }
    }
    answers
}

fn ask_one(
    backend: &Backend<'_>,
    permission: &'static Permission,
    wants: &mut impl FnMut(&Permission) -> Outcome<bool>,
    answers: &mut Answers,
) -> Outcome {
    if backend.status(permission)? == "granted" {
        return Ok(());
    }
    if wants(permission)? {
        if backend.ask_macos(permission)? {
            ui::warn("permission_denied", &declined(permission.id));
            answers.skipped.push(permission.id);
        } else {
            answers.asked.push(permission);
        }
    } else {
        answers.skipped.push(permission.id);
    }
    Ok(())
}

enum SessionRestart {
    NotNeeded,
    NotRunning,
    CouldNotStop,
    Stopped(u64),
}

#[derive(Debug, PartialEq, Eq)]
enum NewGrants {
    NothingAsked,
    Reading,
    NotRunning,
    CouldNotRestart,
    NoReplacement,
    NotConfirmed(Vec<&'static str>),
}

impl NewGrants {
    fn report(&self) -> Option<String> {
        const RESTART: &str = "Run bridge down, then bridge up.";
        match self {
            Self::NothingAsked | Self::Reading => None,
            Self::NotRunning => Some(
                "Apple Bridge is not running here, so nothing new is being read. New permissions start with bridge up.".to_owned(),
            ),
            Self::CouldNotRestart => Some(format!(
                "The Helper could not be restarted, so new permissions are not being read yet. {RESTART}"
            )),
            Self::NoReplacement => Some(format!(
                "The Helper did not come back, so nothing new is being read yet. {RESTART}"
            )),
            Self::NotConfirmed(titles) => Some(format!(
                "Not confirmed: {}. If you allowed it and it stays this way, {}",
                titles.join(", "),
                RESTART.to_lowercase()
            )),
        }
    }
}

async fn let_new_grants_settle(
    backend: &Backend<'_>,
    asked: &[&'static Permission],
    limit: Duration,
) -> Outcome<NewGrants> {
    if asked.is_empty() {
        return Ok(NewGrants::NothingAsked);
    }
    let previous = match backend.restart_session().await? {
        SessionRestart::NotNeeded => return Ok(NewGrants::Reading),
        SessionRestart::NotRunning => return Ok(NewGrants::NotRunning),
        SessionRestart::CouldNotStop => return Ok(NewGrants::CouldNotRestart),
        SessionRestart::Stopped(fence) => fence,
    };
    let deadline = Instant::now() + limit;
    loop {
        let replaced = backend.session_replaced(previous).await;
        let mut waiting = Vec::new();
        for permission in asked {
            if backend.status(permission)? != "granted" {
                waiting.push(permission.title);
            }
        }
        if replaced && waiting.is_empty() {
            return Ok(NewGrants::Reading);
        }
        if Instant::now() >= deadline {
            return Ok(if replaced {
                NewGrants::NotConfirmed(waiting)
            } else {
                NewGrants::NoReplacement
            });
        }
        tokio::time::sleep(POLL.min(limit)).await;
    }
}

fn guided(name: &str) -> Option<&'static Permission> {
    permissions().find(|permission| permission.id == name)
}

pub async fn after_request(home: &Path, name: &str, helper: Option<&str>) -> Outcome {
    let Some(permission) = guided(name) else {
        return Ok(());
    };
    let backend = backend_for(home, None, helper);
    let settled = let_new_grants_settle(&backend, &[permission], NEW_GRANT_SETTLES).await?;
    if let Some(report) = settled.report() {
        ui::warn("not_reading", &report);
    }
    Ok(())
}

fn only(names: &str) -> Outcome<Vec<&'static str>> {
    let mut chosen = Vec::new();
    let mut unknown = Vec::new();
    for name in names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        match permissions().find(|permission| permission.id == name) {
            Some(permission) => chosen.push(permission.id),
            None => unknown.push(name.to_owned()),
        }
    }
    if !unknown.is_empty() {
        let accepted: Vec<&str> = permissions().map(|permission| permission.id).collect();
        return Err(Failure::new(
            Reason::InvalidValue,
            format!(
                "Can't use --only {}. Not permission names: {}.",
                ui::clean(names),
                ui::clean(&unknown.join(", "))
            ),
        )
        .row("Accepted", accepted.join(", ")));
    }
    if chosen.is_empty() {
        return Err(Failure::new(
            Reason::MissingArgument,
            "Can't use an empty --only. Name the permissions to request.",
        ));
    }
    Ok(chosen)
}

const WINDOW_STEPS: [&str; 6] = [
    "contacts",
    "calendar",
    "reminders",
    "photos",
    "full_disk_access",
    "screen_recording",
];

fn window_steps(named: Option<&[&'static str]>) -> Vec<&'static str> {
    named.map_or_else(
        || WINDOW_STEPS.to_vec(),
        |chosen| {
            chosen
                .iter()
                .copied()
                .filter(|id| product_cli::GUIDE_STEPS.contains(id))
                .collect()
        },
    )
}

fn listed(report: &BTreeMap<String, String>, key: &str) -> Vec<&'static Permission> {
    let names: Vec<&str> = report
        .get(key)
        .map(|text| text.split(',').filter(|name| !name.is_empty()).collect())
        .unwrap_or_default();
    permissions()
        .filter(|permission| names.contains(&permission.id))
        .collect()
}

async fn window_guide(home: &Path, helper: &str, steps: &[&str]) -> Outcome {
    let backend = Backend::Mac {
        home,
        helper: helper.to_owned(),
    };
    let was_running = super::gateway::lock_held(home);
    ui::note(
        "Opening the permission guide. Follow the small window next to System Settings; it closes by itself when you're done.",
    );
    let program = Path::new("open");
    let report_dir = product_cli::default_report_dir();
    let helper_path = helper.to_owned();
    let chosen: Vec<&'static str> = product_cli::GUIDE_STEPS
        .iter()
        .copied()
        .filter(|id| steps.contains(id))
        .collect();
    let report = tokio::task::spawn_blocking(move || {
        product_cli::permission_guide(program, &report_dir, &helper_path, &chosen)
    })
    .await
    .map_err(|_| {
        Failure::new(
            Reason::HelperFailed,
            "The guide window stopped unexpectedly.",
        )
    })?
    .map_err(|error| {
        Failure::new(Reason::HelperFailed, "The guide window didn't finish.")
            .row("Detail", ui::clean(&error.to_string()))
            .then(
                "Continue where you left off",
                ["bridge", "permissions", "guide"],
            )
    })?;
    let allowed = listed(&report, "allowed");
    if was_running && !super::gateway::lock_held(home) {
        restart_after_quit(home);
    }
    let settled = let_new_grants_settle(&backend, &allowed, NEW_GRANT_SETTLES).await?;
    if let Some(note) = settled.report() {
        ui::warn("not_reading", &note);
    }
    if Modes::get().json {
        let mut statuses = BTreeMap::new();
        for permission in permissions() {
            statuses.insert(permission.id, backend.status(permission)?);
        }
        ui::json_document(serde_json::json!({
            "permissions": statuses,
            "allowed": allowed.iter().map(|permission| permission.id).collect::<Vec<_>>(),
            "skipped": listed(&report, "skipped").iter().chain(listed(&report, "declined").iter()).map(|permission| permission.id).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if allowed.is_empty() {
        ui::note("Nothing new was allowed. Run bridge permissions guide again any time.");
    } else {
        ui::done(
            "Allowed",
            &allowed
                .iter()
                .map(|permission| permission.title)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    ui::next_steps(&[Step::new("Follow the sync", ["bridge", "stats", "--wait"])]);
    Ok(())
}

fn restart_after_quit(home: &Path) {
    let Ok(bridge) = std::env::current_exe() else {
        return;
    };
    let started = Command::new(bridge)
        .arg("up")
        .arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if started {
        ui::note("macOS quit Apple Bridge to apply a permission, so it was started again.");
    } else {
        ui::warn(
            "not_running",
            "macOS quit Apple Bridge to apply a permission. Run bridge up to start it again.",
        );
    }
}

pub async fn guide(home: &Path, args: &GuideArgs) -> Outcome {
    let named = args.only.as_deref().map(only).transpose()?;
    if args.fixture.is_none()
        && !args.in_terminal
        && cfg!(target_os = "macos")
        && let Backend::Mac { helper, .. } = backend_for(home, None, args.helper_app.as_deref())
        && !helper.is_empty()
    {
        let steps = window_steps(named.as_deref());
        if !steps.is_empty() {
            return window_guide(home, &helper, &steps).await;
        }
    }
    let modes = Modes::get();
    let consent = match &named {
        Some(chosen) => Consent::Only(chosen),
        None if modes.yes => Consent::Everything,
        None if modes.interactive() => Consent::Ask,
        None => {
            let backend = backend_for(home, args.fixture.as_ref(), args.helper_app.as_deref());
            let rows = checklist(&backend)?;
            if !modes.json {
                ui::table(&["Permission", "Status", "Used for"], &rows);
            }
            let listed: Vec<serde_json::Value> = rows
                .iter()
                .map(|row| serde_json::json!({"permission": row[0], "status": row[1], "usedFor": row[2]}))
                .collect();
            return Err(Failure::new(Reason::TerminalRequired, "Can't walk through permissions without a terminal.")
                .sentence(format!("Nothing was requested ({}). Name the ones to request with --only, such as --only contacts,calendar.", modes.why_not_interactive()))
                .data("permissions", listed)
                .then("Request only the ones you name", ["bridge", "permissions", "guide", "--only", "contacts,calendar"]));
        }
    };
    let backend = backend_for(home, args.fixture.as_ref(), args.helper_app.as_deref());
    ui::set_cancel_note(Some(
        "Canceled. Nothing more was requested; grants already made are kept.",
    ));
    ui::note("Mac permissions for Apple Bridge. You can skip any of them; the rest still work.");
    print_checklist_stderr(&backend)?;
    let answers = ask_each(&backend, |permission| {
        if let Some(decided) = consent.covers(permission) {
            return Ok(decided);
        }
        ui::blank();
        ui::note(&format!(
            "{}: {}. macOS asks you in a small window.",
            permission.title, permission.why
        ));
        let wanted = pause(
            "Press Enter to ask now, or type s to skip:",
            consent,
            permission,
        )?;
        if !wanted {
            ui::note("Skipped. Other sources still work.");
        }
        Ok(wanted)
    });
    if answers
        .failure
        .as_ref()
        .is_some_and(|failure| failure.reason == Reason::Canceled)
    {
        return Err(answers
            .failure
            .unwrap_or_else(|| Failure::canceled("Canceled.")));
    }
    let new_grants = let_new_grants_settle(&backend, &answers.asked, NEW_GRANT_SETTLES).await;
    if let Some(report) = new_grants.as_ref().ok().and_then(NewGrants::report) {
        ui::warn("not_reading", &report);
    }
    if let Some(failure) = answers.failure {
        return Err(failure);
    }
    new_grants?;
    for permission in permissions().filter(|permission| !answers.skipped.contains(&permission.id)) {
        settings_step(&backend, permission, consent).await?;
    }
    ui::set_cancel_note(None);
    if modes.json {
        let mut statuses = BTreeMap::new();
        for permission in permissions() {
            statuses.insert(permission.id, backend.status(permission)?);
        }
        ui::json_document(
            serde_json::json!({"permissions": statuses, "requested": answers.asked.iter().map(|permission| permission.id).collect::<Vec<_>>(), "skipped": answers.skipped}),
        );
        return Ok(());
    }
    ui::blank();
    print_checklist(&backend)?;
    ui::next_steps(&[Step::new("Follow the sync", ["bridge", "stats", "--wait"])]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_permission_is_asked_on_its_own_and_a_skip_asks_nothing() -> Outcome {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("fixture.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "statuses": {"contacts":"unknown","calendar":"unknown","reminders":"unknown"},
                "grant_when_asked": ["contacts", "calendar", "reminders"],
            }))?,
        )?;
        let backend = Backend::Fixture(path.clone());
        let mut offered = Vec::new();
        let answers = ask_each(&backend, |permission| {
            offered.push(permission.id);
            Ok(matches!(permission.id, "contacts" | "reminders"))
        });
        assert_eq!(
            offered,
            [
                "contacts",
                "calendar",
                "reminders",
                "photos",
                "camera",
                "microphone",
                "location"
            ]
        );
        assert_eq!(
            answers.skipped,
            ["calendar", "photos", "camera", "microphone", "location"]
        );
        assert_eq!(
            answers.asked.iter().map(|p| p.id).collect::<Vec<_>>(),
            ["contacts", "reminders"]
        );
        let fixture = Backend::fixture(&path)?;
        assert_eq!(fixture.asked, ["contacts", "reminders"]);
        assert_eq!(fixture.statuses["contacts"], "granted");
        assert_eq!(fixture.statuses["calendar"], "unknown");
        assert_eq!(fixture.statuses["reminders"], "granted");
        Ok(())
    }

    #[test]
    fn a_later_failure_keeps_the_grants_already_made() -> Outcome {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("fixture.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "statuses": {"contacts":"unknown","calendar":"unknown","reminders":"unknown"},
                "grant_when_asked": ["contacts", "calendar", "reminders"],
            }))?,
        )?;
        let backend = Backend::Fixture(path);
        let answers = ask_each(&backend, |permission| {
            if permission.id == "calendar" {
                return Err(Failure::new(Reason::HelperFailed, "macOS did not answer."));
            }
            Ok(true)
        });
        let asked: Vec<_> = answers
            .asked
            .iter()
            .map(|permission| permission.id)
            .collect();
        assert_eq!(asked, ["contacts"]);
        assert!(answers.failure.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn a_stale_pid_file_is_never_used_to_restart_a_process() -> Outcome {
        let directory = tempfile::tempdir()?;
        std::fs::create_dir_all(directory.path().join("helper"))?;
        let mut foreign = Command::new("sleep").arg("30").spawn()?;
        std::fs::write(
            directory.path().join("helper/helper.pid"),
            foreign.id().to_string(),
        )?;
        let backend = Backend::Mac {
            home: directory.path(),
            helper: "Fixture.app".into(),
        };
        assert!(matches!(
            backend.restart_session().await?,
            SessionRestart::NotRunning
        ));
        assert!(foreign.try_wait()?.is_none());
        foreign.kill()?;
        foreign.wait()?;
        Ok(())
    }

    #[tokio::test]
    async fn settling_only_claims_what_it_confirmed() -> Outcome {
        let directory = tempfile::tempdir()?;
        let backend = Backend::Mac {
            home: directory.path(),
            helper: "Fixture.app".into(),
        };
        let quick = Duration::from_millis(50);
        assert_eq!(
            let_new_grants_settle(&backend, &[], quick).await?,
            NewGrants::NothingAsked
        );
        assert_eq!(
            let_new_grants_settle(&backend, &[&LEVEL_ONE[1]], quick).await?,
            NewGrants::NotRunning
        );
        assert!(!backend.session_replaced(1).await);
        for outcome in [
            NewGrants::NotRunning,
            NewGrants::CouldNotRestart,
            NewGrants::NoReplacement,
            NewGrants::NotConfirmed(vec!["Contacts"]),
        ] {
            let report = outcome.report().unwrap_or_default();
            assert!(report.contains("bridge up"), "{report}");
        }
        assert_eq!(NewGrants::Reading.report(), None);
        Ok(())
    }

    #[test]
    fn each_requestable_permission_the_guide_offers_restarts_the_helper_after_a_request() {
        for name in [
            "contacts",
            "calendar",
            "reminders",
            "photos",
            "camera",
            "microphone",
            "location",
        ] {
            assert!(product_cli::PERMISSIONS.contains(&name), "{name}");
            assert_eq!(guided(name).map(|permission| permission.id), Some(name));
        }
        assert!(guided("focus").is_none());
    }

    #[tokio::test]
    async fn a_request_with_nothing_running_restarts_nothing() -> Outcome {
        let directory = tempfile::tempdir()?;
        after_request(directory.path(), "photos", Some("Fixture.app")).await?;
        after_request(directory.path(), "focus", Some("Fixture.app")).await?;
        assert!(!directory.path().join("run").exists());
        Ok(())
    }

    #[test]
    fn only_a_settings_only_permission_with_a_known_app_gets_the_add_hint() {
        let [full_disk, contacts, ..] = LEVEL_ONE else {
            panic!("level one lists Full Disk Access and Contacts first");
        };
        let app = "/opt/Apple Bridge.app";
        let hint = not_listed_hint(full_disk, app).unwrap_or_default();
        assert!(hint.ends_with(app), "{hint}");
        assert_eq!(not_listed_hint(contacts, app), None);
        assert_eq!(not_listed_hint(full_disk, ""), None);
    }
}
