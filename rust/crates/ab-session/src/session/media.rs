use super::owned::{Driver, Probe, Refusal, Settled, native_failure, refuse, text};
use crate::{production, staged::digest_of};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use std::{
    fmt::Write as _,
    fs,
    io::Read,
    os::unix::fs::{FileTypeExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const SOUNDS: [&str; 14] = [
    "Basso",
    "Blow",
    "Bottle",
    "Frog",
    "Funk",
    "Glass",
    "Hero",
    "Morse",
    "Ping",
    "Pop",
    "Purr",
    "Sosumi",
    "Submarine",
    "Tink",
];
const MAX_SPEECH: usize = 2048;
const MAPS: &str = "com.apple.Maps";
const MAX_ARTIFACT: u64 = 128 << 20;
const MAX_DOCUMENT: u64 = 8 << 20;
const MAX_ANNOTATED: u64 = 16 << 20;
const MAX_ANNOTATIONS: usize = 100;
const MAX_PAGES: u64 = 1000;
const MAX_NOTE: usize = 4096;
const DURATION_TOLERANCE: f64 = 0.5;

fn only_keys(args: &Value, allowed: &[&str]) -> bool {
    args.as_object()
        .is_some_and(|fields| fields.keys().all(|key| allowed.contains(&key.as_str())))
}

fn no_resolve(_probe: &dyn Probe, _evidence: &Value) -> Option<Value> {
    None
}

fn no_reconcile(_found: &Value, _operation: &str) -> Value {
    json!({})
}

fn playback_checked(action: &Value) -> Result<(), Refusal> {
    let args = &action["args"];
    let volume = &args["volume"];
    if !only_keys(args, &["text", "name", "volume"])
        || !(volume.is_null() || volume.as_f64().is_some_and(|v| (0.0..=1.0).contains(&v)))
    {
        return Err(refuse("invalid"));
    }
    let valid = match text(action, "op") {
        "speech.say" => {
            let words = text(args, "text");
            !words.trim().is_empty() && words.len() <= MAX_SPEECH && text(args, "name").is_empty()
        }
        "capture.play_sound" => {
            SOUNDS.contains(&text(args, "name")) && text(args, "text").is_empty()
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(refuse("invalid"))
    }
}

fn playback_prepare(
    _probe: &dyn Probe,
    action: &Value,
    _expected: &Value,
) -> Result<Value, Refusal> {
    playback_checked(action)?;
    Ok(json!({"route": "playback"}))
}

fn playback_validate(_probe: &dyn Probe, action: &Value, _state: &Value) -> Result<(), Refusal> {
    playback_checked(action)
}

fn playback_write(
    probe: &dyn Probe,
    _command: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    let args = &action["args"];
    let volume = args["volume"].as_f64().unwrap_or(1.0);
    let answer = if text(action, "op") == "speech.say" {
        probe.call(
            "speech.say",
            &json!({"text": args["text"], "volume": volume}),
        )?
    } else {
        probe.call(
            "capture.play_sound",
            &json!({"path": format!("/System/Library/Sounds/{}.aiff", text(args, "name")), "volume": volume}),
        )?
    };
    let finished = answer["finished"] == true;
    Ok(json!({"finished_callback": finished, "observed": finished}))
}

#[allow(clippy::unnecessary_wraps)]
fn playback_settle(_operation: &str, written: &Value) -> Option<Settled> {
    Some(Settled {
        disposition: if written["observed"] == true {
            "locally_observed"
        } else {
            "indeterminate"
        },
        evidence: written.clone(),
    })
}

fn playback_resolved(_probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    (evidence["finished_callback"] == true
        && !text(&evidence["prepared"], "request_hash").is_empty())
    .then(|| {
        let mut found = evidence.clone();
        found["observed"] = json!(true);
        found
    })
}

pub(super) const SPEECH: Driver = Driver {
    name: "speech",
    prepare: playback_prepare,
    prepare_command: None,
    validate: playback_validate,
    write: playback_write,
    settle: playback_settle,
    resolved: playback_resolved,
    reconcile_args: no_reconcile,
};

fn valid_point(point: &Value) -> bool {
    only_keys(point, &["lat", "lon"])
        && point["lat"].as_f64().is_some_and(|lat| lat.abs() <= 90.0)
        && point["lon"].as_f64().is_some_and(|lon| lon.abs() <= 180.0)
}

fn maps_checked(action: &Value) -> Result<(), Refusal> {
    let args = &action["args"];
    let route = !args["from"].is_null() || !args["to"].is_null();
    let valid = text(action, "op") == "maps.open"
        && if route {
            only_keys(args, &["from", "to", "mode"])
                && valid_point(&args["from"])
                && valid_point(&args["to"])
                && matches!(
                    text(args, "mode"),
                    "driving" | "walking" | "transit" | "cycling"
                )
        } else {
            valid_point(args)
        };
    if valid {
        Ok(())
    } else {
        Err(refuse("invalid"))
    }
}

fn maps_prepare(_probe: &dyn Probe, action: &Value, _expected: &Value) -> Result<Value, Refusal> {
    maps_checked(action)?;
    Ok(json!({"route": "maps"}))
}

fn maps_validate(_probe: &dyn Probe, action: &Value, _state: &Value) -> Result<(), Refusal> {
    maps_checked(action)
}

fn maps_write(
    probe: &dyn Probe,
    _command: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    probe.call("maps.show", &action["args"])?;
    for _ in 0..40 {
        let front = probe.call("mac.frontmost", &json!({}))?;
        if text(&front, "bundle_id") == MAPS {
            return Ok(json!({"frontmost_bundle": MAPS}));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(json!({}))
}

#[allow(clippy::unnecessary_wraps)]
fn maps_settle(_operation: &str, written: &Value) -> Option<Settled> {
    Some(Settled {
        disposition: if text(written, "frontmost_bundle") == MAPS {
            "locally_observed"
        } else {
            "indeterminate"
        },
        evidence: written.clone(),
    })
}

fn maps_resolved(_probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let proven = text(evidence, "frontmost_bundle") == MAPS
        && !text(&evidence["prepared"], "request_hash").is_empty()
        && text(&evidence["prepared"]["action"], "op") == "maps.open";
    proven.then(|| {
        let mut found = evidence.clone();
        found["observed"] = json!(true);
        found["disposition"] = json!("locally_observed");
        found
    })
}

pub(super) const MAPS_DRIVER: Driver = Driver {
    name: "maps",
    prepare: maps_prepare,
    prepare_command: None,
    validate: maps_validate,
    write: maps_write,
    settle: maps_settle,
    resolved: maps_resolved,
    reconcile_args: no_reconcile,
};

fn capture_args(action: &Value) -> Result<(), Refusal> {
    let operation = text(action, "op");
    let args = &action["args"];
    if !args.is_object() {
        return Err(refuse("invalid"));
    }
    let allowed: &[&str] = match operation {
        "capture.screenshot" => {
            if !(args["display"].is_null() || args["window"].is_null()) {
                return Err(refuse("invalid"));
            }
            for value in args.as_object().into_iter().flat_map(Map::values) {
                if !value.as_f64().is_some_and(|n| n > 0.0 && n.fract() == 0.0) {
                    return Err(refuse("invalid"));
                }
            }
            &["display", "window"]
        }
        "capture.camera_photo" => &[],
        "capture.record_audio" | "mac.screen_record" => {
            let limit = if operation == "mac.screen_record" {
                30.0
            } else {
                60.0
            };
            if !args["seconds"]
                .as_f64()
                .is_some_and(|s| (1.0..=limit).contains(&s))
            {
                return Err(refuse("invalid"));
            }
            &["seconds"]
        }
        "print.document" => {
            if text(args, "file").is_empty() || text(args, "printer").is_empty() {
                return Err(refuse("unsupported"));
            }
            &["file", "printer"]
        }
        _ => return Err(refuse("invalid")),
    };
    if only_keys(args, allowed) {
        Ok(())
    } else {
        Err(refuse("invalid"))
    }
}

fn capture_checked(probe: &dyn Probe, action: &Value) -> Result<(), Refusal> {
    capture_args(action)?;
    let answer = probe
        .call("capture.authorized", &json!({"op": action["op"]}))
        .map_err(|code| native_failure(&code))?;
    if answer["authorized"] == true {
        Ok(())
    } else {
        let permission = match text(action, "op") {
            "capture.screenshot" | "mac.screen_record" => "Screen & System Audio Recording",
            "capture.camera_photo" => "Camera",
            "capture.record_audio" => "Microphone",
            _ => return Err(refuse("forbidden")),
        };
        Err(Refusal {
            message: Some(format!(
                "{permission} permission is not allowed. Run bridge permissions guide, or enable Apple Bridge in System Settings > Privacy & Security > {permission}, then run bridge down and bridge up. Other sources still work."
            )),
            ..refuse("forbidden")
        })
    }
}

fn capture_prepare(probe: &dyn Probe, action: &Value, expected: &Value) -> Result<Value, Refusal> {
    if text(action, "op") == "capture.play_sound" {
        return playback_prepare(probe, action, expected);
    }
    capture_checked(probe, action)?;
    Ok(Value::Null)
}

fn capture_validate(probe: &dyn Probe, action: &Value, _state: &Value) -> Result<(), Refusal> {
    if text(action, "op") == "capture.play_sound" {
        return playback_checked(action);
    }
    capture_checked(probe, action)
}

fn verified(
    directory: &Path,
    path: &Path,
    action: &Value,
    reported: &Value,
) -> Result<Value, String> {
    let corrupt = || "corrupt".to_owned();
    let parent = path
        .parent()
        .ok_or_else(corrupt)?
        .canonicalize()
        .map_err(|_| corrupt())?;
    if parent != directory.canonicalize().map_err(|_| corrupt())? {
        return Err(corrupt());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| corrupt())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > MAX_ARTIFACT
    {
        return Err(corrupt());
    }
    let duration = reported["duration"].as_f64().unwrap_or(0.0);
    let (width, height) = (
        reported["width"].as_u64().unwrap_or(0),
        reported["height"].as_u64().unwrap_or(0),
    );
    if !duration.is_finite() || duration < 0.0 {
        return Err(corrupt());
    }
    let seconds = action["args"]["seconds"].as_f64().unwrap_or(0.0);
    if seconds != 0.0 && (duration - seconds).abs() > DURATION_TOLERANCE {
        return Err(corrupt());
    }
    let visual = matches!(
        text(action, "op"),
        "capture.screenshot" | "capture.camera_photo" | "mac.screen_record"
    );
    if visual && (width == 0 || height == 0) {
        return Err(corrupt());
    }
    let bytes = fs::read(path).map_err(|_| corrupt())?;
    if bytes.len() as u64 != metadata.len() {
        return Err(corrupt());
    }
    let digest = digest_of(&bytes);
    Ok(json!({
        "handle": format!("{}{digest}", production::CAPTURE_BLOB),
        "digest": digest,
        "size": metadata.len(),
        "duration": duration,
        "width": width,
        "height": height,
        "observed": true,
    }))
}

fn send_to_printer(probe: &dyn Probe, printer: &str, file: &str) -> Result<(), String> {
    let known = probe.call("capture.printers", &json!({}))?;
    if !known["printers"]
        .as_array()
        .is_some_and(|names| names.iter().any(|name| name == printer))
    {
        return Err("unavailable".to_owned());
    }
    let status = Command::new("/usr/bin/lp")
        .args(["-d", printer, "--", file])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "unavailable".to_owned())?;
    if status.success() {
        Ok(())
    } else {
        Err("unavailable".to_owned())
    }
}

fn capture_write(
    probe: &dyn Probe,
    command: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    if text(action, "op") == "capture.play_sound" {
        return playback_write(probe, command, action, state);
    }
    let blobs = probe
        .staged(production::CAPTURE_BLOB)
        .ok_or("unavailable")?;
    let directory = blobs.directory().to_path_buf();
    make_private(&directory)?;
    let produced = probe.call(
        "capture.produce",
        &json!({"op": action["op"], "args": action["args"], "directory": directory.to_string_lossy()}),
    )?;
    let path = PathBuf::from(text(&produced, "path"));
    let done = (|| {
        let evidence = verified(&directory, &path, action, &produced)?;
        if text(action, "op") == "print.document" && text(&action["args"], "printer") != "PDF" {
            send_to_printer(
                probe,
                text(&action["args"], "printer"),
                text(&action["args"], "file"),
            )?;
        }
        Ok::<Value, String>(evidence)
    })();
    match done {
        Ok(evidence) => {
            blobs
                .remember(text(&evidence, "digest"), path)
                .map_err(|_| "unavailable".to_owned())?;
            Ok(evidence)
        }
        Err(code) => {
            if path.parent().and_then(|parent| parent.canonicalize().ok())
                == directory.canonicalize().ok()
            {
                let _ = fs::remove_file(&path);
            }
            Err(code)
        }
    }
}

fn make_private(directory: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|_| "unavailable".to_owned())
}

#[allow(clippy::unnecessary_wraps)]
fn capture_settle(operation: &str, written: &Value) -> Option<Settled> {
    if operation == "capture.play_sound" {
        return playback_settle(operation, written);
    }
    Some(Settled {
        disposition: "locally_observed",
        evidence: written.clone(),
    })
}

fn capture_resolved(probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let action = &evidence["prepared"]["action"];
    if text(action, "op") == "capture.play_sound" {
        return playback_resolved(probe, evidence);
    }
    let blobs = probe.staged(production::CAPTURE_BLOB)?;
    let path = blobs.path_of(text(evidence, "digest")).ok()??;
    let again = verified(blobs.directory(), &path, action, evidence).ok()?;
    (again["digest"] == evidence["digest"] && again["size"] == evidence["size"]).then_some(again)
}

pub(super) const CAPTURE: Driver = Driver {
    name: "capture",
    prepare: capture_prepare,
    prepare_command: None,
    validate: capture_validate,
    write: capture_write,
    settle: capture_settle,
    resolved: capture_resolved,
    reconcile_args: no_reconcile,
};

pub(super) fn document(args: &Value) -> Result<Vec<u8>, &'static str> {
    let encoded = text(args, "data_base64");
    let path = text(args, "file");
    if encoded.is_empty() == path.is_empty() {
        return Err("invalid");
    }
    if !encoded.is_empty() {
        let bytes = STANDARD.decode(encoded).map_err(|_| "invalid")?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_DOCUMENT {
            return Err("invalid");
        }
        return Ok(bytes);
    }
    if !path.starts_with('/') {
        return Err("invalid");
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(ab_sys::O_NONBLOCK)
        .open(path)
        .map_err(|_| "unavailable")?;
    let metadata = file.metadata().map_err(|_| "unavailable")?;
    let kind = metadata.file_type();
    if !metadata.is_file() || kind.is_fifo() || metadata.len() == 0 || metadata.len() > MAX_DOCUMENT
    {
        return Err("invalid");
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "unavailable")?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_DOCUMENT {
        return Err("invalid");
    }
    Ok(bytes)
}

fn annotations(args: &Value) -> Option<&Vec<Value>> {
    args["annotations"]
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= MAX_ANNOTATIONS)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, &'static str> {
        use std::{io::Read as _, os::unix::fs::DirBuilderExt};
        let mut random = [0_u8; 8];
        fs::File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut random))
            .map_err(|_| "unavailable")?;
        let name = random
            .iter()
            .fold(String::from("ab-pdf-"), |mut out, byte| {
                let _ = write!(out, "{byte:02x}");
                out
            });
        let path = std::env::temp_dir().join(name);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "unavailable")?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn inspected(probe: &dyn Probe, path: &Path) -> Result<Value, String> {
    let answer = probe.call("pdf.inspect", &json!({"path": path.to_string_lossy()}))?;
    if answer["locked"] != false
        || answer["page_count"]
            .as_u64()
            .is_none_or(|count| count > MAX_PAGES)
    {
        return Err("invalid".to_owned());
    }
    Ok(answer)
}

