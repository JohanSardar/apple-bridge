use crate::{
    assets::Store,
    body_digest,
    capture::{Error, Result},
    native_json,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
fn string<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key].as_str().unwrap_or_default()
}
pub(crate) fn references(data: &Value) -> Vec<Value> {
    let mut result = Vec::new();
    if let Some(attachments) = data["attachments"].as_array() {
        for attachment in attachments {
            if !attachment["blob"].is_null() {
                result.push(attachment["blob"].clone());
            }
            if let Some(variants) = attachment["derivatives"].as_object() {
                result.extend(variants.values().cloned());
            }
        }
    }
    for key in ["raw_body", "body_blob", "thumbnail"] {
        if let Some(value) = data.get(key) {
            result.push(value.clone());
        }
    }
    result
}
pub(crate) fn pending(original: &Value) -> Result<Value> {
    let mut data = original.clone();
    let text = string(original, "plain_text");
    let mut lines = text.split('\n');
    let first = lines.next().unwrap_or_default();
    let preview = if first == string(original, "title") {
        lines.find(|v| !v.is_empty()).unwrap_or_default()
    } else {
        text.split('\n').find(|v| !v.is_empty()).unwrap_or_default()
    };
    data["preview"] = json!(
        preview
            .replace('\u{fffc}', "")
            .chars()
            .take(160)
            .collect::<String>()
    );
    if text.len() > 160_000 || native_json::foundation_data(&data)?.len() >= 220_000 {
        data.as_object_mut()
            .ok_or(Error::Corrupt)?
            .remove("plain_text");
        data["search_status"] = json!("metadata_only");
    }
    if native_json::foundation_data(&data)?.len() >= 220_000 {
        data["title"] = json!(
            string(original, "title")
                .chars()
                .take(160)
                .collect::<String>()
        );
        data["title_status"] = json!("metadata_capacity");
    }
    Ok(data)
}
fn size(data: &Value) -> Result<usize> {
    let digests = references(data)
        .into_iter()
        .map(|v| v["digest"].as_str().unwrap_or_default().to_owned())
        .collect::<BTreeSet<_>>();
    Ok(native_json::foundation_data(data)?.len() + digests.len() * 160 + 4096)
}
pub(crate) fn fit(original: &Value, assets: &mut Store) -> Result<Value> {
    if size(original)? < 224_000 {
        return Ok(original.clone());
    }
    let mut data = original.clone();
    if original["locked"] == true {
        data["title"] = json!(
            string(original, "title")
                .chars()
                .take(160)
                .collect::<String>()
        );
        data["title_status"] = json!("metadata_capacity");
        return Ok(data);
    }
    let body = json!({"plain_text":original.get("plain_text").cloned().unwrap_or(json!("")),"html":original.get("html").cloned().unwrap_or(json!("")),"blocks":original.get("blocks").cloned().unwrap_or(json!([]))});
    let body = native_json::foundation_data(&body)?;
    let mut lines = string(original, "plain_text")
        .split('\n')
        .collect::<Vec<_>>();
    if lines
        .first()
        .is_some_and(|line| line.trim() == string(original, "title").trim())
    {
        lines.remove(0);
    }
    let preview = lines
        .into_iter()
        .map(|line| {
            line.replace('\u{fffc}', "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    data["preview"] = json!(preview.chars().take(160).collect::<String>());
    for key in ["html", "blocks"] {
        data.as_object_mut().ok_or(Error::Corrupt)?.remove(key);
    }
    data["editable"] = json!("none");
    data["write_limit_reason"] = json!("external_body_write_unqualified");
    if let Ok(reference) = assets.bytes(&body) {
        data["body_blob"] = serde_json::to_value(reference)?;
        data["body_status"] = json!("external");
    } else {
        data["body_blob"] =
            json!({"digest":body_digest(&body),"size":body.len(),"state":"missing"});
        data["body_status"] = json!("unavailable");
        data["content_status"] = json!("offloaded");
    }
    if size(&data)? >= 224_000 {
        data.as_object_mut()
            .ok_or(Error::Corrupt)?
            .remove("plain_text");
        data["search_status"] = json!("metadata_only");
    }
    if size(&data)? >= 224_000 {
        let thumb = data["attachments"]
            .as_array()
            .and_then(|attachments| {
                attachments
                    .iter()
                    .map(|v| &v["derivatives"]["thumb"])
                    .find(|v| !v.is_null())
            })
            .cloned();
        if let Some(thumb) = thumb {
            data["thumbnail"] = thumb;
        } else {
            data.as_object_mut()
                .ok_or(Error::Corrupt)?
                .remove("thumbnail");
        }
        data["attachments"] = json!([]);
        data["attachment_status"] = json!("metadata_capacity");
        data["block_status"] = json!("partial");
    }
    if size(&data)? >= 224_000 {
        data.as_object_mut().ok_or(Error::Corrupt)?.remove("tags");
        data["tags_status"] = json!("metadata_capacity");
        data["title"] = json!(
            string(original, "title")
                .chars()
                .take(160)
                .collect::<String>()
        );
        data["title_status"] = json!("metadata_capacity");
    }
    if size(&data)? >= 224_000 {
        return Err(Error::Corrupt);
    }
    Ok(data)
}
