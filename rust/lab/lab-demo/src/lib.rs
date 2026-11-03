pub mod media;
mod messages;
mod other;
mod people;
mod rich;
pub mod rng;

use base64::Engine;
use jiff::{Span, Timestamp, tz::TimeZone};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub use media::Asset;

pub const EPOCH: i64 = 1_792_512_000;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub seed: i64,
    pub huge: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub records: Vec<Value>,
    pub blobs: BTreeMap<String, Vec<u8>>,
    pub live_photos: BTreeMap<String, String>,
    pub seed: i64,
    pub counts: BTreeMap<String, usize>,
}

pub const EFFECTS: [&str; 12] = [
    "com.apple.MobileSMS.expressivesend.impact",
    "com.apple.MobileSMS.expressivesend.loud",
    "com.apple.MobileSMS.expressivesend.gentle",
    "com.apple.MobileSMS.expressivesend.invisibleink",
    "com.apple.messages.effect.CKEchoEffect",
    "com.apple.messages.effect.CKSpotlightEffect",
    "com.apple.messages.effect.CKHappyBirthdayEffect",
    "com.apple.messages.effect.CKConfettiEffect",
    "com.apple.messages.effect.CKHeartEffect",
    "com.apple.messages.effect.CKLasersEffect",
    "com.apple.messages.effect.CKFireworksEffect",
    "com.apple.messages.effect.CKSparklesEffect",
];

#[must_use]
pub fn ts(second: i64) -> String {
    Timestamp::from_second(second)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

#[must_use]
pub fn date_only(second: i64) -> String {
    ts(second)[..10].to_owned()
}

#[must_use]
pub fn add_date(second: i64, years: i32, months: i32, days: i32) -> i64 {
    Timestamp::from_second(second)
        .ok()
        .and_then(|t| {
            t.to_zoned(TimeZone::UTC)
                .checked_add(Span::new().years(years).months(months).days(days))
                .ok()
        })
        .map_or(second, |z| z.timestamp().as_second())
}

#[must_use]
pub fn civil(year: i16, month: i8, day: i8, hour: i8, minute: i8) -> i64 {
    jiff::civil::date(year, month, day)
        .at(hour, minute, 0, 0)
        .to_zoned(TimeZone::UTC)
        .map_or(0, |z| z.timestamp().as_second())
}

#[must_use]
pub fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref": {"collection": collection, "native_key": key}})
}

#[must_use]
pub fn contact_key(i: usize) -> String {
    format!("demo-contact-{i:02}")
}

#[must_use]
pub fn handle_key(i: usize) -> String {
    if i == 0 {
        "message-self".into()
    } else {
        format!("demo-handle-{i:02}")
    }
}

#[must_use]
pub fn chat_key(i: usize) -> String {
    if i == 0 {
        "message-chat".into()
    } else {
        format!("demo-chat-{i:02}")
    }
}

#[must_use]
pub fn message_key(chat: usize, index: usize) -> String {
    format!("demo-message-{chat:02}-{index:06}")
}

pub type Fields = Map<String, Value>;

impl Dataset {
    pub fn put(&mut self, collection: &str, key: &str, data: Value) {
        let mut record = json!({
            "kind": "upsert",
            "collection": collection,
            "native_key": key,
            "scoped_alias": format!("{collection}/{key}"),
            "data": null,
            "observed_at": ts(EPOCH),
        });
        record["data"] = data;
        self.records.push(record);
        *self.counts.entry(collection.to_owned()).or_default() += 1;
    }

    pub fn blob(&mut self, data: &[u8]) -> String {
        let digest = hex::encode(Sha256::digest(data));
        self.blobs.insert(digest.clone(), data.to_vec());
        digest
    }

    #[must_use]
    pub fn control_json(&self) -> Value {
        let engine = base64::engine::general_purpose::STANDARD;
        let blobs: Map<String, Value> = self
            .blobs
            .iter()
            .map(|(digest, data)| (digest.clone(), json!(engine.encode(data))))
            .collect();
        let mut out = json!({"records": self.records, "blobs": blobs});
        if !self.live_photos.is_empty() {
            out["live_photos"] = json!(self.live_photos);
        }
        out
    }

    #[must_use]
    pub fn digest(&self) -> String {
        let raw = serde_json::to_vec(&self.control_json()).unwrap_or_default();
        hex::encode(Sha256::digest(raw))
    }
}

#[must_use]
pub fn generate(options: Options) -> Dataset {
    let mut dataset = Dataset {
        seed: options.seed,
        ..Dataset::default()
    };
    let mut rng = rng::Pcg::new(
        u64::from_ne_bytes(options.seed.to_ne_bytes()),
        0x9e37_79b9_7f4a_7c15,
    );
    people::people(&mut dataset, &mut rng);
    let media = media::assets(&mut rng);
    for c in 0..40_usize {
        let count = match c {
            0 => 200,
            1 => 1000,
            39 if options.huge => 100_000,
            _ => 100,
        };
        for i in 0..count {
            let key = message_key(c, i);
            let (mut fields, sent) = messages::fields(c, i, count, &mut rng);
            if c == 0 {
                messages::showcase(&mut dataset, i, &key, sent, &mut fields, &media);
                rich::rich_showcase(&mut dataset, i, &key, sent, &mut fields, &media);
            }
            if i == count - 1 {
                messages::preview(&mut dataset, c, &key, sent, &mut fields, &media);
            }
            dataset.put("messages.message", &key, Value::Object(fields));
        }
    }
    other::reactions(&mut dataset);
    other::reminders(&mut dataset);
    other::calendars(&mut dataset);
    dataset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcg_matches_go_vectors() {
        let mut rng = rng::Pcg::new(1, 0x9e37_79b9_7f4a_7c15);
        assert_eq!(rng.uint64(), 17_168_285_775_449_168_363);
        assert_eq!(rng.uint64(), 13_451_632_132_566_249_027);
        assert_eq!(rng.int_n(8), 5);
        assert_eq!(rng.int_n(8), 1);
        assert_eq!(rng.int_n(1000), 703);
        assert!((rng.float64() - 0.541_702_354_789_996_2).abs() < f64::EPSILON);
        let mut other = rng::Pcg::new(7, 7 ^ 0x85eb_ca6b);
        assert!((other.float64() - 0.392_417_190_904_342_8).abs() < f64::EPSILON);
        assert_eq!(other.int_n(3), 2);
    }

    #[test]
    fn generation_is_deterministic_with_expected_shape() {
        let a = generate(Options {
            seed: 17,
            huge: false,
        });
        let b = generate(Options {
            seed: 17,
            huge: false,
        });
        assert_eq!(a.digest(), b.digest());
        assert_ne!(
            a.digest(),
            generate(Options {
                seed: 18,
                huge: false
            })
            .digest()
        );
        assert_eq!(a.records.len(), 5415);
        assert_eq!(a.counts["reminders.reminder"], 10);
        assert_eq!(a.counts["reminders.list"], 6);
        assert_eq!(a.live_photos.len(), 1);
    }

    #[test]
    fn animation_decodes_as_two_frames() {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let bytes = media::animation();
        let opened = options.read_info(bytes.as_slice());
        assert!(opened.is_ok());
        let Ok(mut reader) = opened else { return };
        let mut frames = 0;
        while let Ok(Some(frame)) = reader.read_next_frame() {
            assert_eq!(frame.buffer.len(), 96 * 96);
            assert_eq!(frame.buffer[0], u8::from(frames == 1));
            assert_eq!(frame.buffer[24], u8::from(frames == 0));
            frames += 1;
        }
        assert_eq!(frames, 2);
    }
}
