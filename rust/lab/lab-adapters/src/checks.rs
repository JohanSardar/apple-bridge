use lab_runner::{Evidence, Scenario};
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

pub const SCENARIOS: [(&str, &str); 9] = [
    ("FILES-TRANSFORM", "files_transform"),
    ("FOC-SIM", "focus_sim"),
    ("FOC-PHONE-SYNC", "focus_sim"),
    ("CLK-SIM", "clock_checks"),
    ("CLK-ALARM-REAL", "clock_checks"),
    ("CLK-TIMER-REAL", "clock_checks"),
    ("FOC-IDLE", "real_only"),
    ("FOC-SET-REAL", "real_only"),
    ("FOC-STATUS-REAL", "real_only"),
];

#[must_use]
pub fn real_only_stub(id: &str) -> bool {
    SCENARIOS
        .iter()
        .any(|(scenario, action)| *scenario == id && *action == "real_only")
}

pub fn inject(scenario: &mut Scenario) {
    if !scenario.given.is_empty() || !scenario.when.is_empty() || !scenario.then.is_empty() {
        return;
    }
    if let Some((_, action)) = SCENARIOS.iter().find(|(id, _)| *id == scenario.id) {
        scenario.when.push(json!({ *action: {} }));
    }
}

pub fn real_only(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    evidence.unverified(
        "real_level_only",
        "this scenario only runs at the real level, which the Rust lab never executes",
    );
    Ok(())
}

pub(crate) fn checks_binary() -> Result<PathBuf> {
    let gateway = crate::replicas::binary()?;
    let path = env::var("AB_LAB_SHORTCUTS_CHECKS").map_or_else(
        |_| {
            gateway
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("ab-shortcuts-checks")
        },
        PathBuf::from,
    );
    if path.is_file() {
        Ok(path)
    } else {
        Err(lab_world::Error::Invalid(format!(
            "ab-shortcuts-checks not found at {}",
            path.display()
        )))
    }
}

pub(crate) fn scratch_directory(id: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    env::temp_dir().join(format!("ab-lab-{id}-{}-{nanos}", std::process::id()))
}

pub(crate) fn run_checks(checks: &Path, args: &[&str], evidence: &mut Evidence) -> Option<Value> {
    let output = Command::new(checks).args(args).output();
    let parsed = output
        .as_ref()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
    if parsed.is_none() {
        evidence.check("checks_binary", false, "ab-shortcuts-checks failed");
    }
    parsed
}

pub(crate) fn report_into(evidence: &mut Evidence, report: Option<&Value>) {
    let Some(report) = report else { return };
    for item in report["assertions"].as_array().into_iter().flatten() {
        evidence.check(
            item["name"].as_str().unwrap_or("unnamed"),
            item["passed"] == true,
            "",
        );
    }
    if let Some(reason) = report["limited"].as_str() {
        evidence.check("not_limited", false, reason);
    }
}