fn pdf_prepare(probe: &dyn Probe, action: &Value, _expected: &Value) -> Result<Value, Refusal> {
    if text(action, "op") != "pdf.annotate" || annotations(&action["args"]).is_none() {
        return Err(refuse("invalid"));
    }
    let bytes = document(&action["args"]).map_err(refuse)?;
    let scratch = Scratch::new().map_err(refuse)?;
    let input = scratch.0.join("input.pdf");
    fs::write(&input, &bytes).map_err(|_| refuse("unavailable"))?;
    inspected(probe, &input).map_err(|code| native_failure(&code))?;
    Ok(json!({"digest": digest_of(&bytes)}))
}

#[allow(clippy::unnecessary_wraps)]
fn pdf_validate(_probe: &dyn Probe, _action: &Value, _state: &Value) -> Result<(), Refusal> {
    Ok(())
}

pub(super) fn annotations_fit(items: &[Value], pages: &[Value]) -> bool {
    items.iter().all(|item| {
        let Some(bounds) = item["page"]
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| pages.get(index))
        else {
            return false;
        };
        let (x, y, width, height) = (
            item["x"].as_f64().unwrap_or(f64::NAN),
            item["y"].as_f64().unwrap_or(f64::NAN),
            item["width"].as_f64().unwrap_or(f64::NAN),
            item["height"].as_f64().unwrap_or(f64::NAN),
        );
        let (left, bottom, right, top) = (
            bounds["x"].as_f64().unwrap_or(0.0),
            bounds["y"].as_f64().unwrap_or(0.0),
            bounds["x"].as_f64().unwrap_or(0.0) + bounds["width"].as_f64().unwrap_or(0.0),
            bounds["y"].as_f64().unwrap_or(0.0) + bounds["height"].as_f64().unwrap_or(0.0),
        );
        [x, y, width, height].iter().all(|n| n.is_finite())
            && width > 0.0
            && height > 0.0
            && x >= left
            && y >= bottom
            && x + width <= right
            && y + height <= top
            && matches!(text(item, "type"), "note" | "highlight")
            && text(item, "text").len() <= MAX_NOTE
    })
}

