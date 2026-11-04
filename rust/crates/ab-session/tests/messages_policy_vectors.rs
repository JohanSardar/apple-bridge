#![allow(clippy::expect_used, clippy::unwrap_used)]
use ab_session::messages_policy::{ActionWindow, Origin, Refusal, check};
use serde_json::Value;
use std::collections::BTreeSet;

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../design/spec/messages-send-policy-vectors.json"
    );
    serde_json::from_slice(&std::fs::read(path).expect("vectors")).expect("json")
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item.as_str().expect("string").to_owned())
        .collect()
}

#[test]
fn the_send_policy_matches_every_shared_vector() {
    let vectors = vectors();
    let list = vectors["send"].as_array().expect("send");
    assert!(list.len() >= 20);
    for vector in list {
        let name = vector["name"].as_str().unwrap();
        let origin = match vector["origin"].as_str().unwrap() {
            "interactive" => Origin::Interactive,
            "owner_self" => Origin::OwnerSelf,
            _ => Origin::Gated,
        };
        let own: Option<BTreeSet<String>> =
            (!vector["own"].is_null()).then(|| strings(&vector["own"]).into_iter().collect());
        let recipients = strings(&vector["recipients"]);
        let files = usize::try_from(vector["files"].as_u64().unwrap()).unwrap();
        let result = check(
            origin,
            vector["text"].as_str().unwrap(),
            files,
            own.as_ref(),
            &recipients,
        );
        match vector["expect"].as_str().unwrap() {
            "allow" => {
                let owner = result.unwrap_or_else(|refusal| panic!("{name}: {refusal:?}"));
                if let Some(expected) = vector["owner_self"].as_bool() {
                    assert_eq!(owner, expected, "{name}");
                }
            }
            "text_tag_required" => assert_eq!(result, Err(Refusal::TextTagRequired), "{name}"),
            "invalid" => assert_eq!(result, Err(Refusal::Invalid), "{name}"),
            other => panic!("{name}: {other}"),
        }
    }
}

#[test]
fn the_action_windows_match_every_shared_vector() {
    let vectors = vectors();
    for vector in vectors["windows"].as_array().expect("windows") {
        let name = vector["name"].as_str().unwrap();
        let window = ActionWindow::new(
            vector["service"].as_str().unwrap(),
            vector["is_from_me"].as_bool().unwrap(),
            vector["is_message"].as_bool().unwrap(),
            vector["sent_at"].as_f64(),
            u32::try_from(vector["edits"].as_u64().unwrap()).unwrap(),
        );
        assert_eq!(
            window.is_some(),
            vector["exists"].as_bool().unwrap(),
            "{name}"
        );
        for check in vector["checks"].as_array().unwrap() {
            let allowed = window
                .unwrap()
                .allows(check["kind"] == "edit", check["at"].as_f64().unwrap());
            assert_eq!(
                allowed,
                check["allowed"].as_bool().unwrap(),
                "{name} {check}"
            );
        }
    }
}
