use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const OPERATIONS: [&str; 9] = [
    "messages.send",
    "messages.reply",
    "messages.react",
    "messages.edit",
    "messages.unsend",
    "messages.mark_read",
    "messages.mark_unread",
    "messages.delete_message",
    "messages.delete_chat",
];

fn read(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn routes(reports: &Path, os_build: &str) -> BTreeSet<String> {
    read(&reports.join("messages-command-proofs.json"))
        .filter(|proofs| proofs["os_build"] == os_build)
        .and_then(|proofs| {
            proofs["routes"].as_array().map(|routes| {
                routes
                    .iter()
                    .filter_map(|route| route.as_str().map(ToOwned::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn proven(operation: &str, routes: &BTreeSet<String>) -> bool {
    if operation == "messages.send" {
        return routes
            .iter()
            .any(|route| route.ends_with(":text") || route.ends_with(":attachment"));
    }
    routes
        .iter()
        .any(|route| route.ends_with(&format!(":{operation}")))
}

fn failure_reasons(reports: &Path, os_build: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut reasons: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let Some(report) = read(&reports.join("messages-selftest.json")) else {
        return reasons;
    };
    if report["os_build"] != os_build {
        return reasons;
    }
    for step in report["steps"].as_array().into_iter().flatten() {
        let operation = step["operation"].as_str().unwrap_or_default();
        let reason = step["reason"].as_str().unwrap_or_default();
        if step["proven"] != true && !operation.is_empty() && !reason.is_empty() {
            reasons
                .entry(operation.to_owned())
                .or_default()
                .insert(reason.to_owned());
        }
    }
    reasons
}

#[must_use]
pub(super) fn advertised(reports: &Path, os_build: &str) -> (Vec<&'static str>, Value) {
    let routes = routes(reports, os_build);
    let reasons = failure_reasons(reports, os_build);
    let capabilities = OPERATIONS
        .into_iter()
        .filter(|operation| proven(operation, &routes))
        .collect();
    let coverage = OPERATIONS
        .iter()
        .map(|operation| {
            let supported = proven(operation, &routes);
            let reason = if supported {
                "proven_routes_and_parts_only".to_owned()
            } else {
                reasons.get(*operation).map_or_else(
                    || "route_proof_required".to_owned(),
                    |found| found.iter().cloned().collect::<Vec<_>>().join(","),
                )
            };
            json!({"operation": operation, "proven": supported, "reason": reason})
        })
        .collect();
    (capabilities, Value::Array(coverage))
}

pub(super) fn apply(
    coverage: &mut BTreeMap<String, Box<serde_json::value::RawValue>>,
    capabilities: &mut BTreeMap<String, Box<serde_json::value::RawValue>>,
) {
    let Some(raw) = coverage.get("messages") else {
        return;
    };
    let Ok(mut fields) = serde_json::from_str::<Value>(raw.get()) else {
        return;
    };
    let Some(advertise) = fields
        .as_object_mut()
        .and_then(|map| map.remove("advertise"))
    else {
        return;
    };
    let reports = advertise["report_dir"].as_str().unwrap_or_default();
    let os_build = advertise["os_build"].as_str().unwrap_or_default();
    let (operations, per_operation) = advertised(Path::new(reports), os_build);
    fields["operations"] = per_operation;
    let raw = |value: &Value| serde_json::value::RawValue::from_string(value.to_string()).ok();
    if let Some(value) = raw(&fields) {
        coverage.insert("messages".into(), value);
    }
    if let Some(value) = raw(&json!(operations)) {
        capabilities.insert("messages".into(), value);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn directory(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("ab-advertise-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn nothing_is_advertised_without_a_proof_for_this_os_build() {
        let reports = directory("none");
        let (operations, coverage) = advertised(&reports, "build-1");
        assert_eq!(operations, Vec::<&str>::new());
        assert_eq!(coverage[0]["reason"], "route_proof_required");
        fs::write(
            reports.join("messages-command-proofs.json"),
            json!({"os_build": "old", "routes": ["x:text"]}).to_string(),
        )
        .unwrap();
        assert_eq!(advertised(&reports, "build-1").0, Vec::<&str>::new());
        let _ = fs::remove_dir_all(reports);
    }

    #[test]
    fn each_operation_needs_its_own_route_and_a_failed_step_explains_a_missing_one() {
        let reports = directory("some");
        fs::write(
            reports.join("messages-command-proofs.json"),
            json!({"os_build": "b", "routes": ["x:attachment", "x:messages.edit"]}).to_string(),
        )
        .unwrap();
        fs::write(
            reports.join("messages-selftest.json"),
            json!({"os_build": "b", "steps": [{"operation": "messages.unsend", "proven": false, "reason": "late"}]}).to_string(),
        )
        .unwrap();
        let (operations, coverage) = advertised(&reports, "b");
        assert_eq!(operations, ["messages.send", "messages.edit"]);
        let unsend = coverage
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["operation"] == "messages.unsend")
            .unwrap();
        assert_eq!(unsend["proven"], false);
        assert_eq!(unsend["reason"], "late");
        let _ = fs::remove_dir_all(reports);
    }

    #[test]
    fn a_peer_with_no_paths_keeps_its_own_list() {
        let raw =
            |value: Value| serde_json::value::RawValue::from_string(value.to_string()).unwrap();
        let mut coverage = BTreeMap::from([(
            "messages".to_owned(),
            raw(json!({"fields": "complete", "operations": ["sim"]})),
        )]);
        let mut capabilities =
            BTreeMap::from([("messages".to_owned(), raw(json!(["messages.send"])))]);
        apply(&mut coverage, &mut capabilities);
        assert_eq!(capabilities["messages"].get(), r#"["messages.send"]"#);
        assert!(coverage["messages"].get().contains("sim"));
    }

    #[test]
    fn the_paths_never_reach_the_gateway() {
        let reports = directory("strip");
        let raw =
            |value: Value| serde_json::value::RawValue::from_string(value.to_string()).unwrap();
        let mut coverage = BTreeMap::from([(
            "messages".to_owned(),
            raw(
                json!({"fields": "complete", "advertise": {"report_dir": reports, "os_build": "b"}}),
            ),
        )]);
        let mut capabilities = BTreeMap::new();
        apply(&mut coverage, &mut capabilities);
        assert!(!coverage["messages"].get().contains("advertise"));
        assert!(coverage["messages"].get().contains("operations"));
        assert_eq!(capabilities["messages"].get(), "[]");
        let _ = fs::remove_dir_all(reports);
    }
}