fn pdf_write(
    probe: &dyn Probe,
    command: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    let args = &action["args"];
    let invalid = || "invalid".to_owned();
    let items = annotations(args).ok_or_else(invalid)?;
    let bytes = document(args).map_err(str::to_owned)?;
    if digest_of(&bytes) != text(state, "digest") {
        return Err("conflict".to_owned());
    }
    let scratch = Scratch::new().map_err(str::to_owned)?;
    let input = scratch.0.join("input.pdf");
    let output = scratch.0.join("output.pdf");
    fs::write(&input, &bytes).map_err(|_| "unavailable".to_owned())?;
    let facts = inspected(probe, &input)?;
    if !annotations_fit(items, facts["pages"].as_array().ok_or_else(invalid)?) {
        return Err(invalid());
    }
    let answer = probe.call(
        "pdf.annotate",
        &json!({"input": input.to_string_lossy(), "output": output.to_string_lossy(), "annotations": items}),
    )?;
    let found = answer["found"].as_array().ok_or("corrupt")?;
    let size = fs::metadata(&output)
        .map_err(|_| "corrupt".to_owned())?
        .len();
    if found.len() != items.len()
        || found.iter().any(|one| one != true)
        || size == 0
        || size > MAX_ANNOTATED
    {
        return Err("corrupt".to_owned());
    }
    let blobs = probe.staged(production::PDF_BLOB).ok_or("unavailable")?;
    let (digest, _) = blobs
        .stage(&fs::read(&output).map_err(|_| "corrupt".to_owned())?)
        .map_err(|_| "unavailable".to_owned())?;
    let key = format!("{}-{}", &digest[..16], &digest_of(command.as_bytes())[..16]);
    let metadata = json!({
        "blob": {"digest": digest, "size": size, "state": "present"},
        "annotation_count": items.len(),
    });
    probe.call(
        "pdf.add_artifact",
        &json!({"native_key": key, "data": metadata, "digest": digest, "handle": blobs.handle(&digest)}),
    )?;
    Ok(json!({"native_key": key, "annotation_count": items.len(), "observed": true}))
}

