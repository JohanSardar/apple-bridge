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
        &["remote", "on", "--domain", "example.com"][..],
    ] {
        assert_eq!(bridge(home.path(), arguments)?.status.code(), Some(1));
    }
    Ok(())
}
