use crate::{
    assertions::await_until,
    http::{Client, random_id},
    mac::{self, command_readback, query_call},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SNAPSHOT: &str = r"ObjC.import('AppKit');
function run() {
 const board = $.NSPasteboard.generalPasteboard;
 if (Number(board.accessBehavior) !== 2) throw Error('Clipboard requires Always Allow');
 const change = Number(board.changeCount);
 const items = board.pasteboardItems;
 const result = [];
 let size = 0;
 for (let i = 0; items && i < items.count; i++) {
  const item = items.objectAtIndex(i), saved = Object.create(null), types = item.types;
  const verifier = $.NSPasteboardItem.alloc.init;
  for (let j = 0; j < types.count; j++) {
   const type = types.objectAtIndex(j), data = item.dataForType(type);
   if (!data || ObjC.unwrap(data) == null) throw Error('clipboard data unavailable');
   size += Number(data.length);
   if (size > 16777216) throw Error('clipboard snapshot too large');
   if (!verifier.setDataForType(data, type)) throw Error('clipboard format cannot be restored');
   saved[ObjC.unwrap(type)] = ObjC.unwrap(data.base64EncodedStringWithOptions(0));
  }
  result.push(saved);
 }
 if (Number(board.changeCount) !== change) throw Error('clipboard changed during snapshot');
 return JSON.stringify(result);
}";
const RESTORE: &str = r"ObjC.import('AppKit');
function run() {
 const input = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
 const saved = JSON.parse(ObjC.unwrap($.NSString.alloc.initWithDataEncoding(input, $.NSUTF8StringEncoding)));
 const items = [];
 for (const savedItem of saved) {
  const item = $.NSPasteboardItem.alloc.init;
  for (const type of Object.keys(savedItem)) {
   const data = $.NSData.alloc.initWithBase64EncodedStringOptions($(savedItem[type]), 0);
   if (!item.setDataForType(data, $(type))) throw Error('clipboard restore failed');
  }
  items.push(item);
 }
 const board = $.NSPasteboard.generalPasteboard;
 board.clearContents;
 if (items.length && !board.writeObjects($(items))) throw Error('clipboard restore failed');
 return 'restored';
}";
const FRONTMOST: &str = "ObjC.import('AppKit'); ObjC.unwrap($.NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier)";
const CALCULATOR: &str = "com.apple.calculator";
const CALCULATOR_RUNNING: &str = "application id \"com.apple.calculator\" is running";
const VOLUME: &str = "output volume of (get volume settings)";
const MUTED: &str = "output muted of (get volume settings)";

fn output(program: &str, args: &[&str], input: Option<&[u8]>, seconds: u64) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let (Some(mut stdin), Some(data)) = (child.stdin.take(), input) {
        stdin.write_all(data).ok()?;
    }
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let result = child.wait_with_output().ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}
fn script(source: &str) -> Option<String> {
    output("/usr/bin/osascript", &["-e", source], None, 30)
}
fn javascript(source: &str) -> Option<String> {
    output(
        "/usr/bin/osascript",
        &["-l", "JavaScript", "-e", source],
        None,
        30,
    )
}
fn frontmost() -> Option<String> {
    javascript(FRONTMOST).filter(|app| !app.is_empty() && app != "undefined")
}
fn helper_app() -> Result<PathBuf> {
    env::var_os("APPLE_BRIDGE_HELPER_APP")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Invalid("APPLE_BRIDGE_HELPER_APP required".into()))
}
fn support(name: &str) -> Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| Error::Invalid("HOME".into()))?;
    Ok(Path::new(&home)
        .join("Library")
        .join("Application Support")
        .join("AppleBridgeDev")
        .join(name))
}
fn pasteboard(source: &str, input: Option<&[u8]>) -> Option<String> {
    output(
        "/usr/bin/osascript",
        &["-l", "JavaScript", "-e", source],
        Some(input.unwrap_or_default()),
        20,
    )
}

