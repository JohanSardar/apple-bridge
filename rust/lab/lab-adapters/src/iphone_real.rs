use crate::iphone::project;
use lab_runner::{Evidence, Scenario};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const SENSOR_REAL: [&str; 8] = [
    "APP-HEALTH-REAL",
    "APP-HEALTH-WRITE-REAL",
    "APP-MOTION-REAL",
    "APP-ALARM-REAL",
    "APP-DEVICE-REAL",
    "APP-SCREENTIME-REAL",
    "APP-LOC-REAL",
    "APP-CAPTURE-REAL",
];

pub fn summarize(evidence: &mut Evidence, raw: &[u8], succeeded: bool) {
    let parsed = serde_json::from_slice::<Value>(raw).ok();
    let checks: BTreeMap<String, bool> = parsed
        .as_ref()
        .and_then(|v| v["checks"].as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_bool().unwrap_or(false)))
                .collect()
        })
        .unwrap_or_default();
    if checks.is_empty() {
        evidence.check("real_device_result", false, "No valid real iPhone result");
        return;
    }
    evidence.passed = succeeded;
    for (name, passed) in &checks {
        evidence.check(name, *passed, "");
    }
    let summary = json!({
        "counts": parsed.as_ref().map_or(Value::Null, |v| v["counts"].clone()),
        "digests": parsed.as_ref().map_or(Value::Null, |v| v["digests"].clone()),
    });
    evidence.check("redacted_observation", true, &summary.to_string());
}

#[must_use]
pub fn handles(id: &str) -> bool {
    SENSOR_REAL.contains(&id)
        || id == "APP-BUILD-REAL"
        || crate::iphone_phone_real::SCENARIOS.contains(&id)
}

#[must_use]
pub fn run_real(scenario: &Scenario) -> Option<Evidence> {
    let mut evidence = Evidence {
        scenario: scenario.id.clone(),
        passed: true,
        assertions: vec![],
    };
    if let Some(phone) = crate::iphone_phone_real::run(scenario) {
        return Some(phone);
    }
    if SENSOR_REAL.contains(&scenario.id.as_str()) {
        let output = project("python3", &["lab/iphone-real/run.py", &scenario.id]);
        let (raw, succeeded) = output
            .map(|o| (o.stdout.clone(), o.status.success()))
            .unwrap_or_default();
        summarize(&mut evidence, &raw, succeeded);
        return Some(evidence);
    }
    if scenario.id == "APP-BUILD-REAL" {
        evidence.check(
            "real_device_acceptance",
            false,
            "Real iPhone acceptance harness is not implemented; generic replica proof cannot qualify the phone",
        );
        return Some(evidence);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Evidence {
        Evidence {
            scenario: "T".into(),
            passed: true,
            assertions: vec![],
        }
    }

    #[test]
    fn invalid_output_fails() {
        let mut evidence = blank();
        summarize(&mut evidence, b"not json", true);
        assert!(!evidence.passed);
        assert_eq!(evidence.assertions[0].name, "real_device_result");
    }

    #[test]
    fn checks_sorted_and_summarized() {
        let mut evidence = blank();
        summarize(
            &mut evidence,
            br#"{"checks":{"b":true,"a":true},"counts":{"n":1},"digests":{}}"#,
            true,
        );
        let names: Vec<_> = evidence
            .assertions
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "redacted_observation"]);
        assert!(evidence.passed);
    }

    #[test]
    fn failed_exit_fails_even_with_passing_checks() {
        let mut evidence = blank();
        summarize(&mut evidence, br#"{"checks":{"a":true}}"#, false);
        assert!(!evidence.passed);
    }
}
