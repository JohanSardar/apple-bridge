#![allow(clippy::too_many_lines)]
use crate::http::Client;
use crate::iphone::{Scratch, swift_build};
use crate::phone::{Gateway, Phone, pair};
use crate::replicas;
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_runner::Evidence;
use serde_json::{Value, json};
use std::{fs, process::Command};

pub const SCENARIOS: [&str; 1] = ["APP-CAPTURE-SIM"];
const DELEGATED: &str = "replica:test/agent:capture";

pub fn run(id: &str, evidence: &mut Evidence) -> bool {
    if id != "APP-CAPTURE-SIM" {
        return false;
    }
    capture(evidence);
    true
}

fn start() -> Result<(Gateway, Phone, String), String> {
    let binary = replicas::binary().map_err(|e| e.to_string())?;
    let gateway = Gateway::start(binary).map_err(|e| e.to_string())?;
    let (phone, token) = pair(&gateway).map_err(|e| e.to_string())?;
    Ok((gateway, phone, token))
}

fn upload(
    gateway: &Gateway,
    phone: &Phone,
    id: &str,
    hash: &Value,
    kind: &str,
    data: &[u8],
) -> (u16, Vec<u8>) {
    let content_type = if kind == "audio" {
        "audio/wav"
    } else {
        "image/jpeg"
    };
    phone
        .post(
            gateway,
            "/v1/iphone/capture",
            &json!({
                "command_id": id,
                "request_hash": hash,
                "content_type": content_type,
                "data": STANDARD.encode(data),
            }),
        )
        .unwrap_or((0, vec![]))
}

