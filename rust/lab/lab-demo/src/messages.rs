use crate::{
    Dataset, EFFECTS, EPOCH, Fields, add_date, chat_key, civil, handle_key, media::Asset,
    message_key, reference, rng::Pcg, ts,
};
use serde_json::{Value, json};

const PHRASES: [&str; 8] = [
    "The imaginary garden looks lovely today.",
    "Let's meet at the synthetic observatory.",
    "I finished the paper lantern prototype.",
    "Sending a little color from the demo studio.",
    "That sounds wonderful — see you soon!",
    "Could you bring the blue sketchbook?",
    "A quiet moment by our pretend lake.",
    "The example project is ready to explore.",
];

const ITEM_TYPES: [i64; 5] = [2, 1, 1, 3, 3];
const ACTION_TYPES: [i64; 5] = [0, 0, 1, 0, 1];

fn text_parts(text: &Value) -> Value {
    json!([{"index": 0, "kind": "text", "text": text}])
}

#[allow(clippy::cast_possible_wrap)]
pub fn fields(c: usize, i: usize, count: usize, rng: &mut Pcg) -> (Fields, i64) {
    let minutes = 730 * 24 * 60_i64;
    let mut sent = EPOCH - minutes * 60 + (i as i64) * (minutes / count as i64) * 60;
    if c == 0 {
        let dates = [
            add_date(EPOCH, -2, 0, 0),
            civil(2024, 12, 31, 23, 59),
            civil(2025, 1, 1, 0, 1),
            civil(2025, 12, 31, 23, 59),
            civil(2026, 1, 1, 0, 1),
            add_date(EPOCH, 0, 0, -15),
            add_date(EPOCH, 0, 0, -3),
            add_date(EPOCH, 0, 0, -1),
            EPOCH - 6 * 3600,
            EPOCH - 3600,
        ];
        sent = dates[(i / 20).min(dates.len() - 1)] + (i % 20) as i64 * 60;
    }
    let from_me = i.is_multiple_of(3);
    let text = PHRASES[usize::try_from(rng.int_n(PHRASES.len() as u64)).unwrap_or_default()];
    let mut map = Fields::new();
    let values = [
        ("chat_id", reference("messages.chat", &chat_key(c))),
        ("is_from_me", json!(from_me)),
        ("sent_at", json!(ts(sent))),
        ("text", json!(text)),
        ("parts", text_parts(&json!(text))),
        ("kind", json!("message")),
        ("decode_status", json!("decoded")),
        ("error_code", json!(0)),
        ("edit_history", json!([])),
        ("retracted_parts", json!([])),
    ];
    for (name, value) in values {
        map.insert(name.into(), value);
    }
    let mut sender = c;
    if c == 0 || (25..39).contains(&c) {
        sender = (c + i % (3 + c % 10)) % 60;
    }
    if from_me {
        sender = 0;
    }
    if !from_me && (c == 35 || c == 36) {
        sender = 58 + c - 35;
    }
    map.insert(
        "sender_handle_id".into(),
        reference("messages.handle", &handle_key(sender)),
    );
    if c == 34 {
        map.insert("deleted_at".into(), json!(ts(EPOCH - 24 * 3600)));
        map.insert(
            "recoverable_until".into(),
            json!(ts(EPOCH + 29 * 24 * 3600)),
        );
    }
    if c == 36 {
        map.insert("is_spam".into(), json!(true));
    }
    if from_me {
        map.insert("delivered_at".into(), json!(ts(sent + 1)));
        if i.is_multiple_of(2) {
            map.insert("read_at".into(), json!(ts(sent + 120)));
        }
    } else if !c.is_multiple_of(4) || i + 15 < count {
        map.insert("read_at".into(), json!(ts(sent + 120)));
    }
    (map, sent)
}

fn set_sender(fields: &mut Fields, from_me: bool, sender: usize) {
    fields.insert("is_from_me".into(), json!(from_me));
    fields.insert(
        "sender_handle_id".into(),
        reference("messages.handle", &handle_key(sender)),
    );
}

