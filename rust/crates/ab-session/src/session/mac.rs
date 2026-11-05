use super::owned::{Driver, Probe, Refusal, Settled, native_failure, refuse, text};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const CAPABILITIES: [&str; 17] = [
    "mac.set_volume",
    "mac.set_mute",
    "mac.set_appearance",
    "mac.launch_app",
    "mac.activate_app",
    "mac.quit_app",
    "mac.keep_awake",
    "mac.clipboard_write",
    "mac.clipboard_read",
    "mac.notify",
    "mac.open_settings",
    "mac.set_wallpaper",
    "mac.network_check",
    "mac.bluetooth_devices",
    "mac.time_machine_status",
    "mac.recognize_song",
    "mac.iwork_export",
];
const MAX_TITLE: usize = 256;
const MAX_BODY: usize = 4096;
const MAX_CLIPBOARD: usize = 65536;
const MAX_IMAGE: usize = 8 * 1024 * 1024;
const MAX_KEEP_AWAKE: f64 = 86_400.0;
const MAX_EXPORT: u64 = 16 * 1024 * 1024;
const SYSTEM_EVENTS: &str = "com.apple.systemevents";
const SETTINGS: &str = "com.apple.systempreferences";
const POLL: Duration = Duration::from_millis(100);
const PANES: [(&str, &str); 6] = [
    ("general", "com.apple.systempreferences.GeneralSettings"),
    (
        "notifications",
        "com.apple.Notifications-Settings.extension",
    ),
    ("network", "com.apple.Network-Settings.extension"),
    ("displays", "com.apple.Displays-Settings.extension"),
    ("wallpaper", "com.apple.Wallpaper-Settings.extension"),
    ("privacy", "com.apple.settings.PrivacySecurity.extension"),
];
const BLUETOOTH_TIMEOUT: Duration = Duration::from_secs(30);

fn invalid() -> String {
    "invalid".to_owned()
}

fn valid_action_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    matches!(scheme, "http" | "https")
        && rest
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|host| !host.is_empty())
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn iwork_application(app: &str) -> Option<&'static str> {
    match app {
        "pages" => Some("Pages"),
        "numbers" => Some("Numbers"),
        "keynote" => Some("Keynote"),
        _ => None,
    }
}

fn args_ok(action: &Value) -> Result<(), &'static str> {
    let args = &action["args"];
    let ok = match text(action, "op") {
        "mac.notify" => {
            let (title, body, url) = (
                text(args, "title"),
                text(args, "body"),
                text(args, "action_url"),
            );
            !title.is_empty()
                && title.len() <= MAX_TITLE
                && body.len() <= MAX_BODY
                && (url.is_empty() || valid_action_url(url))
        }
        "mac.open_settings" => PANES.iter().any(|(pane, _)| *pane == text(args, "pane")),
        "mac.set_wallpaper" => STANDARD
            .decode(text(args, "image_blob"))
            .is_ok_and(|bytes| !bytes.is_empty() && bytes.len() <= MAX_IMAGE),
        "mac.clipboard_write" => text(args, "text").len() <= MAX_CLIPBOARD,
        "mac.set_volume" => args["volume"]
            .as_f64()
            .is_some_and(|volume| (0.0..=100.0).contains(&volume)),
        "mac.set_mute" => args["muted"].is_boolean(),
        "mac.set_appearance" => matches!(text(args, "appearance"), "dark" | "light"),
        "mac.launch_app" | "mac.activate_app" | "mac.quit_app" => {
            !text(args, "bundle_id").is_empty()
        }
        "mac.keep_awake" => {
            args["on"].is_boolean()
                && args["duration"].as_f64().unwrap_or(0.0) <= MAX_KEEP_AWAKE
                && args["duration"].as_f64().unwrap_or(0.0) >= 0.0
        }
        "mac.iwork_export" => {
            if text(args, "format") == "text"
                && text(args, "app") != "pages"
                && iwork_application(text(args, "app")).is_some()
            {
                return Err("unsupported");
            }
            iwork_application(text(args, "app")).is_some()
                && matches!(text(args, "format"), "pdf" | "text")
                && STANDARD
                    .decode(text(args, "document_blob"))
                    .is_ok_and(|bytes| !bytes.is_empty() && bytes.len() <= MAX_IMAGE)
        }
        _ => return Err("unsupported"),
    };
    if ok { Ok(()) } else { Err("invalid") }
}

