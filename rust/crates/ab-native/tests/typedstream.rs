#![allow(clippy::unwrap_used)]

use ab_native::typedstream;
use std::path::Path;

#[test]
fn swift_fixture_parity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../lab/fixtures/messages");
    for line in include_str!("../parity/typedstream.jsonl").lines() {
        let vector: serde_json::Value = serde_json::from_str(line).unwrap();
        let name = vector["fixture"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let body = typedstream::decode(&bytes);
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            vector["body"],
            "{name}"
        );
    }
}

#[test]
fn every_synthetic_truncation_is_undecoded() {
    let data = include_bytes!("../../../../lab/fixtures/messages/synthetic.typedstream");
    for n in 0..data.len() {
        let body = typedstream::decode(&data[..n]);
        assert_eq!(body.decode_status, "undecoded", "length {n}");
        assert!(body.text.is_none());
        assert_eq!(
            body.raw_body_digest,
            Some(ab_native::body_digest(&data[..n]))
        );
    }
}
