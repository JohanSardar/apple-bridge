use crate::Error;
use rand::RngCore;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const PERMISSIONS: [&str; 14] = [
    "contacts",
    "calendar",
    "reminders",
    "location",
    "camera",
    "microphone",
    "screen_recording",
    "speech_recognition",
    "focus",
    "bluetooth",
    "notifications",
    "automation_mail",
    "automation_safari",
    "automation_shortcuts_events",
];
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(180);
const SELFTEST_TIMEOUT: Duration = Duration::from_secs(600);
const GATEWAY_PLIST: &str = "Contents/Library/LaunchAgents/dev.applebridge.gateway.plist";

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn default_helper_app() -> PathBuf {
    home_dir().join("Applications/AppleBridgeDev/AppleBridgeHelper.app")
}

pub fn default_report_dir() -> PathBuf {
    home_dir().join("Library/Application Support/AppleBridgeDev")
}

pub fn bundle_of(executable: &Path) -> Option<PathBuf> {
    let bundle = executable.parent()?.parent()?.parent()?;
    bundle
        .join(GATEWAY_PLIST)
        .exists()
        .then(|| bundle.to_owned())
}

pub fn detect_helper_app() -> Option<PathBuf> {
    bundle_of(&std::env::current_exe().ok()?)
}

fn run_id() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("run_{}", data_encoding::HEXLOWER.encode(&bytes))
}

fn helper_or_default(app: &str) -> String {
    if app.is_empty() {
        default_helper_app().to_string_lossy().into_owned()
    } else {
        app.to_owned()
    }
}

pub fn terminal_qr(text: &str) -> Result<String, Error> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L)
        .map_err(|_| Error::Invalid)?;
    let mut rendered = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    rendered.push('\n');
    Ok(rendered)
}

pub fn permission_arguments(app: &str, name: &str, report: &Path) -> Vec<String> {
    let mut arguments: Vec<String> = ["-n", "-W", "-g", "-a"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    arguments.push(helper_or_default(app));
    arguments.extend(["--args", "permission-request"].map(str::to_owned));
    arguments.push(name.to_owned());
    arguments.push("--report".into());
    arguments.push(report.to_string_lossy().into_owned());
    arguments
}

pub fn messages_selftest_arguments(
    app: &str,
    report: &Path,
    include_ui: bool,
    force: bool,
) -> Vec<String> {
    let mut arguments: Vec<String> = ["-n", "-W", "-g", "-a"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    arguments.push(helper_or_default(app));
    arguments.extend(["--args", "selftest", "messages", "--report"].map(str::to_owned));
    arguments.push(report.to_string_lossy().into_owned());
    if !include_ui {
        arguments.push("--no-ui".into());
    }
    if force {
        arguments.push("--force".into());
    }
    arguments
}

fn run_bounded(program: &Path, arguments: &[String], timeout: Duration) -> Result<(), Error> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let mut message = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut message);
            }
            return Err(Error::Io(std::io::Error::other(format!(
                "{} exited with {status}: {}",
                program.display(),
                message.trim()
            ))));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "helper request timed out",
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Report(PathBuf);

impl Drop for Report {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn permissions_request(
    program: &Path,
    report_dir: &Path,
    app: &str,
    name: &str,
    timeout: Option<Duration>,
) -> Result<BTreeMap<String, String>, Error> {
    if !PERMISSIONS.contains(&name) {
        return Err(Error::Invalid);
    }
    let report = Report(report_dir.join(format!("permissions-{}.json", run_id())));
    run_bounded(
        program,
        &permission_arguments(app, name, &report.0),
        timeout.unwrap_or(PERMISSION_TIMEOUT),
    )?;
    let states: BTreeMap<String, String> = serde_json::from_slice(&std::fs::read(&report.0)?)?;
    if states.get("outcome").is_some_and(|outcome| outcome == "timed_out") {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("nobody answered the macOS prompt for {name} in time; run this again to retry"),
        )));
    }
    Ok(states)
}

