#![allow(clippy::unwrap_used)]

use ab_native::{infocus, notes, rich, summary};
use std::{collections::BTreeMap, path::Path};

#[test]
fn swift_fixture_parity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../lab/fixtures");
    let tables = BTreeMap::from([(
        "fixture-table".into(),
        std::fs::read(root.join("notes/full-table.gz")).unwrap(),
    )]);
    for line in include_str!("../parity/decoders.jsonl").lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let decoder = v["decoder"].as_str().unwrap();
        let name = v["fixture"].as_str().unwrap();
        let family = match decoder {
            "notes" => "notes",
            "infocus" => "usage",
            _ => "messages",
        };
        let data = std::fs::read(root.join(family).join(name)).unwrap();
        let actual = match decoder {
            "notes" => notes::decode(&data, &tables).map(|v| serde_json::to_value(v).unwrap()),
            "summary" => Ok(serde_json::to_value(summary::decode(&data)).unwrap()),
            "infocus" => {
                let expected: Vec<infocus::InFocusEvent> =
                    serde_json::from_value(v["body"].clone()).unwrap();
                assert_eq!(infocus::decode(&data), expected, "{name}");
                continue;
            }
            "rich" => {
                let attachments = vec!["synthetic-image".into(), "synthetic-icon".into()];
                Ok(
                    serde_json::json!({"balloon":rich::balloon("URLBalloonProvider",Some(&data),&attachments),"app":rich::balloon("MSMessageExtensionBalloonPlugin:synthetic",Some(&data),&attachments),"sticker":rich::sticker(&data,"p:2/synthetic-parent"),"chat":rich::chat_properties(Some(&data))}),
                )
            }
            _ => panic!("unknown decoder"),
        };
        if v["error"] == true {
            assert!(actual.is_err(), "{name}");
        } else {
            assert_eq!(actual.unwrap(), v["body"], "{name}");
        }
    }
}

#[test]
fn notes_truncations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../lab/fixtures/notes");
    for name in ["plain", "formatting", "checklist", "table", "attachment"] {
        let data = std::fs::read(root.join(format!("{name}.gz"))).unwrap();
        for length in 0..data.len() {
            assert!(
                notes::decode(&data[..length], &BTreeMap::new()).is_err(),
                "{name} {length}"
            );
        }
    }
}