fn prepare(probe: &dyn Probe, action: &Value, _expected: &Value) -> Result<Value, Refusal> {
    let operation = text(action, "op");
    if !CAPABILITIES.contains(&operation) {
        return Err(refuse("unsupported"));
    }
    args_ok(action).map_err(refuse)?;
    if operation == "mac.clipboard_write" {
        let policy = probe
            .call("mac.pasteboard_policy", &json!({}))
            .map_err(|code| native_failure(&code))?;
        if policy["always_allow"] != true {
            return Err(refuse("unavailable"));
        }
    }
    Ok(Value::Null)
}

fn validate(probe: &dyn Probe, action: &Value, state: &Value) -> Result<(), Refusal> {
    prepare(probe, action, state).map(|_| ())
}

fn script(probe: &dyn Probe, source: &str, target: Option<&str>) -> Result<String, String> {
    let mut args = json!({"source": source});
    if let Some(target) = target {
        args["target"] = json!(target);
    }
    let answer = probe.call("mac.script", &args)?;
    if let Some(number) = answer["error_number"].as_i64() {
        return Err(if matches!(number, -1743 | -1744) {
            "permission_missing"
        } else {
            "unavailable"
        }
        .to_owned());
    }
    Ok(text(&answer, "result").to_owned())
}

fn poll<T>(
    tries: u32,
    mut ready: impl FnMut() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    for _ in 0..tries {
        if let Some(found) = ready()? {
            return Ok(Some(found));
        }
        thread::sleep(POLL);
    }
    Ok(None)
}

fn random_id() -> Result<String, String> {
    use std::io::Read;
    let mut bytes = [0_u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| "unavailable".to_owned())?;
    Ok(bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    }))
}

fn notify(probe: &dyn Probe, args: &Value) -> Result<Value, String> {
    let identifier = format!("apple-bridge-{}", random_id()?);
    probe.call(
        "mac.notification_post",
        &json!({"title": args["title"], "body": args["body"], "action_url": args["action_url"], "identifier": identifier}),
    )?;
    let delivered = poll(20, || {
        let answer = probe.call(
            "mac.notification_delivered",
            &json!({"identifier": identifier}),
        )?;
        Ok((answer["delivered"] == true).then_some(()))
    })?;
    delivered.ok_or("unavailable")?;
    Ok(json!({"delivered": true, "identifier": identifier}))
}

fn open_settings(probe: &dyn Probe, args: &Value) -> Result<Value, String> {
    let pane = text(args, "pane");
    let (_, identifier) = PANES
        .iter()
        .find(|(name, _)| *name == pane)
        .ok_or_else(invalid)?;
    probe.call(
        "mac.open_url",
        &json!({"url": format!("x-apple.systempreferences:{identifier}")}),
    )?;
    let front = poll(30, || {
        let answer = probe.call("mac.frontmost", &json!({}))?;
        Ok((text(&answer, "bundle_id") == SETTINGS).then_some(()))
    })?;
    front.ok_or("unavailable")?;
    Ok(json!({"frontmost": true, "pane": pane}))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("[ab-test] mac {}", random_id()?));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "unavailable".to_owned())?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wallpaper(probe: &dyn Probe, args: &Value) -> Result<Value, String> {
    let bytes = STANDARD
        .decode(text(args, "image_blob"))
        .map_err(|_| invalid())?;
    let scratch = Scratch::new()?;
    let input = scratch.0.join("image");
    fs::write(&input, bytes).map_err(|_| "unavailable".to_owned())?;
    let answer = probe.call(
        "mac.set_wallpaper",
        &json!({"path": input.to_string_lossy()}),
    )?;
    if answer["matches"] != true {
        return Err("conflict".to_owned());
    }
    Ok(json!({"matches": true}))
}

fn volume(probe: &dyn Probe, wanted: f64) -> Result<Value, String> {
    let target = wanted.round();
    script(probe, &format!("set volume output volume {target}"), None)?;
    let actual: f64 = script(probe, "output volume of (get volume settings)", None)?
        .parse()
        .map_err(|_| "conflict".to_owned())?;
    if (actual - wanted).abs() > 1.0 {
        return Err("conflict".to_owned());
    }
    Ok(json!(actual))
}

