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
        why: "Read your lists",
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
    asked: u32,
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

    fn ask_macos(&self) -> Outcome {
        match self {
            Self::Fixture(path) => {
                let mut fixture = Self::fixture(path)?;
                fixture.asked += 1;
                for id in fixture.grant_when_asked.clone() {
                    fixture.statuses.insert(id, "granted".to_owned());
                }
                Self::save(path, &fixture)
            }
            Self::Mac { helper, .. } => {
                if helper.is_empty() {
                    return Err(Failure::new("No Helper app was found to ask macOS with.")
                        .next("Install Apple Bridge, or pass --helper-app."));
                }
                product_cli::onboard(
                    Path::new("open"),
                    &product_cli::default_report_dir(),
                    helper,
                )?;
                Ok(())
            }
        }
    }
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
    let mut asked = false;
    for permission in LEVEL_ONE.iter().filter(|permission| permission.asks) {
        if backend.status(permission)? != "granted" && !asked {
            println!();
            println!("Contacts, Calendar and Reminders: macOS asks you in a small window.");
            if pause("  Press Enter to ask now, or type s to skip:", args.yes)? {
                backend.ask_macos()?;
            }
            asked = true;
        }
    }
    for permission in LEVEL_ONE {
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
