use std::process::{Command, Output};

type Check = Result<(), Box<dyn std::error::Error>>;

fn bridge(home: &std::path::Path, arguments: &[&str]) -> Result<Output, std::io::Error> {
    Command::new(env!("CARGO_BIN_EXE_bridge"))
        .args(arguments)
        .arg("--home")
        .arg(home)
        .output()
}

#[test]
fn web_devices_and_lock_work_on_a_fresh_home() -> Check {
    let home = tempfile::tempdir()?;
    let devices = bridge(home.path(), &["web", "devices"])?;
    assert!(devices.status.success());
    assert_eq!(String::from_utf8(devices.stdout)?.trim(), "[]");
    assert!(bridge(home.path(), &["web", "lock"])?.status.success());
    assert_eq!(
        bridge(home.path(), &["web", "other"])?.status.code(),
        Some(1)
    );
    Ok(())
}

#[test]
fn permission_and_selftest_verbs_reject_bad_input_without_launching_anything() -> Check {
    let home = tempfile::tempdir()?;
    for arguments in [
        &["permissions", "request", "fake"][..],
        &["permissions", "request"][..],
        &["permissions", "list", "camera"][..],
        &["selftest", "other"][..],
    ] {
        assert_eq!(bridge(home.path(), arguments)?.status.code(), Some(1));
    }
    Ok(())
}

fn bridge_with_file_store(
    home: &std::path::Path,
    arguments: &[&str],
) -> Result<Output, std::io::Error> {
    Command::new(env!("CARGO_BIN_EXE_bridge"))
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .args(arguments)
        .arg("--home")
        .arg(home)
        .output()
}

#[test]
fn agent_json_matches_go_key_order_and_id_shape() -> Check {
    let home = tempfile::tempdir()?;
    let created = bridge_with_file_store(
        home.path(),
        &[
            "agent",
            "create",
            "--name",
            "Test Agent",
            "--level",
            "read",
            "--scope",
            "reminders:read",
        ],
    )?;
    assert!(created.status.success());
    let text = String::from_utf8(created.stdout)?;
    let value: serde_json::Value = serde_json::from_str(text.trim())?;
    let id = value["agent"]["id"].as_str().ok_or("id")?;
    assert!(id.starts_with("agent:") && id.len() == "agent:".len() + 26);
    assert!(text.starts_with("{\"agent\":{\"id\":\"agent:"));
    let order = [
        "\"id\"",
        "\"name\"",
        "\"level\"",
        "\"scopes\"",
        "\"created_at\"",
        "\"token\"",
    ];
    let positions = order
        .iter()
        .map(|key| text.find(key).ok_or("key"))
        .collect::<Result<Vec<_>, _>>()?;
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    let listed = bridge_with_file_store(home.path(), &["agent", "list"])?;
    let list = String::from_utf8(listed.stdout)?;
    assert!(list.starts_with("[{\"id\":\"agent:"));
    assert!(list.contains("\"created_at\":\""));
    Ok(())
}

#[test]
fn remote_flags_validate_like_go() -> Check {
    let home = tempfile::tempdir()?;
    for arguments in [
        &["remote", "on", "--private", "--via", "relay"][..],
        &["remote", "on", "--private", "--domain", "example.com"][..],
        &["remote", "on", "--snippets"][..],
        &["remote", "status", "--snippets"][..],
        &[
            "remote",
            "on",
            "--domain",
            "example.com",
            "--via",
            "private",
        ][..],
    ] {
        assert_eq!(bridge(home.path(), arguments)?.status.code(), Some(1));
    }
    Ok(())
}

#[test]
fn doctor_reports_credential_store_last_error_without_a_database() -> Check {
    let home = tempfile::tempdir()?;
    let failure = serde_json::json!({
        "code": "credential_store_locked",
        "store": "file_0600",
        "remedy": "Unlock the login Keychain in an interactive macOS session, then retry bridge start.",
        "retryable": true,
    });
    std::fs::write(
        home.path().join("credential-store-error.json"),
        serde_json::to_vec(&failure)?,
    )?;
    let output = bridge_with_file_store(home.path(), &["doctor"])?;
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(report["credentials"]["store"], "file_0600");
    assert_eq!(report["credentials"]["last_error"], failure);
    assert_eq!(report["ok"], false);
    Ok(())
}

#[test]
fn doctor_ignores_a_last_error_from_another_store() -> Check {
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("credential-store-error.json"),
        br#"{"code":"credential_store_locked","store":"keychain","remedy":"x","retryable":true}"#,
    )?;
    let output = bridge_with_file_store(home.path(), &["doctor"])?;
    assert_eq!(output.status.code(), Some(1));
    Ok(())
}

#[test]
fn web_enroll_prints_the_url_then_a_terminal_qr() -> Check {
    let home = tempfile::tempdir()?;
    let output = bridge_with_file_store(home.path(), &["web", "enroll"])?;
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout)?;
    let mut lines = text.lines();
    let url = lines.next().ok_or("url")?;
    assert!(url.starts_with("http://localhost:7780/enroll#"));
    let code: Vec<&str> = lines.collect();
    assert!(code.len() > 10);
    assert!(code.iter().any(|line| line.contains('\u{2588}')
        || line.contains('\u{2580}')
        || line.contains('\u{2584}')));
    Ok(())
}