fn mute(probe: &dyn Probe, muted: bool) -> Result<Value, String> {
    script(probe, &format!("set volume output muted {muted}"), None)?;
    let actual = script(probe, "output muted of (get volume settings)", None)?;
    if (actual == "true") != muted {
        return Err("conflict".to_owned());
    }
    Ok(json!(muted))
}

fn appearance(probe: &dyn Probe, value: &str) -> Result<Value, String> {
    let dark = value == "dark";
    script(
        probe,
        &format!(
            "tell application \"System Events\" to tell appearance preferences to set dark mode to {dark}"
        ),
        Some(SYSTEM_EVENTS),
    )?;
    let actual = script(
        probe,
        "tell application \"System Events\" to tell appearance preferences to get dark mode",
        Some(SYSTEM_EVENTS),
    )?;
    if (actual == "true") != dark {
        return Err("conflict".to_owned());
    }
    Ok(json!(value))
}

fn app(probe: &dyn Probe, operation: &str, bundle: &str) -> Result<Value, String> {
    let primitive = match operation {
        "mac.launch_app" => "mac.app_launch",
        "mac.activate_app" => "mac.app_activate",
        _ => "mac.app_quit",
    };
    probe.call(primitive, &json!({"bundle_id": bundle}))?;
    let settled = poll(50, || {
        let state = probe.call("mac.app_state", &json!({"bundle_id": bundle}))?;
        let (running, active) = (state["running"] == true, state["active"] == true);
        let matches = match operation {
            "mac.quit_app" => !running,
            "mac.activate_app" => active,
            _ => running,
        };
        Ok(matches.then_some(running))
    })?;
    let running = settled.ok_or("conflict")?;
    Ok(json!({"bundle_id": bundle, "running": running}))
}

fn quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

fn export_script(identifier: &str, input: &Path, output: &Path, format: &str) -> String {
    let format = if format == "pdf" {
        "PDF"
    } else {
        "unformatted text"
    };
    format!(
        "with timeout of 30 seconds\ntell application id {identifier}\nset bridgeDocument to open POSIX file {input}\ntry\nexport bridgeDocument to POSIX file {output} as {format}\non error errorText number errorNumber\nclose bridgeDocument saving no\nerror errorText number errorNumber\nend try\nclose bridgeDocument saving no\nend tell\nend timeout",
        identifier = quote(identifier),
        input = quote(&input.to_string_lossy()),
        output = quote(&output.to_string_lossy()),
    )
}

fn iwork_export(probe: &dyn Probe, args: &Value) -> Result<Value, String> {
    let application = iwork_application(text(args, "app")).ok_or_else(invalid)?;
    let format = text(args, "format");
    let bytes = STANDARD
        .decode(text(args, "document_blob"))
        .map_err(|_| invalid())?;
    let identifier = format!("com.apple.iWork.{application}");
    let installed = probe.call("mac.app_installed", &json!({"bundle_id": identifier}))?;
    if installed["installed"] != true {
        return Err("unavailable".to_owned());
    }
    let scratch = Scratch::new()?;
    let plain = application == "Pages"
        && std::str::from_utf8(&bytes).is_ok()
        && !bytes.starts_with(&[0x50, 0x4b]);
    let extension = if plain {
        "txt"
    } else if application == "Keynote" {
        "key"
    } else {
        text(args, "app")
    };
    let input = scratch.0.join(format!("[ab-test] document.{extension}"));
    let output = scratch.0.join(format!(
        "[ab-test] export.{}",
        if format == "pdf" { "pdf" } else { "txt" }
    ));
    fs::write(&input, &bytes).map_err(|_| "unavailable".to_owned())?;
    script(
        probe,
        &export_script(&identifier, &input, &output, format),
        Some(&identifier),
    )?;
    let metadata = fs::symlink_metadata(&output).map_err(|_| "corrupt".to_owned())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_EXPORT {
        return Err("corrupt".to_owned());
    }
    let exported = fs::read(&output).map_err(|_| "corrupt".to_owned())?;
    if format == "pdf" {
        let inspected = probe.call("pdf.inspect", &json!({"path": output.to_string_lossy()}))?;
        if inspected["page_count"].as_u64().unwrap_or(0) == 0 {
            return Err("corrupt".to_owned());
        }
    } else if std::str::from_utf8(&exported).is_err() {
        return Err("corrupt".to_owned());
    }
    Ok(json!({"app": args["app"], "format": format, "data_base64": STANDARD.encode(&exported)}))
}

