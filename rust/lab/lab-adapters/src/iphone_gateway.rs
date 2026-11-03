#![allow(clippy::too_many_lines, clippy::float_cmp)]
use crate::iphone::{Scratch, project, swift_build};
use crate::phone::{Gateway, Phone, pair};
use crate::replicas;
use lab_runner::Evidence;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, process::Command};

pub const SCENARIOS: [&str; 7] = [
    "APP-PAIR-SIM",
    "APP-DEVICE-SIM",
    "APP-MOTION-SIM",
    "APP-HEALTH-WRITE-SIM",
    "APP-SCREENTIME-SIM",
    "APP-HEALTH-SIM",
    "APP-LOC-SIM",
];

pub fn run(id: &str, evidence: &mut Evidence) -> bool {
    match id {
        "APP-PAIR-SIM" => pairing(evidence),
        "APP-DEVICE-SIM" => capability(
            evidence,
            id,
            &[
                "ios/Capabilities/DeviceContext/DeviceModel.swift",
                "lab/ios/DeviceProof.swift",
            ],
        ),
        "APP-MOTION-SIM" => capability(
            evidence,
            id,
            &[
                "ios/Capabilities/Motion/MotionModel.swift",
                "lab/ios/MotionProof.swift",
            ],
        ),
        "APP-HEALTH-WRITE-SIM" => health_write(evidence),
        "APP-SCREENTIME-SIM" => screentime(evidence),
        "APP-HEALTH-SIM" => health(evidence),
        "APP-LOC-SIM" => location(evidence),
        _ => return false,
    }
    true
}

fn start() -> Option<(Gateway, Phone, String)> {
    let binary = replicas::binary().ok()?;
    let gateway = Gateway::start(binary).ok()?;
    let (phone, token) = pair(&gateway).ok()?;
    Some((gateway, phone, token))
}

fn status_of(reply: &lab_world::Result<(u16, Vec<u8>)>) -> u16 {
    reply.as_ref().map_or(0, |r| r.0)
}

fn pairing(evidence: &mut Evidence) {
    let dir = Scratch::new("iphone");
    evidence.check("temporary_home", dir.is_ok(), "");
    let Ok(_dir) = dir else { return };
    let gateway = replicas::binary()
        .map_err(|e| e.to_string())
        .and_then(|b| Gateway::start(b).map_err(|e| e.to_string()));
    evidence.check(
        "authority",
        gateway.is_ok(),
        gateway.as_ref().err().map_or("", String::as_str),
    );
    let Ok(gateway) = gateway else { return };
    let output = gateway.command(&["pair", "--iphone", "--via-remote"]);
    let code = output
        .as_deref()
        .unwrap_or_default()
        .split('\n')
        .next()
        .unwrap_or_default()
        .to_owned();
    evidence.check("one_time_code", output.is_ok(), "");
    let phone = Phone::new();
    evidence.check("device_keys", phone.is_ok(), "");
    let Ok(mut phone) = phone else { return };
    evidence.check(
        "http_pair",
        phone.pair(&gateway, &code).is_ok_and(|s| s == 200),
        "",
    );
    let original = phone.id.clone();
    evidence.check(
        "replay_rejected",
        phone.pair(&gateway, &code).is_ok_and(|s| s == 401),
        "",
    );
    phone.id.clone_from(&original);
    let status = phone.request(&gateway, "/v1/iphone/status", None);
    evidence.check("signed_status", status_of(&status) == 200, "");
    evidence.check(
        "revoke",
        gateway.command(&["revoke", &original]).is_ok(),
        "",
    );
    let status = phone.request(&gateway, "/v1/iphone/status", None);
    evidence.check("revoked_key_rejected", status_of(&status) == 401, "");
}