fn audio(client: &Client, evidence: &mut Evidence) {
    let muted = script(MUTED);
    evidence.check("original_mute_saved", muted.is_some(), "mute state read");
    let Some(muted) = muted else {
        return;
    };
    let before = script(VOLUME);
    let volume = before.as_deref().and_then(|v| v.parse::<i64>().ok());
    evidence.check("original_volume_saved", volume.is_some(), "volume read");
    if let (Some(before), Some(volume)) = (before.as_deref(), volume) {
        let next = if volume == 23 { 24 } else { 23 };
        evidence.check(
            "volume_command_readback",
            command_readback(client, "mac.set_volume", &json!({"volume": next})),
            "volume command observed",
        );
        evidence.check(
            "volume_truth",
            script(VOLUME).is_some_and(|actual| actual == next.to_string()),
            "system volume matches",
        );
        let flipped = muted != "true";
        evidence.check(
            "mute_command_readback",
            command_readback(client, "mac.set_mute", &json!({"muted": flipped})),
            "mute command observed",
        );
        evidence.check(
            "mute_truth",
            script(MUTED).is_some_and(|actual| actual == flipped.to_string()),
            "system mute matches",
        );
        evidence.check(
            "volume_restored",
            command_readback(client, "mac.set_volume", &json!({"volume": volume})),
            "volume restored",
        );
        evidence.check(
            "restore_truth",
            script(VOLUME).is_some_and(|actual| actual == before),
            "system volume restored",
        );
    }
    evidence.check(
        "mute_restored",
        command_readback(client, "mac.set_mute", &json!({"muted": muted == "true"})),
        "mute restored",
    );
    evidence.check(
        "mute_restore_truth",
        script(MUTED).is_some_and(|actual| actual == muted),
        "system mute restored",
    );
}
fn dark() -> &'static str {
    let style = output(
        "/usr/bin/defaults",
        &["read", "-g", "AppleInterfaceStyle"],
        None,
        10,
    );
    if style.as_deref() == Some("Dark") {
        "dark"
    } else {
        "light"
    }
}
fn appearance(client: &Client, evidence: &mut Evidence) {
    let before = dark();
    let next = if before == "dark" { "light" } else { "dark" };
    evidence.check(
        "appearance_command_readback",
        command_readback(client, "mac.set_appearance", &json!({"appearance": next})),
        "appearance command observed",
    );
    evidence.check(
        "appearance_truth",
        dark() == next,
        "system appearance matches",
    );
    evidence.check(
        "appearance_restored",
        command_readback(client, "mac.set_appearance", &json!({"appearance": before})),
        "appearance restored",
    );
    evidence.check(
        "restore_truth",
        dark() == before,
        "system appearance restored",
    );
}
fn apps(client: &Client, evidence: &mut Evidence) {
    let running = script(CALCULATOR_RUNNING);
    let idle = running.as_deref() == Some("false");
    evidence.check("calculator_not_running", idle, "calculator is not running");
    if !idle {
        return;
    }
    let original = frontmost();
    evidence.check(
        "frontmost_app_saved",
        original.is_some(),
        "frontmost app recorded",
    );
    let Some(original) = original else {
        return;
    };
    let launched = command_readback(client, "mac.launch_app", &json!({"bundle_id": CALCULATOR}));
    evidence.check("calculator_launched", launched, "calculator launched");
    if launched {
        evidence.check(
            "original_app_activated_between",
            command_readback(client, "mac.activate_app", &json!({"bundle_id": original})),
            "original app activated",
        );
        evidence.check(
            "calculator_activated",
            command_readback(
                client,
                "mac.activate_app",
                &json!({"bundle_id": CALCULATOR}),
            ),
            "calculator activated",
        );
        evidence.check(
            "calculator_frontmost_truth",
            javascript(FRONTMOST).as_deref() == Some(CALCULATOR),
            "calculator is frontmost",
        );
    }
    if script(CALCULATOR_RUNNING).as_deref() == Some("true") {
        let quit = command_readback(client, "mac.quit_app", &json!({"bundle_id": CALCULATOR}));
        evidence.check("calculator_cleanup", quit, "calculator quit");
        if !quit {
            let fallback = javascript(
                "ObjC.import('AppKit'); const apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier($('com.apple.calculator')); if (apps.count > 0) apps.objectAtIndex(0).terminate;",
            );
            evidence.check(
                "calculator_fallback_cleanup",
                fallback.is_some(),
                "calculator terminated",
            );
        }
    }
    evidence.check(
        "calculator_restore_truth",
        script(CALCULATOR_RUNNING).as_deref() == Some("false"),
        "calculator is not running",
    );
    evidence.check(
        "frontmost_app_restored",
        command_readback(client, "mac.activate_app", &json!({"bundle_id": original})),
        "original app restored",
    );
}
fn keep_awake(client: &Client, evidence: &mut Evidence) {
    let assertions = || output("/usr/bin/pmset", &["-g", "assertions"], None, 10);
    evidence.check(
        "assertion_readback",
        command_readback(
            client,
            "mac.keep_awake",
            &json!({"on": true, "duration": 60}),
        ),
        "keep awake observed",
    );
    evidence.check(
        "assertion_truth",
        assertions().is_some_and(|text| text.contains("Apple Bridge keep awake")),
        "pmset lists the assertion",
    );
    evidence.check(
        "assertion_released",
        command_readback(client, "mac.keep_awake", &json!({"on": false})),
        "keep awake released",
    );
    evidence.check(
        "release_truth",
        assertions().is_some_and(|text| !text.contains("Apple Bridge keep awake")),
        "pmset no longer lists the assertion",
    );
}
fn clipboard_allowed(client: &Client) -> Option<bool> {
    let mut allowed = None;
    let ready = await_until(20, || {
        let Some(status) = client.call("sources.status", &json!({}), 20).ok().flatten() else {
            return false;
        };
        let Some(record) = status["records"]
            .as_array()
            .and_then(|records| records.iter().find(|r| r["source"] == "mac"))
        else {
            return false;
        };
        allowed = Some(
            !record["coverage"]["missing"]
                .as_array()
                .is_some_and(|missing| {
                    missing.iter().any(|item| {
                        item == "Clipboard requires Always Allow in Paste from Other Apps"
                    })
                }),
        );
        true
    });
    ready.then_some(allowed).flatten()
}
pub fn manager_markers(evidence: &mut Evidence) {
    let marked = pasteboard(SNAPSHOT, None)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|items| {
            let items = items.as_array()?.clone();
            (items.len() == 1).then(|| {
                items[0]["org.nspasteboard.TransientType"].is_string()
                    && items[0]["org.nspasteboard.ConcealedType"].is_string()
            })
        })
        .unwrap_or(false);
    evidence.check(
        "clipboard_manager_markers_present",
        marked,
        "clipboard carries transient and concealed markers",
    );
}
fn clipboard(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    let allowed = clipboard_allowed(client);
    evidence.check(
        "clipboard_permission_status_ready",
        allowed.is_some(),
        "mac source status answered",
    );
    evidence.check(
        "clipboard_helper_access_granted",
        allowed == Some(true),
        "helper has Always Allow",
    );
    if allowed != Some(true) {
        return Ok(());
    }
    let saved = pasteboard(SNAPSHOT, None);
    let before = saved
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    evidence.check(
        "clipboard_snapshot_saved",
        before.is_some(),
        "clipboard snapshot saved",
    );
    let (Some(saved), Some(before)) = (saved, before) else {
        return Ok(());
    };
    mac::clipboard(world, client, true, evidence)?;
    evidence.check(
        "clipboard_restored",
        pasteboard(RESTORE, Some(saved.as_bytes())).is_some(),
        "clipboard restored",
    );
    let after = pasteboard(SNAPSHOT, None).and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    evidence.check(
        "clipboard_restore_truth",
        after == Some(before),
        "clipboard equals its snapshot",
    );
    Ok(())
}
fn notify(evidence: &mut Evidence) -> Result<()> {
    let report = support(&format!("{}.json", random_id("notify-proof")?))?;
    let app = helper_app()?;
    let probe = output(
        "open",
        &[
            "-n",
            "-W",
            "-g",
            "-a",
            &app.to_string_lossy(),
            "--args",
            "mac-notify-proof",
            "--report",
            &report.to_string_lossy(),
        ],
        None,
        30,
    );
    evidence.check(
        "signed_helper_notification_probe",
        probe.is_some(),
        "signed helper ran the notification probe",
    );
    if probe.is_none() {
        return Ok(());
    }
    let proof = fs::read(&report)
        .ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Map<String, Value>>(&data).ok());
    let _ = fs::remove_file(&report);
    evidence.check(
        "notification_report",
        proof.as_ref().is_some_and(|p| !p.is_empty()),
        "probe wrote a report",
    );
    for (name, value) in proof.iter().flatten() {
        evidence.check(name, value == true, "probe check");
    }
    Ok(())
}
fn battery(text: &str) -> Option<f64> {
    let end = text.find("%;")?;
    let digits: String = text[..end]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}
