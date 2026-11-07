use crate::CommandRecord;
use ab_domain::{CommandState, Source};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::Verifier;
use serde_json::Value;

pub(crate) fn completion(r: &CommandRecord) -> bool {
    let Some(raw) = &r.evidence else {
        return false;
    };
    let Ok(e) = serde_json::from_str::<Value>(raw.get()) else {
        return false;
    };
    let hash = !r.request_hash.is_empty();
    let prepared = hash && e["prepared"]["request_hash"].as_str() == Some(&r.request_hash);
    let observed = r.state == CommandState::Observed;
    let runner = r.command.op == "shortcuts.run"
        && r.source == Source::Shortcuts
        && observed
        && r.commit_seq == 0
        && prepared
        && e["exit_status"].as_i64() == Some(0)
        && !e["timed_out"].as_bool().unwrap_or(false)
        && e["effect_scope"] == "runner completion only"
        && STANDARD
            .decode(e["output_base64"].as_str().unwrap_or(""))
            .is_ok_and(|b| b.len() <= 32 << 10);
    let maps = r.command.op == "maps.open"
        && r.source == Source::Maps
        && observed
        && r.commit_seq == 0
        && prepared
        && e["frontmost_bundle"] == "com.apple.Maps";
    let playback = matches!(r.command.op.as_str(), "speech.say" | "capture.play_sound")
        && serde_json::to_value(r.source)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .as_deref()
            == r.command.op.split('.').next()
        && observed
        && r.commit_seq == 0
        && prepared
        && e["finished_callback"] == true;
    let mac = r.source == Source::Mac
        && observed
        && hash
        && e["observed"] == true
        && e["fingerprint"].as_str() == Some(&r.request_hash)
        && e.get("readback").is_some_and(|v| !v.is_null());
    let capture = matches!(
        r.command.op.as_str(),
        "capture.screenshot"
            | "capture.camera_photo"
            | "capture.record_audio"
            | "mac.screen_record"
            | "print.document"
    ) && prepared
        && e["observed"] == true
        && e["digest"]
            .as_str()
            .is_some_and(crate::projection::valid_digest)
        && e["size"].as_i64().is_some_and(|n| n > 0 && n <= 128 << 20);
    let location = r.command.op == "location.current"
        && r.source == Source::Location
        && observed
        && r.commit_seq == 0
        && e["approval_consumed"] == true
        && e["query_succeeded"] == true
        && e["request_hash"].as_str() == Some(&r.request_hash);
    let photos = r.source == Source::Photos
        && observed
        && r.commit_seq == 0
        && hash
        && e["approval_consumed"] == true
        && e["query_succeeded"] == true
        && e["request_hash"].as_str() == Some(&r.request_hash)
        && matches!(
            r.command.op.as_str(),
            "photos.favorite"
                | "photos.album_create"
                | "photos.album_rename"
                | "photos.album_add"
                | "photos.album_remove"
                | "photos.album_delete"
        );
    runner || maps || playback || mac || capture || location || photos || iphone(r, &e)
}
fn iphone(r: &CommandRecord, e: &Value) -> bool {
    if r.source != Source::Iphone || r.state != CommandState::Observed {
        return false;
    }
    let decode = |key| STANDARD.decode(e[key].as_str().unwrap_or(""));
    let (Ok(payload), Ok(signature), Ok(key)) = (
        decode("payload"),
        decode("signature"),
        decode("signing_key"),
    ) else {
        return false;
    };
    let (Ok(result), Ok(args)) = (
        serde_json::from_slice::<Value>(&payload),
        serde_json::from_str::<Value>(r.command.args.get()),
    ) else {
        return false;
    };
    if result["command_id"].as_str() != Some(r.command.id.as_str())
        || result["request_hash"] != r.request_hash
        || result["state"] != "observed"
        || e["device_id"] != args["device_id"]
    {
        return false;
    }
    let Ok(key) = <&[u8; 32]>::try_from(key.as_slice()) else {
        return false;
    };
    let (Ok(key), Ok(signature)) = (
        ed25519_dalek::VerifyingKey::from_bytes(key),
        ed25519_dalek::Signature::from_slice(&signature),
    ) else {
        return false;
    };
    let mut message = format!(
        "apple-bridge-iphone/1\0/v1/iphone/result\0{}\0",
        e["at"].as_i64().unwrap_or(0)
    )
    .into_bytes();
    message.extend(payload);
    key.verify(&message, &signature).is_ok()
}