fn deliver(dir: &Scratch, payload: &[u8], ops: &[&str], admission: &mut bool) -> bool {
    let _ = dir;
    let Some((gateway, phone, token)) = start() else {
        return false;
    };
    for (index, op) in ops.iter().enumerate() {
        let id = ["cmd_capability_first", "cmd_capability_next"][index];
        let mut args = json!({"device_id": phone.id});
        if *op == "iphone.health.log" {
            args["kind"] = json!("water");
            args["value"] = json!(250);
            args["start"] = json!("2026-10-29T12:00:00Z");
            args["tag"] = json!("[ab-test]");
        }
        if *op == "iphone.motion.history" {
            args["days"] = json!(7);
        }
        let admitted = gateway.invoke(
            op,
            &json!({"command_id": id, "op": op, "args": args}),
            &token,
        );
        *admission = admitted.as_ref().is_ok_and(|v| v["state"] == "admitted");
        if !*admission {
            return false;
        }
        let Ok((_, packets)) = phone.pending(&gateway) else {
            return false;
        };
        if packets.len() != 1 {
            return false;
        }
        let Ok(job) = phone.open(&packets[0]) else {
            return false;
        };
        if job["command"]["command_id"] != id {
            return false;
        }
        let data: Value = if index == 1 {
            json!({"activity": "walking", "confidence": "high"})
        } else {
            serde_json::from_slice(payload).unwrap_or(Value::Null)
        };
        let result = json!({"command_id": id, "request_hash": job["request_hash"], "state": "observed", "data": data});
        if !phone
            .post(&gateway, "/v1/iphone/result", &result)
            .is_ok_and(|r| r.0 == 200)
        {
            return false;
        }
        let observed = gateway.invoke("commands.status", &json!({"command_id": id}), &token);
        if !observed.is_ok_and(|v| v["state"] == "observed") {
            return false;
        }
    }
    true
}

fn capability(evidence: &mut Evidence, id: &str, sources: &[&str]) {
    let Ok(dir) = Scratch::new("ios-proof") else {
        return;
    };
    let binary = dir.path().join("proof");
    let out = binary.to_string_lossy().into_owned();
    let mut args = vec!["swiftc", "-parse-as-library", "-o", out.as_str()];
    args.extend_from_slice(sources);
    let built = project("xcrun", &args).is_ok_and(|o| o.status.success());
    evidence.check("swift_adapter_build", built, "");
    if !built {
        return;
    }
    let payload_file = dir.path().join("result.json");
    let output = Command::new(&binary).arg(&payload_file).output();
    let expected = if id == "APP-DEVICE-SIM" {
        "PASS 5\n"
    } else {
        "PASS 8\n"
    };
    let functional = output.as_ref().is_ok_and(|o| {
        o.status.success()
            && (o.stdout == b"PASS 8\n"
                || id == "APP-DEVICE-SIM" && o.stdout == expected.as_bytes())
    });
    evidence.check("fake_adapter_functional", functional, "");
    if functional {
        let ops: &[&str] = if id == "APP-DEVICE-SIM" {
            &["iphone.device.status"]
        } else {
            &["iphone.motion.history", "iphone.motion.activity"]
        };
        let payload = fs::read(&payload_file);
        let mut admission = false;
        let ok = payload
            .as_ref()
            .is_ok_and(|p| p.len() <= 20000 && deliver(&dir, p, ops, &mut admission));
        evidence.check("signed_result_upload_then_next_command", ok, "");
    }
}

fn health_write(evidence: &mut Evidence) {
    let Ok(dir) = Scratch::new("health-write") else {
        return;
    };
    let binary = dir.path().join("proof");
    let built = swift_build(
        &binary,
        &["-parse-as-library"],
        &[
            "ios/Capabilities/HealthWrite/HealthLog.swift",
            "lab/ios/HealthWriteProof.swift",
        ],
    );
    evidence.check("swift_write_build", built, "");
    if !built {
        return;
    }
    let output = Command::new(&binary).output();
    let proof = output
        .as_ref()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "PASS");
    evidence.check("four_tagged_samples_written_read_deleted", proof, "");
    let mut admission = false;
    let delivered = deliver(
        &dir,
        br#"{"id":"ab-test"}"#,
        &["iphone.health.log"],
        &mut admission,
    );
    evidence.check("gateway_admission", admission, "");
    evidence.check("signed_health_command_roundtrip", delivered, "");
}

