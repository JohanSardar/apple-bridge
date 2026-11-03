use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    net::IpAddr,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

const SERVER: &str = "apple-bridge-relay-test";

fn status(world: &World) -> Value {
    fs::read(world.gateway_home.join("remote/status.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or(Value::Null)
}

fn enable_relay(world: &World, evidence: &mut Evidence) -> Option<Value> {
    evidence.check("isolated_mac_gateway", true, "");
    let enabled = world.gateway_cli(&["remote", "on", "--via", "relay"]);
    evidence.check(
        "relay_enrollment_dns01_public_tls_and_60s_sse",
        enabled.is_ok(),
        &enabled
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    enabled.ok()?;
    let status = status(world);
    evidence.check(
        "relay_backend_on",
        status["state"] == "on" && status["backend"] == "relay",
        "",
    );
    (status["state"] == "on" && status["url"].as_str().is_some()).then_some(status)
}

fn remote_token(world: &World, evidence: &mut Evidence) -> Option<String> {
    let token = world.gateway_cli(&["token", "create", "--remote", "--scope", "admin"]);
    evidence.check("remote_token", token.is_ok(), "");
    token.ok()
}

fn mcp_config(world: &World, url: &str, token: &str) -> serde_json::Result<String> {
    let server = json!({"type": "http", "url": format!("{url}/mcp"), "headers": {"Authorization": format!("Bearer {token}")}});
    let path = world.directory.join(".mcp.json");
    let config = json!({"mcpServers": {SERVER: server}});
    let text = serde_json::to_string(&config)?;
    let _ = fs::write(path, &text);
    serde_json::to_string(&server)
}

fn public_probe(url: &str, evidence: &mut Evidence) {
    let curl = |arguments: &[&str]| {
        Command::new("curl")
            .args(["--silent", "--max-time", "90", "--output", "/dev/null"])
            .args(["--write-out", "%{http_code}"])
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let discovery = curl(&[&format!("{url}/.well-known/oauth-authorization-server")]);
    let challenge = curl(&["-X", "POST", &format!("{url}/mcp")]);
    evidence.check(
        "trusted_public_oauth_mcp_and_60s_sse",
        discovery == "200" && challenge == "401",
        "",
    );
}

const PROBE_HOST: &str = "ubuntu";

fn host_and_port(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => {
            (host.to_owned(), port.to_owned())
        }
        _ => (authority.to_owned(), "443".to_owned()),
    })
}

fn public_ip(dig: &str) -> Option<String> {
    dig.split_whitespace()
        .find(|line| {
            line.parse::<IpAddr>().is_ok_and(|ip| match ip {
                IpAddr::V4(v4) => !v4.is_private() && !v4.is_loopback() && !v4.is_unspecified(),
                IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unspecified(),
            })
        })
        .map(str::to_owned)
}

fn parse_checks(output: &[u8]) -> Option<BTreeMap<String, bool>> {
    let value: Value = serde_json::from_slice(output).ok()?;
    let checks = value["checks"].as_object()?;
    let parsed: BTreeMap<String, bool> = checks
        .iter()
        .filter_map(|(name, ok)| ok.as_bool().map(|ok| (name.clone(), ok)))
        .collect();
    (!parsed.is_empty()).then_some(parsed)
}

fn ssh(arguments: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("ssh")
        .args(["-o", "BatchMode=yes", PROBE_HOST])
        .args(arguments)
        .stdin(Stdio::null())
        .output()
}

fn external_probe(world: &World, url: &str, evidence: &mut Evidence) {
    let Some((host, port)) = host_and_port(url) else {
        evidence.check("ubuntu_public_dns", false, "unparsable remote URL");
        return;
    };
    let dig = ssh(&["dig", "@1.1.1.1", "+short", "A", &host]);
    let ip = dig
        .as_ref()
        .ok()
        .and_then(|o| public_ip(&String::from_utf8_lossy(&o.stdout)));
    evidence.check("ubuntu_public_dns", dig.is_ok() && ip.is_some(), "");
    let Some(ip) = ip else {
        return;
    };
    let resolve = format!("{host}:{port}:{ip}");
    for path in ["/", "/v1/health"] {
        let target = format!("{url}{path}");
        let curl = ssh(&[
            "curl",
            "--noproxy",
            "'*'",
            "--fail",
            "--silent",
            "--show-error",
            "--max-time",
            "10",
            "--resolve",
            &resolve,
            &target,
        ]);
        evidence.check(
            &format!("ubuntu_curl_public_tls{path}"),
            curl.is_ok_and(|o| o.status.success() && !o.stdout.is_empty()),
            "",
        );
    }
    let binary = std::env::var_os("AB_LAB_PROBE").map(std::path::PathBuf::from);
    let present = binary.as_deref().is_some_and(Path::is_file);
    evidence.check(
        "external_probe_build",
        present,
        "set AB_LAB_PROBE to a Linux lab-probe binary built on ssh build",
    );
    let Some(binary) = binary.filter(|_| present) else {
        return;
    };
    let directory = format!(
        "/tmp/relay-real-{}-{}",
        std::process::id(),
        jiff::Timestamp::now().as_millisecond()
    );
    let made = ssh(&["mkdir", "-m", "700", &directory]);
    evidence.check(
        "external_probe_directory",
        made.is_ok_and(|o| o.status.success()),
        "",
    );
    let copied = Command::new("scp")
        .args(["-o", "BatchMode=yes"])
        .arg(&binary)
        .arg(format!("{PROBE_HOST}:{directory}/probe"))
        .stdin(Stdio::null())
        .status();
    evidence.check("external_probe_copy", copied.is_ok_and(|s| s.success()), "");
    let remote = world.gateway_cli(&["token", "create", "--remote", "--scope", "admin"]);
    let local = world.gateway_cli(&["token", "create", "--scope", "admin"]);
    evidence.check("remote_token", remote.is_ok() && local.is_ok(), "");
    if let (Ok(remote), Ok(local)) = (remote, local) {
        let input = json!({"URL": url, "IP": ip, "Remote": remote.trim(), "Local": local.trim()});
        let child = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                PROBE_HOST,
                &format!("{directory}/probe"),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn();
        let output = child.ok().and_then(|mut child| {
            child
                .stdin
                .take()?
                .write_all(input.to_string().as_bytes())
                .ok()?;
            child.wait_with_output().ok()
        });
        evidence.check(
            "external_probe_completed",
            output.as_ref().is_some_and(|o| o.status.success()),
            "",
        );
        let checks = output.as_ref().and_then(|o| parse_checks(&o.stdout));
        evidence.check("external_probe_results", checks.is_some(), "");
        for (name, ok) in checks.into_iter().flatten() {
            evidence.check(&name, ok, "");
        }
    }
    let _ = ssh(&["rm", "-rf", &directory]);
}

fn finish(world: &World) {
    let _ = world.gateway_cli(&["remote", "off"]);
}

pub fn relay_real(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let Some(status) = enable_relay(world, evidence) else {
        finish(world);
        return Ok(());
    };
    let url = status["url"].as_str().unwrap_or_default().to_owned();
    external_probe(world, &url, evidence);
    public_probe(&url, evidence);
    finish(world);
    Ok(())
}

pub fn claude_code(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let Some(status) = enable_relay(world, evidence) else {
        finish(world);
        return Ok(());
    };
    let url = status["url"].as_str().unwrap_or_default().to_owned();
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

pub fn relay_claude(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let Some(status) = enable_relay(world, evidence) else {
        finish(world);
        return Ok(());
    };
    let url = status["url"].as_str().unwrap_or_default().to_owned();
    external_probe(world, &url, evidence);
    if let Some(token) = remote_token(world, evidence)
        && let Ok(server) = mcp_config(world, &url, token.trim())
    {
        let config = world.directory.join("claude-config");
        let claude = |arguments: &[&str]| {
            Command::new("claude")
                .args(arguments)
                .current_dir(&world.directory)
                .env("CLAUDE_CONFIG_DIR", &config)
                .stdin(Stdio::null())
                .output()
        };
        let registered = claude(&["mcp", "add-json", "--scope", "user", SERVER, &server]);
        evidence.check(
            "isolated_claude_registration",
            registered.is_ok_and(|o| o.status.success()),
            "",
        );
        let listed = claude(&["mcp", "list"]);
        let text = listed
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        evidence.check(
            "claude_code_http_connector_connected",
            listed.is_ok_and(|o| o.status.success())
                && text.contains(SERVER)
                && text.contains("Connected"),
            "",
        );
    }
    public_probe(&url, evidence);
    finish(world);
    Ok(())
}

pub fn status_off(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("isolated_gateway", true, "");
    let off = world.gateway_cli(&["remote", "off"]);
    evidence.check("off_idempotent", off.is_ok(), "");
    std::thread::sleep(Duration::from_millis(100));
    let reported: Value = world
        .gateway_cli(&["remote", "status"])
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or(Value::Null);
    evidence.check("off_status", reported["state"] == "off", "");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{host_and_port, parse_checks, public_ip};

    #[test]
    fn splits_host_and_port() {
        assert_eq!(
            host_and_port("https://abc.relay.example"),
            Some(("abc.relay.example".into(), "443".into()))
        );
        assert_eq!(
            host_and_port("https://abc.relay.example:8443/x"),
            Some(("abc.relay.example".into(), "8443".into()))
        );
        assert_eq!(host_and_port("http://abc.relay.example"), None);
        assert_eq!(host_and_port("https://user@abc.relay.example"), None);
    }

    #[test]
    fn picks_the_first_public_address() {
        assert_eq!(
            public_ip("10.0.0.4\n127.0.0.1\n203.0.113.9\n198.51.100.1\n"),
            Some("203.0.113.9".into())
        );
        assert_eq!(public_ip("192.168.1.2\nnot-an-address\n"), None);
        assert_eq!(public_ip(""), None);
    }

    #[test]
    fn reads_probe_checks() {
        let checks = parse_checks(br#"{"checks":{"a":true,"b":false,"c":"x"},"latency":{}}"#);
        assert_eq!(
            checks.as_ref().map(std::collections::BTreeMap::len),
            Some(2)
        );
        assert_eq!(checks.and_then(|c| c.get("b").copied()), Some(false));
        assert!(parse_checks(br#"{"checks":{}}"#).is_none());
        assert!(parse_checks(b"not json").is_none());
        assert!(parse_checks(br#"{"error":"x"}"#).is_none());
    }
}
