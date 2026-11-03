use crate::{
    assertions::await_until,
    http::{Client, random_id},
    journey::command,
    mac_real,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

pub const SIM_SCENARIOS: [&str; 6] = [
    "MAC-ACTIONS",
    "MAC-COMMANDS",
    "MAC-CAPTURE",
    "MAC-IDLE",
    "MAC-EXTRAS-SIM",
    "IWORK-REAL",
];
pub(crate) const SCOPE: &str = "mac:read,mac:write,system:read";
const POWER_OPS: [&str; 4] = [
    "mac.lock_screen",
    "mac.sleep_display",
    "mac.restart",
    "mac.shutdown",
];
const COLLECTIONS: [&str; 5] = [
    "mac.audio_device",
    "mac.display",
    "mac.power",
    "mac.network",
    "mac.app",
];
const COMMAND_OPS: [&str; 16] = [
    "mac.set_volume",
    "mac.set_mute",
    "mac.set_appearance",
    "mac.launch_app",
    "mac.activate_app",
    "mac.quit_app",
    "mac.keep_awake",
    "mac.clipboard_write",
    "mac.notify",
    "mac.open_settings",
    "mac.set_wallpaper",
    "mac.lock_screen",
    "mac.sleep_display",
    "mac.restart",
    "mac.shutdown",
    "mac.iwork_export",
];
const QUERY_OPS: [&str; 5] = [
    "mac.clipboard_read",
    "mac.network_check",
    "mac.bluetooth_devices",
    "mac.time_machine_status",
    "mac.recognize_song",
];

pub(crate) fn query_call(client: &Client, op: &str, args: &Value) -> Option<Value> {
    let query = json!({"id": random_id("qry").ok()?, "args": args, "deadline_ms": 20000});
    client.call(op, &query, 40).ok().flatten()
}
pub(crate) fn command_call(client: &Client, op: &str, args: &Value) -> Option<Value> {
    client.call(op, &command(op, args).ok()?, 40).ok().flatten()
}
pub(crate) fn readback(record: &Value) -> bool {
    let evidence = &record["evidence"];
    let hash = record["request_hash"].as_str().unwrap_or_default();
    record["state"] == "observed"
        && evidence["observed"] == true
        && !hash.is_empty()
        && evidence["fingerprint"] == hash
        && !evidence["readback"].is_null()
}
pub(crate) fn command_readback(client: &Client, op: &str, args: &Value) -> bool {
    command_call(client, op, args).is_some_and(|record| readback(&record))
}
fn rejected(client: &Client, op: &str, body: &Value) -> bool {
    client
        .call_raw(op, body, 40)
        .ok()
        .flatten()
        .is_none_or(|(status, value)| {
            status != 200 || !value["error"].is_null() || value["state"] == "rejected"
        })
}
pub(crate) fn setup(world: &World, evidence: &mut Evidence) -> Result<Option<Client>> {
    let client = Client::new(world, SCOPE)?;
    evidence.check("scope_granted", true, "scoped token created");
    let ready = await_until(20, || {
        query_call(&client, "helper.version", &json!({})).is_some()
    });
    evidence.check("helper_ready", ready, "helper version answers");
    Ok(ready.then_some(client))
}
fn fixture(collection: &str, value: &str) -> Value {
    match collection {
        "mac.audio_device" => {
            json!({"name": value, "input": true, "output": true, "default": true, "volume": 0.5, "muted": value == "changed"})
        }
        "mac.display" => json!({"appearance": value, "displays": []}),
        "mac.power" => {
            json!({"source": value, "charging": true, "low_power_mode": false, "keep_awake_held": false, "battery_percent": 75})
        }
        "mac.network" => {
            json!({"interface": value, "connected": true, "vpn": false, "wifi_power": true})
        }
        _ => {
            json!({"bundle_id": "com.example.bridge", "name": value, "running": true, "frontmost": true, "hidden": false})
        }
    }
}
fn shape(collection: &str, data: &Value) -> bool {
    let fields: &[(&str, char)] = match collection {
        "mac.audio_device" => &[
            ("name", 's'),
            ("input", 'b'),
            ("output", 'b'),
            ("default", 'b'),
            ("volume", 'n'),
            ("muted", 'b'),
        ],
        "mac.display" => &[("appearance", 's'), ("displays", 'a')],
        "mac.power" => &[
            ("source", 's'),
            ("charging", 'b'),
            ("low_power_mode", 'b'),
            ("keep_awake_held", 'b'),
            ("battery_percent", 'n'),
        ],
        "mac.network" => &[
            ("interface", 's'),
            ("connected", 'b'),
            ("vpn", 'b'),
            ("wifi_power", 'b'),
        ],
        "mac.bluetooth_device" => &[("name", 's'), ("connected", 'b'), ("battery_percent", 'n')],
        _ => &[
            ("bundle_id", 's'),
            ("name", 's'),
            ("running", 'b'),
            ("frontmost", 'b'),
            ("hidden", 'b'),
        ],
    };
    fields.iter().all(|(name, kind)| match kind {
        's' => data[*name].is_string(),
        'b' => data[*name].is_boolean(),
        'n' => data[*name].is_number(),
        _ => data[*name].is_array(),
    })
}
fn actions(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    for (op, args) in [
        ("mac.notify", json!({"title":"bridge-lab","body":"fixture"})),
        ("mac.open_settings", json!({"pane":"notifications"})),
        ("mac.set_wallpaper", json!({"image_blob":"aW1hZ2U="})),
    ] {
        evidence.check(
            op,
            command_readback(client, op, &args),
            "observed with fingerprint readback",
        );
    }
    for permission in ["not_determined", "denied"] {
        let set = world
            .control("/mac-notifications", &json!({"permission": permission}))
            .is_ok();
        evidence.check(
            "notification_permission_fixture",
            set,
            "permission fixture accepted",
        );
        let body = command(
            "mac.notify",
            &json!({"title":"bridge-lab","body":"fixture"}),
        )?;
        evidence.check(
            &format!("notify_{permission}_rejected"),
            rejected(client, "mac.notify", &body),
            "notify without permission is rejected",
        );
    }
    for op in POWER_OPS {
        let body = json!({"command_id": random_id("cmd")?, "args": {}});
        evidence.check(
            &format!("{op}_requires_approve"),
            rejected(client, op, &body),
            "scoped token cannot run power command",
        );
    }
    let listed = world.gateway_cli(&["agent", "list"]).is_ok();
    evidence.check("approval_authority", listed, "agent list succeeds");
    if listed {
        for level in ["full", "approve"] {
            let created = world
                .gateway_cli(&[
                    "agent",
                    "create",
                    "--name",
                    "Bridge Test Mac power",
                    "--level",
                    level,
                    "--scope",
                    "mac:write",
                ])
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(raw.trim()).ok())
                .and_then(|value| value["token"].as_str().map(str::to_owned));
            evidence.check(
                "power_agent_created",
                created.is_some(),
                "power agent created",
            );
            let agent = match &created {
                Some(token) => Client::with_token(world, token)?,
                None => Client::with_token(world, "")?,
            };
            for op in POWER_OPS {
                let body = json!({"command_id": random_id("cmd")?, "args": {}});
                let reply = agent.call_raw(op, &body, 40)?;
                if level == "approve" {
                    evidence.check(
                        &format!("{op}_awaiting_approval"),
                        reply.is_some_and(|(status, value)| {
                            status == 200
                                && value["error"].is_null()
                                && value["state"] == "awaiting_approval"
                        }),
                        "approve agent waits for approval",
                    );
                } else {
                    evidence.check(
                        &format!("{op}_full_agent_denied"),
                        reply.is_none_or(|(status, value)| {
                            status != 200 || !value["error"].is_null()
                        }),
                        "full agent is denied",
                    );
                }
            }
        }
    }
    evidence.check(
        "network_query",
        query_call(client, "mac.network_check", &json!({})).is_some(),
        "network check answers",
    );
    Ok(())
}
fn schema_of(document: &Value, op: &str) -> Value {
    document["paths"][format!("/v1/op/{op}")]["post"]["requestBody"]["content"]["application/json"]
        ["schema"]
        .clone()
}
fn required_is(value: &Value, expected: &[&str]) -> bool {
    value.as_array().is_some_and(|items| {
        items.len() == expected.len()
            && items
                .iter()
                .zip(expected)
                .all(|(item, name)| item.as_str() == Some(*name))
    })
}
fn schemas(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    let reply = client.request("GET", "/v1/openapi.json", &[], None, 30)?;
    let document: Option<Value> = (reply.status == 200)
        .then(|| serde_json::from_slice(&reply.body).ok())
        .flatten();
    let all: Vec<&str> = COMMAND_OPS.iter().chain(&QUERY_OPS).copied().collect();
    let present = document
        .as_ref()
        .is_some_and(|d| all.iter().all(|op| !schema_of(d, op).is_null()));
    let _ = world;
    evidence.check(
        "binary_mac_schemas",
        present,
        "gateway OpenAPI describes every mac operation",
    );
    let Some(document) = document.filter(|_| present) else {
        return Ok(());
    };
    let expected = [
        ("mac.set_volume", "volume"),
        ("mac.set_mute", "muted"),
        ("mac.set_appearance", "appearance"),
        ("mac.launch_app", "bundle_id"),
        ("mac.activate_app", "bundle_id"),
        ("mac.quit_app", "bundle_id"),
        ("mac.keep_awake", "on"),
        ("mac.clipboard_write", "text"),
    ];
    for op in all {
        let schema = schema_of(&document, op);
        let args = &schema["properties"]["args"];
        let fields = args["properties"].as_object();
        let count = fields.map_or(0, serde_json::Map::len);
        if op == "mac.recognize_song" {
            let field = &args["properties"]["audio_blob"];
            let valid = field.is_object()
                && count == 1
                && field["type"] == "string"
                && field["minLength"].as_u64() == Some(4)
                && field["maxLength"].as_u64() == Some(11_184_812)
                && !field["pattern"].is_null()
                && required_is(&args["required"], &["audio_blob"])
                && args["additionalProperties"] == false;
            evidence.check(&format!("{op}_argument_schema"), valid, "schema matches");
            continue;
        }
        if QUERY_OPS.contains(&op) {
            evidence.check(
                &format!("{op}_empty_schema"),
                count == 0 && args["additionalProperties"] == false,
                "schema is empty",
            );
            continue;
        }
        let Some((_, key)) = expected.iter().find(|(name, _)| *name == op) else {
            continue;
        };
        let field = &args["properties"][*key];
        let wanted = if op == "mac.keep_awake" { 2 } else { 1 };
        evidence.check(
            &format!("{op}_argument_schema"),
            field.is_object()
                && field["nullable"].is_null()
                && count == wanted
                && required_is(&args["required"], &[*key])
                && required_is(&schema["required"], &["args"]),
            "schema matches",
        );
    }
    Ok(())
}
fn private_files(root: &std::path::Path, tag: &str) -> std::io::Result<bool> {
    let mut private = true;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            private &= private_files(&entry.path(), tag)?;
        } else if kind.is_file() {
            match fs::read(entry.path()) {
                Ok(data) => {
                    private &= !data
                        .windows(tag.len())
                        .any(|window| window == tag.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(private)
}
pub(crate) fn clipboard(
    world: &World,
    client: &Client,
    real: bool,
    evidence: &mut Evidence,
) -> Result<()> {
    let tag = format!("[ab-test {}]", random_id("clipboard")?);
    let record = command_call(client, "mac.clipboard_write", &json!({"text": tag}));
    evidence.check(
        "clipboard_write_readback",
        record.as_ref().is_some_and(readback),
        "write observed with readback",
    );
    let raw = record.as_ref().map(Value::to_string).unwrap_or_default();
    evidence.check(
        "clipboard_command_redacted",
        record.as_ref().is_some_and(|r| {
            r["command"]["op"] == "mac.clipboard_write" && r["command"]["args"] == json!({})
        }) && !raw.contains(&tag),
        "command record carries no clipboard text",
    );
    let echoed = query_call(client, "mac.clipboard_read", &json!({}));
    evidence.check(
        "clipboard_read_matches",
        echoed.is_some_and(|r| r["result_json"]["text"] == tag),
        "read returns the written text",
    );
    if real {
        mac_real::manager_markers(evidence);
    }
    evidence.check(
        "clipboard_absent_from_durable_files",
        private_files(&world.directory, &tag).unwrap_or(false),
        "no durable file contains the clipboard text",
    );
    Ok(())
}
fn commands(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    schemas(world, client, evidence)?;
    for (op, args) in [
        ("mac.set_volume", json!({"volume": 23})),
        ("mac.set_mute", json!({"muted": true})),
        ("mac.set_appearance", json!({"appearance": "dark"})),
        (
            "mac.launch_app",
            json!({"bundle_id": "com.apple.calculator"}),
        ),
        (
            "mac.activate_app",
            json!({"bundle_id": "com.apple.calculator"}),
        ),
        ("mac.quit_app", json!({"bundle_id": "com.apple.calculator"})),
        ("mac.keep_awake", json!({"on": true, "duration": 60})),
        ("mac.keep_awake", json!({"on": false})),
    ] {
        evidence.check(
            &format!("{op}_readback"),
            command_readback(client, op, &args),
            "observed with fingerprint readback",
        );
    }
    clipboard(world, client, false, evidence)
}
fn visible(client: &Client, collection: &str, value: &str) -> bool {
    let field = match collection {
        "mac.display" => "appearance",
        "mac.power" => "source",
        "mac.network" => "interface",
        _ => "name",
    };
    client
        .call(
            "records.query",
            &json!({"collection": collection, "limit": 100}),
            10,
        )
        .ok()
        .flatten()
        .is_some_and(|page| {
            page["records"].as_array().is_some_and(|records| {
                records
                    .iter()
                    .any(|r| r["data"][field] == value && shape(collection, &r["data"]))
            })
        })
}
fn capture(world: &World, client: &Client, evidence: &mut Evidence) {
    for collection in COLLECTIONS {
        for value in ["first", "changed"] {
            let start = Instant::now();
            let accepted = world
                .control(
                    "/mac",
                    &json!({"collection": collection, "key": "fixture", "data": fixture(collection, value)}),
                )
                .is_ok();
            evidence.check("fake_change_accepted", accepted, "fake change accepted");
            let deadline = Instant::now() + Duration::from_secs(1);
            let seen = loop {
                if visible(client, collection, value) {
                    break true;
                }
                if Instant::now() >= deadline {
                    break false;
                }
                thread::sleep(Duration::from_millis(10));
            };
            evidence.check(
                "change_visible_under_second",
                seen && start.elapsed() < Duration::from_secs(1),
                "change visible through records.query within one second",
            );
        }
    }
    if evidence.scenario == "MAC-IDLE" {
        let read = || {
            client
                .call(
                    "records.query",
                    &json!({"collection": "mac.power", "limit": 100}),
                    10,
                )
                .ok()
                .flatten()
                .map(|page| page["records"].clone())
        };
        let before = read();
        thread::sleep(Duration::from_secs(1));
        let after = read();
        evidence.check(
            "idle_records_unchanged",
            before.is_some()
                && before == after
                && before
                    .as_ref()
                    .and_then(Value::as_array)
                    .is_some_and(|r| !r.is_empty()),
            "records are unchanged after one idle second",
        );
    }
}
fn decode(record: &Value) -> Option<Vec<u8>> {
    STANDARD
        .decode(record["evidence"]["readback"]["data_base64"].as_str()?)
        .ok()
}
fn extras(world: &World, client: &Client, evidence: &mut Evidence) -> Result<()> {
    let injected = world
        .control(
            "/mac",
            &json!({"collection": "mac.bluetooth_device", "key": "bridge-test-headphones", "data": {"name": "Bridge Test Headphones", "connected": true, "battery_percent": 75}}),
        )
        .is_ok();
    evidence.check(
        "bluetooth_capture_injected",
        injected,
        "bluetooth fixture accepted",
    );
    let seen = await_until(1, || {
        client
            .call(
                "records.query",
                &json!({"collection": "mac.bluetooth_device", "limit": 10}),
                10,
            )
            .ok()
            .flatten()
            .is_some_and(|page| {
                page["records"].as_array().is_some_and(|records| {
                    records.len() == 1 && shape("mac.bluetooth_device", &records[0]["data"])
                })
            })
    });
    evidence.check(
        "bluetooth_capture_visible",
        seen,
        "bluetooth device visible through records.query",
    );
    for (op, args, field, expected) in [
        ("mac.bluetooth_devices", json!({}), "devices", Value::Null),
        (
            "mac.time_machine_status",
            json!({}),
            "running",
            json!(false),
        ),
        (
            "mac.recognize_song",
            json!({"audio_blob": "YnJpZGdlLWxhYi1hdWRpbw=="}),
            "title",
            json!("Bridge Test Song"),
        ),
    ] {
        let result = query_call(client, op, &args).map(|r| r["result_json"].clone());
        let ok = result.as_ref().is_some_and(|result| {
            let value = &result[field];
            if expected.is_null() {
                value.as_array().is_some_and(|items| items.len() == 1)
            } else {
                *value == expected
            }
        });
        evidence.check(&format!("{op}_fixture"), ok, "fixture query answers");
    }
    exports(client, evidence)
}
fn exports(client: &Client, evidence: &mut Evidence) -> Result<()> {
    let record = command_call(
        client,
        "mac.iwork_export",
        &json!({"app":"pages","format":"text","document_blob":"YnJpZGdlLWxhYg=="}),
    );
    evidence.check(
        "mac.iwork_export_readback",
        record.as_ref().is_some_and(|r| {
            readback(r) && r["evidence"]["readback"]["data_base64"] == "YnJpZGdlLWxhYg=="
        }),
        "export observed with readback",
    );
    for app in ["pages", "numbers", "keynote"] {
        for format in ["pdf", "text"] {
            let record = command_call(
                client,
                "mac.iwork_export",
                &json!({"app": app, "format": format, "document_blob": "YnJpZGdlLWxhYg=="}),
            );
            let data = record.as_ref().and_then(decode);
            let valid = data.as_ref().is_some_and(|data| {
                if format == "pdf" {
                    data.starts_with(b"%PDF-1.4")
                        && data.windows(10).any(|w| w == b"bridge-lab")
                        && data.ends_with(b"%%EOF\n")
                } else {
                    data == b"bridge-lab"
                }
            });
            evidence.check(
                &format!("{app}_{format}_export"),
                valid && record.is_some_and(|r| r["state"] == "observed"),
                "export content matches",
            );
        }
    }
    let invalid = json!({"command_id": random_id("cmd")?, "args": {}});
    evidence.check(
        "mac.iwork_export_invalid_rejected",
        rejected(client, "mac.iwork_export", &invalid),
        "empty export arguments are rejected",
    );
    evidence.check(
        "invalid_audio_rejected",
        query_call(client, "mac.recognize_song", &json!({"audio_blob": ""})).is_none(),
        "empty audio is rejected",
    );
    Ok(())
}
pub(crate) fn iwork(client: &Client, evidence: &mut Evidence) {
    let record = command_call(
        client,
        "mac.iwork_export",
        &json!({"app":"pages","format":"text","document_blob":"W2FiLXRlc3Rd"}),
    );
    evidence.check(
        "iwork_export_observed",
        record.as_ref().is_some_and(readback),
        "export observed with readback",
    );
    let tagged = record
        .as_ref()
        .and_then(decode)
        .is_some_and(|data| String::from_utf8_lossy(&data).trim() == "[ab-test]");
    evidence.check("exported_tagged_content", tagged, "export carries the tag");
}
pub fn mac(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if !SIM_SCENARIOS.contains(&id.as_str()) {
        return mac_real::run(world, evidence);
    }
    let Some(client) = setup(world, evidence)? else {
        return Ok(());
    };
    match id.as_str() {
        "MAC-ACTIONS" => actions(world, &client, evidence),
        "MAC-COMMANDS" => commands(world, &client, evidence),
        "MAC-CAPTURE" | "MAC-IDLE" => {
            capture(world, &client, evidence);
            Ok(())
        }
        "MAC-EXTRAS-SIM" => extras(world, &client, evidence),
        "IWORK-REAL" => {
            iwork(&client, evidence);
            Ok(())
        }
        other => Err(Error::Invalid(format!("mac scenario not ported: {other}"))),
    }
}
