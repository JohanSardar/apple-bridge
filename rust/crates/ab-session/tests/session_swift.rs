#![cfg(unix)]

#[test]
fn the_swift_helper_native_mode_runs_against_the_real_session()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(helper) = std::env::var_os("AB_SESSION_SWIFT_CHECKS") else {
        return Ok(());
    };
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/session_swift.py");
    let output = std::process::Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_ab-session"))
        .arg(helper)
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("ok swift helper against ab-session"));
    Ok(())
}
