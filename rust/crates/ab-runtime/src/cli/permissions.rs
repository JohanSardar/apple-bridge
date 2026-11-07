use super::{
    Failure, Outcome, stats,
    tree::{GuideArgs, GuideStatusArgs},
};
use crate::{Config, product_cli};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
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
        why: "Read Messages, Notes, Mail and Safari",
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
                        Failure::new("System Settings could not be opened.").next(format!(
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
                    return Err(Failure::new("No Helper app was found to ask macOS with.")
                        .next("Install Apple Bridge, or pass --helper-app."));
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
    let settings = if name == "notifications" {
        "System Settings > Notifications".to_owned()
    } else {
        format!("System Settings > Privacy & Security > {title}")
    };
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

fn mark(status: &str) -> &'static str {
    match status {
        "granted" => "✓",
        "denied" | "needs_permission" => "!",
        _ => "?",
    }
}

fn print_checklist(backend: &Backend<'_>) -> Outcome {
    for permission in permissions() {
        let status = backend.status(permission)?;
        println!(
            "  {} {:<18} {:<18} {}",
            mark(&status),
            permission.title,
            label(&status),
            permission.why
        );
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
    if args.json {
        let mut statuses = BTreeMap::new();
        for permission in permissions() {
            statuses.insert(permission.id, backend.status(permission)?);
        }
        println!("{}", serde_json::to_string(&statuses)?);
    } else {
        print_checklist(&backend)?;
    }
    Ok(())
}

fn pause(prompt: &str, yes: bool) -> Outcome<bool> {
    if yes {
        return Ok(true);
    }
    print!("{prompt} ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(!matches!(line.trim(), "s" | "skip" | "n" | "no"))
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

async fn settings_step(backend: &Backend<'_>, permission: &Permission, yes: bool) -> Outcome {
    if backend.status(permission)? == "granted" {
        return Ok(());
    }
    println!();
    println!("{}: {}.", permission.title, permission.why);
    println!(
        "  System Settings > Privacy & Security > {}: switch on Apple Bridge.",
        permission.title
    );
    if let Some(hint) = not_listed_hint(permission, backend.helper()) {
        println!("{hint}");
    }
    if !pause("  Press Enter to open it, or type s to skip:", yes)? {
        println!("  Skipped. Other sources still work.");
        return Ok(());
    }
    backend.open_settings(permission)?;
    println!(
        "  If Settings is not visible, click System Settings in the Dock or use Mission Control to find its Space."
    );
    println!(
        "  Waiting up to 10 minutes. You can press Ctrl-C and rerun bridge permissions guide; existing grants are kept."
    );
    if wait_for(backend, permission).await? {
        println!("  ✓ {} allowed.", permission.title);
    } else {
        println!(
            "  Not confirmed yet. Rerun bridge permissions guide to continue. If you allowed it, run bridge down, then bridge up."
        );
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
            println!("  {}", declined(permission.id));
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

pub async fn guide(home: &Path, args: &GuideArgs) -> Outcome {
    let backend = backend_for(home, args.fixture.as_ref(), args.helper_app.as_deref());
    println!("Mac permissions for Apple Bridge. You can skip any of them; the rest still work.");
    print_checklist(&backend)?;
    if !args.yes && !super::interactive() {
        println!();
        println!(
            "Nothing was changed or requested. Run this in a terminal to be walked through it."
        );
        return Ok(());
    }
    let answers = ask_each(&backend, |permission| {
        println!();
        println!(
            "{}: {}. macOS asks you in a small window.",
            permission.title, permission.why
        );
        let wanted = pause("  Press Enter to ask now, or type s to skip:", args.yes)?;
        if !wanted {
            println!("  Skipped. Other sources still work.");
        }
        Ok(wanted)
    });
    let new_grants = let_new_grants_settle(&backend, &answers.asked, NEW_GRANT_SETTLES).await;
    if let Some(report) = new_grants.as_ref().ok().and_then(NewGrants::report) {
        println!();
        println!("{report}");
    }
    if let Some(failure) = answers.failure {
        return Err(failure);
    }
    new_grants?;
    for permission in permissions().filter(|permission| !answers.skipped.contains(&permission.id)) {
        settings_step(&backend, permission, args.yes).await?;
    }
    println!();
    print_checklist(&backend)?;
    println!();
    println!("Follow the sync with: bridge stats --watch");
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
                "camera",
                "microphone",
                "location"
            ]
        );
        assert_eq!(
            answers.skipped,
            ["calendar", "camera", "microphone", "location"]
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
                return Err(Failure::new("macOS did not answer."));
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
