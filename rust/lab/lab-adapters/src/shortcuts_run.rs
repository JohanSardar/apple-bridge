use crate::{
    http::{Client, random_id},
    queries::agent_token,
    shortcuts::{
        SHORTCUTS_NATIVE_INPUTS, await_until, checks_binary, native_directory, native_target,
        run_checks, run_native, scratch_directory,
    },
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    process::Command,
    time::{Duration, Instant},
};

const ECHO_DIGEST: &str = "b3cc0475bb78a5026098858e9889acf666d31062d513d303314eca31d36e72f2";
const SCOPE: &str = "calendar:read,reminders:read,shortcuts:read,shortcuts:write,system:read";

fn rejected(client: &Client, args: &Value) -> bool {
    let Ok(id) = random_id("cmd") else {
        return false;
    };
    client
        .call_raw(
            "shortcuts.run",
            &json!({"command_id": id, "args": args}),
            20,
        )
        .ok()
        .flatten()
        .is_none_or(|(status, value)| {
            status != 200 || !value["error"].is_null() || value["state"] == "rejected"
        })
}

fn registered(world: &World) -> bool {
    let Ok(client) = Client::new(world, "calendar:read,system:read") else {
        return false;
    };
    let document = client
        .request("GET", "/v1/openapi.json", &[], None, 30)
        .ok()
        .and_then(|reply| serde_json::from_slice::<Value>(&reply.body).ok());
    let listed = document.is_some_and(|d| d["paths"]["/v1/op/shortcuts.run"].is_object());
    let gated = client
        .call_raw(
            "shortcuts.run",
            &json!({"command_id": random_id("cmd").unwrap_or_default(), "args": {"shortcut": "AB Echo"}}),
            20,
        )
        .ok()
        .flatten()
        .is_some_and(|(status, _)| status == 403);
    listed && gated
}

fn gateway_runner(world: &World, evidence: &mut Evidence) -> Result<()> {
    evidence.check(
        "public_command_registered",
        registered(world),
        "shortcuts.run is a command gated by shortcuts:write",
    );
    let client = Client::new(world, SCOPE)?;
    evidence.check("public_gateway_start", true, "gateway ready");
    evidence.check(
        "public_helper_ready",
        await_until(5, || {
            client
                .call("helper.version", &json!({"args":{}}), 5)
                .ok()
                .flatten()
                .is_some()
        }),
        "helper answered",
    );
    let args = json!({"shortcut":"AB Echo","input_base64":"c3ludGhldGlj","timeout_ms":1000});
    let record = client
        .call(
            "shortcuts.run",
            &json!({"command_id": random_id("cmd")?, "args": args, "wait_ms": 15000}),
            40,
        )
        .ok()
        .flatten();
    evidence.check(
        "public_command_validated",
        record.is_some(),
        "valid arguments accepted",
    );
    for invalid in [
        json!({"shortcut":"AB Echo","input_base64":"!"}),
        json!({"shortcut":"AB Echo","input":"text","input_base64":"YQ=="}),
        json!({"shortcut":"AB Echo","input_base64":"YWFh".repeat(6000)}),
    ] {
        evidence.check(
            "invalid_blob_rejected",
            rejected(&client, &invalid),
            "invalid blob arguments rejected",
        );
    }
    let observed = record.as_ref().is_some_and(|r| r["state"] == "observed");
    evidence.check("public_runner_completion", observed, "runner observed");
    let output = record.as_ref().map_or(&Value::Null, |r| &r["evidence"]);
    evidence.check(
        "public_runner_output",
        output["output_base64"] == "c3ludGhldGlj",
        "echoed output",
    );
    evidence.check(
        "public_runner_digest",
        output["output_bytes"] == 9 && output["output_sha256"] == ECHO_DIGEST,
        "output digest",
    );
    let slow_id = random_id("cmd")?;
    let started = Instant::now();
    let admitted = client
        .call(
            "shortcuts.run",
            &json!({"command_id": slow_id, "wait_ms": 15000, "args": {"shortcut":"[ab-test slow runner]","input":"synthetic","timeout_ms":20000}}),
            60,
        )
        .ok()
        .flatten()
        .is_some();
    evidence.check("long_runner_admitted", admitted, "long runner admitted");
    let finished = await_until(10, || {
        client
            .call("commands.status", &json!({"command_id": slow_id}), 20)
            .ok()
            .flatten()
            .is_some_and(|v| v["state"] == "observed" || v["command"]["state"] == "observed")
    });
    evidence.check(
        "gateway_waits_beyond_15_seconds",
        finished && started.elapsed() > Duration::from_secs(15),
        "gateway waited for the long runner",
    );
    Ok(())
}

