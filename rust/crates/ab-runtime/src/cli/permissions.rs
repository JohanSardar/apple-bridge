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

const WAIT_FOR_GRANT: Duration = Duration::from_secs(120);
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct Fixture {
    statuses: BTreeMap<String, String>,
    #[serde(default)]
    grant_when_opened: Vec<String>,
    #[serde(default)]
    grant_when_asked: Vec<String>,
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
            Self::Mac { home, .. } => source_status(home, permission.source),
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

    fn restart_session(&self) -> Outcome<SessionRestart> {
        let Self::Mac { home, helper } = self else {
            return Ok(SessionRestart::NotNeeded);
        };
        let pid = match std::fs::read_to_string(home.join("helper/helper.pid")) {
            Ok(pid) if is_session_helper(pid.trim(), helper, home) => pid.trim().to_owned(),
            _ => return Ok(SessionRestart::NotRunning),
        };
        let stopped = Command::new("/bin/kill")
            .args(["-TERM", &pid])
            .stderr(Stdio::null())
            .status()?
            .success();
        Ok(if stopped {
            SessionRestart::Stopped(pid)
        } else {
            SessionRestart::CouldNotStop
        })
    }

    fn session_replaced(&self, previous: &str) -> bool {
        let Self::Mac { home, helper } = self else {
            return true;
        };
        std::fs::read_to_string(home.join("helper/helper.pid"))
            .is_ok_and(|pid| pid.trim() != previous && is_session_helper(pid.trim(), helper, home))
    }

    fn ask_macos(&self, permission: &Permission) -> Outcome {
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
                Self::save(path, &fixture)
            }
            Self::Mac { helper, .. } => {
                if helper.is_empty() {
                    return Err(Failure::new("No Helper app was found to ask macOS with.")
                        .next("Install Apple Bridge, or pass --helper-app."));
                }
                product_cli::permissions_request(
                    Path::new("open"),
                    &product_cli::default_report_dir(),
                    helper,
                    permission.id,
                    None,
                )?;
                Ok(())
            }
        }
    }
}

fn is_session_command(command: &str, helper: &str, home: &Path) -> bool {
    let executable = format!("{helper}/Contents/MacOS/AppleBridgeHelper session ");
    let home_argument = format!(" --home {} --gateway-pid ", home.join("helper").display());
    !helper.is_empty() && command.starts_with(&executable) && command.contains(&home_argument)
}

fn is_session_helper(pid: &str, helper: &str, home: &Path) -> bool {
    pid.parse::<u32>().is_ok()
        && Command::new("ps")
            .args(["-p", pid, "-o", "command="])
            .output()
            .is_ok_and(|output| {
                is_session_command(String::from_utf8_lossy(&output.stdout).trim(), helper, home)
            })
}

fn source_status(home: &Path, source: &str) -> Outcome<String> {
    if !home.join("state.db").exists() {
        return Ok("unknown".to_owned());
    }
    let connection = rusqlite::Connection::open_with_flags(
        home.join("state.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| Failure::not_set_up(home))?;
    let snapshot = stats::read_states(&connection)?;
    Ok(match snapshot.get(source).map(String::as_str) {
        Some("ready" | "degraded") => "granted",
        Some("permission_missing") => "denied",
        _ => "unknown",
    }
    .to_owned())
}

fn label(status: &str) -> &'static str {
    match status {
        "granted" => "allowed",
        "denied" => "needs permission",
        _ => "not known yet",
    }
}

fn mark(status: &str) -> &'static str {
    match status {
        "granted" => "✓",
        "denied" => "!",
        _ => "?",
    }
}

