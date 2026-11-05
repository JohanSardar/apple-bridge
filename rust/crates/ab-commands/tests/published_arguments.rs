#![allow(clippy::expect_used, clippy::too_many_lines)]
use ab_commands::{Command, validate};
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

const DEVICE: &str = "iphone:one";
const ID: &str = "4f2c7d0e-3a55-4b6e-9f10-2d7a1c9e8b00";

fn published() -> std::result::Result<BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    let mut inputs = BTreeMap::new();
    for operation in ab_catalog::operations()? {
        let described = serde_json::to_value(&operation)?;
        inputs.insert(operation.name.clone(), described["input"].clone());
    }
    Ok(inputs)
}

fn accepted(op: &str, args: &Value, version: u64) -> bool {
    let body = json!({"command_id": "c1", "op": op, "args": args, "expected_version": version});
    let command: Command = serde_json::from_str(&body.to_string()).expect("command");
    let now = "2026-01-01T00:00:00Z".parse().expect("time");
    validate(&command, now).is_ok()
}

fn examples() -> Vec<(&'static str, Value, bool)> {
    let target = json!({"device_id": DEVICE});
    let live =
        json!({"device_id": DEVICE, "task_id": "t", "title": "x", "status": "", "progress": 0.5});
    let at = 1_900_000_000;
    vec![
        ("contacts.create", json!({"given_name": "A"}), false),
        (
            "contacts.update",
            json!({"id": "c", "given_name": "A"}),
            true,
        ),
        ("contacts.delete", json!({"id": "c"}), true),
        ("contacts.create_group", json!({"name": "G"}), false),
        (
            "contacts.rename_group",
            json!({"id": "g", "name": "G"}),
            true,
        ),
        ("contacts.delete_group", json!({"id": "g"}), true),
        (
            "contacts.add_member",
            json!({"id": "g", "contact_id": "c"}),
            true,
        ),
        (
            "contacts.remove_member",
            json!({"id": "g", "contact_id": "c"}),
            true,
        ),
        ("notes.move", json!({"id": "n", "folder_id": "f"}), true),
        (
            "notes.rename_folder",
            json!({"id": "f", "title": "Bridge Test [ab-test r]"}),
            true,
        ),
        (
            "iphone.alarm.create",
            json!({"device_id": DEVICE, "fire_at": at}),
            false,
        ),
        (
            "iphone.alarm.update",
            json!({"device_id": DEVICE, "id": ID, "fire_at": at}),
            false,
        ),
        (
            "iphone.alarm.cancel",
            json!({"device_id": DEVICE, "id": ID}),
            false,
        ),
        ("iphone.alarm.list", target.clone(), false),
        (
            "iphone.timer.start",
            json!({"device_id": DEVICE, "duration_seconds": 60}),
            false,
        ),
        (
            "iphone.timer.cancel",
            json!({"device_id": DEVICE, "id": ID}),
            false,
        ),
        ("iphone.capture.audio", target.clone(), false),
        ("iphone.capture.photo", target.clone(), false),
        ("iphone.device.status", target.clone(), false),
        (
            "iphone.geofence.add",
            json!({"device_id": DEVICE, "name": "H", "latitude": 1.5, "longitude": 2.5, "radius": 100}),
            false,
        ),
        ("iphone.geofence.list", target.clone(), false),
        (
            "iphone.geofence.remove",
            json!({"device_id": DEVICE, "name": "H"}),
            false,
        ),
        (
            "iphone.health.log",
            json!({"device_id": DEVICE, "kind": "water", "value": 1, "start": "2026-01-01T00:00:00Z", "tag": "t"}),
            false,
        ),
        ("iphone.live.start", live.clone(), false),
        ("iphone.live.update", live.clone(), false),
        ("iphone.live.end", live, false),
        ("iphone.location.current", target.clone(), false),
        (
            "iphone.location.significant",
            json!({"device_id": DEVICE, "enabled": true}),
            false,
        ),
        ("iphone.motion.activity", target.clone(), false),
        (
            "iphone.motion.history",
            json!({"device_id": DEVICE, "days": 3}),
            false,
        ),
        (
            "iphone.notify",
            json!({"device_id": DEVICE, "title": "T", "body": ""}),
            false,
        ),
        ("iphone.ping", target.clone(), false),
        ("iphone.screentime.cancel", target.clone(), false),
        (
            "iphone.screentime.schedule",
            json!({"device_id": DEVICE, "start_minute": 60, "end_minute": 120}),
            false,
        ),
        ("iphone.screentime.shield", target.clone(), false),
        ("iphone.screentime.unshield", target, false),
    ]
}