fn screentime(evidence: &mut Evidence) {
    let dir = Scratch::new("screentime");
    evidence.check("isolated_fixture", dir.is_ok(), "");
    let Ok(dir) = dir else { return };
    let binary = dir.path().join("shield-policy");
    let built = project(
        "swiftc",
        &[
            "ios/ScreenTimeShared/ShieldPolicy.swift",
            "lab/ios/main.swift",
            "-o",
            &binary.to_string_lossy(),
        ],
    )
    .is_ok_and(|o| o.status.success());
    evidence.check("swift_policy_compiled", built, "");
    if built {
        let output = Command::new(&binary).output();
        evidence.check(
            "fake_managed_settings_shield_unshield_downtime_isolation",
            output.as_ref().is_ok_and(|o| {
                o.status.success()
                    && String::from_utf8_lossy(&o.stdout).trim() == "shield_policy_pass"
            }),
            "",
        );
    }
    let fixture = start();
    evidence.check("binary_gateway_fixture", fixture.is_some(), "");
    let Some((gateway, phone, token)) = fixture else {
        return;
    };
    for op in ["shield", "unshield", "cancel", "schedule"] {
        let mut args = json!({"device_id": phone.id});
        if op == "schedule" {
            args["start_minute"] = json!(1380);
            args["end_minute"] = json!(60);
        }
        let admitted = gateway.invoke(
            &format!("iphone.screentime.{op}"),
            &json!({"command_id": format!("cmd_screentime_{op}"), "args": args}),
            &token,
        );
        evidence.check(
            &format!("gateway_{op}"),
            admitted.is_ok_and(|v| v["state"] == "admitted"),
            "",
        );
    }
    let invalid = gateway.request(
        "/v1/op/iphone.screentime.schedule",
        &json!({"command_id": "cmd_screentime_invalid", "args": {"device_id": phone.id, "start_minute": 60, "end_minute": 61}}),
        &token,
    );
    evidence.check(
        "invalid_schedule_rejected",
        invalid.is_ok_and(|r| r.0 == 400),
        "",
    );
}

fn f64_at(value: &Value, key: &str, field: &str) -> f64 {
    value["metrics"][key][field].as_f64().unwrap_or(f64::NAN)
}

