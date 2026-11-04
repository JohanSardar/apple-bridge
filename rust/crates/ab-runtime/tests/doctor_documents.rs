use ab_trust::domain::{Coverage, Source};
use std::process::Command;

type Check = Result<(), Box<dyn std::error::Error>>;

fn coverage(fields: &str) -> Coverage {
    Coverage {
        pack_version: String::new(),
        operations: Vec::new(),
        fields: fields.into(),
        missing: None,
    }
}

fn gateway_home(home: &std::path::Path) -> Check {
    use std::os::unix::fs::PermissionsExt;
    let path = home.join("runtime.json");
    std::fs::write(
        &path,
        serde_json::to_vec(
            &serde_json::json!({"role":"gateway","home":home,"peer_address":"","api_address":"","socket":""}),
        )?,
    )?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn doctor(home: &std::path::Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_bridge"))
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .args(["doctor", "--home"])
        .arg(home)
        .output()?;
    assert!(output.status.success());
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn doctor_reports_document_sources_like_go() -> Check {
    let home = tempfile::tempdir()?;
    let mut state = ab_state::State::<ab_state::Gateway>::open(home.path())?;
    drop(state.source_state(Source::Vision, "ready", coverage("complete"), None, ""));
    drop(state.source_state(Source::Pdf, "ready", coverage("partial"), None, ""));
    drop(state);
    gateway_home(home.path())?;
    let report = doctor(home.path())?;
    assert_eq!(
        report["document_checks"],
        serde_json::json!({"vision":"ready","pdf":"unavailable"})
    );
    let warnings = report["warnings"].as_array().ok_or("warnings")?;
    assert!(warnings.iter().any(|w| w
        == "pdf document source is not ready; start the current Helper and check sources.status"));
    assert!(!warnings.iter().any(|w| {
        w.as_str()
            .is_some_and(|w| w.starts_with("vision document source"))
    }));
    Ok(())
}

#[test]
fn doctor_marks_missing_document_sources_unavailable() -> Check {
    let home = tempfile::tempdir()?;
    drop(ab_state::State::<ab_state::Gateway>::open(home.path())?);
    gateway_home(home.path())?;
    let report = doctor(home.path())?;
    assert_eq!(
        report["document_checks"],
        serde_json::json!({"vision":"unavailable","pdf":"unavailable"})
    );
    let count = report["warnings"]
        .as_array()
        .ok_or("warnings")?
        .iter()
        .filter(|w| {
            w.as_str()
                .is_some_and(|w| w.contains("document source is not ready"))
        })
        .count();
    assert_eq!(count, 2);
    Ok(())
}
