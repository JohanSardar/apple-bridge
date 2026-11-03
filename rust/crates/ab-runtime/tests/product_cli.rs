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
