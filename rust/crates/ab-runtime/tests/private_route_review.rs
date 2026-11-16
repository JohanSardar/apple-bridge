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