fn power_records(client: &Client, evidence: &mut Evidence) -> Option<Value> {
    let mut first = None;
    let captured = await_until(20, || {
        first = client
            .call(
                "records.query",
                &json!({"collection": "mac.power", "limit": 100}),
                10,
            )
            .ok()
            .flatten()
            .and_then(|page| page["records"].as_array()?.first().cloned());
        first.is_some()
    });
    evidence.check("power_captured", captured, "mac.power record captured");
    first
}
fn power(client: &Client, evidence: &mut Evidence) {
    let Some(record) = power_records(client, evidence) else {
        return;
    };
    let truth = output("/usr/bin/pmset", &["-g", "batt"], None, 10);
    let expected = truth.as_deref().and_then(battery);
    evidence.check(
        "pmset_battery_available",
        expected.is_some(),
        "pmset reports a battery percentage",
    );
    let (Some(truth), Some(expected)) = (truth, expected) else {
        return;
    };
    let data = &record["data"];
    let level = data["battery_percent"].as_f64().unwrap_or(f64::NAN);
    evidence.check(
        "battery_matches_pmset",
        level >= expected && level < expected + 1.0,
        "battery percentage matches pmset",
    );
    evidence.check(
        "charging_matches_pmset",
        data["charging"].as_bool() == Some(truth.contains("; charging;")),
        "charging state matches pmset",
    );
    evidence.check(
        "source_matches_pmset",
        truth.contains("AC Power") == data["source"].as_str().is_some_and(|s| s.contains("AC")),
        "power source matches pmset",
    );
}
pub fn idle(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    let Some(_) = power_records(client, evidence) else {
        return Ok(());
    };
    let report = support(&format!("{}-idle-permissions.json", random_id("run")?))?;
    let app = helper_app()?;
    let status = output(
        "open",
        &[
            "-n",
            "-W",
            "-g",
            "-a",
            &app.to_string_lossy(),
            "--args",
            "status",
            "--report",
            &report.to_string_lossy(),
        ],
        None,
        15,
    );
    let permissions = fs::read(&report)
        .ok()
        .and_then(|data| serde_json::from_slice::<Value>(&data).ok());
    let _ = fs::remove_file(&report);
    let granted = status.is_some()
        && permissions
            .as_ref()
            .is_some_and(|p| p["full_disk_access"] == "granted");
    if !granted {
        evidence.check(
            "full_disk_access_granted",
            false,
            "needs Full Disk Access for the signed helper; capture cannot settle",
        );
        return Ok(());
    }
    let marker = world.gateway_home.join("helper").join("messages-idle.json");
    let started = Instant::now();
    let settled = await_until(25 * 60, || {
        fs::read(&marker)
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .is_some_and(|state| state["idle"] == true)
    });
    evidence.check(
        "messages_capture_and_media_settled",
        settled,
        "messages capture and media settled",
    );
    if !settled {
        return Ok(());
    }
    let _ = started;
    let pid = fs::read_to_string(world.gateway_home.join("helper").join("helper.pid"))
        .map(|text| text.trim().to_owned());
    evidence.check("helper_pid", pid.is_ok(), "helper pid file read");
    let Ok(pid) = pid else {
        return Ok(());
    };
    let cpu = || -> Option<f64> {
        let text = output("ps", &["-o", "time=", "-p", &pid], None, 10)?;
        let (minutes, seconds) = text.split_once(':')?;
        Some(minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?)
    };
    let first = cpu();
    let window = Instant::now();
    thread::sleep(Duration::from_secs(300));
    let second = cpu();
    let percent =
        100.0 * (second.unwrap_or(0.0) - first.unwrap_or(0.0)) / window.elapsed().as_secs_f64();
    evidence.check(
        "helper_cpu_under_two_percent",
        first.is_some() && second >= first && percent < 2.0,
        &format!("helper used {percent:.2}% CPU over the idle window"),
    );
    Ok(())
}
fn truth_running(text: &str) -> Option<char> {
    let rest = text[text.find("Running")? + 7..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    rest.chars().next().filter(|c| matches!(c, '0' | '1'))
}
fn native(client: &Client, evidence: &mut Evidence) {
    let (op, args) = match evidence.scenario.as_str() {
        "TM-STATUS" => ("mac.time_machine_status", json!({})),
        "BT-REAL" => ("mac.bluetooth_devices", json!({})),
        _ => ("mac.recognize_song", json!({"audio_blob": "W2FiLXRlc3Rd"})),
    };
    let result = query_call(client, op, &args).map(|reply| reply["result_json"].clone());
    evidence.check(
        "native_query_completed",
        result.is_some(),
        "native query completed",
    );
    let available = result.as_ref().is_some_and(|r| r["limited"] != true);
    evidence.check(
        "native_operation_available",
        available,
        "native operation is available",
    );
    let Some(result) = result.filter(|_| available) else {
        return;
    };
    match evidence.scenario.as_str() {
        "TM-STATUS" => {
            let truth = output("/usr/bin/tmutil", &["status"], None, 20);
            let observed = result["status"].as_str().and_then(truth_running);
            let expected = truth.as_deref().and_then(truth_running);
            evidence.check(
                "tmutil_status_truth",
                observed.is_some() && observed == expected,
                "Time Machine running flag matches tmutil",
            );
        }
        "BT-REAL" => evidence.check(
            "devices_returned",
            result["devices"].is_array(),
            "device list returned",
        ),
        _ => evidence.check(
            "recognized_fixture",
            result["title"].as_str().is_some_and(|t| !t.is_empty()),
            "fixture recognized",
        ),
    }
}
pub fn iwork(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let installed = Path::new("/Applications/Pages.app").exists()
        || home.join("Applications/Pages.app").exists();
    evidence.check(
        "pages_installed",
        installed,
        "Pages is installed on the qualification host",
    );
    if !installed {
        return Ok(());
    }
    mac::iwork(client, evidence);
    Ok(())
}
pub const SCENARIOS: [&str; 11] = [
    "MAC-AUDIO-REAL",
    "MAC-REPLICA",
    "MAC-APPEARANCE-REAL",
    "MAC-APPS-REAL",
    "MAC-KEEPAWAKE-REAL",
    "MAC-CLIPBOARD-REAL",
    "MAC-NOTIFY-REAL",
    "MAC-POWER-REAL",
    "TM-STATUS",
    "BT-REAL",
    "SHAZAM-REAL",
];

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let Some(client) = mac::setup(world, evidence)? else {
        return Ok(());
    };
    match evidence.scenario.clone().as_str() {
        "MAC-AUDIO-REAL" | "MAC-REPLICA" => audio(&client, evidence),
        "MAC-APPEARANCE-REAL" => appearance(&client, evidence),
        "MAC-APPS-REAL" => apps(&client, evidence),
        "MAC-KEEPAWAKE-REAL" => keep_awake(&client, evidence),
        "MAC-CLIPBOARD-REAL" => clipboard(world, &client, evidence)?,
        "MAC-NOTIFY-REAL" => notify(evidence)?,
        "MAC-POWER-REAL" => power(&client, evidence),
        "TM-STATUS" | "BT-REAL" | "SHAZAM-REAL" => native(&client, evidence),
        other => return Err(Error::Invalid(format!("mac scenario not ported: {other}"))),
    }
    Ok(())
}
