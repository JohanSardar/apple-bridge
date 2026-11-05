use crate::Error;
use ab_wire::messages::IpcFrame;
use serde_json::Value;

pub(super) fn part_keys(evidence: &Value) -> Vec<String> {
    evidence["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["native_key"].as_str().map(str::to_owned))
        .filter(|key| !key.is_empty())
        .collect()
}

pub(super) fn facts_source(operation: &str) -> bool {
    matches!(
        operation.split('.').next().unwrap_or_default(),
        "calendar" | "contacts" | "reminders"
    )
}

pub(super) fn reopenable(saved: &Value) -> bool {
    match saved["disposition"].as_str().unwrap_or_default() {
        "indeterminate" | "locally_observed" => true,
        "delivery_failed" => saved["evidence"]["parts"].as_array().is_some_and(|parts| {
            parts.iter().any(|part| {
                matches!(
                    part["disposition"].as_str(),
                    Some("locally_observed" | "observed" | "indeterminate")
                )
            })
        }),
        _ => false,
    }
}

pub(super) fn capturable(disposition: &str, evidence: &Value) -> bool {
    evidence["observed"] == true
        && (disposition != "indeterminate" || evidence["source"] == "messages")
}

pub(super) fn needs_capture(operation: &str) -> bool {
    let source = operation.split('.').next().unwrap_or_default();
    source != "mac"
        && !matches!(
            operation,
            "focus.set"
                | "speech.say"
                | "capture.play_sound"
                | "capture.screenshot"
                | "capture.camera_photo"
                | "capture.record_audio"
                | "print.document"
                | "maps.open"
                | "shortcuts.run"
        )
}

pub(super) fn saved_frame(saved: &Value) -> Result<IpcFrame, Error> {
    Ok(serde_json::from_value(saved.clone())?)
}

pub(super) fn source_of(operation: &str) -> &str {
    operation.split('.').next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reopening_failed_delivery_requires_an_unresolved_part() {
        for (part, expected) in [
            ("locally_observed", true),
            ("observed", true),
            ("indeterminate", true),
            ("delivered", false),
            ("delivery_failed", false),
            ("", false),
        ] {
            let saved = json!({"disposition": "delivery_failed", "evidence": {"parts": [{"disposition": part}]}});
            assert_eq!(reopenable(&saved), expected, "{part}");
        }
        for disposition in ["delivered", "failed_no_effect", "", "unknown"] {
            assert!(!reopenable(&json!({"disposition": disposition})));
        }
        for disposition in ["indeterminate", "locally_observed"] {
            assert!(reopenable(&json!({"disposition": disposition})));
        }
        assert!(!reopenable(
            &json!({"disposition": "delivery_failed", "evidence": {"parts": []}})
        ));
    }

    #[test]
    fn capture_requires_observation_and_limits_indeterminate_to_messages() {
        for source in ["messages", "calendar", "", "mac"] {
            for disposition in [
                "indeterminate",
                "locally_observed",
                "delivered",
                "delivery_failed",
            ] {
                for observed in [true, false] {
                    assert_eq!(
                        capturable(
                            disposition,
                            &json!({"source": source, "observed": observed})
                        ),
                        observed && (disposition != "indeterminate" || source == "messages"),
                        "{source} {disposition} {observed}"
                    );
                }
            }
        }
        assert!(!capturable("delivered", &json!({"observed": "true"})));
    }

    #[test]
    fn capture_exemptions_are_exact_except_for_the_mac_source() {
        for operation in [
            "focus.set",
            "speech.say",
            "capture.play_sound",
            "capture.screenshot",
            "capture.camera_photo",
            "capture.record_audio",
            "print.document",
            "maps.open",
            "shortcuts.run",
            "mac.open",
            "mac.other",
        ] {
            assert!(!needs_capture(operation), "{operation}");
        }
        for operation in [
            "messages.send",
            "calendar.create",
            "files.write",
            "capture.other",
            "focus.other",
            "shortcuts.other",
            "macaroni.open",
        ] {
            assert!(needs_capture(operation), "{operation}");
        }
    }
}
