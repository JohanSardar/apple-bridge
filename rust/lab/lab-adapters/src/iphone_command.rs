use crate::iphone::{Scratch, swift_build};
use crate::iphone_push::{Kind, command_flow};
use crate::phone::Gateway;
use crate::webauthn::Owner;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use lab_runner::Evidence;
use serde_json::{Value, json};
use std::process::Command;

pub const SCENARIOS: [&str; 2] = ["APP-CMD-SIM", "APP-NOTIFY-SIM"];

pub fn run(id: &str, evidence: &mut Evidence) -> bool {
    match id {
        "APP-CMD-SIM" => command_flow(evidence, Kind::Cmd),
        "APP-NOTIFY-SIM" => command_flow(evidence, Kind::Notify),
        _ => return false,
    }
    true
}

pub(crate) fn notification_setup(
    evidence: &mut Evidence,
    gateway: &Gateway,
    token: &str,
    device: &str,
    command: &mut Value,
) {
    let dir = Scratch::new("notification");
    if let Ok(dir) = dir {
        let proof = dir.path().join("notification-completion");
        let built = swift_build(
            &proof,
            &["-swift-version", "6", "-parse-as-library"],
            &[
                "ios/Delivery/NotificationCompletion.swift",
                "lab/ios/NotificationProof.swift",
            ],
        );
        evidence.check("notification_completion_build", built, "");
        if built {
            let output = Command::new(&proof).output();
            evidence.check(
                "background_notification_completes_on_main_after_delivery",
                output
                    .as_ref()
                    .is_ok_and(|o| o.status.success() && o.stdout == b"PASS\n"),
                "",
            );
        }
    } else {
        evidence.check("notification_completion_build", false, "scratch directory");
    }
    command["op"] = json!("iphone.notify");
    command["args"] = json!({
        "device_id": device,
        "title": "[ab-test] Notification",
        "body": "Bridge Test",
        "buttons": [{"id": "approve", "title": "Approve"}, {"id": "deny", "title": "Deny"}],
        "thread": "ab-test",
        "time_sensitive": true,
    });
    let mut invalid = command.clone();
    invalid["command_id"] = json!("cmd_iphone_duplicate_button");
    invalid["args"] = json!({
        "device_id": device,
        "title": "test",
        "body": "test",
        "buttons": [{"id": "same", "title": "A"}, {"id": "same", "title": "B"}],
    });
    evidence.check(
        "duplicate_button_rejected",
        gateway.invoke("iphone.notify", &invalid, token).is_err(),
        "",
    );
}

pub(crate) fn notification_preserved(args: &Value) -> bool {
    args["title"] == "[ab-test] Notification"
        && args["body"] == "Bridge Test"
        && args["thread"] == "ab-test"
        && args["time_sensitive"] == true
        && args["buttons"].as_array().is_some_and(|b| b.len() == 2)
}

fn decode(value: &Value) -> Option<Vec<u8>> {
    STANDARD.decode(value.as_str()?).ok()
}

pub(crate) fn completion_evidence(evidence: &Value, device: &str, hash: &Value) -> Option<Value> {
    let payload = decode(&evidence["payload"])?;
    let signature = Signature::from_slice(&decode(&evidence["signature"])?).ok()?;
    let key: [u8; 32] = decode(&evidence["signing_key"])?.try_into().ok()?;
    let result: Value = serde_json::from_slice(&payload).ok()?;
    if evidence["device_id"] != device
        || result["command_id"] != "cmd_iphone_roundtrip"
        || &result["request_hash"] != hash
        || result["state"] != "observed"
    {
        return None;
    }
    let mut message = format!(
        "apple-bridge-iphone/1\0/v1/iphone/result\0{}\0",
        evidence["at"].as_i64()?
    )
    .into_bytes();
    message.extend_from_slice(&payload);
    VerifyingKey::from_bytes(&key)
        .ok()?
        .verify(&message, &signature)
        .ok()?;
    Some(result)
}

pub(crate) fn evidence_result(evidence: &Value) -> Option<Value> {
    serde_json::from_slice(&decode(&evidence["payload"])?).ok()
}

pub(crate) fn button_result(evidence: &Value, device: &str, hash: &Value) -> bool {
    completion_evidence(evidence, device, hash)
        .is_some_and(|result| result["data"] == json!({"action_id": "approve"}))
}

pub(crate) fn owner_approval(evidence: &mut Evidence, gateway: &Gateway, id: &str) {
    let owner = Owner::enroll(gateway);
    evidence.check(
        "owner_device",
        owner.is_ok(),
        owner.as_ref().err().map_or("", |e| match e {
            lab_world::Error::Invalid(message) => message.as_str(),
            _ => "owner enrollment",
        }),
    );
    let Ok(owner) = owner else { return };
    evidence.check("owner_authenticated", owner.cookie.is_some(), "");
    let decided = owner.decide(id, "approve");
    evidence.check(
        "iphone_owner_approval_without_helper",
        decided.as_ref().is_ok_and(|(status, record)| {
            *status == 200 && record.as_ref().is_some_and(|r| r["state"] == "admitted")
        }),
        "",
    );
}