#[derive(Debug)]
pub struct SelfTest {
    pub data: String,
    pub state: String,
}

impl SelfTest {
    pub fn proven(&self) -> bool {
        self.state == "proven"
    }
}

pub fn messages_selftest(
    program: &Path,
    report_dir: &Path,
    app: &str,
    force: bool,
    include_ui: bool,
    timeout: Option<Duration>,
) -> Result<SelfTest, Error> {
    if !cfg!(target_os = "macos") && program == Path::new("open") {
        return Err(Error::Invalid);
    }
    let report = Report(report_dir.join(format!("messages-selftest-cli-{}.json", run_id())));
    run_bounded(
        program,
        &messages_selftest_arguments(app, &report.0, include_ui, force),
        timeout.unwrap_or(SELFTEST_TIMEOUT),
    )?;
    let data = std::fs::read_to_string(&report.0)?;
    let state = serde_json::from_str::<serde_json::Value>(&data)
        .ok()
        .and_then(|value| value["state"].as_str().map(str::to_owned))
        .filter(|state| !state.is_empty())
        .ok_or(Error::Invalid)?;
    Ok(SelfTest { data, state })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    type Check = Result<(), Box<dyn std::error::Error>>;

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SPAWNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SPAWNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn script(dir: &Path, body: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let path = dir.join("open");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        Ok(path)
    }

    #[test]
    fn terminal_qr_encodes_the_text_in_block_characters() -> Check {
        let url = "https://example.test/enroll#token";
        let rendered = terminal_qr(url)?;
        assert!(rendered.ends_with('\n'));
        assert!(rendered.lines().count() > 10);
        assert!(
            rendered
                .chars()
                .any(|c| c == '\u{2588}' || c == '\u{2580}' || c == '\u{2584}')
        );
        Ok(())
    }

    #[test]
    fn permission_chain_matches_go() {
        let report = Path::new("/r/report.json");
        assert_eq!(
            permission_arguments("/Apps/Helper.app", "camera", report),
            [
                "-n",
                "-W",
                "-g",
                "-a",
                "/Apps/Helper.app",
                "--args",
                "permission-request",
                "camera",
                "--report",
                "/r/report.json"
            ]
        );
        let defaults = permission_arguments("", "camera", report);
        assert!(defaults[4].ends_with("Applications/AppleBridgeDev/AppleBridgeHelper.app"));
    }

    #[test]
    fn selftest_chain_matches_go() {
        let report = Path::new("/r/s.json");
        assert_eq!(
            messages_selftest_arguments("/H.app", report, false, true),
            [
                "-n",
                "-W",
                "-g",
                "-a",
                "/H.app",
                "--args",
                "selftest",
                "messages",
                "--report",
                "/r/s.json",
                "--no-ui",
                "--force"
            ]
        );
        assert_eq!(
            messages_selftest_arguments("/H.app", report, true, false).len(),
            10
        );
    }

    #[test]
    fn permission_request_runs_chain_and_reads_report() -> Check {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let log = dir.path().join("argv");
        let program = script(
            dir.path(),
            &format!(
                "printf '%s\\n' \"$@\" > '{}'\nwhile [ \"$1\" != --report ]; do shift; done\nprintf '{{\"camera\":\"granted\"}}' > \"$2\"",
                log.display()
            ),
        )?;
        let states = permissions_request(&program, dir.path(), "/H.app", "camera", None)?;
        assert_eq!(states["camera"], "granted");
        let argv = std::fs::read_to_string(&log)?;
        assert!(
            argv.starts_with(
                "-n\n-W\n-g\n-a\n/H.app\n--args\npermission-request\ncamera\n--report\n"
            )
        );
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 2);
        Ok(())
    }

    #[test]
    fn contacts_calendar_and_reminders_are_requested_one_at_a_time() -> Check {
        let _serial = serial();
        for name in ["contacts", "calendar", "reminders"] {
            let dir = tempfile::tempdir()?;
            let seen = dir.path().join("seen");
            let program = script(
                dir.path(),
                &format!(
                    "printf '%s\\n' \"$@\" > '{}'\nwhile [ \"$1\" != --report ]; do shift; done\nprintf '{{\"request\":\"{name}\"}}' > \"$2\"\n",
                    seen.display()
                ),
            )?;
            let states = permissions_request(&program, dir.path(), "/H.app", name, None)?;
            assert_eq!(states["request"], name);
            let arguments = std::fs::read_to_string(&seen)?;
            assert!(
                arguments.starts_with(&format!(
                    "-n\n-W\n-g\n-a\n/H.app\n--args\npermission-request\n{name}\n--report\n"
                )),
                "{arguments}"
            );
            assert!(!arguments.contains("onboard"), "{arguments}");
        }
        Ok(())
    }

    #[test]
    fn an_unanswered_prompt_is_reported_as_a_timeout() -> Check {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let program = script(
            dir.path(),
            "while [ \"$1\" != --report ]; do shift; done\nprintf '{\"request\":\"contacts\",\"outcome\":\"timed_out\"}' > \"$2\"",
        )?;
        let failure = permissions_request(&program, dir.path(), "/H.app", "contacts", None);
        assert!(
            failure
                .err()
                .is_some_and(|error| error.to_string().contains("nobody answered")),
        );
        Ok(())
    }

    #[test]
    fn permission_request_rejects_unknown_names() -> Check {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let program = script(dir.path(), "exit 0")?;
        assert!(permissions_request(&program, dir.path(), "", "fake", None).is_err());
        assert!(permissions_request(&program, dir.path(), "", "", None).is_err());
        Ok(())
    }

    #[test]
    fn selftest_reports_state() -> Check {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let program = script(
            dir.path(),
            "while [ \"$1\" != --report ]; do shift; done\nprintf '{\"state\":\"partial\"}' > \"$2\"",
        )?;
        let result = messages_selftest(&program, dir.path(), "/H.app", false, true, None)?;
        assert!(!result.proven());
        assert_eq!(result.state, "partial");
        let proven = script(
            dir.path(),
            "while [ \"$1\" != --report ]; do shift; done\nprintf '{\"state\":\"proven\"}' > \"$2\"",
        )?;
        assert!(messages_selftest(&proven, dir.path(), "/H.app", true, false, None)?.proven());
        let corrupt = script(
            dir.path(),
            "while [ \"$1\" != --report ]; do shift; done\nprintf '{}' > \"$2\"",
        )?;
        assert!(messages_selftest(&corrupt, dir.path(), "/H.app", false, true, None).is_err());
        Ok(())
    }

    #[test]
    fn run_times_out_and_surfaces_failure() -> Check {
        let _serial = serial();
        let dir = tempfile::tempdir()?;
        let slow = script(dir.path(), "sleep 5")?;
        assert!(
            permissions_request(
                &slow,
                dir.path(),
                "",
                "camera",
                Some(Duration::from_millis(200))
            )
            .is_err()
        );
        let failing = script(dir.path(), "exit 3")?;
        assert!(permissions_request(&failing, dir.path(), "", "camera", None).is_err());
        Ok(())
    }

    #[test]
    fn bundle_is_detected_from_launch_agent_plist() -> Check {
        let dir = tempfile::tempdir()?;
        let bundle = dir.path().join("Apple Bridge.app");
        std::fs::create_dir_all(bundle.join("Contents/Library/LaunchAgents"))?;
        std::fs::create_dir_all(bundle.join("Contents/MacOS"))?;
        let executable = bundle.join("Contents/MacOS/bridge");
        assert_eq!(bundle_of(&executable), None);
        std::fs::write(
            bundle.join("Contents/Library/LaunchAgents/dev.applebridge.gateway.plist"),
            b"x",
        )?;
        assert_eq!(bundle_of(&executable), Some(bundle));
        Ok(())
    }
}
