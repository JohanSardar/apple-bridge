use crate::{Code, Command, Error};
use serde_json::Value;

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

pub(crate) fn validate(command: &Command, args: &Value) -> Result<(), Error> {
    if command.op == "maps.open" {
        return maps(args);
    }
    if command.op.starts_with("files.")
        || matches!(command.op.as_str(), "speech.say" | "capture.play_sound")
    {
        ab_commands::validate(
            &ab_commands::Command {
                id: if command.id.is_empty() {
                    "cmd_validation".into()
                } else {
                    command.id.clone()
                },
                op: command.op.clone(),
                args: command.args.clone(),
                expected_version: command.expected_version,
                wait_ms: i64::try_from(command.wait_ms).map_err(|_| Code::Invalid)?,
            },
            jiff::Timestamp::now(),
        )
        .map_err(|error| match error {
            ab_commands::Error::Unsupported => Error::from(Code::Unsupported),
            _ => Error::from(Code::Invalid),
        })?;
    }
    Ok(())
}

pub(crate) fn transient_completion(command: &Command, proof: &Value, hash: &str) -> Option<bool> {
    match command.op.as_str() {
        "maps.open" => Some(
            !hash.is_empty()
                && proof["prepared"]["request_hash"] == hash
                && proof["frontmost_bundle"] == "com.apple.Maps",
        ),
        "speech.say" | "capture.play_sound" => Some(
            !hash.is_empty()
                && proof["prepared"]["request_hash"] == hash
                && proof["finished_callback"] == true,
        ),
        _ => None,
    }
}

pub(crate) fn file_completion(command: &Command, proof: &Value, hash: &str) -> bool {
    let Ok(args) = serde_json::from_str::<Value>(command.args.get()) else {
        return false;
    };
    if hash.is_empty() || proof["fingerprint"] != hash || proof["observed"] != true {
        return false;
    }
    if command.op == "files.trash" {
        let trash = text(proof, "trash_path");
        return proof["trashed"] == true
            && (trash.contains("/.Trash/") || trash.contains("/.Trashes/"));
    }
    let mut expected = std::path::PathBuf::from(text(&args, "root"));
    if matches!(
        command.op.as_str(),
        "files.create" | "files.write" | "files.move" | "files.rename"
    ) {
        expected.push(text(&args, "parent"));
        expected.push(text(&args, "name"));
    } else {
        expected.push(text(&args, "path"));
    }
    if proof["path"].as_str() != expected.to_str() {
        return false;
    }
    match command.op.as_str() {
        "files.create" | "files.write" => proof["digest"] == args["digest"],
        "files.download" | "files.evict" => proof["downloaded"] == (command.op == "files.download"),
        "files.set_tags" => {
            let tags = |value: &Value| -> Option<Vec<String>> {
                if value.is_null() {
                    return Some(Vec::new());
                }
                let mut tags = value
                    .as_array()?
                    .iter()
                    .map(|value| value.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?;
                tags.sort_unstable();
                Some(tags)
            };
            let observed = tags(&proof["tags"]);
            observed.is_some() && observed == tags(&args["tags"])
        }
        _ => true,
    }
}

fn fields(args: &Value, allowed: &[&str]) -> bool {
    args.as_object()
        .is_some_and(|args| args.keys().all(|key| allowed.contains(&key.as_str())))
}

fn coordinate(point: &Value) -> bool {
    fields(point, &["lat", "lon"])
        && point["lat"].as_f64().is_some_and(|n| n.abs() <= 90.0)
        && point["lon"].as_f64().is_some_and(|n| n.abs() <= 180.0)
}

fn maps(args: &Value) -> Result<(), Error> {
    let route = !args["from"].is_null()
        || !args["to"].is_null()
        || args["mode"].as_str().is_some_and(|mode| !mode.is_empty());
    if !fields(args, &["from", "to", "mode", "lat", "lon"])
        || args
            .get("mode")
            .is_some_and(|mode| !mode.is_null() && !mode.is_string())
    {
        return Err(Code::Invalid.into());
    }
    let valid = if route {
        args["lat"].is_null()
            && args["lon"].is_null()
            && coordinate(&args["from"])
            && coordinate(&args["to"])
            && matches!(
                args["mode"].as_str(),
                Some("driving" | "walking" | "transit" | "cycling")
            )
    } else {
        coordinate(&serde_json::json!({"lat":args["lat"], "lon":args["lon"]}))
    };
    if valid {
        Ok(())
    } else {
        Err(Code::Invalid.into())
    }
}
