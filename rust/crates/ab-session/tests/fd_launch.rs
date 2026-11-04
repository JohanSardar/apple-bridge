#![cfg(all(feature = "rust-effects", feature = "rust-capture", unix))]

#[test]
fn the_launched_binary_pages_facts_and_serves_effects_on_two_inherited_streams()
-> Result<(), Box<dyn std::error::Error>> {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fd_launch.py");
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