fn print_checklist(backend: &Backend<'_>) -> Outcome {
    for permission in LEVEL_ONE {
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
        for permission in LEVEL_ONE {
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
    if wait_for(backend, permission).await? {
        println!("  ✓ {} allowed.", permission.title);
    } else {
        println!("  Not confirmed yet. It can take a restart: bridge down, then bridge up.");
    }
    Ok(())
}

#[derive(Default)]
struct Answers {
    asked: Vec<&'static Permission>,
    skipped: Vec<&'static str>,
}

fn ask_each(
    backend: &Backend<'_>,
    mut wants: impl FnMut(&Permission) -> Outcome<bool>,
) -> Outcome<Answers> {
    let mut answers = Answers::default();
    for permission in LEVEL_ONE.iter().filter(|permission| permission.asks) {
        if backend.status(permission)? == "granted" {
            continue;
        }
        if wants(permission)? {
            backend.ask_macos(permission)?;
            answers.asked.push(permission);
        } else {
            answers.skipped.push(permission.id);
        }
    }
    Ok(answers)
}

enum SessionRestart {
    NotNeeded,
    NotRunning,
    CouldNotStop,
    Stopped(String),
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
    let previous = match backend.restart_session()? {
        SessionRestart::NotNeeded => return Ok(NewGrants::Reading),
        SessionRestart::NotRunning => return Ok(NewGrants::NotRunning),
        SessionRestart::CouldNotStop => return Ok(NewGrants::CouldNotRestart),
        SessionRestart::Stopped(pid) => pid,
    };
    let deadline = Instant::now() + limit;
    loop {
        let replaced = backend.session_replaced(&previous);
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
    })?;
    let new_grants = let_new_grants_settle(&backend, &answers.asked, NEW_GRANT_SETTLES).await?;
    if let Some(report) = new_grants.report() {
        println!();
        println!("{report}");
    }
    for permission in LEVEL_ONE
        .iter()
        .filter(|permission| !answers.skipped.contains(&permission.id))
    {
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
            Ok(permission.id != "calendar")
        })?;
        assert_eq!(offered, ["contacts", "calendar", "reminders"]);
        assert_eq!(answers.skipped, ["calendar"]);
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

    fn wait_for_exit(child: &mut std::process::Child) -> Outcome<bool> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait()?.is_some() {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(false)
    }

    fn running_as(app: &Path, home: &Path) -> Outcome<std::process::Child> {
        let command = format!(
            "{}/Contents/MacOS/AppleBridgeHelper session --socket s --secret-file f --home {} --gateway-pid 7",
            app.display(),
            home.join("helper").display()
        );
        let child = Command::new("/bin/bash")
            .args(["-c", "exec -a \"$0\" sleep 30"])
            .arg(command)
            .spawn()?;
        std::fs::create_dir_all(home.join("helper"))?;
        std::fs::write(home.join("helper/helper.pid"), child.id().to_string())?;
        std::thread::sleep(Duration::from_millis(200));
        Ok(child)
    }

    #[test]
    fn the_session_is_matched_by_its_exact_executable_and_home() {
        let app = "/Apps/Apple Bridge.app";
        let home = Path::new("/tmp/world");
        let exact = "/Apps/Apple Bridge.app/Contents/MacOS/AppleBridgeHelper session --socket /tmp/world/run/peer.sock --secret-file /tmp/world/apple-secret --home /tmp/world/helper --gateway-pid 7";
        assert!(is_session_command(exact, app, home));
        assert!(is_session_command(exact, app, Path::new("/tmp/world/")));
        for other in [
            exact.replace("--home /tmp/world/helper", "--home /tmp/world2/helper"),
            exact.replace("--home /tmp/world/helper", "--home /tmp/world/helper2"),
            exact.replace("--home /tmp/world/helper", "--home /old/tmp/world/helper"),
            exact.replace("/Apps/Apple Bridge.app", "/Other/Apple Bridge.app"),
            exact.replace(" session ", " status "),
            format!("/bin/sh {exact}"),
        ] {
            assert!(!is_session_command(&other, app, home), "{other}");
        }
        assert!(!is_session_command(exact, "", home));
    }

    #[test]
    fn a_new_grant_restarts_only_this_homes_session_helper() -> Outcome {
        let directory = tempfile::tempdir()?;
        let app = directory.path().join("Apple Bridge.app");
        let home = directory.path().join("world");
        let backend = Backend::Mac {
            home: &home,
            helper: app.display().to_string(),
        };
        assert!(matches!(
            backend.restart_session()?,
            SessionRestart::NotRunning
        ));

        let mut helper = running_as(&app, &home)?;
        assert!(matches!(
            backend.restart_session()?,
            SessionRestart::Stopped(_)
        ));
        assert!(wait_for_exit(&mut helper)?);

        let collision = directory.path().join("world2");
        let mut foreign = running_as(&app, &collision)?;
        std::fs::write(
            home.join("helper/helper.pid"),
            std::fs::read(collision.join("helper/helper.pid"))?,
        )?;
        assert!(matches!(
            backend.restart_session()?,
            SessionRestart::NotRunning
        ));
        assert!(foreign.try_wait()?.is_none());
        foreign.kill()?;
        foreign.wait()?;

        std::fs::write(home.join("helper/helper.pid"), "1; rm -rf /")?;
        assert!(matches!(
            backend.restart_session()?,
            SessionRestart::NotRunning
        ));
        Ok(())
    }

    #[tokio::test]
    async fn settling_only_claims_what_it_confirmed() -> Outcome {
        let contacts: &'static Permission = &LEVEL_ONE[1];
        let directory = tempfile::tempdir()?;
        let app = directory.path().join("Apple Bridge.app");
        let home = directory.path().join("world");
        let backend = Backend::Mac {
            home: &home,
            helper: app.display().to_string(),
        };
        let quick = Duration::from_millis(50);
        assert_eq!(
            let_new_grants_settle(&backend, &[], quick).await?,
            NewGrants::NothingAsked
        );
        assert_eq!(
            let_new_grants_settle(&backend, &[contacts], quick).await?,
            NewGrants::NotRunning
        );

        let mut helper = running_as(&app, &home)?;
        assert_eq!(
            let_new_grants_settle(&backend, &[contacts], quick).await?,
            NewGrants::NoReplacement
        );
        assert!(wait_for_exit(&mut helper)?);

        let mut first = running_as(&app, &home)?;
        let stopped = first.id().to_string();
        assert!(matches!(
            backend.restart_session()?,
            SessionRestart::Stopped(_)
        ));
        assert!(wait_for_exit(&mut first)?);
        let mut replacement = running_as(&app, &home)?;
        assert!(backend.session_replaced(&stopped));
        assert!(!backend.session_replaced(&replacement.id().to_string()));
        replacement.kill()?;
        replacement.wait()?;

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
