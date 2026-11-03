use crate::{
    Dataset, EPOCH, Fields, handle_key,
    media::{Asset, fixture},
    message_key, reference, ts,
};
use serde_json::{Value, json};

fn rich_attachment(d: &mut Dataset, id: &str, message: &str, asset: &Asset, extra: Option<&Value>) {
    let digest = d.blob(&asset.data);
    let mut fields = json!({
        "message_id": reference("messages.message", message),
        "filename": asset.name,
        "mime_type": asset.mime,
        "uti": asset.mime,
        "size": asset.data.len(),
        "blob": {"digest": digest, "state": "present"},
    });
    for (name, value) in extra
        .as_ref()
        .and_then(|v| v.as_object())
        .into_iter()
        .flatten()
    {
        fields[name] = value.clone();
    }
    d.put("messages.attachment", id, fields);
}

fn text_message(fields: &mut Fields, text: &str) {
    fields.insert("text".into(), json!(text));
    fields.insert(
        "parts".into(),
        json!([{"index": 0, "kind": "text", "text": text}]),
    );
}

fn balloon(d: &mut Dataset, i: usize, key: &str, fields: &mut Fields, media: &[Asset]) {
    let kind = [
        "link",
        "app",
        "digital_touch",
        "handwriting",
        "poll",
        "check_in",
        "apple_cash",
        "find_my",
        "chatbot",
        "unknown",
    ][i - 100];
    let attachment = format!("demo-balloon-image-{i}");
    rich_attachment(
        d,
        &attachment,
        key,
        &media[1],
        Some(&json!({"hidden": true})),
    );
    let image = reference("messages.attachment", &attachment);
    let mut balloon = json!({
        "bundle_id": format!("synthetic.{kind}"),
        "kind": kind,
        "decode_status": "decoded",
        "app_name": format!("Demo {kind}"),
    });
    if kind == "link" {
        balloon["bundle_id"] = json!("com.apple.messages.URLBalloonProvider");
        balloon["link"] = json!({
            "url": "https://example.com/garden",
            "title": "The imaginary garden",
            "summary": "A generated preview from the synthetic studio.",
            "site_name": "Example Garden",
            "image_attachment_id": image,
            "icon_attachment_id": image,
            "video_url": "https://example.com/garden/preview.mp4",
        });
    } else {
        balloon["app"] = json!({
            "caption": format!("Demo {kind}"),
            "subcaption": "Synthetic app preview",
            "trailing_caption": "Open in the demo",
            "image_attachment_id": image,
            "url": format!("https://example.com/apps/{kind}"),
        });
    }
    fields.insert("text".into(), json!(""));
    fields.insert("parts".into(), json!([]));
    fields.insert("balloon".into(), balloon);
}

