use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::Value;
use std::{fs, process::Command};

const SETUP: [&str; 23] = [
    "CATALOG",
    "FRESH",
    "COMPLETE",
    "ALL-SKIPPED",
    "PERSIST",
    "REOPEN",
    "UNKNOWN",
    "LIMITED",
    "NO-PROMPTS",
    "SYNC",
    "ONLY-CLICKED-ROWS",
    "REMOTE",
    "QR",
    "CONNECT",
    "FLOW",
    "EXPIRED-QR",
    "PAIR-FAILURE",
    "CONNECT-FAILURE",
    "DENIED-CONTINUE",
    "MODEL-RESUME",
    "REVOKED",
    "STALE",
    "CLOSE",
];
const RELEASE: [&str; 4] = ["APPCAST", "VERSION", "SIGNATURE", "UNINSTALL"];

fn script(
    evidence: &mut Evidence,
    path: &str,
    summary: &str,
    prefix: &str,
    names: &[&str],
) -> Result<()> {
    let output = Command::new(path).output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    evidence.check(summary, output.status.success(), "");
    for name in names {
        evidence.check(
            &name.to_lowercase(),
            text.contains(&format!("{prefix}{name} passed=true")),
            "",
        );
    }
    if !output.status.success() {
        eprint!("{text}");
    }
    Ok(())
}

pub fn release_dmg(evidence: &mut Evidence) {
    let dmg = std::env::var("APPLE_BRIDGE_RELEASE_DMG").unwrap_or_default();
    evidence.check("local_notarized_dmg_available", !dmg.is_empty(), "");
    if dmg.is_empty() {
        return;
    }
    let ok = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .status()
            .is_ok_and(|s| s.success())
    };
    evidence.check(
        "stapled_dmg",
        ok("xcrun", &["stapler", "validate", &dmg]),
        "",
    );
    let assessment = Command::new("spctl")
        .args([
            "-a",
            "-vv",
            "--type",
            "open",
            "--context",
            "context:primary-signature",
            &dmg,
        ])
        .output();
    let accepted = assessment.as_ref().is_ok_and(|o| {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        o.status.success()
            && text.contains("accepted")
            && text.contains("source=Notarized Developer ID")
    });
    evidence.check("gatekeeper_accepts_dmg", accepted, "");
    let mount = std::env::temp_dir().join(format!("ab-test-release-{}", std::process::id()));
    let made = fs::create_dir(&mount).is_ok();
    evidence.check("private_mount_directory", made, "");
    if !made {
        return;
    }
    let mount_text = mount.to_string_lossy().into_owned();
    let attached = ok(
        "hdiutil",
        &[
            "attach",
            "-readonly",
            "-nobrowse",
            "-mountpoint",
            &mount_text,
            &dmg,
        ],
    );
    evidence.check("readonly_dmg_mount", attached, "");
    if !attached {
        let _ = fs::remove_dir(&mount);
        return;
    }
    let app = mount.join("Apple Bridge.app");
    evidence.check(
        "bundled_app_signature",
        ok(
            "codesign",
            &["--verify", "--deep", "--strict", &app.to_string_lossy()],
        ),
        "",
    );
    let link = fs::read_link(mount.join("Applications"));
    evidence.check(
        "drag_to_applications",
        link.is_ok_and(|p| p.to_string_lossy() == "/Applications"),
        "",
    );
    evidence.check("dmg_unmounted", ok("hdiutil", &["detach", &mount_text]), "");
    let _ = fs::remove_dir(&mount);
}

pub fn robustness(_: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    match (input["case"].as_str(), evidence.scenario.as_str()) {
        (Some("setup"), "Q1-SETUP") => script(
            evidence,
            "packaging/setup/check.sh",
            "setup_sim_checks",
            "Q1-",
            &SETUP,
        ),
        (Some("release"), "R1-RELEASE-SIM") => script(
            evidence,
            "packaging/release/check.sh",
            "release_unit_checks",
            "R1-",
            &RELEASE,
        ),
        (Some("release"), "R1-DMG-REAL") => {
            release_dmg(evidence);
            Ok(())
        }
        (case, id) => Err(Error::Invalid(format!(
            "robustness case {case:?} not ported for {id}"
        ))),
    }
}
