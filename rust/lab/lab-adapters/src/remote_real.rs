use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Stdio},
    time::Duration,
};

const SERVER: &str = "apple-bridge-tailnet-test";

fn status(world: &World) -> Value {
    fs::read(world.gateway_home.join("remote/status.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or(Value::Null)
}

fn enable_tailnet(world: &World, evidence: &mut Evidence) -> Option<String> {
    evidence.check("isolated_mac_gateway", true, "");
    let enabled = world.gateway_cli(&["tailscale"]);
    evidence.check(
        "tailnet_route_enabled",
        enabled.is_ok(),
        &enabled
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    enabled.ok()?;
    let status = status(world);
    let route = &status["routes"]["tailnet"];
    let url = route["url"].as_str().unwrap_or_default().to_owned();
    let on = route["state"] == "on" && url.starts_with("https://");
    evidence.check("tailnet_route_on", on, "");
    on.then_some(url)
}

fn remote_token(world: &World, evidence: &mut Evidence) -> Option<String> {
    let token = world.gateway_cli(&["token", "create", "--remote", "--scope", "admin"]);
    evidence.check("remote_token", token.is_ok(), "");
    token.ok()
}

fn mcp_config(world: &World, url: &str, token: &str) -> serde_json::Result<()> {
    let server = json!({"type": "http", "url": format!("{url}/mcp"), "headers": {"Authorization": format!("Bearer {token}")}});
    let config = json!({"mcpServers": {SERVER: server}});
    let text = serde_json::to_string(&config)?;
    let _ = fs::write(world.directory.join(".mcp.json"), &text);
    Ok(())
}

fn finish(world: &World) {
    let _ = world.gateway_cli(&["tailscale", "--off"]);
}

pub fn claude_code(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let Some(url) = enable_tailnet(world, evidence) else {
        finish(world);
        return Ok(());
    };
    if let Some(token) = remote_token(world, evidence) {
        let configured = mcp_config(world, &url, token.trim()).is_ok();
        evidence.check("isolated_claude_configuration", configured, "");
        if configured {
            let output = Command::new("claude")
                .args([
                    "-p",
                    &format!(
                        "Call mcp__{SERVER}__records_query exactly once with limit 1. Do not use any other tool. Report only success or failure."
                    ),
                    "--mcp-config",
                ])
                .arg(world.directory.join(".mcp.json"))
                .args([
                    "--strict-mcp-config",
                    "--allowedTools",
                    &format!("mcp__{SERVER}__records_query"),
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--max-turns",
                    "3",
                ])
                .current_dir(&world.directory)
                .stdin(Stdio::null())
                .output();
            let text = output
                .as_ref()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            evidence.check(
                "claude_code_real_tool_call",
                output.is_ok_and(|o| o.status.success())
                    && text.contains(&format!("\"name\":\"mcp__{SERVER}__records_query\""))
                    && text.contains("\"type\":\"tool_result\"")
                    && !text.contains("\"is_error\":true"),
                "",
            );
        }
    }
    finish(world);
    Ok(())
}

pub fn status_off(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("isolated_gateway", true, "");
    let off = world.gateway_cli(&["tailscale", "--off"]);
    evidence.check("off_idempotent", off.is_ok(), "");
    std::thread::sleep(Duration::from_millis(100));
    evidence.check("off_status", status(world)["state"] == "off", "");
    Ok(())
}