fn health(evidence: &mut Evidence) {
    let dir = Scratch::new("health");
    evidence.check("isolated_home", dir.is_ok(), "");
    let Ok(dir) = dir else { return };
    let binary = dir.path().join("proof");
    let built = swift_build(
        &binary,
        &["-parse-as-library"],
        &[
            "ios/Capabilities/HealthRead/HealthCapture.swift",
            "lab/ios/HealthProof.swift",
        ],
    );
    evidence.check("swift_capture_build", built, "");
    if built {
        let output = Command::new(&binary).output();
        evidence.check(
            "anchored_retry_restart_deletion",
            output.as_ref().is_ok_and(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "PASS"
            }),
            "",
        );
    }
    let fixture = start();
    evidence.check("binary_gateway_fixture", fixture.is_some(), "");
    let Some((gateway, phone, token)) = fixture else {
        return;
    };
    let health_token = gateway.command(&["token", "create", "--scope", "health:read"]);
    evidence.check("health_read_scope", health_token.is_ok(), "");
    let health_token = health_token.unwrap_or_default();
    let summary = |period: &str, date: &str| {
        gateway.invoke(
            "health.summary",
            &json!({"period": period, "date": date}),
            &health_token,
        )
    };
    let upload = |batch: &Value| {
        phone
            .post(&gateway, "/v1/iphone/health", batch)
            .map_or(0, |r| r.0)
    };
    let sample = json!({"id": "ab-test-steps", "type": "steps", "start": "2026-10-28T12:00:00Z", "end": "2026-10-28T13:00:00Z", "value": 120.0, "unit": "count"});
    let mut batch =
        json!({"stream": "steps", "sequence": 1, "samples": [sample.clone()], "deleted": []});
    evidence.check("signed_capture", upload(&batch) == 200, "");
    evidence.check("retry_deduplicated", upload(&batch) == 200, "");
    let mut changed = batch.clone();
    changed["samples"][0]["unit"] = json!("m");
    evidence.check("altered_retry_rejected", upload(&changed) == 400, "");
    batch["sequence"] = json!(3);
    evidence.check("sequence_gap_rejected", upload(&batch) == 400, "");
    batch["sequence"] = json!(1);
    let query = |include_deleted: bool| {
        let mut q = json!({"collection": "health.sample"});
        if include_deleted {
            q["include_deleted"] = json!(true);
        }
        gateway.invoke("records.query", &q, &health_token)
    };
    let result = query(false);
    evidence.check(
        "sample_record",
        result.as_ref().is_ok_and(|r| {
            r["records"].as_array().is_some_and(|a| a.len() == 1) && r["records"][0]["version"] == 1
        }),
        "",
    );
    let unauthorized = gateway.invoke(
        "health.summary",
        &json!({"period": "day", "date": "2026-10-28"}),
        &token,
    );
    evidence.check("summary_scope_denied", unauthorized.is_err(), "");
    let day = summary("day", "2026-10-28");
    evidence.check(
        "day_summary",
        day.as_ref()
            .is_ok_and(|s| f64_at(s, "steps/count", "sum") == 120.0),
        "",
    );
    let week = summary("week", "2026-10-29");
    evidence.check(
        "week_summary",
        week.as_ref().is_ok_and(|s| {
            s["start"].as_str().and_then(|t| t.get(8..10)) == Some("26")
                && f64_at(s, "steps/count", "count") == 1.0
        }),
        "",
    );
    let mut sleep = json!({"stream": "sleep", "sequence": 1, "samples": [{"id": "ab-test-sleep", "type": "sleep", "start": "2026-10-27T23:00:00Z", "end": "2026-10-28T07:00:00Z", "category": 3}], "deleted": []});
    evidence.check("sleep_stage_capture", upload(&sleep) == 200, "");
    let day = summary("day", "2026-10-28");
    evidence.check(
        "sleep_stage_clipped_to_day",
        day.as_ref()
            .is_ok_and(|s| f64_at(s, "sleep//3", "seconds") == 25200.0),
        "",
    );
    sleep["sequence"] = json!(2);
    sleep["samples"] = json!([]);
    sleep["deleted"] = json!(["ab-test-sleep"]);
    evidence.check("sleep_deletion", upload(&sleep) == 200, "");
    let deletion =
        json!({"stream": "steps", "sequence": 2, "samples": [], "deleted": ["ab-test-steps"]});
    evidence.check("signed_deletion", upload(&deletion) == 200, "");
    let result = query(true);
    evidence.check(
        "deletion_record",
        result.as_ref().is_ok_and(|r| {
            r["records"].as_array().is_some_and(|a| {
                a.len() == 2 && a.iter().all(|x| x["deleted"] == true && x["version"] == 2)
            })
        }),
        "",
    );
    let day = summary("day", "2026-10-28");
    evidence.check(
        "deleted_excluded",
        day.as_ref().is_ok_and(|s| {
            s["metrics"]
                .as_object()
                .is_none_or(serde_json::Map::is_empty)
        }),
        "",
    );
    let workouts = json!({"stream": "workouts", "sequence": 1, "samples": [
        {"id": "ab-test-workout-1", "type": "workouts", "start": "2026-10-28T12:00:00Z", "end": "2026-10-28T13:00:00Z"},
        {"id": "ab-test-workout-2", "type": "workouts", "start": "2026-10-28T14:00:00Z", "end": "2026-10-28T15:00:00Z"},
    ], "deleted": []});
    evidence.check("workouts_capture", upload(&workouts) == 200, "");
    for period in ["day", "week"] {
        let s = summary(period, "2026-10-28");
        evidence.check(
            &format!("{period}_unitless_workouts_accumulate"),
            s.as_ref().is_ok_and(|s| {
                f64_at(s, "workouts", "count") == 2.0 && f64_at(s, "workouts", "seconds") == 7200.0
            }),
            "",
        );
    }
    let records = result.map_or_else(|_| String::new(), |r| r["records"].to_string());
    evidence.check(
        "digest",
        true,
        &hex::encode(Sha256::digest(records.as_bytes())),
    );
}

