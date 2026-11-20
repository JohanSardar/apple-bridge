mod support;

use ab_frontdoor::web_connector::Connectors;
use serde_json::json;
use std::{io::Write, process::Stdio, sync::Arc, time::Duration};
use support::*;

#[tokio::test]
async fn review_s2_ssh_approval_keeps_the_complete_source_address() -> Check {
    let home = short_home()?;
    let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
    std::fs::create_dir_all(home.path().join("run"))?;
    let authority = Arc::new(ab_trust::Authority::open(home.path())?);
    let connectors = Connectors::new(authority);
    let (asked, waiting) = tokio::sync::oneshot::channel();
    let asked = std::sync::Mutex::new(Some(asked));
    connectors.on_waiting(Arc::new(move || {
        if let Some(asked) = asked.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = asked.send(());
        }
    }));
    let listener = ab_runtime::connector::bind(home.path())?;
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(ab_runtime::connector::serve(
        listener,
        connectors.clone(),
        stopped,
    ));
    let source = "fd7a:115c:a1e0:ab12:4843:cd96:6264:c938";
    let mut child = bridge()
        .args(["mcp", "--home"])
        .arg(home.path())
        .env("SSH_CONNECTION", format!("{source} 50000 100.101.1.10 22"))
        .env(
            "APPLE_BRIDGE_TAILSCALE_SOCKET",
            home.path().join("no-tailscale.sock"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    writeln!(
        child.stdin.as_mut().ok_or("stdin")?,
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-03-26","capabilities":{},
            "clientInfo":{"name":"Claude Code","version":"1"}
        }})
    )?;
    let arrived = tokio::time::timeout(Duration::from_secs(10), waiting).await;
    child.kill()?;
    child.wait()?;
    stop.send(true)?;
    server.await??;
    arrived??;
    let pending = connectors.pending()?;
    let label = &pending.first().ok_or("missing approval")?.client_name;
    assert!(
        label.ends_with(&format!(" over SSH from {source}")),
        "SSH approval lost its source: {label:?}"
    );
    Ok(())
}

#[tokio::test]
async fn grok_link_collects_a_private_key_only_after_the_mac_decision() -> Check {
    for (approve, abandon) in [(false, false), (true, false), (true, true)] {
        let home = short_home()?;
        let (_lock, _log) = scripted_gateway(home.path(), |_, _| ok(json!({})))?;
        std::fs::create_dir_all(home.path().join("run"))?;
        let authority = Arc::new(ab_trust::Authority::open(home.path())?);
        let connectors = Connectors::new(authority.clone());
        let (asked, waiting) = tokio::sync::oneshot::channel();
        let asked = std::sync::Mutex::new(Some(asked));
        connectors.on_waiting(Arc::new(move || {
            if let Some(asked) = asked.lock().ok().and_then(|mut slot| slot.take()) {
                let _ = asked.send(());
            }
        }));
        let listener = ab_runtime::connector::bind(home.path())?;
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let server = tokio::spawn(ab_runtime::connector::serve(
            listener,
            connectors.clone(),
            stopped,
        ));
        let route_home = home.path().to_owned();
        let remote = tokio::spawn(async move {
            loop {
                if let Ok(config) =
                    ab_remote::state::read::<ab_remote::state::Config>(&route_home, "config")
                {
                    ab_remote::state::write(
                        &route_home,
                        "status",
                        &json!({
                            "id": config.id, "state": "on", "url": "https://fixture.example.ts.net:8443",
                            "routes": {"tailnet": {"state": "on", "url": "https://fixture.example.ts.net:8443"}}
                        }),
                    )?;
                    return Ok::<(), ab_remote::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let terminal = Terminal::open()?;
        let mut command = bridge();
        command
            .args(["link", "grok-bot", "--json", "--home"])
            .arg(home.path())
            .env(
                "APPLE_BRIDGE_TAILSCALE_SOCKET",
                home.path().join("no-live-tailscale.sock"),
            )
            .stdout(Stdio::piped());
        terminal.attach(&mut command)?;
        let mut child = tokio::process::Command::from(command)
            .kill_on_drop(true)
            .spawn()?;
        tokio::time::timeout(Duration::from_secs(10), waiting).await??;
        assert!(child.try_wait()?.is_none());
        assert!(authority.agents()?.is_empty(), "no credential before Allow");
        let pending = connectors.pending()?;
        let request = pending.first().ok_or("missing request")?;
        assert_eq!(request.client_name, "Grok Bot");
        assert_eq!(request.source, "this_mac");
        assert_eq!(
            request.resource.as_deref(),
            Some("https://fixture.example.ts.net:8443/mcp")
        );
        assert!(
            connectors
                .agent_answer(&request.id, "wrong secret")
                .is_err()
        );
        if abandon {
            child.kill().await?;
        }
        connectors.decide(&request.id, "", approve)?;
        let output =
            tokio::time::timeout(Duration::from_secs(10), child.wait_with_output()).await??;
        let stderr = terminal.read()?;
        remote.await??;
        stop.send(true)?;
        server.await??;
        if abandon {
            assert!(
                authority.agents()?.is_empty(),
                "late Allow after CLI exit creates no credential"
            );
        } else if approve {
            assert!(output.status.success(), "{stderr}");
            check_grok_connection(&output, &authority)?;
        } else {
            assert!(!output.status.success());
            assert_eq!(output.stdout, [] as [u8; 0]);
            assert!(
                authority.agents()?.is_empty(),
                "denial creates no credential"
            );
            assert!(stderr.contains("permission_denied"), "{stderr}");
        }
        assert!(connectors.pending()?.is_empty());
    }
    Ok(())
}

fn check_grok_connection(output: &std::process::Output, authority: &ab_trust::Authority) -> Check {
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["outcome"], "allowed");
    assert_eq!(result["mcp"], "https://fixture.example.ts.net:8443/mcp");
    let token = result["token"].as_str().ok_or("token")?;
    assert!(!text(&output.stderr).contains(token));
    let agent = authority.authenticate_token(token)?;
    assert!(agent.remote);
    assert_eq!(agent.extra["resource"], result["mcp"]);
    authority.authorize(&agent, &ab_trust::Grant::parse("reminders:read")?)?;
    assert_eq!(
        result["config"]["mcpServers"]["apple-bridge"]["headers"]["Authorization"],
        format!("Bearer {token}")
    );
    Ok(())
}
