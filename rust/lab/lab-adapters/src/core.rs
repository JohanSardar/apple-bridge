use crate::{assertions::await_until, replicas};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Map, Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

pub const IGNORE_REPLICAS: [&str; 1] = ["CORE-CREDENTIAL-STORE"];
pub const ACK_RECORD: &str = "ack-durable-record";

pub fn external_change(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    world.control(
        "/external-change",
        &json!({"key": ACK_RECORD, "collection": "reminders.reminder", "data": ack_data()}),
    )?;
    world.control("/flush", &json!({}))?;
    let acked = await_until(15, || {
        world
            .control("/protocol", &Value::Null)
            .is_ok_and(|p| p["commits"].as_array().is_some_and(|c| !c.is_empty()))
    });
    if !acked {
        evidence.check(
            "peer_ack_observed",
            false,
            "peer never saw Committed for the change",
        );
    }
    Ok(())
}
fn ack_data() -> Value {
    json!({"title": "[ab-test] acked data must be durable", "completed": false})
}
pub fn acked_data_visible(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if evidence
        .assertions
        .iter()
        .any(|a| a.name == "peer_ack_observed")
    {
        return Ok(());
    }
    let page = world.query(
        "records.query",
        &json!({"collection": "reminders.reminder", "limit": 1000}),
    )?;
    let want = ack_data();
    let visible = page["records"].as_array().is_some_and(|records| {
        records
            .iter()
            .any(|r| r["deleted"] != true && r["data"] == want)
    });
    evidence.check(
        "acked_data_visible",
        visible,
        &format!("peer_acked=true visible_through_public_query={visible}"),
    );
    Ok(())
}

fn admin_query(world: &World, token: &str, collection: &str) -> Result<Vec<Value>> {
    let page = world.query_with_token(
        "records.query",
        &json!({"collection": collection, "limit": 20}),
        token,
    )?;
    Ok(page["records"].as_array().cloned().unwrap_or_default())
}
fn alias_put(
    world: &World,
    sequence: &mut u32,
    steps: (&str, &str, &str),
    key: &str,
    text: &str,
) -> Result<()> {
    *sequence += 1;
    let at = format!("2031-01-01T00:00:{:02}Z", *sequence);
    let reference =
        |collection: &str| json!({"$ref": {"collection": collection, "native_key": key}});
    let observation = |collection: &str, data: Value| {
        json!({"kind": "upsert", "collection": collection, "native_key": key,
            "scoped_alias": format!("readmodel/{collection}"), "data": data, "observed_at": at})
    };
    let records = [
        observation(
            "messages.message",
            json!({"chat_id": reference("messages.chat"), "sender_handle_id": reference("messages.handle"),
                "kind": "message", "text": text, "sent_at": at, "is_from_me": false}),
        ),
        observation(
            "messages.chat",
            json!({"display_name": "[ab-test] alias chat", "participants": []}),
        ),
        observation(
            "messages.handle",
            json!({"address": "probe@example.com", "service": "iMessage"}),
        ),
    ];
    world.control(
        "/capture-exact",
        &json!({"source": "messages", "generation": steps.0, "predecessor": steps.1,
            "checkpoint": steps.2, "observations": records}),
    )?;
    Ok(())
}
fn alias_contract(world: &World) -> Result<bool> {
    let token = world
        .gateway_cli(&["token", "create", "--scope", "admin"])?
        .trim()
        .to_owned();
    let read = |collection: &str| admin_query(world, &token, collection);
    let mut sequence = 0;
    alias_put(
        world,
        &mut sequence,
        ("first", "", "one"),
        "same",
        "[ab-test] original",
    )?;
    let (messages, chats, handles) = (
        read("messages.message")?,
        read("messages.chat")?,
        read("messages.handle")?,
    );
    if messages.len() != 1 || chats.len() != 1 || handles.len() != 1 {
        return Ok(false);
    }
    let (message_id, chat_id, handle_id) = (&messages[0]["id"], &chats[0]["id"], &handles[0]["id"]);
    if message_id == chat_id || chat_id == handle_id || message_id == handle_id {
        return Ok(false);
    }
    if messages[0]["data"]["chat_id"] != *chat_id
        || messages[0]["data"]["sender_handle_id"] != *handle_id
    {
        return Ok(false);
    }
    alias_put(
        world,
        &mut sequence,
        ("second", "", "two"),
        "renamed",
        "[ab-test] resumed",
    )?;
    let messages = read("messages.message")?;
    if messages.len() != 1 || messages[0]["id"] != *message_id || messages[0]["version"] != 2 {
        return Ok(false);
    }
    if messages[0]["data"]["chat_id"] != *chat_id
        || messages[0]["data"]["sender_handle_id"] != *handle_id
    {
        return Ok(false);
    }
    let at = "2031-01-01T00:00:03Z";
    let reference = |key: &str, collection: &str| json!({"$ref": {"collection": collection, "native_key": key}});
    let independent = json!({"kind": "upsert", "collection": "messages.message", "native_key": "same",
        "data": {"chat_id": reference("renamed", "messages.chat"),
            "sender_handle_id": reference("renamed", "messages.handle"),
            "kind": "message", "text": "[ab-test] independent", "sent_at": at, "is_from_me": false},
        "observed_at": at});
    world.control(
        "/capture-exact",
        &json!({"source": "messages", "generation": "second", "predecessor": "two",
            "checkpoint": "three", "observations": [independent]}),
    )?;
    let messages = read("messages.message")?;
    if messages.len() != 2 {
        return Ok(false);
    }
    let distinct = messages
        .iter()
        .any(|r| r["data"]["text"] == "[ab-test] independent" && r["id"] != *message_id);
    if !distinct {
        return Ok(false);
    }
    let alias_at = "2031-01-01T00:00:04Z";
    let alias = json!({"kind": "alias", "collection": "messages.message", "native_key": "alias-only",
        "scoped_alias": "readmodel/messages.message", "observed_at": alias_at});
    let mut fields = messages[0]["data"]
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    fields.insert("chat_id".into(), reference("renamed", "messages.chat"));
    fields.insert(
        "sender_handle_id".into(),
        reference("renamed", "messages.handle"),
    );
    fields.insert("text".into(), json!("[ab-test] alias update"));
    let updated = json!({"kind": "upsert", "collection": "messages.message", "native_key": "alias-only",
        "scoped_alias": "readmodel/messages.message", "data": fields, "observed_at": alias_at});
    world.control(
        "/capture-exact",
        &json!({"source": "messages", "generation": "second", "predecessor": "three",
            "checkpoint": "four", "observations": [alias, updated]}),
    )?;
    let messages = read("messages.message")?;
    if messages.len() != 2 {
        return Ok(false);
    }
    Ok(messages.iter().any(|r| {
        r["data"]["text"] == "[ab-test] alias update" && r["id"] == *message_id && r["version"] == 3
    }))
}
pub fn capture_aliases(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let passed = alias_contract(world).unwrap_or(false);
    evidence.check(
        "capture_alias_scopes_generations_and_forward_references",
        passed,
        "alias scopes, generations and forward references",
    );
    crate::labcore::oversized_resets(world, evidence);
    Ok(())
}

