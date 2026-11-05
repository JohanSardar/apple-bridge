#![cfg(all(unix, feature = "lab"))]

#[test]
fn the_launched_binary_is_the_only_ipc_peer_when_given_a_session_secret()
-> Result<(), Box<dyn std::error::Error>> {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/session_launch.py");
    let output = std::process::Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_ab-session"))
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        text.lines().filter(|line| line.starts_with("ok ")).count(),
        5
    );
    Ok(())
}