fn pdf_settle(_operation: &str, written: &Value) -> Option<Settled> {
    (written["observed"] == true).then(|| Settled {
        disposition: "locally_observed",
        evidence: written.clone(),
    })
}

pub(super) const PDF: Driver = Driver {
    name: "pdf",
    prepare: pdf_prepare,
    prepare_command: None,
    validate: pdf_validate,
    write: pdf_write,
    settle: pdf_settle,
    resolved: no_resolve,
    reconcile_args: no_reconcile,
};

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps
)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        calls: RefCell<Vec<String>>,
        authorized: bool,
    }

    impl Fake {
        fn new(authorized: bool) -> Self {
            Self {
                calls: RefCell::default(),
                authorized,
            }
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, _: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push(operation.to_owned());
            match operation {
                "capture.authorized" => Ok(json!({"authorized": self.authorized})),
                "speech.say" | "capture.play_sound" => Ok(json!({"finished": true})),
                "maps.show" => Ok(json!({})),
                "mac.frontmost" => Ok(json!({"bundle_id": MAPS})),
                _ => Err("unsupported".into()),
            }
        }
    }

    fn action(op: &str, args: Value) -> Value {
        json!({"op": op, "args": args})
    }

    #[test]
    fn playback_refuses_what_it_should_not_say_or_play() {
        for (op, args) in [
            ("speech.say", json!({"text": "   "})),
            ("speech.say", json!({"text": "x".repeat(MAX_SPEECH + 1)})),
            ("speech.say", json!({"text": "x", "name": "Ping"})),
            ("speech.say", json!({"text": "x", "volume": 2})),
            ("speech.say", json!({"text": "x", "other": 1})),
            ("capture.play_sound", json!({"name": "../../../tmp/x"})),
            ("capture.play_sound", json!({"name": "Unknown"})),
            ("capture.play_sound", json!({"name": "Ping", "text": "x"})),
            (
                "capture.play_sound",
                json!({"name": "Ping", "volume": -0.1}),
            ),
        ] {
            assert_eq!(
                playback_checked(&action(op, args.clone()))
                    .expect_err("refused")
                    .code,
                "invalid",
                "{op} {args}"
            );
        }
        playback_checked(&action("speech.say", json!({"text": "x", "volume": 0.1}))).unwrap();
        playback_checked(&action("capture.play_sound", json!({"name": "Ping"}))).unwrap();
        let probe = Fake::new(true);
        let written = playback_write(
            &probe,
            "c",
            &action("capture.play_sound", json!({"name": "Ping"})),
            &Value::Null,
        )
        .unwrap();
        assert_eq!(written["observed"], true);
    }

    #[test]
    fn maps_only_opens_real_coordinates() {
        for args in [
            json!({"lat": 91, "lon": 0}),
            json!({"lat": 0, "lon": 181}),
            json!({"lat": "0", "lon": 0}),
            json!({"lat": 0}),
            json!({"lat": 0, "lon": 0, "extra": 1}),
            json!({"from": {"lat": 0, "lon": 0}, "to": {"lat": 0, "lon": 0}}),
            json!({"from": {"lat": 0, "lon": 0}, "to": {"lat": 0, "lon": 0}, "mode": "teleport"}),
            json!({"from": {"lat": 100, "lon": 0}, "to": {"lat": 0, "lon": 0}, "mode": "driving"}),
            json!({"from": {"lat": 0, "lon": 0}, "mode": "driving"}),
        ] {
            assert_eq!(
                maps_checked(&action("maps.open", args.clone()))
                    .expect_err("refused")
                    .code,
                "invalid",
                "{args}"
            );
        }
        maps_checked(&action("maps.open", json!({"lat": 1.5, "lon": 2.5}))).unwrap();
        maps_checked(&action(
            "maps.open",
            json!({"from": {"lat": 0, "lon": 0}, "to": {"lat": 1, "lon": 1}, "mode": "walking"}),
        ))
        .unwrap();
        assert_eq!(
            maps_settle("maps.open", &json!({})).unwrap().disposition,
            "indeterminate"
        );
    }

    #[test]
    fn capture_arguments_are_decided_before_any_permission_check() {
        let probe = Fake::new(true);
        for (op, args) in [
            ("capture.screenshot", json!({"display": 1, "window": 2})),
            ("capture.screenshot", json!({"display": 1.5})),
            ("capture.screenshot", json!({"display": -1})),
            ("capture.screenshot", json!({"file": "/etc/hosts"})),
            ("capture.camera_photo", json!({"seconds": 1})),
            ("capture.record_audio", json!({"seconds": 0})),
            ("capture.record_audio", json!({"seconds": 61})),
            ("mac.screen_record", json!({"seconds": 31})),
            ("mac.screen_record", json!({})),
            ("capture.other", json!({})),
        ] {
            assert_eq!(
                capture_checked(&probe, &action(op, args.clone()))
                    .expect_err("refused")
                    .code,
                "invalid",
                "{op} {args}"
            );
        }
        assert_eq!(
            capture_checked(&probe, &action("print.document", json!({"printer": "PDF"})))
                .expect_err("refused")
                .code,
            "unsupported"
        );
        assert!(probe.calls.borrow().is_empty());
        let denied = Fake::new(false);
        assert_eq!(
            capture_checked(&denied, &action("capture.camera_photo", json!({})))
                .expect_err("refused")
                .code,
            "forbidden"
        );
    }

    #[test]
    fn denied_captures_name_the_permission_without_producing_a_file() {
        let probe = Fake::new(false);
        for (op, args, permission) in [
            (
                "capture.screenshot",
                json!({}),
                "Screen & System Audio Recording",
            ),
            (
                "mac.screen_record",
                json!({"seconds":1}),
                "Screen & System Audio Recording",
            ),
            ("capture.camera_photo", json!({}), "Camera"),
            ("capture.record_audio", json!({"seconds":1}), "Microphone"),
        ] {
            let refusal = capture_checked(&probe, &action(op, args)).expect_err("no permission");
            assert_eq!(refusal.code, "forbidden");
            let message = refusal.message.expect("helpful message");
            assert!(message.contains(permission), "{message}");
            assert!(message.contains("bridge permissions guide"), "{message}");
        }
        assert!(
            probe
                .calls
                .borrow()
                .iter()
                .all(|op| op == "capture.authorized")
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ab-media-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path.canonicalize().unwrap()
    }

    #[test]
    fn an_artifact_must_be_real_and_match_the_request() {
        let directory = scratch("artifact");
        let other = scratch("artifact-other");
        let good = directory.join("a");
        fs::write(&good, b"bytes").unwrap();
        let photo = action("capture.screenshot", json!({}));
        let reported = json!({"width": 2, "height": 2});
        let evidence = verified(&directory, &good, &photo, &reported).unwrap();
        assert_eq!(evidence["digest"], digest_of(b"bytes"));
        assert_eq!(evidence["size"], 5);
        assert!(text(&evidence, "handle").starts_with(production::CAPTURE_BLOB));
        let outside = other.join("b");
        fs::write(&outside, b"bytes").unwrap();
        assert!(verified(&directory, &outside, &photo, &reported).is_err());
        let link = directory.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(verified(&directory, &link, &photo, &reported).is_err());
        let empty = directory.join("empty");
        fs::write(&empty, b"").unwrap();
        assert!(verified(&directory, &empty, &photo, &reported).is_err());
        assert!(verified(&directory, &good, &photo, &json!({"width": 0, "height": 2})).is_err());
        let clip = action("capture.record_audio", json!({"seconds": 5}));
        assert!(verified(&directory, &good, &clip, &json!({"duration": 5.4})).is_ok());
        assert!(verified(&directory, &good, &clip, &json!({"duration": 4.0})).is_err());
    }

    #[test]
    fn a_document_is_inline_or_one_regular_file() {
        assert_eq!(document(&json!({})), Err("invalid"));
        assert_eq!(
            document(&json!({"data_base64": "eA==", "file": "/x"})),
            Err("invalid")
        );
        assert_eq!(document(&json!({"data_base64": "!!"})), Err("invalid"));
        assert_eq!(document(&json!({"file": "relative"})), Err("invalid"));
        assert_eq!(
            document(&json!({"file": "/nonexistent/ab-test"})),
            Err("unavailable")
        );
        assert_eq!(document(&json!({"file": "/dev/null"})), Err("invalid"));
        assert_eq!(document(&json!({"file": "/tmp"})), Err("invalid"));
        assert_eq!(document(&json!({"data_base64": "eA=="})), Ok(b"x".to_vec()));
    }

    #[test]
    fn an_annotation_must_fit_on_its_page() {
        let pages = vec![json!({"x": 0, "y": 0, "width": 100, "height": 200})];
        let note = |page: Value, x: f64, y: f64, w: f64, h: f64| json!({"page": page, "type": "note", "text": "t", "x": x, "y": y, "width": w, "height": h});
        assert!(annotations_fit(
            &[note(json!(1), 0.0, 0.0, 100.0, 200.0)],
            &pages
        ));
        for bad in [
            note(json!(0), 0.0, 0.0, 1.0, 1.0),
            note(json!(2), 0.0, 0.0, 1.0, 1.0),
            note(json!(1.5), 0.0, 0.0, 1.0, 1.0),
            note(json!(1), 0.0, 0.0, 101.0, 1.0),
            note(json!(1), -1.0, 0.0, 1.0, 1.0),
            note(json!(1), 0.0, 0.0, 0.0, 1.0),
            json!({"page": 1, "type": "stamp", "text": "", "x": 0, "y": 0, "width": 1, "height": 1}),
            json!({"page": 1, "type": "note", "text": "x".repeat(MAX_NOTE + 1), "x": 0, "y": 0, "width": 1, "height": 1}),
        ] {
            assert!(
                !annotations_fit(std::slice::from_ref(&bad), &pages),
                "{bad}"
            );
        }
    }
}