fn location(evidence: &mut Evidence) {
    let dir = Scratch::new("location");
    evidence.check("isolated_home", dir.is_ok(), "");
    let Ok(dir) = dir else { return };
    let binary = dir.path().join("location-proof");
    let built = swift_build(
        &binary,
        &["-swift-version", "6", "-parse-as-library"],
        &[
            "ios/Capabilities/Location/LocationStore.swift",
            "ios/App/QualificationWait.swift",
            "lab/fixtures/location/LocationProof.swift",
        ],
    );
    evidence.check("swift_adapter_build", built, "");
    if !built {
        return;
    }
    let fixture = dir.path().join("records.json");
    let output = Command::new(&binary).arg(&fixture).output();
    let mut combined = vec![];
    if let Ok(o) = &output {
        combined.extend_from_slice(&o.stdout);
        combined.extend_from_slice(&o.stderr);
    }
    let proof = output.as_ref().is_ok_and(|o| o.status.success())
        && combined == b"location_adapter_proof=pass\n";
    evidence.check(
        "fake_adapter_current_geofence_events_significant_restart_permission",
        proof,
        "",
    );
    if !proof {
        return;
    }
    let started = start();
    evidence.check("binary_gateway_fixture", started.is_some(), "");
    let Some((gateway, phone, token)) = started else {
        return;
    };
    let read_token = gateway.command(&["token", "create", "--scope", "iphone:read"]);
    evidence.check("read_grant", read_token.is_ok(), "");
    let read_token = read_token.unwrap_or_default();
    let raw = fs::read(&fixture);
    evidence.check("fixture", raw.is_ok(), "");
    let raw = raw.unwrap_or_default();
    for _ in 0..2 {
        let reply = phone.request(&gateway, "/v1/iphone/location-records", Some(&raw));
        evidence.check("signed_records_accepted", status_of(&reply) == 200, "");
    }
    let result = gateway.invoke(
        "records.query",
        &json!({"collection": "iphone.location", "limit": 100}),
        &read_token,
    );
    evidence.check(
        "durable_deduplicated_location_records",
        result.is_ok_and(|r| r["records"].as_array().is_some_and(|a| a.len() == 5)),
        "",
    );
    let invalid = gateway.request(
        "/v1/op/iphone.geofence.add",
        &json!({"command_id": "cmd_loc_invalid", "args": {"device_id": phone.id, "name": "[ab-test] fence", "latitude": 91, "longitude": 34, "radius": 150}}),
        &token,
    );
    evidence.check(
        "invalid_geofence_rejected",
        invalid.is_ok_and(|r| r.0 == 400),
        "",
    );
    let ops = [
        "iphone.location.current",
        "iphone.geofence.add",
        "iphone.geofence.list",
        "iphone.geofence.remove",
        "iphone.location.significant",
    ];
    for (index, op) in ops.iter().enumerate() {
        let mut values = json!({"device_id": phone.id});
        match *op {
            "iphone.geofence.add" => {
                values["name"] = json!("[ab-test] fence");
                values["latitude"] = json!(12);
                values["longitude"] = json!(34);
                values["radius"] = json!(150);
            }
            "iphone.geofence.remove" => values["name"] = json!("[ab-test] fence"),
            "iphone.location.significant" => values["enabled"] = json!(true),
            _ => {}
        }
        let id = format!("cmd_location_{index}");
        let admitted = gateway.invoke(
            op,
            &json!({"command_id": id, "op": op, "args": values}),
            &token,
        );
        let ok = admitted.is_ok_and(|v| v["state"] == "admitted");
        evidence.check("location_command_admitted", ok, "");
        if *op == "iphone.geofence.add" {
            evidence.check("geofence_command_registered", ok, "");
        }
        let Ok((_, packets)) = phone.pending(&gateway) else {
            evidence.check("pending_command", false, "");
            return;
        };
        evidence.check("encrypted_location_command", packets.len() == 1, "");
        if packets.len() != 1 {
            return;
        }
        let job = phone.open(&packets[0]);
        evidence.check(
            "location_command_decrypted",
            job.as_ref().is_ok_and(|j| j["command"]["op"] == *op),
            "",
        );
        let hash = job.map_or(Value::Null, |j| j["request_hash"].clone());
        let summary = json!({"command_id": id, "request_hash": hash, "state": "observed", "data": {"count": 1, "significant": true}});
        let mut unsafe_result = summary.clone();
        unsafe_result["data"] =
            json!({"count": 1, "significant": true, "latitude": 12, "longitude": 34});
        let unsafe_reply = phone.post(&gateway, "/v1/iphone/result", &unsafe_result);
        evidence.check(
            "coordinate_bearing_receipt_rejected",
            status_of(&unsafe_reply) == 400,
            "",
        );
        let reply = phone.post(&gateway, "/v1/iphone/result", &summary);
        evidence.check("location_completion_accepted", status_of(&reply) == 200, "");
        let completed = gateway.invoke("commands.status", &json!({"command_id": id}), &token);
        evidence.check(
            "location_completion_observed",
            completed.as_ref().is_ok_and(|c| c["state"] == "observed"),
            "",
        );
        let free = completed.as_ref().is_ok_and(|c| {
            use base64::{Engine, engine::general_purpose::STANDARD};
            c["evidence"]["payload"]
                .as_str()
                .and_then(|p| STANDARD.decode(p).ok())
                .and_then(|p| serde_json::from_slice::<Value>(&p).ok())
                .is_some_and(|p| {
                    p["data"].as_object().is_some_and(|d| {
                        d.len() == 2 && d["count"] == 1.0 && d["significant"] == true
                    })
                })
        });
        evidence.check("coordinate_free_receipt", free, "");
    }
}
