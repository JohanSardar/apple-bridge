#![allow(clippy::unwrap_used)]
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[derive(Debug, serde::Deserialize)]
struct Vector {
    name: String,
    input: String,
    encoding: String,
    result: Option<Box<serde_json::value::RawValue>>,
    error: Option<bool>,
}
#[test]
fn swift_transcript_and_mp4_fixtures() {
    for line in include_str!("../parity/voice-memos/decoders.jsonl").lines() {
        let vector: Vector = serde_json::from_str(line).unwrap();
        let input = STANDARD.decode(vector.input).unwrap();
        let actual = if vector.encoding == "mp4" {
            ab_native::voice_memo::embedded_transcript(&input)
        } else {
            ab_native::voice_memo::transcript(&input)
        };
        if vector.error == Some(true) {
            assert!(actual.is_err(), "{}", vector.name);
            continue;
        }
        let actual = serde_json::to_value(actual.unwrap()).unwrap();
        let actual = ab_native::native_json::record_data(&actual).unwrap();
        let expected = ab_wire::gojson::record_data(
            vector.result.unwrap().get().as_bytes(),
            ab_wire::gojson::EscapeMode::Html,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(actual).unwrap(),
            String::from_utf8(expected).unwrap(),
            "{}",
            vector.name
        );
    }
}
