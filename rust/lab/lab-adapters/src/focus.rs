use crate::{
    checks::{checks_binary, report_into, run_checks, scratch_directory},
    http::{Client, random_id},
    queries::agent_token,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{env, fs};

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
        let waiting = reply.as_ref().is_some_and(|(status, value)| {
            *status == 200 && value["error"].is_null() && value["state"] == "awaiting_approval"
        });
        if level == "approve" {
            contract |= waiting;
            evidence.check(
                "ask_first_agent_requires_approval",
                waiting,
                "ask-first agent waits for approval",
            );
        } else {
            let seen = reply.map_or_else(String::new, |(status, value)| {
                format!("{status} {} {}", value["state"], value["error"])
            });
            evidence.check(
                "full_agent_runs_at_once",
                !waiting && !seen.starts_with("401") && !seen.starts_with("403"),
                &format!("full agent does not wait for approval: {seen}"),
            );
        }
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
    let report = run_checks(
        &checks,
        &["run", &id, &directory_text, &root_text],
        evidence,
    );
    let _ = fs::remove_dir_all(&directory);
    report_into(evidence, report.as_ref());
    crate::native_core::rust_tests(
        evidence,
        "native_set_readback_restore",
        "ab-session",
        &["--lib", "session::focus"],
    );
    Ok(())
}