#[allow(clippy::too_many_lines, clippy::cast_possible_wrap)]
pub fn showcase(
    d: &mut Dataset,
    i: usize,
    key: &str,
    sent: i64,
    fields: &mut Fields,
    media: &[Asset],
) {
    match i {
        0 => {
            fields.insert(
                "text".into(),
                json!("A long synthetic story about moonlit gardens and paper boats. ".repeat(80)),
            );
        }
        1 => {
            fields.insert("text".into(), json!("🌿"));
        }
        2 => {
            fields.insert("text".into(), json!("🌿✨"));
        }
        3 => {
            fields.insert("text".into(), json!("🌿✨🪁"));
        }
        4 => {
            fields.insert(
                "text".into(),
                json!(
                    "Sketches at https://example.com/demo?view=garden and https://example.com/notes"
                ),
            );
        }
        5 => {
            let text = json!("Demo Aster00, your paper boats are ready.");
            fields.insert("text".into(), text.clone());
            fields.insert(
                "parts".into(),
                json!([{"index": 0, "kind": "text", "text": text, "mention": "demo.person00@example.com", "utf16_location": 0, "utf16_length": 12}]),
            );
        }
        7..=10 => {
            fields.insert(
                "reply_to_id".into(),
                reference("messages.message", &message_key(0, i - 1)),
            );
            fields.insert(
                "thread_originator_id".into(),
                reference("messages.message", &message_key(0, 6)),
            );
        }
        11 => {
            fields.insert("text".into(), json!("The lantern walk starts at sunset."));
            fields.insert("edited_at".into(), json!(ts(sent + 180)));
            fields.insert(
                "edit_history".into(),
                json!([
                    {"text": "The lantern walk starts at noon.", "timestamp": ts(sent)},
                    {"text": "The lantern walk starts at dusk.", "timestamp": ts(sent + 60)},
                ]),
            );
        }
        12 => {
            fields.insert("text".into(), json!(""));
            fields.insert("parts".into(), json!([]));
            fields.insert("retracted_parts".into(), json!([0]));
            fields.insert("retracted_at".into(), json!(ts(sent + 60)));
        }
        13 => {
            set_sender(fields, true, 0);
            fields.insert("error_code".into(), json!(1001));
            fields.remove("delivered_at");
            fields.remove("read_at");
        }
        _ => {}
    }
    if (58..=65).contains(&i) {
        let from_me = i >= 62;
        set_sender(fields, from_me, usize::from(!from_me));
        let at = civil(2025, 1, 1, 1, 0) + (i as i64 - 58) * 10;
        fields.insert("sent_at".into(), json!(ts(at)));
        fields.insert("read_at".into(), json!(ts(at + 60)));
        fields.remove("delivered_at");
        if from_me {
            fields.insert("delivered_at".into(), json!(ts(at + 1)));
        }
    }
    if i >= 14 && i < 14 + EFFECTS.len() {
        fields.insert("effect".into(), json!(EFFECTS[i - 14]));
    }
    if (26..=30).contains(&i) {
        let kind = [
            "rename",
            "member_added",
            "member_removed",
            "member_left",
            "photo_changed",
        ][i - 26];
        fields.insert("kind".into(), json!("group_event"));
        fields.insert(
            "group_event".into(),
            json!({
                "kind": kind,
                "title": "Demo Lantern Club",
                "item_type": ITEM_TYPES[i - 26],
                "action_type": ACTION_TYPES[i - 26],
                "target_handle_id": reference("messages.handle", &handle_key(i)),
                "actor_handle_id": fields.get("sender_handle_id").cloned().unwrap_or(Value::Null),
            }),
        );
    }
    if (i >= 32 && i < 32 + media.len()) || (42..=45).contains(&i) {
        let mut which = i - 32;
        if i >= 42 {
            which = (i - 42) % 3;
            set_sender(fields, false, 1);
            let stacked = civil(2025, 1, 1, 0, 7) + 10 + (i as i64 - 42) * 10;
            fields.insert("sent_at".into(), json!(ts(stacked)));
            fields.insert("read_at".into(), json!(ts(stacked + 60)));
            fields.remove("delivered_at");
        }
        let asset = &media[which];
        let attachment = format!("demo-attachment-{i:02}");
        let digest = d.blob(&asset.data);
        d.put(
            "messages.attachment",
            &attachment,
            json!({
                "message_id": reference("messages.message", key),
                "filename": asset.name,
                "mime_type": asset.mime,
                "uti": asset.mime,
                "size": asset.data.len(),
                "blob": {"digest": digest, "state": "present"},
            }),
        );
        fields.insert(
            "parts".into(),
            json!([{"index": 0, "kind": "attachment", "attachment_id": reference("messages.attachment", &attachment)}]),
        );
        fields.insert("text".into(), json!(""));
        if i == 32 {
            fields.insert(
                "parts".into(),
                json!([
                    {"index": 0, "kind": "text", "text": "A synthetic portrait from our studio."},
                    {"index": 1, "kind": "attachment", "attachment_id": reference("messages.attachment", &attachment)},
                ]),
            );
        }
    } else if i != 5 && i != 12 {
        let text = fields.get("text").cloned().unwrap_or(Value::Null);
        fields.insert("parts".into(), text_parts(&text));
    }
}

pub fn preview(
    d: &mut Dataset,
    c: usize,
    key: &str,
    sent: i64,
    fields: &mut Fields,
    media: &[Asset],
) {
    match c {
        2 => {
            let asset = &media[2];
            let digest = d.blob(&asset.data);
            d.put(
                "messages.attachment",
                "demo-preview-image",
                json!({
                    "message_id": reference("messages.message", key),
                    "filename": asset.name,
                    "mime_type": asset.mime,
                    "size": asset.data.len(),
                    "blob": {"digest": digest, "state": "present"},
                }),
            );
            fields.insert("text".into(), json!(""));
            fields.insert(
                "parts".into(),
                json!([{"index": 0, "kind": "attachment", "attachment_id": reference("messages.attachment", "demo-preview-image")}]),
            );
        }
        3 => {
            fields.insert("edited_at".into(), json!(ts(sent + 60)));
            fields.insert(
                "edit_history".into(),
                json!([{"text": "An earlier synthetic draft.", "timestamp": ts(sent)}]),
            );
        }
        4 => {
            fields.insert("retracted_at".into(), json!(ts(sent + 60)));
            fields.insert("retracted_parts".into(), json!([0]));
            fields.insert("text".into(), json!(""));
            fields.insert("parts".into(), json!([]));
        }
        26 => {
            fields.insert("kind".into(), json!("group_event"));
            fields.insert(
                "group_event".into(),
                json!({"kind": "rename", "title": "Demo Lantern Club", "item_type": 2, "action_type": 0}),
            );
        }
        _ => {}
    }
}
