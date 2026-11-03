#![allow(clippy::unwrap_used)]
use ab_native::native_json;

#[test]
fn swift_number_vectors() {
    for line in include_str!("../parity/native-json-numbers.tsv").lines() {
        let (bits, expected) = line.split_once('\t').unwrap();
        let n = f64::from_bits(u64::from_str_radix(bits, 16).unwrap());
        assert_eq!(native_json::number(n).unwrap(), expected, "{bits}");
        assert_eq!(
            native_json::record_data(&serde_json::json!(n)).unwrap(),
            expected.as_bytes(),
            "{bits}"
        );
    }
}
#[test]
fn non_finite_numbers_fail() {
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(native_json::number(n).is_err());
    }
}

#[test]
fn foundation_string_escaping() {
    let bytes = include_bytes!("../parity/native-json-strings.json");
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(native_json::foundation_data(&value).unwrap(), bytes);
    assert_eq!(
        native_json::record_data(&value).unwrap(),
        ab_wire::gojson::record_data(bytes, ab_wire::gojson::EscapeMode::Html).unwrap()
    );
}

#[test]
fn swift_double_descriptions() {
    let samples: serde_json::Value =
        serde_json::from_str(include_str!("../parity/native-json-descriptions.json")).unwrap();
    for sample in samples.as_array().unwrap() {
        let n = f64::from_bits(u64::from_str_radix(sample["bits"].as_str().unwrap(), 16).unwrap());
        assert_eq!(
            native_json::description(n).unwrap(),
            sample["description"].as_str().unwrap()
        );
    }
}
#[test]
fn fractional_notes_font_html() {
    let body = ab_native::notes::decode(
        include_bytes!("../parity/native-font.gz"),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("../parity/native-font.json")).unwrap();
    assert_eq!(serde_json::to_value(body).unwrap(), expected);
}