fn readback(probe: &dyn Probe, operation: &str, args: &Value) -> Result<Value, String> {
    match operation {
        "mac.iwork_export" => iwork_export(probe, args),
        "mac.notify" => notify(probe, args),
        "mac.open_settings" => open_settings(probe, args),
        "mac.set_wallpaper" => wallpaper(probe, args),
        "mac.clipboard_write" => {
            let answer = probe.call("mac.clipboard_write", &json!({"text": args["text"]}))?;
            if answer["matches"] != true {
                return Err("conflict".to_owned());
            }
            Ok(json!(true))
        }
        "mac.set_volume" => volume(probe, args["volume"].as_f64().ok_or_else(invalid)?),
        "mac.set_mute" => mute(probe, args["muted"] == true),
        "mac.set_appearance" => appearance(probe, text(args, "appearance")),
        "mac.launch_app" | "mac.activate_app" | "mac.quit_app" => {
            app(probe, operation, text(args, "bundle_id"))
        }
        "mac.keep_awake" => {
            let answer = probe.call(
                "mac.keep_awake",
                &json!({"on": args["on"], "duration": args["duration"]}),
            )?;
            Ok(json!(answer["held"] == true))
        }
        _ => Err("unsupported".to_owned()),
    }
}

fn write(
    probe: &dyn Probe,
    _command: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    let answer = readback(probe, text(action, "op"), &action["args"])?;
    Ok(json!({"observed": true, "readback": answer}))
}

#[allow(clippy::unnecessary_wraps)]
fn settle(_operation: &str, written: &Value) -> Option<Settled> {
    Some(Settled {
        disposition: "locally_observed",
        evidence: written.clone(),
    })
}

fn resolved(_probe: &dyn Probe, _evidence: &Value) -> Option<Value> {
    None
}

fn reconcile_args(_found: &Value, _operation: &str) -> Value {
    json!({})
}

pub(super) const DRIVER: Driver = Driver {
    name: "mac",
    prepare,
    prepare_command: None,
    validate,
    write,
    settle,
    resolved,
    reconcile_args,
};

fn captured(mut command: Command, wait: Duration) -> Result<Vec<u8>, String> {
    let directory = Scratch::new()?;
    let path = directory.0.join("stdout");
    let file = fs::File::create(&path).map_err(|_| "unavailable".to_owned())?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "unavailable".to_owned())?;
    let deadline = Instant::now() + wait;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| "unavailable".to_owned())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("unavailable".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        return Err("unavailable".to_owned());
    }
    fs::read(&path).map_err(|_| "unavailable".to_owned())
}

fn time_machine() -> Result<Value, String> {
    let mut command = Command::new("/usr/bin/tmutil");
    command.arg("status");
    let output = captured(command, BLUETOOTH_TIMEOUT)?;
    let status = String::from_utf8(output).map_err(|_| "unavailable".to_owned())?;
    Ok(json!({"status": status}))
}

pub(super) fn bluetooth_devices(report: &Value) -> Option<Value> {
    let mut devices = Vec::new();
    for controller in report["SPBluetoothDataType"].as_array()? {
        for (key, connected) in [("device_connected", true), ("device_not_connected", false)] {
            for group in controller[key].as_array().into_iter().flatten() {
                for name in group.as_object().into_iter().flat_map(|group| group.keys()) {
                    devices.push(json!({"name": name, "connected": connected}));
                }
            }
        }
    }
    Some(json!({"devices": devices}))
}

fn bluetooth() -> Result<Value, String> {
    let mut command = Command::new("/usr/sbin/system_profiler");
    command.args(["SPBluetoothDataType", "-json", "-timeout", "15"]);
    let output = captured(command, BLUETOOTH_TIMEOUT)?;
    let report = serde_json::from_slice(&output).map_err(|_| "unavailable".to_owned())?;
    bluetooth_devices(&report).ok_or_else(|| "unavailable".to_owned())
}

pub(super) fn recognize_song(args: &Value) -> Result<Value, String> {
    let bytes = STANDARD
        .decode(text(args, "audio_blob"))
        .map_err(|_| invalid())?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE {
        return Err(invalid());
    }
    Ok(
        json!({"limited": true, "reason": "ShazamKit entitlement and fixture qualification required"}),
    )
}

