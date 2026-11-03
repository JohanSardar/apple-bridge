use lab_runner::{Evidence, Scenario};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

pub const SIM_SCENARIOS: [&str; 2] = ["APP-ALARM-SIM", "APP-SHELL-SIM"];

#[derive(Debug)]
pub struct Scratch(PathBuf);
impl Scratch {
    pub fn new(label: &str) -> std::io::Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = env::temp_dir().join(format!("ab-test-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn project_root() -> Result<PathBuf, String> {
    let root = env::var("APPLE_BRIDGE_IPHONE_ROOT").map_err(|_| {
        "set APPLE_BRIDGE_IPHONE_ROOT to a checkout of the iPhone project".to_owned()
    })?;
    let root = fs::canonicalize(root).map_err(|e| format!("iPhone checkout: {e}"))?;
    root.join("ios")
        .join("project.yml")
        .is_file()
        .then_some(root)
        .ok_or_else(|| "iPhone checkout: ios/project.yml missing".to_owned())
}

pub fn project(program: &str, args: &[&str]) -> Result<Output, String> {
    let root = project_root()?;
    Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())
}

pub fn swift_build(binary: &Path, flags: &[&str], sources: &[&str]) -> bool {
    let out = binary.to_string_lossy();
    let mut args = vec!["swiftc"];
    args.extend_from_slice(flags);
    args.extend_from_slice(sources);
    args.extend_from_slice(&["-o", &out]);
    project("xcrun", &args).is_ok_and(|o| o.status.success())
}

pub fn run(scenario: &Scenario) -> Option<Evidence> {
    let mut evidence = Evidence {
        scenario: scenario.id.clone(),
        passed: true,
        assertions: vec![],
    };
    match scenario.id.as_str() {
        "APP-ALARM-SIM" => alarm(&mut evidence),
        "APP-SHELL-SIM" => shell(&mut evidence),
        id if crate::iphone_gateway::run(id, &mut evidence)
            || crate::iphone_push::run(id, &mut evidence) => {}
        _ => return None,
    }
    Some(evidence)
}

fn alarm(evidence: &mut Evidence) {
    let Ok(dir) = Scratch::new("alarms") else {
        return;
    };
    let binary = dir.path().join("checks");
    let built = swift_build(
        &binary,
        &["-parse-as-library"],
        &[
            "ios/Capabilities/Alarms/AlarmCommands.swift",
            "lab/iphonealarms/main.swift",
        ],
    );
    evidence.check("swift_handler_build", built, "");
    if built {
        let ran = Command::new(&binary)
            .output()
            .is_ok_and(|o| o.status.success());
        evidence.check("alarm_timer_lifecycle_and_permission", ran, "");
    }
}

const SHELL_CHECKS: [&str; 19] = [
    "tab_structure",
    "unpaired_status",
    "unpaired_no_request",
    "pair_failure",
    "paired_status",
    "connecting_state",
    "connected_status",
    "offline_status",
    "reconnect_status",
    "failed_repair_keeps_identity",
    "status_recovers_after_error",
    "existing_pair_status",
    "approval_order_and_expiry",
    "approval_list_rendering",
    "approval_empty_state",
    "approval_expiry_refresh",
    "approval_removal_refresh",
    "stored_approval_rendering",
    "decided_approval_removed",
];

fn shell(evidence: &mut Evidence) {
    let dir = Scratch::new("shell");
    evidence.check("isolated_build", dir.is_ok(), "");
    let Ok(dir) = dir else {
        return;
    };
    let binary = dir.path().join("shell-proof");
    let built = swift_build(
        &binary,
        &[
            "-swift-version",
            "6",
            "-warnings-as-errors",
            "-parse-as-library",
        ],
        &[
            "ios/Delivery/PhoneJob.swift",
            "ios/Capabilities/Approvals/ApprovalInbox.swift",
            "ios/Pairing/PairingState.swift",
            "ios/App/ShellModel.swift",
            "lab/ios/ShellProof.swift",
        ],
    );
    evidence.check("shell_view_models_build", built, "");
    if !built {
        return;
    }
    let output = Command::new(&binary).output();
    evidence.check(
        "shell_view_models_execute",
        output.as_ref().is_ok_and(|o| o.status.success()),
        "",
    );
    let mut combined = vec![];
    if let Ok(o) = &output {
        combined.extend_from_slice(&o.stdout);
        combined.extend_from_slice(&o.stderr);
    }
    let checks = serde_json::from_slice::<BTreeMap<String, Value>>(&combined);
    evidence.check("shell_proof_decoded", checks.is_ok(), "");
    let checks = checks.unwrap_or_default();
    for name in SHELL_CHECKS {
        evidence.check(name, checks.get(name) == Some(&Value::Bool(true)), "");
    }
}