#[test]
fn no_command_publishes_a_required_argument_it_does_not_list() -> Result {
    for (name, input) in published()? {
        let args = &input["properties"]["args"];
        let Some(required) = args["required"].as_array() else {
            continue;
        };
        for field in required {
            let field = field.as_str().ok_or("field")?;
            assert!(
                !args["properties"][field].is_null(),
                "{name} requires {field} but does not publish it"
            );
        }
    }
    Ok(())
}

#[test]
fn exactly_what_is_published_as_required_is_enough_for_the_gateway() -> Result {
    let inputs = published()?;
    for (op, args, versioned) in examples() {
        let input = inputs.get(op).ok_or(op)?;
        let schema = &input["properties"]["args"];
        let required: Vec<&str> = schema["required"]
            .as_array()
            .ok_or(op)?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        for key in args.as_object().ok_or(op)?.keys() {
            assert!(
                !schema["properties"][key].is_null(),
                "{op}: {key} unpublished"
            );
        }
        if op != "contacts.create" && op != "contacts.update" {
            let mut sent: Vec<&str> = args
                .as_object()
                .ok_or(op)?
                .keys()
                .map(String::as_str)
                .collect();
            sent.sort_unstable();
            let mut wanted = required.clone();
            wanted.sort_unstable();
            assert_eq!(sent, wanted, "{op}");
        }
        let version_required = input["required"]
            .as_array()
            .ok_or(op)?
            .iter()
            .any(|field| field == "expected_version");
        assert_eq!(version_required, versioned, "{op} expected_version");
        assert!(
            accepted(op, &args, u64::from(versioned)),
            "{op}: published arguments refused"
        );
    }
    Ok(())
}

#[test]
fn a_versioned_command_is_refused_without_its_version() {
    for (op, args, versioned) in examples() {
        if versioned {
            assert!(!accepted(op, &args, 0), "{op} accepted without a version");
        }
    }
}

#[test]
fn every_published_contact_field_is_accepted() -> Result {
    let inputs = published()?;
    let schema =
        &inputs.get("contacts.update").ok_or("update")?["properties"]["args"]["properties"];
    let date = json!({"year": 1990, "month": 2, "day": 3, "is_leap_month": false, "calendar": "gregorian"});
    let mut args = json!({"id": "c"});
    for (field, shape) in schema.as_object().ok_or("properties")? {
        if field == "id" {
            continue;
        }
        args[field] = match field.as_str() {
            "image" => json!({"base64": "AA=="}),
            "birthday" | "non_gregorian_birthday" => date.clone(),
            "dates" => json!([{"label": "x", "value": date}]),
            "postal_addresses" | "social_profiles" | "instant_messages" => {
                json!([{"label": "x", "value": {"a": "b"}}])
            }
            _ if shape["type"] == "array" => json!([{"label": "x", "value": "y"}]),
            _ => json!("v"),
        };
    }
    assert!(accepted("contacts.update", &args, 1), "{args}");
    Ok(())
}

#[test]
fn every_optional_iphone_field_published_is_accepted() {
    let cases = [
        (
            "iphone.alarm.create",
            json!({"device_id": DEVICE, "id": ID, "fire_at": 1_900_000_000, "snooze_seconds": 60, "title": "T"}),
        ),
        (
            "iphone.timer.start",
            json!({"device_id": DEVICE, "id": ID, "duration_seconds": 60, "title": "T"}),
        ),
        (
            "iphone.capture.audio",
            json!({"device_id": DEVICE, "seconds": 5}),
        ),
        (
            "iphone.notify",
            json!({"device_id": DEVICE, "title": "T", "body": "B", "thread": "x", "time_sensitive": true, "buttons": [{"id": "a", "title": "A"}]}),
        ),
        (
            "iphone.health.log",
            json!({"device_id": DEVICE, "kind": "workout", "value": 1, "start": "2026-01-01T00:00:00Z", "tag": "t", "activity": "running"}),
        ),
    ];
    for (op, args) in cases {
        assert!(accepted(op, &args, 0), "{op}: {args}");
    }
}