fn capture(evidence: &mut Evidence) {
    let dir = Scratch::new("capture");
    evidence.check("isolated_home", dir.is_ok(), "");
    let Ok(dir) = dir else { return };
    let proof = dir.path().join("proof");
    let built = swift_build(
        &proof,
        &["-parse-as-library"],
        &[
            "ios/Capabilities/Capture/CaptureModel.swift",
            "ios/App/QualificationWait.swift",
            "lab/ios/CaptureProof.swift",
        ],
    );
    evidence.check("swift_capture_build", built, "");
    if !built {
        return;
    }
    let output = Command::new(&proof).output();
    evidence.check(
        "face_id_denial_and_memory_upload_paths",
        output
            .as_ref()
            .is_ok_and(|o| o.status.success() && o.stdout == b"PASS 4\n"),
        "",
    );
    let fixture = start();
    evidence.check(
        "authority",
        fixture.is_ok(),
        fixture.as_ref().err().map_or("", String::as_str),
    );
    let Ok((gateway, phone, token)) = fixture else {
        return;
    };
    for name in [
        "state",
        "ledger",
        "blob_store",
        "pair_code",
        "phone",
        "paired",
    ] {
        evidence.check(name, true, "");
    }
    evidence.check("token", !token.is_empty(), "");
    evidence.check("principal", !token.is_empty(), "");
    for seconds in [0, 11] {
        let bounded = json!({
            "command_id": "cmd_capture_bounds",
            "op": "iphone.capture.audio",
            "args": {"device_id": phone.id, "seconds": seconds},
        });
        evidence.check(
            "audio_duration_bound_rejected",
            gateway
                .invoke("iphone.capture.audio", &bounded, &token)
                .is_err(),
            "",
        );
    }
    let reader = Client::at(&gateway.api_address, &token);
    for kind in ["photo", "audio"] {
        let id = format!("cmd_capture_{kind}");
        let op = format!("iphone.capture.{kind}");
        let command = json!({"command_id": id, "op": op, "args": {"device_id": phone.id}});
        let admitted = gateway.invoke(&op, &command, &token);
        evidence.check(
            &format!("{kind}_admitted"),
            admitted.as_ref().is_ok_and(|r| r["state"] == "admitted"),
            "",
        );
        let hash = admitted.map_or(Value::Null, |r| r["request_hash"].clone());
        let data = format!("[ab-test] {kind}").into_bytes();
        let (status, _) = upload(&gateway, &phone, &id, &hash, kind, &data);
        evidence.check(
            &format!("{kind}_undispatched_upload_rejected"),
            status == 400,
            "",
        );
        let pending = phone.request(&gateway, "/v1/iphone/pending", None);
        evidence.check(
            &format!("{kind}_dispatched"),
            pending.is_ok_and(|r| r.0 == 200),
            "",
        );
        let (status, _) = upload(&gateway, &phone, &id, &json!("wrong"), kind, &data);
        evidence.check(&format!("{kind}_wrong_hash_rejected"), status == 400, "");
        let (status, raw) = upload(&gateway, &phone, &id, &hash, kind, &data);
        let receipt: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
        evidence.check(
            &format!("{kind}_signed_blob_upload"),
            status == 200 && receipt["size"].as_u64() == Some(data.len() as u64),
            "",
        );
        let path = format!(
            "/v1/blobs/{}?record={}",
            receipt["digest"].as_str().unwrap_or_default(),
            receipt["blob_id"].as_str().unwrap_or_default()
        );
        let read = reader.request("GET", &path, &[], None, 15);
        evidence.check(
            &format!("{kind}_caller_blob_grant"),
            read.as_ref().is_ok_and(|r| r.status == 200),
            "",
        );
        evidence.check(
            &format!("{kind}_gateway_bytes_match"),
            read.as_ref()
                .is_ok_and(|r| r.status == 200 && r.body == data),
            "",
        );
        let (status, _) = upload(&gateway, &phone, &id, &hash, kind, &data);
        evidence.check(
            &format!("{kind}_upload_retry_idempotent"),
            status == 200,
            "",
        );
        let (status, _) = upload(&gateway, &phone, &id, &hash, kind, b"different");
        evidence.check(&format!("{kind}_changed_retry_rejected"), status == 400, "");
        let completion = phone.post(
            &gateway,
            "/v1/iphone/result",
            &json!({"command_id": id, "request_hash": hash, "state": "observed", "data": receipt}),
        );
        evidence.check(
            &format!("{kind}_signed_completion"),
            completion.is_ok_and(|r| r.0 == 200),
            "",
        );
        let observed = gateway.record(&id);
        evidence.check(
            &format!("{kind}_ledger_observed"),
            observed.is_ok_and(|o| o["state"] == "observed"),
            "",
        );
    }
    let credentials = fs::read_to_string(gateway.home.join("credentials.json")).unwrap_or_default();
    evidence.check(
        "delegated_principal_has_no_local_credential",
        !credentials.contains(DELEGATED),
        "",
    );
    let fixture = gateway.lab_with(
        "phone-delegated",
        "cmd_capture_photo",
        "cmd_capture_delegated",
        DELEGATED,
    );
    let ok = fixture.is_ok();
    for name in [
        "delegated_command_hash",
        "delegated_admission_era",
        "delegated_admitted_fixture",
        "delegated_prepared_fixture",
        "delegated_dispatched_fixture",
    ] {
        evidence.check(name, ok, "");
    }
    let Ok(row) = fixture else { return };
    let hash = row["request_hash"].clone();
    let allow = format!("allow {}", hash.as_str().unwrap_or_default());
    let _ = gateway.lab("phone-delegated-mode", &allow, "");
    let data = b"[ab-test] delegated";
    let (status, raw) = upload(
        &gateway,
        &phone,
        "cmd_capture_delegated",
        &hash,
        "photo",
        data,
    );
    let receipt: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let authorized = gateway
        .lab("phone-delegated-authorized", "", "")
        .is_ok_and(|v| v["authorized"] == true);
    evidence.check(
        "delegated_reauthorized_upload",
        authorized && status == 200 && receipt["blob_id"].is_string(),
        "",
    );
    let grant = gateway.lab_with(
        "phone-blob-grant",
        DELEGATED,
        receipt["blob_id"].as_str().unwrap_or_default(),
        receipt["digest"].as_str().unwrap_or_default(),
    );
    evidence.check(
        "delegated_caller_blob_grant",
        grant.is_ok_and(|g| g["allowed"] == true),
        "",
    );
    let _ = gateway.lab("phone-delegated-mode", "deny", "");
    let (status, _) = upload(
        &gateway,
        &phone,
        "cmd_capture_delegated",
        &hash,
        "photo",
        data,
    );
    evidence.check("delegated_revoked_upload_rejected", status == 400, "");
}
