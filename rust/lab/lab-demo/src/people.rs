use crate::{
    Dataset, EPOCH, chat_key, contact_key, handle_key, media::picture, reference, rng::Pcg, ts,
};
use serde_json::{Value, json};

const NAMES: [&str; 12] = [
    "Aster", "Beryl", "Cobalt", "Dune", "Ember", "Fern", "Garnet", "Halo", "Indigo", "Juniper",
    "Kite", "Lumen",
];
const SERVICES: [&str; 3] = ["iMessage", "SMS", "RCS"];

pub fn people(d: &mut Dataset, rng: &mut Pcg) {
    for i in 0..60_usize {
        let name = format!("{}{i:02}", NAMES[i % NAMES.len()]);
        let mut data = json!({
            "given_name": format!("Demo {name}"),
            "family_name": "Example",
            "organization": "Synthetic Studio",
            "phones": [{"label": "mobile", "value": format!("+1 555 01{i:02}")}],
            "emails": [{"label": "home", "value": format!("demo.person{i:02}@example.com")}],
            "container_id": "demo-synthetic",
            "writable": true,
        });
        if i % 2 == 0 {
            let seed = rng.uint64();
            let digest = d.blob(&picture(seed, 96, 96, false));
            data["image"] = json!({"digest": digest, "state": "present"});
        }
        d.put("contacts.contact", &contact_key(i), data);
    }
    for i in 0..60_usize {
        let address = if i >= 58 {
            format!("demo.unknown{i:02}@example.com")
        } else {
            format!("+1 555 01{i:02}")
        };
        d.put(
            "messages.handle",
            &handle_key(i),
            json!({"address": address, "service": SERVICES[i % 3]}),
        );
    }
    for c in 0..40_usize {
        let members = if c == 0 || (25..39).contains(&c) {
            3 + (c % 10)
        } else {
            1
        };
        let participants: Vec<Value> = (0..members)
            .map(|j| reference("messages.handle", &handle_key((c + j) % 60)))
            .collect();
        let style = if members > 1 { "group" } else { "direct" };
        let mut name = String::new();
        if members > 1 && c % 2 == 0 {
            name = format!("Demo {} Club", NAMES[c % NAMES.len()]);
        }
        match c {
            0 => name = "Demo Studio".into(),
            1 => name = "Demo Busy Workshop".into(),
            39 => name = "Demo Performance Archive".into(),
            _ => {}
        }
        let receipts = ["default", "on", "off"][c % 3];
        let mut data = json!({
            "display_name": name,
            "service": SERVICES[c % 3],
            "style": style,
            "participants": participants,
            "is_archived": c == 37 || c == 38,
            "capabilities": [],
            "last_read_at": ts(EPOCH - 48 * 3600),
            "filter": "known",
            "read_receipts": receipts,
            "muted_until": null,
            "pinned_index": null,
        });
        if c == 0 {
            data["group_photo_attachment_id"] =
                reference("messages.attachment", "demo-attachment-32");
            data["pinned_index"] = json!(0);
        }
        if c == 1 {
            data["muted_until"] = json!(ts(EPOCH + 24 * 3600));
        }
        if c == 35 || c == 36 {
            data["filter"] = json!("unknown");
            data["is_spam"] = json!(c == 36);
            data["participants"] = json!([reference("messages.handle", &handle_key(58 + c - 35))]);
        }
        if c == 34 {
            data["deleted_at"] = json!(ts(EPOCH - 24 * 3600));
            data["recoverable_until"] = json!(ts(EPOCH + 29 * 24 * 3600));
        }
        d.put("messages.chat", &chat_key(c), data);
    }
}
