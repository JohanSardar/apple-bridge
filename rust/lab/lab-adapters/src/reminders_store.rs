use lab_runner::Evidence;
use lab_world::{Result, World};

pub const SCENARIOS: [&str; 4] = [
    "REM-STORE-READ",
    "REM-STORE-DECODE",
    "REM-STORE-PERF",
    "REM-SECTIONS",
];

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "REM-STORE-PERF" => perf(world, evidence),
        "REM-SECTIONS" => sections(world, evidence),
        _ => read(world, evidence),
    }
    Ok(())
}

fn read(_: &World, evidence: &mut Evidence) {
    evidence.check("world_start", true, "gateway and SimApple peer ready");
    crate::native_core::rust_tests(
        evidence,
        "reader_fields_and_blobs",
        "ab-native",
        &["--test", "reminders_store_oracle"],
    );
}

fn perf(_: &World, evidence: &mut Evidence) {
    crate::native_core::rust_tests(
        evidence,
        "store_profile_completed",
        "ab-native",
        &["--test", "reminders_store_oracle"],
    );
}

fn sections(_: &World, evidence: &mut Evidence) {
    crate::native_core::rust_tests(
        evidence,
        "store_effects_and_invalidation",
        "ab-native",
        &["--test", "reminders_store_oracle"],
    );
}
