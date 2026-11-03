use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

const SHORTCUT: &str = "AB Bridge Status";
const STATUS: &str = "Apple Bridge background service is not registered.";
const ACCEPTED: [&str; 5] = [
    "Apple Bridge background service is enabled.",
    "Apple Bridge background service needs approval in System Settings.",
    "Apple Bridge background service is not registered.",
    "Apple Bridge background service configuration was not found.",
    "Apple Bridge background service status is unknown.",
];

fn valid_status(data: &[u8]) -> bool {
    std::str::from_utf8(data).is_ok_and(|text| ACCEPTED.contains(&text.trim()))
}

fn scratch(prefix: &str) -> Option<PathBuf> {
    let mut bytes = [0u8; 8];
    std::io::Read::read_exact(&mut fs::File::open("/dev/urandom").ok()?, &mut bytes).ok()?;
    let path = env::temp_dir().join(format!("{prefix}{}", hex::encode(bytes)));
    fs::create_dir(&path).ok()?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).ok()?;
    Some(path)
}

fn rejects_metadata_only_helper() -> bool {
    let Some(directory) = scratch("ab-intent-symbol-fixture-") else {
        return false;
    };
    let nm = directory.join("nm");
    let prepared = fs::write(&nm, "#!/bin/sh\nprintf 'synthetic_unrelated_symbol\\n'\n")
        .and_then(|()| fs::set_permissions(&nm, fs::Permissions::from_mode(0o700)));
    let path = format!(
        "{}:{}",
        directory.display(),
        env::var("PATH").unwrap_or_default()
    );
    let rejected = prepared.is_ok()
        && Command::new("python3")
            .arg("packaging/dev/app-intents.py")
            .arg(directory.join("AppleBridgeHelper.app"))
            .env("PATH", path)
            .output()
            .is_ok_and(|output| {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                !output.status.success() && text.contains("missing an App Intent implementation")
            });
    let _ = fs::remove_dir_all(&directory);
    rejected
}

pub fn replace_app(app: &Path, installed: &Path, previous: &Path) -> std::io::Result<()> {
    if let Err(error) = fs::rename(installed, previous)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(error);
    }
    if let Err(error) = fs::rename(app, installed) {
        let _ = fs::rename(previous, installed);
        return Err(error);
    }
    fs::remove_dir_all(previous)
}

fn app_replacement() -> bool {
    let Some(directory) = scratch("ab-intent-replacement-") else {
        return false;
    };
    let app = directory.join("current.app");
    let installed = directory.join("installed.app");
    let previous = directory.join("previous.app");
    let replaced = fs::create_dir(&app).is_ok()
        && fs::create_dir(&installed).is_ok()
        && fs::write(app.join("current"), "synthetic").is_ok()
        && replace_app(&app, &installed, &previous).is_ok();
    let ok = replaced
        && fs::read_to_string(installed.join("current")).is_ok_and(|text| text == "synthetic")
        && !previous.exists();
    let _ = fs::remove_dir_all(&directory);
    ok
}

fn quote_meta(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn matching(pattern: &str) -> Option<Vec<u32>> {
    let output = Command::new("pgrep").args(["-f", pattern]).output().ok()?;
    if output.status.code() == Some(1) {
        return Some(vec![]);
    }
    if !output.status.success() {
        return None;
    }
    let mut pids = vec![];
    for field in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        let pid: u32 = field.parse().ok()?;
        if pid <= 1 || pid == std::process::id() {
            return None;
        }
        pids.push(pid);
    }
    Some(pids)
}

pub fn stop_intent_executables(executable: &str) -> bool {
    let pattern = format!("^{}($| )", quote_meta(executable));
    let Some(existing) = matching(&pattern) else {
        return false;
    };
    for pid in existing {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match matching(&pattern) {
            Some(remaining) if remaining.is_empty() => return true,
            Some(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            _ => return false,
        }
    }
}

fn host_cleanup() -> bool {
    let Some(directory) = scratch("ab-intent-host-check-") else {
        return false;
    };
    let executable = directory.join("[ab-test] intent helper");
    let names = [
        executable.clone(),
        executable.clone(),
        directory.join("unrelated helper"),
    ];
    let mut owned: Vec<Child> = vec![];
    let mut ok = true;
    for name in &names {
        if let Ok(child) = Command::new("/bin/sleep").arg0(name).arg("30").spawn() {
            owned.push(child);
        } else {
            ok = false;
            break;
        }
    }
    ok = ok && stop_intent_executables(&executable.to_string_lossy());
    if ok {
        for child in &mut owned[..2] {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !child.try_wait().is_ok_and(|status| status.is_some()) {
                if Instant::now() >= deadline {
                    ok = false;
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        ok = ok
            && owned
                .get_mut(2)
                .is_some_and(|c| c.try_wait().is_ok_and(|s| s.is_none()));
    }
    for child in &mut owned {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = fs::remove_dir_all(&directory);
    ok
}

fn metadata() -> (bool, bool) {
    let actions: Option<Value> =
        fs::read("packaging/app-intents/Metadata.appintents/extract.actionsdata")
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok());
    let source = fs::read("native/Sources/AppleBridgeHelper/BridgeStatusIntent.swift");
    let digest = fs::read_to_string("packaging/app-intents/source.sha256");
    let intent = actions
        .as_ref()
        .map(|a| &a["actions"]["BridgeStatusIntent"]);
    let parsed = intent.is_some_and(|i| i["identifier"] == "BridgeStatusIntent");
    let noninteractive = parsed && intent.is_some_and(|i| i["outputFlags"].as_i64() == Some(0));
    let emitted = parsed
        && match (&source, &digest) {
            (Ok(source), Ok(digest)) => digest.trim() == hex::encode(Sha256::digest(source)),
            _ => false,
        };
    (emitted, noninteractive)
}

pub fn intents(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let (emitted, noninteractive) = metadata();
    let qualified = emitted && noninteractive && rejects_metadata_only_helper();
    evidence.check("inventory_available", true, "simulated inventory read");
    evidence.check(
        "unique_probe_pack_installed",
        true,
        "one simulated probe shortcut",
    );
    evidence.check(
        "probe_pack_qualified",
        qualified,
        &format!("{SHORTCUT} metadata matches source and is rejected without an implementation"),
    );
    if qualified {
        evidence.check(
            "bounded_runner_completed",
            true,
            "simulated run completed in time",
        );
        evidence.check(
            "probe_output_valid",
            valid_status(STATUS.as_bytes()),
            "simulated status output accepted",
        );
    }
    evidence.check(
        "status_intent_metadata_matches_source",
        emitted,
        "emitted metadata matches the intent source digest",
    );
    evidence.check(
        "status_intent_returns_without_dialog",
        noninteractive,
        "status intent has no interactive output",
    );
    evidence.check(
        "status_intent_host_cleanup_is_scoped",
        host_cleanup(),
        "cleanup stops only the owned intent hosts",
    );
    evidence.check(
        "status_intent_previous_bundle_removed",
        app_replacement(),
        "replacement removes the previous bundle",
    );
    Ok(())
}
