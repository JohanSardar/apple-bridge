use crate::{
    body_digest,
    plist::{self, Value},
};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

pub fn group_event_kind(
    item: i64,
    action: i64,
    has_member: bool,
    sender_is_member: bool,
    title: &str,
) -> &'static str {
    if item == 2 && !title.is_empty() {
        return "rename";
    }
    if item == 1 && has_member {
        if action == 1 {
            return "member_removed";
        }
        if action == 0 {
            return if sender_is_member {
                "phone_changed"
            } else {
                "member_added"
            };
        }
    }
    if item == 3 {
        return match action {
            0 => "member_left",
            1 => "photo_changed",
            2 => "photo_removed",
            _ => "unknown",
        };
    }
    "unknown"
}
pub fn balloon_kind(bundle: &str) -> &'static str {
    let v = bundle.to_lowercase();
    for (keys, kind) in [
        (&["urlballoon"][..], "link"),
        (&["digitaltouch"][..], "digital_touch"),
        (&["handwriting"][..], "handwriting"),
        (&["peerpayment", "applecash"][..], "apple_cash"),
        (&["findmy"][..], "find_my"),
        (&["checkin", "safetymonitor"][..], "check_in"),
        (&["poll"][..], "poll"),
        (&["chatbot"][..], "chatbot"),
        (&["msmessageextension"][..], "app"),
    ] {
        if keys.iter().any(|k| v.contains(k)) {
            return kind;
        }
    }
    "unknown"
}
fn text<'a>(m: &'a BTreeMap<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::string)
}
pub fn balloon(bundle: &str, payload: Option<&[u8]>, attachments: &[String]) -> Option<Json> {
    if bundle.is_empty() {
        return None;
    }
    let root = payload
        .and_then(|b| plist::archive(b).ok())
        .unwrap_or_default();
    let mut value = json!({"bundle_id":bundle,"kind":balloon_kind(bundle),"decode_status":if payload.is_none(){"unavailable"}else if root.is_empty(){"undecoded"}else{"decoded"}});
    if let Some(name) = text(&root, "an").or_else(|| {
        if bundle.contains("gamepigeon") {
            Some("GamePigeon")
        } else if bundle.contains("youtube") {
            Some("YouTube")
        } else if bundle.contains("PhotosMessagesApp") {
            Some("Photos")
        } else {
            None
        }
    }) {
        value["app_name"] = json!(name);
    }
    if let Some(bytes) = payload.filter(|_| root.is_empty()) {
        value["raw_payload_digest"] = json!(body_digest(bytes));
    }
    project_balloon(&mut value, &root, bundle, attachments);
    Some(value)
}
pub fn sticker(data: &[u8], target: &str) -> Option<Json> {
    let value = plist::archive(data).ok()?;
    if target.is_empty() {
        return None;
    }
    let number = |k| {
        value
            .get(k)
            .and_then(|v| {
                v.number()
                    .or_else(|| v.string().and_then(|s| s.parse::<f64>().ok()))
            })
            .filter(|n| n.is_finite())
    };
    let (x, y, scale, rotation) = (
        number("sxs")?,
        number("sys")?,
        number("ssa")?,
        number("sro")?,
    );
    if scale <= 0.0 {
        return None;
    }
    let mut guid = target;
    let mut part = number("sai")
        .filter(|n| n.fract() == 0.0)
        .and_then(|n| n.to_string().parse::<i64>().ok())
        .unwrap_or(0);
    if let Some(rest) = target.strip_prefix("p:") {
        if let Some((index, id)) = rest.split_once('/') {
            part = index.parse::<i64>().ok()?;
            guid = id;
        }
    } else if let Some(rest) = target.strip_prefix("bp:") {
        guid = rest;
    }
    if guid.is_empty() || part < 0 {
        return None;
    }
    Some(
        json!({"target_message_id":guid,"part_index":part,"x":x,"y":y,"scale":scale,"rotation":rotation}),
    )
}
pub fn chat_properties(data: Option<&[u8]>) -> Json {
    let props = data
        .and_then(|b| plist::archive(b).ok())
        .unwrap_or_default();
    let mut result = json!({"read_receipts":"default","muted_until":null,"pinned_index":null});
    if let Some(flag) = props
        .get("EnableReadReceiptForChat")
        .and_then(Value::number)
    {
        result["read_receipts"] = json!(if flag == 0.0 { "off" } else { "on" });
    }
    if let Some(photo) = text(&props, "groupPhotoGuid").filter(|s| !s.is_empty()) {
        result["group_photo_attachment_id"] = json!(photo);
    }
    result
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageReaction {
    #[serde(rename = "messageGUID")]
    pub message_guid: String,
    pub part_index: usize,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    pub removed: bool,
}
impl MessageReaction {
    pub fn decode(kind: i64, target: &str, emoji: Option<String>) -> Option<Self> {
        let removed = (3000..=3007).contains(&kind);
        let index = kind - if removed { 3000 } else { 2000 };
        let name = [
            "love",
            "like",
            "dislike",
            "laugh",
            "emphasize",
            "question",
            "emoji",
            "sticker",
        ]
        .get(usize::try_from(index).ok()?)?;
        let mut guid = target;
        let mut part = 0;
        if let Some(rest) = target.strip_prefix("p:") {
            let (index, id) = rest.split_once('/')?;
            part = index.parse().ok()?;
            guid = id;
        } else if let Some(rest) = target.strip_prefix("bp:") {
            guid = rest;
        }
        if guid.is_empty() {
            return None;
        }
        Some(Self {
            message_guid: guid.into(),
            part_index: part,
            kind: (*name).into(),
            emoji,
            removed,
        })
    }
}

fn project_balloon(
    value: &mut Json,
    root: &BTreeMap<String, Value>,
    bundle: &str,
    attachments: &[String],
) {
    let metadata = root
        .get("richLinkMetadata")
        .and_then(Value::dictionary)
        .unwrap_or(root);
    if balloon_kind(bundle) == "link" {
        if metadata
            .get("specialization")
            .and_then(Value::dictionary)
            .is_some_and(|m| {
                ["address", "addressComponents", "isPointOfInterest"]
                    .iter()
                    .any(|k| m.contains_key(*k))
            })
        {
            value["kind"] = json!("find_my");
        }
        let mut link = serde_json::Map::new();
        for (key, out) in [
            ("URL", "url"),
            ("title", "title"),
            ("summary", "summary"),
            ("siteName", "site_name"),
        ] {
            if let Some(s) = text(metadata, key) {
                link.insert(out.into(), json!(s));
            }
        }
        if !link.contains_key("url")
            && let Some(s) = text(metadata, "originalURL")
        {
            link.insert("url".into(), json!(s));
        }
        let video = metadata
            .get("video")
            .and_then(Value::dictionary)
            .or_else(|| metadata.get("videoMetadata").and_then(Value::dictionary));
        if let Some(s) = video
            .and_then(|m| text(m, "URL"))
            .or_else(|| text(metadata, "youTubeURL"))
        {
            link.insert("video_url".into(), json!(s));
        }
        for (key, out) in [
            ("image", "image_attachment_id"),
            ("icon", "icon_attachment_id"),
        ] {
            if let Some(Value::Integer(n)) = metadata
                .get(key)
                .and_then(Value::dictionary)
                .and_then(|m| m.get("richLinkImageAttachmentSubstituteIndex"))
                && let Some(id) = usize::try_from(*n).ok().and_then(|i| attachments.get(i))
            {
                link.insert(out.into(), json!(id));
            }
        }
        if link.contains_key("url") {
            value["link"] = Json::Object(link);
        }
    } else {
        let layout = root
            .get("layout")
            .and_then(Value::dictionary)
            .unwrap_or(root);
        let mut app = serde_json::Map::new();
        for (key, out) in [
            ("caption", "caption"),
            ("subcaption", "subcaption"),
            ("secondary-subcaption", "trailing_caption"),
            ("URL", "url"),
        ] {
            if let Some(s) = text(layout, key) {
                app.insert(out.into(), json!(s));
            }
        }
        if !app.contains_key("caption")
            && let Some(s) = text(layout, "ldtext")
        {
            app.insert("caption".into(), json!(s));
        }
        if let Some(id) = attachments.first() {
            app.insert("image_attachment_id".into(), json!(id));
        }
        if !app.is_empty() {
            value["app"] = Json::Object(app);
        }
    }
}
