use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}
pub fn capture(op: &str) -> bool {
    matches!(
        op,
        "capture.screenshot"
            | "capture.camera_photo"
            | "capture.record_audio"
            | "mac.screen_record"
            | "print.document"
    )
}
pub fn playback(op: &str) -> bool {
    matches!(op, "speech.say" | "capture.play_sound")
}
pub(crate) fn requires_capture(op: &str) -> bool {
    !capture(op)
        && !playback(op)
        && !op.starts_with("mac.")
        && !matches!(op, "maps.open" | "shortcuts.run" | "focus.set")
}
fn clean_path(parts: &[&str]) -> String {
    let joined = parts.join("/");
    let mut result = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                result.pop();
            }
            _ => result.push(part),
        }
    }
    format!("/{}", result.join("/"))
}
#[allow(clippy::too_many_lines)]
pub fn valid(op: &str, args: &Value, e: &Value, hash: &str) -> bool {
    let bound = !hash.is_empty() && e["fingerprint"] == hash && e["observed"] == true;
    if capture(op) {
        return !hash.is_empty()
            && e["observed"] == true
            && e["prepared"]["request_hash"] == hash
            && ab_blobs::Digest::try_from(s(e, "digest").to_owned()).is_ok()
            && e["size"].as_u64().is_some_and(|n| n > 0 && n <= 128 << 20);
    }
    if playback(op) {
        return !hash.is_empty()
            && e["prepared"]["request_hash"] == hash
            && e["finished_callback"] == true;
    }
    if op == "maps.open" {
        return !hash.is_empty()
            && e["prepared"]["request_hash"] == hash
            && e["frontmost_bundle"] == "com.apple.Maps";
    }
    if op == "shortcuts.run" {
        return !hash.is_empty()
            && e["prepared"]["request_hash"] == hash
            && e["exit_status"].as_i64() == Some(0)
            && e["timed_out"] != true
            && e["effect_scope"] == "runner completion only"
            && STANDARD
                .decode(s(e, "output_base64").replace(['\r', '\n'], ""))
                .is_ok_and(|v| v.len() <= 32 << 10);
    }
    if op == "focus.set" {
        return bound
            && e["prepared"]["request_hash"] == hash
            && e["readback"]["authorization"] == "authorized"
            && args["on"].is_boolean()
            && e["readback"]["focused"] == args["on"];
    }
    if op.starts_with("mac.") {
        return bound && !e["readback"].is_null();
    }
    if op.starts_with("files.") {
        if !bound {
            return false;
        }
        if op == "files.trash" {
            return e["trashed"] == true
                && (s(e, "trash_path").contains("/.Trash/")
                    || s(e, "trash_path").contains("/.Trashes/"));
        }
        let expected = if matches!(
            op,
            "files.create" | "files.write" | "files.move" | "files.rename"
        ) {
            clean_path(&[s(args, "root"), s(args, "parent"), s(args, "name")])
        } else {
            clean_path(&[s(args, "root"), s(args, "path")])
        };
        if e["path"] != expected {
            return false;
        }
        if matches!(op, "files.create" | "files.write") {
            return e["digest"] == args["digest"];
        }
        if matches!(op, "files.download" | "files.evict") {
            return e["downloaded"] == Value::Bool(op == "files.download");
        }
        if op == "files.set_tags" {
            let mut actual = e["tags"].as_array().cloned().unwrap_or_default();
            let mut expected = args["tags"].as_array().cloned().unwrap_or_default();
            actual.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
            expected.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
            return actual == expected;
        }
    }
    true
}
