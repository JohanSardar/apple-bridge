use lab_runner::{Evidence, Scenario};
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

const SCENARIOS: [(&str, &str); 1] = [("CORE-SOURCE-SUPPORT", "source_support")];

pub fn inject(scenario: &mut Scenario) {
    if !scenario.given.is_empty() || !scenario.when.is_empty() || !scenario.then.is_empty() {
        return;
    }
    if let Some((_, action)) = SCENARIOS.iter().find(|(id, _)| *id == scenario.id) {
        scenario.when.push(json!({ *action: {} }));
    }
}
pub fn source_support(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", "SourceSupportChecks")
        .output()
        .is_ok_and(|output| output.status.success());
    evidence.check(
        "native_support_checks_build",
        built,
        "the SourceSupport checks executable builds",
    );
    let passed = built
        && fs::read_to_string("native/.build/native-output-1").is_ok_and(|directory| {
            Command::new(Path::new(directory.trim()).join("SourceSupportChecks"))
                .output()
                .is_ok_and(|output| {
                    let text = format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    output.status.success() && text.trim() == "all SourceSupport checks passed"
                })
        });
    evidence.check(
        "generation_wal_complete_scan_blob_checks",
        passed,
        "generation, WAL, complete-scan and blob checks pass",
    );
    Ok(())
}
