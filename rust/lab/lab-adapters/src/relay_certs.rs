use lab_runner::Evidence;
use lab_world::{Error, Result};
use std::{
    collections::HashSet,
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const CT_EXPECTED: [&str; 9] = [
    "fake_all_clear_ignored",
    "unexpected_wildcard",
    "unexpected_precertificate",
    "aggregator_outage_unknown",
    "aggregator_lag_unknown",
    "stale_monitor_unknown",
    "doctor_and_push_hook",
    "all_owned_names",
    "precertificate_der",
];
const CAA_EXPECTED: [&str; 9] = [
    "enrollment_caa_records",
    "wrong_account_denied",
    "wrong_method_denied",
    "alternate_issuer_denied",
    "wildcard_denied",
    "signed_rotation_recovery",
    "cname_target_policy",
    "dns_operator_residual_risk",
    "dns_failure_preserves_binding",
];

pub fn run(id: &str, directory: &Path, evidence: &mut Evidence) -> Result<()> {
    if !cfg!(target_os = "linux") {
        evidence.check("linux_sim_required", false, "");
        return Ok(());
    }
    let (package, test, expected) = if id == "RELAY-CAA-ACCOUNT" {
        ("ab-relay-control", "relay_caa_account", CAA_EXPECTED)
    } else {
        ("ab-remote", "relay_ct_independent", CT_EXPECTED)
    };
    let log = directory.join("cargo-test.log");
    let sink = fs::File::create(&log)?;
    let mut child = Command::new("cargo")
        .args([
            "test",
            "--manifest-path",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml"),
            "--offline",
            "--locked",
            "-p",
            package,
            "--features",
            "r2-certificates",
            "--test",
            test,
            "--",
            "--nocapture",
        ])
        .stdin(Stdio::null())
        .stdout(sink.try_clone()?)
        .stderr(sink)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(200));
    };
    let output = String::from_utf8_lossy(&fs::read(&log)?).into_owned();
    let seen: HashSet<&str> = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("R2_ASSERT\t"))
        .collect();
    let executed = status.is_some_and(|s| s.success());
    let detail = if executed {
        String::new()
    } else {
        output
            .chars()
            .rev()
            .take(4096)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    };
    evidence.check("rust_fixture_executed", executed, &detail);
    for name in expected {
        evidence.check(name, seen.contains(name), "");
    }
    if status.is_none() {
        return Err(Error::Invalid("certificate fixture timed out".into()));
    }
    Ok(())
}
