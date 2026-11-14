use crate::{
    checks::{checks_binary, report_into, run_checks, scratch_directory},
    http::Client,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{env, fs};

const UNQUALIFIED: [&str; 5] = [
    "clock.create_alarm",
    "clock.toggle_alarm",
    "clock.delete_alarm",
    "clock.start_timer",
    "clock.world_time",
];

pub fn clock_checks(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let checks = checks_binary()?;
    let directory = scratch_directory(&id);
    fs::create_dir_all(&directory)?;
    let directory_text = directory.to_string_lossy().into_owned();
    let root_text = env::current_dir()?.to_string_lossy().into_owned();
    let report = run_checks(
        &checks,
        &["run", &id, &directory_text, &root_text],
        evidence,
    );
    let _ = fs::remove_dir_all(&directory);
    let Some(report) = report else { return Ok(()) };
    report_into(evidence, Some(&report));
    if id == "CLK-SIM" {
        let client = Client::new(world, "calendar:read,reminders:read,system:read")?;
        for op in UNQUALIFIED {
            let advertised = client
                .call_raw(op, &json!({"args":{}}), 10)?
                .is_some_and(|(status, _)| (200..300).contains(&status));
            evidence.check(
                "unqualified_operation_not_advertised",
                !advertised,
                "operation is not registered",
            );
        }
    }
    Ok(())
}