pub fn shortcuts_run(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let checks = checks_binary()?;
    let directory = scratch_directory(&id);
    let directory_text = directory.to_string_lossy().into_owned();
    let root_text = env::current_dir()?.to_string_lossy().into_owned();
    let prepared = Command::new(&checks)
        .args(["prepare", &directory_text])
        .output()
        .is_ok_and(|o| o.status.success());
    evidence.check("fixture_directory", prepared, "temporary directory");
    evidence.check("fake_cli", prepared, "fake shortcuts executable");
    if !prepared {
        return Ok(());
    }
    let binary = native_target(
        evidence,
        "ShortcutsChecks",
        &SHORTCUTS_NATIVE_INPUTS,
        ("native_shortcuts_build", "native_shortcuts_output"),
    );
    let Some(binary) = binary else {
        let _ = fs::remove_dir_all(&directory);
        return Ok(());
    };
    let executable = directory.join("shortcuts").to_string_lossy().into_owned();
    let (ok, _) = run_native(&binary, &[&executable, &id]);
    evidence.check("native_source_fixture", ok, "native source fixture");
    gateway_runner(world, evidence)?;
    let report = run_checks(&checks, &["run", &id, &directory_text, &root_text], evidence);
    let _ = fs::remove_dir_all(&directory);
    report_into(evidence, report.as_ref());
    Ok(())
}

fn report_into(evidence: &mut Evidence, report: Option<&Value>) {
    let Some(report) = report else { return };
    for item in report["assertions"].as_array().into_iter().flatten() {
        evidence.check(
            item["name"].as_str().unwrap_or("unnamed"),
            item["passed"] == true,
            "",
        );
    }
    if let Some(reason) = report["limited"].as_str() {
        evidence.check("not_limited", false, reason);
    }
}

fn focus_agents(world: &World, evidence: &mut Evidence) -> Result<()> {
    let command = |args: Value| -> Result<Value> {
        Ok(json!({"command_id": random_id("cmd")?, "args": args}))
    };
    let client = Client::new(world, "reminders:read,focus:read,focus:write,system:read")?;
    let invalid = client
        .call_raw("focus.set", &command(json!({"mode":"Do Not Disturb"}))?, 20)?
        .is_none_or(|(status, value)| status != 200 || !value["error"].is_null());
    evidence.check("explicit_state_required", invalid, "state is required");
    evidence.check("gateway", true, "gateway ready");
    let listed = world.gateway_cli(&["agent", "list"]).is_ok();
    evidence.check("authority", listed, "agent list succeeds");
    if !listed {
        return Ok(());
    }
    let mut contract = false;
    for level in ["full", "approve"] {
        let token = agent_token(world, "Bridge Test Focus", level, "focus:write")?;
        evidence.check("agent", token.is_some(), "agent created");
        let Some(token) = token else { continue };
        let reply = Client::with_token(world, &token)?.call_raw(
            "focus.set",
            &command(json!({"mode":"Do Not Disturb","on":true}))?,
            20,
        )?;
        let waiting = reply.is_some_and(|(status, value)| {
            status == 200 && value["error"].is_null() && value["state"] == "awaiting_approval"
        });
        contract |= waiting;
        evidence.check("agent_requires_approval", waiting, "agent waits for approval");
    }
    evidence.check("set_contract", contract, "valid focus.set accepted");
    Ok(())
}

pub fn focus_sim(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let checks = checks_binary()?;
    let directory = scratch_directory(&id);
    fs::create_dir_all(&directory)?;
    let directory_text = directory.to_string_lossy().into_owned();
    let root_text = env::current_dir()?.to_string_lossy().into_owned();
    focus_agents(world, evidence)?;
    let report = run_checks(&checks, &["run", &id, &directory_text, &root_text], evidence);
    let _ = fs::remove_dir_all(&directory);
    report_into(evidence, report.as_ref());
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", "FocusChecks")
        .output();
    let ok = built.as_ref().is_ok_and(|o| o.status.success());
    evidence.check("native_build", ok, "native focus checks build");
    if !ok {
        return Ok(());
    }
    let Some(binary) = native_directory().map(|d| d.join("FocusChecks")) else {
        evidence.check("native_output", false, "native output directory");
        return Ok(());
    };
    evidence.check("native_output", true, "native output directory");
    let (passed, output) = run_native(&binary, &["sim"]);
    if let Some(reason) = output.strip_prefix("limited:") {
        evidence.check("not_limited", false, reason.trim());
        return Ok(());
    }
    evidence.check(
        "native_set_readback_restore",
        passed && output.trim() == "passed",
        "native set, readback and restore",
    );
    Ok(())
}