#[must_use]
pub(super) fn handles_query(operation: &str) -> bool {
    matches!(
        operation,
        "mac.time_machine_status" | "mac.bluetooth_devices" | "mac.recognize_song"
    )
}

pub(super) fn query(frame: &Value) -> Value {
    let args = &frame["args"];
    let empty = args.as_object().is_some_and(serde_json::Map::is_empty);
    let answer = match text(frame, "op") {
        "mac.recognize_song" => recognize_song(args),
        _ if !empty => Err(invalid()),
        "mac.time_machine_status" => time_machine(),
        _ => bluetooth(),
    };
    match answer {
        Ok(result) => {
            json!({"t": "QueryResult", "id": frame["id"], "result_json": result, "blobs": []})
        }
        Err(code) => json!({"t": "QueryError", "id": frame["id"], "code": code}),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps
)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    fn action(op: &str, args: Value) -> Value {
        json!({"op": op, "args": args})
    }

    struct Fake {
        calls: RefCell<Vec<(String, Value)>>,
        answers: RefCell<VecDeque<Result<Value, String>>>,
    }

    impl Fake {
        fn new(answers: Vec<Result<Value, String>>) -> Self {
            Self {
                calls: RefCell::default(),
                answers: RefCell::new(answers.into()),
            }
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            self.calls
                .borrow_mut()
                .push((operation.to_owned(), args.clone()));
            self.answers
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Ok(json!({})))
        }
    }

    #[test]
    fn power_actions_and_unknown_operations_are_refused() {
        let probe = Fake::new(vec![]);
        for op in [
            "mac.lock_screen",
            "mac.sleep_display",
            "mac.restart",
            "mac.shutdown",
            "mac.other",
        ] {
            assert_eq!(
                prepare(&probe, &action(op, json!({})), &Value::Null)
                    .expect_err(op)
                    .code,
                "unsupported"
            );
        }
        assert!(probe.calls.borrow().is_empty());
    }

    #[test]
    fn bounds_refuse_before_anything_reaches_the_mac() {
        let probe = Fake::new(vec![]);
        for (op, args) in [
            ("mac.notify", json!({"title": "", "body": "b"})),
            (
                "mac.notify",
                json!({"title": "x".repeat(MAX_TITLE + 1), "body": "b"}),
            ),
            (
                "mac.notify",
                json!({"title": "t", "body": "x".repeat(MAX_BODY + 1)}),
            ),
            (
                "mac.notify",
                json!({"title": "t", "body": "b", "action_url": "file:///etc/hosts"}),
            ),
            (
                "mac.notify",
                json!({"title": "t", "body": "b", "action_url": "javascript:alert(1)"}),
            ),
            (
                "mac.notify",
                json!({"title": "t", "body": "b", "action_url": "https://"}),
            ),
            (
                "mac.open_settings",
                json!({"pane": "x-apple.systempreferences:com.apple.other"}),
            ),
            ("mac.set_wallpaper", json!({"image_blob": ""})),
            ("mac.set_wallpaper", json!({"image_blob": "not base64!"})),
            (
                "mac.clipboard_write",
                json!({"text": "x".repeat(MAX_CLIPBOARD + 1)}),
            ),
            ("mac.set_volume", json!({"volume": 101})),
            ("mac.set_volume", json!({"volume": -1})),
            ("mac.set_volume", json!({})),
            ("mac.set_mute", json!({"muted": "yes"})),
            ("mac.set_appearance", json!({"appearance": "auto"})),
            ("mac.launch_app", json!({"bundle_id": ""})),
            ("mac.keep_awake", json!({"on": true, "duration": 86_401})),
            ("mac.keep_awake", json!({"on": true, "duration": -1})),
            (
                "mac.iwork_export",
                json!({"app": "word", "format": "pdf", "document_blob": "eA=="}),
            ),
            (
                "mac.iwork_export",
                json!({"app": "pages", "format": "docx", "document_blob": "eA=="}),
            ),
        ] {
            assert_eq!(
                prepare(&probe, &action(op, args.clone()), &Value::Null)
                    .expect_err(op)
                    .code,
                "invalid",
                "{op} {args}"
            );
        }
        assert_eq!(
            prepare(
                &probe,
                &action(
                    "mac.iwork_export",
                    json!({"app": "numbers", "format": "text", "document_blob": "eA=="})
                ),
                &Value::Null
            )
            .expect_err("text")
            .code,
            "unsupported"
        );
        assert!(probe.calls.borrow().is_empty());
    }

    #[test]
    fn the_clipboard_needs_always_allow() {
        let write = action("mac.clipboard_write", json!({"text": "x"}));
        let denied = Fake::new(vec![Ok(json!({"always_allow": false}))]);
        assert_eq!(
            prepare(&denied, &write, &Value::Null)
                .expect_err("refused")
                .code,
            "unavailable"
        );
        let allowed = Fake::new(vec![Ok(json!({"always_allow": true}))]);
        prepare(&allowed, &write, &Value::Null).unwrap();
    }

    #[test]
    fn volume_is_confirmed_by_reading_it_back() {
        let probe = Fake::new(vec![Ok(json!({"result": ""})), Ok(json!({"result": "30"}))]);
        let written = write(
            &probe,
            "c",
            &action("mac.set_volume", json!({"volume": 30})),
            &Value::Null,
        )
        .unwrap();
        assert_eq!(written, json!({"observed": true, "readback": 30.0}));
        assert_eq!(
            probe.calls.borrow()[0].1["source"],
            "set volume output volume 30"
        );
        let wrong = Fake::new(vec![Ok(json!({"result": ""})), Ok(json!({"result": "80"}))]);
        assert_eq!(
            write(
                &wrong,
                "c",
                &action("mac.set_volume", json!({"volume": 30})),
                &Value::Null
            ),
            Err("conflict".to_owned())
        );
    }

    #[test]
    fn appearance_needs_system_events_and_a_read_back() {
        let probe = Fake::new(vec![
            Ok(json!({"result": ""})),
            Ok(json!({"result": "false"})),
        ]);
        let written = write(
            &probe,
            "c",
            &action("mac.set_appearance", json!({"appearance": "light"})),
            &Value::Null,
        )
        .unwrap();
        assert_eq!(written["readback"], "light");
        assert_eq!(probe.calls.borrow()[0].1["target"], SYSTEM_EVENTS);
        let denied = Fake::new(vec![Ok(json!({"error_number": -1743}))]);
        assert_eq!(
            write(
                &denied,
                "c",
                &action("mac.set_appearance", json!({"appearance": "dark"})),
                &Value::Null
            ),
            Err("permission_missing".to_owned())
        );
    }

    #[test]
    fn a_quit_is_seen_only_when_the_app_is_gone() {
        let probe = Fake::new(vec![
            Ok(json!({})),
            Ok(json!({"running": true})),
            Ok(json!({"running": false})),
        ]);
        let written = write(
            &probe,
            "c",
            &action("mac.quit_app", json!({"bundle_id": "com.example"})),
            &Value::Null,
        )
        .unwrap();
        assert_eq!(
            written["readback"],
            json!({"bundle_id": "com.example", "running": false})
        );
    }

    #[test]
    fn a_notification_is_delivered_or_not_observed() {
        let probe = Fake::new(vec![
            Ok(json!({})),
            Ok(json!({"delivered": false})),
            Ok(json!({"delivered": true})),
        ]);
        let written = write(
            &probe,
            "c",
            &action("mac.notify", json!({"title": "t", "body": "b"})),
            &Value::Null,
        )
        .unwrap();
        assert_eq!(written["readback"]["delivered"], true);
    }

    #[test]
    fn export_paths_cannot_leave_their_strings() {
        let script = export_script(
            "com.apple.iWork.Pages",
            Path::new("/t/a\"b.txt"),
            Path::new("/t/o.pdf"),
            "pdf",
        );
        assert!(script.contains("open POSIX file \"/t/a\\\"b.txt\""));
        assert!(script.contains("as PDF"));
    }

    #[test]
    fn bluetooth_names_come_from_the_report() {
        let report = json!({"SPBluetoothDataType": [{"device_connected": [{"One": {}}], "device_not_connected": [{"Two": {}}]}]});
        assert_eq!(
            bluetooth_devices(&report).unwrap(),
            json!({"devices": [{"name": "One", "connected": true}, {"name": "Two", "connected": false}]})
        );
        assert!(bluetooth_devices(&json!({})).is_none());
    }
}
