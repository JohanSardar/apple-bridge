use crate::shortcuts::scratch_directory;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const NOOP: &str = "packaging/shortcuts/noop.json";
const REFERENCES: &str = r#"{"version":1,"name":"Synthetic references","actions":[{"key":"text","identifier":"is.workflow.actions.gettext","parameters":{"WFTextActionText":"a < b & c","Number":2,"Flag":true,"Items":["x"]}},{"key":"child","identifier":"is.workflow.actions.runworkflow","parameters":{"WFInput":{"$output":"text"},"Input":{"$input":true},"GroupingIdentifier":{"$group":"branch"}}}]}"#;
const INVALID: [&str; 5] = [
    r#"{"version":2,"name":"Invalid","actions":[]}"#,
    r#"{"version":1,"name":"Invalid","actions":[],"typo":true}"#,
    r#"{"version":1,"name":"Invalid","actions":[{"key":"a","identifier":"x","parameters":{"value":null}}]}"#,
    r#"{"version":1,"name":"Invalid","actions":[{"key":"a","identifier":"x","parameters":{"value":{"$output":"missing"}}}]}"#,
    r#"{"version":1,"name":"Invalid","actions":[{"key":"a","identifier":"x"},{"key":"a","identifier":"x"}]}"#,
];

fn tool() -> Result<PathBuf> {
    let path = env::var("AB_LAB_SHORTCUTS").map_or_else(
        |_| {
            crate::replicas::binary().map(|gateway| {
                gateway
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("ab-shortcuts")
            })
        },
        |path| Ok(PathBuf::from(path)),
    )?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(Error::Invalid(format!(
            "ab-shortcuts not found at {}",
            path.display()
        )))
    }
}
fn build(tool: &Path, source: &str, out: &Path, unsigned: bool) -> Option<Vec<u8>> {
    let mut command = Command::new(tool);
    command.args(["build", "-source", source, "-out"]).arg(out);
    if unsigned {
        command.arg("-unsigned");
    }
    command
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
}
fn plist_json(path: &Path) -> Option<Value> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    serde_json::from_slice(&output.stdout).ok()
}
fn references_valid(plist: &Value) -> bool {
    let actions = plist["WFWorkflowActions"].as_array();
    let Some([first, second]) = actions.map(Vec::as_slice) else {
        return false;
    };
    let first = &first["WFWorkflowActionParameters"];
    let second = &second["WFWorkflowActionParameters"];
    let token = &second["WFInput"];
    let value = &token["Value"];
    first["WFTextActionText"] == "a < b & c"
        && first["Flag"] == true
        && first["Number"].as_f64() == Some(2.0)
        && value["OutputUUID"] == first["UUID"]
        && value["Type"] == "ActionOutput"
        && token["WFSerializationType"] == "WFTextTokenAttachment"
}

pub fn sim(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    cases(evidence, false);
    Ok(())
}
pub fn real(evidence: &mut Evidence) -> Result<()> {
    cases(evidence, true);
    Ok(())
}
fn cases(evidence: &mut Evidence, signed: bool) {
    let directory = scratch_directory("shortcut-sign");
    let created = fs::create_dir_all(&directory).is_ok();
    evidence.check("temporary_build_directory", created, "");
    if !created {
        return;
    }
    steps(evidence, &directory, signed);
    let _ = fs::remove_dir_all(&directory);
}
fn steps(evidence: &mut Evidence, directory: &Path, signed: bool) {
    let tool = tool();
    evidence.check("tool_build", tool.is_ok(), "");
    let Ok(tool) = tool else {
        return;
    };
    let (first, second) = (directory.join("first"), directory.join("second"));
    let built = build(&tool, NOOP, &first, true).is_some();
    evidence.check("first_compile", built, "");
    if !built {
        return;
    }
    evidence.check(
        "second_compile",
        build(&tool, NOOP, &second, true).is_some(),
        "",
    );
    let name = "noop.unsigned.shortcut";
    let one = fs::read(first.join(name));
    evidence.check("unsigned_plist_exists", one.is_ok(), "");
    let two = fs::read(second.join(name));
    let one = one.unwrap_or_default();
    evidence.check(
        "deterministic_plist",
        two.is_ok_and(|two| !one.is_empty() && two == one),
        "",
    );
    let valid = plist_json(&first.join(name)).is_some_and(|plist| {
        plist["WFWorkflowActions"]
            .as_array()
            .is_some_and(|a| a.len() == 1)
    });
    evidence.check("valid_noop_workflow_plist", valid, "");
    let fixture = directory.join("references.json");
    let written = fs::write(&fixture, REFERENCES).is_ok();
    evidence.check("reference_fixture_written", written, "");
    let fixture_text = fixture.to_string_lossy().into_owned();
    evidence.check(
        "native_parameters_and_references_compile",
        build(&tool, &fixture_text, directory, true).is_some(),
        "",
    );
    evidence.check(
        "independent_plist_reference_readback",
        plist_json(&directory.join("references.unsigned.shortcut"))
            .is_some_and(|plist| references_valid(&plist)),
        "",
    );
    for source in INVALID {
        let rejected = fs::write(&fixture, source).is_ok()
            && build(&tool, &fixture_text, directory, true).is_none();
        evidence.check("invalid_source_rejected", rejected, "");
    }
    if signed {
        signed_cases(evidence, &tool, directory);
    }
}
fn signed_cases(evidence: &mut Evidence, tool: &Path, directory: &Path) {
    let output = directory.join("signed");
    let manifest = build(tool, NOOP, &output, false);
    evidence.check("shortcuts_sign_succeeds", manifest.is_some(), "");
    let Some(manifest) = manifest else {
        return;
    };
    let modes: Vec<Value> = serde_json::from_slice(&manifest).unwrap_or_default();
    evidence.check("signing_mode_recorded", modes.len() == 1, "");
    let signed = output.join("[ab-test noop] Apple Bridge Signing Test.shortcut");
    let archive = fs::read(&signed);
    evidence.check(
        "signed_aea_archive",
        archive.as_ref().is_ok_and(|a| a.starts_with(b"AEA1")),
        "",
    );
    let (Ok(archive), [mode]) = (archive, modes.as_slice()) else {
        return;
    };
    let accepted = Command::new("/usr/bin/shortcuts")
        .args(["sign", "--mode", mode["mode"].as_str().unwrap_or_default()])
        .arg("--input")
        .arg(&signed)
        .arg("--output")
        .arg(directory.join("validated.shortcut"))
        .status()
        .is_ok_and(|status| status.success());
    evidence.check(
        "shortcuts_accepts_signed_archive",
        accepted,
        &hex::encode(Sha256::digest(&archive)),
    );
}