#[allow(clippy::too_many_lines)]
pub fn rich_showcase(
    d: &mut Dataset,
    i: usize,
    key: &str,
    sent: i64,
    fields: &mut Fields,
    media: &[Asset],
) {
    if (100..110).contains(&i) {
        balloon(d, i, key, fields, media);
    }
    match i {
        39 => {
            fields.insert(
                "audio".into(),
                json!({"is_audio_message": true, "played_at": ts(sent + 60), "expirable": true, "kept": true, "transcription": "A synthetic hello from the demo studio."}),
            );
        }
        110 => {
            let target = message_key(0, 111);
            rich_attachment(
                d,
                "demo-placed-sticker",
                key,
                &media[4],
                Some(
                    &json!({"hidden": true, "is_sticker": true, "sticker": {"target_message_id": reference("messages.message", &target), "part_index": 0, "x": 0.65, "y": 0.3, "scale": 1.1, "rotation": -0.12}}),
                ),
            );
        }
        111 => text_message(fields, "A little color for the paper boats."),
        112 => {
            rich_attachment(
                d,
                "demo-genmoji",
                key,
                &media[4],
                Some(&json!({"genmoji": true, "description": "A synthetic glowing paper kite"})),
            );
            fields.insert("text".into(), json!(""));
            fields.insert(
                "parts".into(),
                json!([{"index": 0, "kind": "genmoji", "attachment_id": reference("messages.attachment", "demo-genmoji"), "description": "A synthetic glowing paper kite"}]),
            );
        }
        113 => {
            text_message(fields, "Bold paper boats");
            fields.insert(
                "formatting".into(),
                json!([
                    {"utf16_location": 0, "utf16_length": 4, "styles": ["bold", "italic"]},
                    {"utf16_location": 5, "utf16_length": 5, "styles": ["underline", "strikethrough"]},
                ]),
            );
        }
        114..=121 => {
            text_message(fields, "Moving paper boats");
            let effect = [
                "big", "small", "shake", "nod", "explode", "ripple", "bloom", "jitter",
            ][i - 114];
            fields.insert(
                "formatting".into(),
                json!([{"utf16_location": 0, "utf16_length": 6, "styles": [], "effect": effect}]),
            );
        }
        122 => {
            fields.insert("is_from_me".into(), json!(true));
            fields.insert(
                "schedule".into(),
                json!({"state": "native_3", "scheduled_for": ts(EPOCH + 3600)}),
            );
            fields.remove("delivered_at");
            fields.remove("read_at");
        }
        123 => {
            fields.insert("subject".into(), json!("Synthetic studio update"));
            fields.insert(
                "delivery".into(),
                json!({"quietly": true, "notified_anyway": false}),
            );
        }
        124 => {
            fields.insert(
                "delivery".into(),
                json!({"quietly": true, "notified_anyway": true}),
            );
            fields.insert("has_unseen_mention".into(), json!(true));
        }
        125 => {
            fields.insert("via_satellite".into(), json!(true));
            fields.insert("is_sos".into(), json!(true));
        }
        129 => {
            let still = Asset {
                name: "demo-live-still.png",
                mime: "image/png",
                data: fixture("live-still"),
            };
            rich_attachment(d, "demo-live-still", key, &still, None);
            let movie = Asset {
                name: "demo-live.mov",
                mime: "video/quicktime",
                data: fixture("video-live"),
            };
            rich_attachment(
                d,
                "demo-live-movie",
                key,
                &movie,
                Some(&json!({"hidden": true})),
            );
            d.live_photos
                .insert("demo-live-still".into(), "demo-live-movie".into());
            fields.insert("text".into(), json!(""));
            fields.insert(
                "parts".into(),
                json!([{"index": 0, "kind": "attachment", "attachment_id": reference("messages.attachment", "demo-live-still")}]),
            );
        }
        130 => {
            let video = Asset {
                name: "demo-ultrawide.mp4",
                mime: "video/mp4",
                data: fixture("video-panorama"),
            };
            rich_attachment(d, "demo-ultrawide", key, &video, None);
            fields.insert("is_from_me".into(), json!(true));
            fields.insert(
                "sender_handle_id".into(),
                reference("messages.handle", &handle_key(0)),
            );
            fields.insert("delivered_at".into(), json!(ts(sent + 1)));
            fields.insert("text".into(), json!(""));
            fields.insert(
                "parts".into(),
                json!([{"index": 0, "kind": "attachment", "attachment_id": reference("messages.attachment", "demo-ultrawide")}]),
            );
        }
        126..=128 => {
            fields.insert("kind".into(), json!("group_event"));
            fields.insert("text".into(), json!(""));
            fields.insert("parts".into(), json!([]));
            let actor = fields
                .get("sender_handle_id")
                .cloned()
                .unwrap_or(Value::Null);
            let kind = ["phone_changed", "photo_removed", "unknown"][i - 126];
            let item_type = [1, 3, 99][i - 126];
            let action_type = [0, 2, 0][i - 126];
            let mut event = json!({
                "kind": kind,
                "item_type": item_type,
                "action_type": action_type,
                "title": "",
                "actor_handle_id": actor,
            });
            if i == 126 {
                event["target_handle_id"] = actor.clone();
            }
            fields.insert("group_event".into(), event);
        }
        _ => {}
    }
}