fn credential_store(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let secrets = world.gateway_home.join("secrets");
    let private = fs::read_dir(&secrets).is_ok_and(|entries| {
        let mut any = false;
        let mut all = true;
        for entry in entries.flatten() {
            any = true;
            all &= entry
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o777 == 0o600);
        }
        any && all
    });
    evidence.check(
        "explicit_private_world_credentials",
        private,
        "world credentials are private files",
    );
    if cfg!(target_os = "macos") {
        locked_keychain(world, evidence)?;
    }
    Ok(())
}
fn locked_keychain(world: &World, evidence: &mut Evidence) -> Result<()> {
    let root = world.directory.join("locked-store");
    let mock = root.join("bin");
    fs::create_dir_all(&mock)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let script = mock.join("python3");
    fs::write(&script, "#!/bin/sh\nprintf '%s\\n' '{\"status\":-25308}'\n")?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    let home = root.join("home");
    let path = format!(
        "{}:{}",
        mock.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let run = |args: &[&str]| -> Result<std::process::Output> {
        Ok(Command::new(replicas::binary()?)
            .args(args)
            .env_remove("APPLE_BRIDGE_SECRET_STORE")
            .env("PATH", &path)
            .output()?)
    };
    let home_arg = home.to_string_lossy().into_owned();
    let started = run(&[
        "start",
        "--home",
        &home_arg,
        "--role",
        "gateway",
        "--api-addr",
        "127.0.0.1:0",
        "--peer-addr",
        "127.0.0.1:0",
    ])?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&started.stdout),
        String::from_utf8_lossy(&started.stderr)
    );
    evidence.check(
        "locked_keychain_startup_typed_error_and_remedy",
        !started.status.success()
            && text.contains("credential_store_locked")
            && text.contains("keychain")
            && text.contains("Unlock"),
        "locked keychain startup names the typed error and remedy",
    );
    let doctor = run(&["doctor", "--home", &home_arg])?;
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap_or(Value::Null);
    let last = &report["credentials"]["last_error"];
    evidence.check(
        "doctor_reports_locked_keychain_and_remedy",
        doctor.status.success()
            && report["ok"] == false
            && report["credentials"]["store"] == "keychain"
            && last["code"] == "credential_store_locked"
            && last["remedy"]
                .as_str()
                .is_some_and(|r| r.contains("Unlock")),
        "doctor reports the locked keychain and remedy",
    );
    Ok(())
}

pub fn robustness(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    match input["case"].as_str().unwrap_or_default() {
        "credential-store" => credential_store(world, evidence),
        other => Err(Error::Invalid(format!(
            "robustness case not ported: {other}"
        ))),
    }
}
