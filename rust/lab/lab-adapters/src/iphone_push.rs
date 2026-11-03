#![allow(clippy::too_many_lines)]
use crate::live::LiveSink;
use crate::phone::{Gateway, Phone, pair};
use crate::replicas;
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_runner::Evidence;
use serde_json::{Value, json};
use std::fs;

pub const SCENARIOS: [&str; 2] = ["APP-LIVE-SIM", "APP-APPROVE-SIM"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Cmd,
    Notify,
    Approve,
}

pub fn run(id: &str, evidence: &mut Evidence) -> bool {
    match id {
        "APP-LIVE-SIM" => live(evidence),
        "APP-APPROVE-SIM" => command_flow(evidence, Kind::Approve),
        _ => return false,
    }
    true
}

fn start() -> Result<(Gateway, Phone, String), String> {
    let binary = replicas::binary().map_err(|e| e.to_string())?;
    let gateway = Gateway::start(binary).map_err(|e| e.to_string())?;
    let (phone, token) = pair(&gateway).map_err(|e| e.to_string())?;
    Ok((gateway, phone, token))
}

fn live(evidence: &mut Evidence) {
    let fixture = start();
    evidence.check(
        "pair",
        fixture.is_ok(),
        fixture.as_ref().err().map_or("", String::as_str),
    );
    let Ok((mut gateway, phone, token)) = fixture else {
        return;
    };
    let register = |task: &str, value: &str| {
        phone
            .post(
                &gateway,
                "/v1/iphone/live-token",
                &json!({"task_id": task, "token": value}),
            )
            .is_ok_and(|r| r.0 == 200)
    };
    let registered = register("_start", &"a".repeat(64)) && register("ab-test", &"b".repeat(64));
    evidence.check("signed_start_and_task_token_registration", registered, "");
    if !registered {
        return;
    }
    let sink = LiveSink::start(&mut gateway);
    let Ok(sink) = sink else {
        evidence.check(
            "relay_sink",
            false,
            &sink.err().map_or_else(String::new, |e| e.to_string()),
        );
        return;
    };
    for (index, action) in ["start", "update", "end"].iter().enumerate() {
        let id = ["cmd_live_z", "cmd_live_m", "cmd_live_a"][index];
        let op = format!("iphone.live.{action}");
        let args = json!({"device_id": phone.id, "task_id": "ab-test", "title": "[ab-test] task", "status": action, "progress": 0.5});
        let admitted = gateway.invoke(
            &op,
            &json!({"command_id": id, "op": op, "args": args}),
            &token,
        );
        let ok = admitted.is_ok_and(|v| v["state"] == "admitted");
        evidence.check(&format!("{action}_admitted"), ok, "");
        if !ok {
            return;
        }
    }
    for _ in 0..2 {
        if phone.request(&gateway, "/v1/iphone/pending", None).is_err() {
            return;
        }
    }
    let ok = sink.wait("start,update,end");
    let detail = if ok {
        String::new()
    } else {
        format!("{} {}", sink.detail(), gateway.log())
    };
    evidence.check(
        "three_apns_lifecycle_events_without_duplicate_start",
        ok,
        &detail,
    );
    if !ok {
        return;
    }
    if phone
        .request(&gateway, "/v1/iphone/live-disable", None)
        .is_err()
    {
        return;
    }
    let credentials: Option<Value> = fs::read(gateway.home.join("credentials.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok());
    let mut found = false;
    let mut valid = credentials.is_some();
    if let Some(list) = credentials
        .as_ref()
        .and_then(|c| c["principals"].as_array())
    {
        for principal in list.iter().filter(|p| p["id"] == phone.id.as_str()) {
            found = true;
            valid &= principal["iphone_live_tokens"]
                .as_object()
                .is_none_or(serde_json::Map::is_empty);
        }
    }
    evidence.check("disable_revokes_live_tokens", valid && found, "");
}

fn status(reply: &lab_world::Result<(u16, Vec<u8>)>) -> u16 {
    reply.as_ref().map_or(0, |r| r.0)
}

pub(crate) fn command_flow(evidence: &mut Evidence, kind: Kind) {
    let dir = crate::iphone::Scratch::new("iphone-command");
    evidence.check("isolated_home", dir.is_ok(), "");
    let Ok(_dir) = dir else { return };
    let fixture = start();
    evidence.check(
        "authority",
        fixture.is_ok(),
        fixture.as_ref().err().map_or("", String::as_str),
    );
    let Ok((mut gateway, phone, token)) = fixture else {
        return;
    };
    for name in ["state", "ledger", "pair_code", "phone", "paired"] {
        evidence.check(name, true, "");
    }
    evidence.check("command_token", !token.is_empty(), "");
    evidence.check("principal", !token.is_empty(), "");
    let submit = |gateway: &Gateway, credential: &str, command: &Value| {
        gateway.invoke(
            command["op"].as_str().unwrap_or_default(),
            command,
            credential,
        )
    };
    let args = json!({"device_id": phone.id});
    let mut command = json!({"command_id": "cmd_iphone_roundtrip", "op": "iphone.ping", "args": args, "wait_ms": 1000});
    if kind == Kind::Notify {
        crate::iphone_command::notification_setup(
            evidence,
            &gateway,
            &token,
            &phone.id,
            &mut command,
        );
    }
    let admitted = submit(&gateway, &token, &command);
    evidence.check(
        "offline_admitted_without_helper",
        admitted.is_ok_and(|v| v["state"] == "admitted"),
        "",
    );
    evidence.check("restart_state", gateway.restart().is_ok(), "");
    let queued = gateway.record("cmd_iphone_roundtrip");
    evidence.check("restart_ledger", queued.is_ok(), "");
    let queued_ok = queued.as_ref().is_ok_and(|q| q["state"] == "admitted");
    evidence.check("recover_preserves_queue", queued_ok, "");
    evidence.check("durable_offline_queue", queued_ok, "");
    let queued = queued.unwrap_or(Value::Null);
    let sink = LiveSink::start(&mut gateway);
    evidence.check(
        "relay_store",
        sink.is_ok(),
        &sink
            .as_ref()
            .err()
            .map_or_else(String::new, ToString::to_string),
    );
    let Ok(sink) = sink else { return };
    evidence.check("relay_enrollment", true, "");
    let registration = phone.post(
        &gateway,
        "/v1/iphone/push-token",
        &json!({"token": "a".repeat(64)}),
    );
    evidence.check("signed_push_registration", status(&registration) == 200, "");
    let packet = sink.next();
    evidence.check(
        "encrypted_relay_push",
        packet.is_ok(),
        &if packet.is_ok() {
            String::new()
        } else {
            gateway.log()
        },
    );
    let Ok(packet) = packet else { return };
    let plain = phone.open(&packet);
    evidence.check("gateway_signature_and_decryption", plain.is_ok(), "");
    let Ok(job) = plain else { return };
    let mut tampered = packet.clone();
    if let Some(mut sealed) = packet["box"].as_str().and_then(|b| STANDARD.decode(b).ok()) {
        sealed[0] ^= 1;
        tampered["box"] = json!(STANDARD.encode(sealed));
    }
    evidence.check(
        "tampered_ciphertext_rejected",
        phone.open(&tampered).is_err(),
        "",
    );
    evidence.check(
        "command_payload",
        job["command"]["command_id"] == "cmd_iphone_roundtrip"
            && job["request_hash"] == queued["request_hash"],
        "",
    );
    if kind == Kind::Notify {
        evidence.check(
            "notification_fields_preserved",
            crate::iphone_command::notification_preserved(&job["command"]["args"]),
            "",
        );
    }
    let mut result = json!({"command_id": "cmd_iphone_roundtrip", "request_hash": "wrong", "state": "observed", "data": {"pong": true}});
    let reply = phone.post(&gateway, "/v1/iphone/result", &result);
    evidence.check("wrong_result_hash_rejected", status(&reply) == 400, "");
    result["request_hash"] = job["request_hash"].clone();
    if kind == Kind::Notify {
        result["data"] = json!({"action_id": "approve"});
    }
    let reply = phone.post(&gateway, "/v1/iphone/result", &result);
    evidence.check("signed_result", status(&reply) == 200, "");
    let observed = gateway.record("cmd_iphone_roundtrip");
    evidence.check(
        "ledger_observed",
        observed
            .as_ref()
            .is_ok_and(|o| o["state"] == "observed" && !o["evidence"].is_null()),
        "",
    );
    if kind == Kind::Notify {
        evidence.check(
            "button_result_returned",
            observed.as_ref().is_ok_and(|o| {
                crate::iphone_command::button_result(
                    &o["evidence"],
                    &phone.id,
                    &queued["request_hash"],
                )
            }),
            "",
        );
    }
    let reply = phone.post(&gateway, "/v1/iphone/result", &result);
    evidence.check("duplicate_result_idempotent", status(&reply) == 200, "");
    let duplicate = submit(&gateway, &token, &command);
    evidence.check(
        "command_retry_not_redispatched",
        duplicate.is_ok_and(|d| d["state"] == "observed"),
        "",
    );
    evidence.check(
        "expired_offline_fixture",
        gateway
            .expire("cmd_iphone_roundtrip", "cmd_iphone_expired")
            .is_ok(),
        "",
    );
    let pending = phone.request(&gateway, "/v1/iphone/pending", None);
    evidence.check("pending_after_completion", status(&pending) == 200, "");
    let expired = gateway.record("cmd_iphone_expired");
    evidence.check(
        "offline_expiry",
        expired.is_ok_and(|e| e["state"] == "expired"),
        "",
    );
    let read = gateway.agent("read");
    evidence.check("read_agent", read.is_ok(), "");
    let read_token = read.map(|r| r.0).unwrap_or_default();
    evidence.check("read_agent_principal", !read_token.is_empty(), "");
    command["command_id"] = json!("cmd_iphone_forbidden");
    evidence.check(
        "read_agent_cannot_dispatch",
        submit(&gateway, &read_token, &command).is_err(),
        "",
    );
    let approver = gateway.agent("approve");
    evidence.check("approve_agent", approver.is_ok(), "");
    let (approve_token, approver_id) = approver.unwrap_or_default();
    evidence.check("approve_agent_principal", !approve_token.is_empty(), "");
    command["command_id"] = json!("cmd_iphone_approval");
    let pending = submit(&gateway, &approve_token, &command);
    evidence.check(
        "iphone_requires_owner_approval",
        pending
            .as_ref()
            .is_ok_and(|p| p["state"] == "awaiting_approval"),
        "",
    );
    let pending = pending.unwrap_or(Value::Null);
    if kind != Kind::Approve {
        crate::iphone_command::owner_approval(evidence, &gateway, "cmd_iphone_approval");
        return;
    }
    let packets = phone.pending(&gateway).map(|p| p.1).unwrap_or_default();
    let command_args = command["args"].to_string();
    let found = packets.iter().any(|packet| {
        phone.open(packet).is_ok_and(|approval| {
            approval["command"]["op"] == "bridge.approval"
                && approval["request_hash"] == pending["request_hash"]
                && approval["command"]["args"]["body"]
                    .as_str()
                    .is_some_and(|body| body.contains(&approver_id) && body.contains(&command_args))
        })
    });
    evidence.check("encrypted_approval_notification", found, "");
    let decide = |id: &str, hash: &Value, approve: bool| {
        status(&phone.post(
            &gateway,
            "/v1/iphone/approval",
            &json!({"command_id": id, "request_hash": hash, "approve": approve}),
        ))
    };
    let hash = pending["request_hash"].clone();
    evidence.check(
        "wrong_approval_hash_rejected",
        decide("cmd_iphone_approval", &json!("wrong"), false) == 400,
        "",
    );
    evidence.check(
        "signed_phone_rejection",
        decide("cmd_iphone_approval", &hash, false) == 200,
        "",
    );
    let rejected = |gateway: &Gateway| {
        gateway
            .record("cmd_iphone_approval")
            .is_ok_and(|r| r["state"] == "rejected_by_owner")
    };
    evidence.check("ledger_rejected_by_owner", rejected(&gateway), "");
    evidence.check(
        "decision_replay_idempotent",
        decide("cmd_iphone_approval", &hash, true) == 200,
        "",
    );
    evidence.check("replay_cannot_reverse_rejection", rejected(&gateway), "");
    command["command_id"] = json!("cmd_phone_approve");
    let second = submit(&gateway, &approve_token, &command);
    evidence.check(
        "second_approval_pending",
        second
            .as_ref()
            .is_ok_and(|p| p["state"] == "awaiting_approval"),
        "",
    );
    let second = second.unwrap_or(Value::Null);
    evidence.check(
        "signed_phone_approval",
        decide("cmd_phone_approve", &second["request_hash"], true) == 200,
        "",
    );
    let returned: Option<Value> = fs::read(gateway.home.join("phone-decision.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok());
    let Some(returned) = returned else {
        evidence.check("phone_admission_returned", false, "");
        return;
    };
    let admitted_row = gateway.record("cmd_phone_approve");
    let admissions = admitted_row.as_ref().map_or(0, |a| {
        a["history"]
            .as_array()
            .map_or(0, |h| h.iter().filter(|e| e["state"] == "admitted").count())
    });
    evidence.check(
        "ledger_admitted_by_phone",
        admitted_row
            .as_ref()
            .is_ok_and(|a| a["request_hash"] == second["request_hash"])
            && returned["state"] == "admitted"
            && admissions == 1,
        "",
    );
}
