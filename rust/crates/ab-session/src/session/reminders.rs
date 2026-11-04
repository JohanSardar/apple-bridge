use super::owned::{
    Driver, Probe, Refusal, Settled, conflict, equal, native_failure, refuse, text,
};
use serde_json::{Map, Value, json};

const OPERATIONS: [&str; 8] = [
    "reminders.create",
    "reminders.update",
    "reminders.complete",
    "reminders.uncomplete",
    "reminders.delete",
    "reminders.create_list",
    "reminders.rename_list",
    "reminders.delete_list",
];
const LIST_OPERATIONS: [&str; 3] = [
    "reminders.create_list",
    "reminders.rename_list",
    "reminders.delete_list",
];
const LIST_EDITS: [&str; 2] = ["reminders.rename_list", "reminders.delete_list"];
const PROJECTION: [&str; 28] = [
    "store_identifier",
    "flagged",
    "parent_id",
    "section_id",
    "tags",
    "manual_order",
    "created_at",
    "early_reminders",
    "assignment",
    "store_alarms",
    "store_recurrence",
    "attachment_ids",
    "badge_emblem",
    "is_group",
    "display_order",
    "pinned",
    "pinned_at",
    "sorting_style",
    "categorize_groceries",
    "grocery_locale",
    "sharing_status",
    "shared_owner_name",
    "smart_list_type",
    "smart_filter",
    "sections",
    "store_color",
    "store_status",
    "store_coverage",
];

fn base(data: &Value) -> Map<String, Value> {
    data.as_object()
        .map(|fields| {
            fields
                .iter()
                .filter(|(key, _)| !PROJECTION.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn without_list(mut fields: Map<String, Value>) -> Map<String, Value> {
    fields.remove("list_id");
    fields
}

fn read(probe: &dyn Probe, operation: &str, key: &str) -> Result<Value, Refusal> {
    probe
        .call(operation, &json!({"native_key": key}))
        .map_err(|code| native_failure(&code))
}

fn present(read: &Value) -> bool {
    read["exists"].as_bool().unwrap_or(false)
}

fn writable(read: &Value) -> bool {
    read["writable"].as_bool().unwrap_or(false)
}

fn writable_list(probe: &dyn Probe, key: &str) -> Result<Value, Refusal> {
    let list = read(probe, "reminders.read_list", key)?;
    if present(&list) && writable(&list) {
        Ok(list)
    } else {
        Err(refuse("unsupported"))
    }
}

pub(super) fn prepare(
    probe: &dyn Probe,
    action: &Value,
    expected_state: &Value,
) -> Result<Value, Refusal> {
    let operation = text(action, "op");
    if !OPERATIONS.contains(&operation) {
        return Err(refuse("unsupported"));
    }
    let key = text(action, "native_key");
    let mut before = Value::Null;
    if !key.is_empty() {
        if LIST_EDITS.contains(&operation) {
            let list = writable_list(probe, key)?;
            before = list["data"].clone();
            if !equal(&before, &Value::Object(base(expected_state))) {
                return Err(conflict(key));
            }
        } else {
            let reminder = read(probe, "reminders.read_reminder", key)?;
            if !present(&reminder) {
                return Err(conflict(key));
            }
            before = reminder["data"].clone();
            let expected_list = text(action, "expected_list_key");
            if !expected_list.is_empty() && text(&reminder, "list_key") != expected_list {
                return Err(conflict(key));
            }
            let current = Value::Object(without_list(base(&before)));
            if !equal(&Value::Object(without_list(base(expected_state))), &current) {
                return Err(conflict(key));
            }
            if !writable(&reminder) {
                return Err(refuse("unsupported"));
            }
        }
    }
    if !LIST_OPERATIONS.contains(&operation) {
        writable_list(probe, text(action, "list_key"))?;
    }
    if operation == "reminders.create_list" {
        let can = probe
            .call("reminders.can_create_list", &json!({}))
            .map_err(|code| native_failure(&code))?;
        if can["allowed"] != true {
            return Err(refuse("unsupported"));
        }
    }
    Ok(before)
}

pub(super) fn validate(
    probe: &dyn Probe,
    action: &Value,
    native_state: &Value,
) -> Result<(), Refusal> {
    let operation = text(action, "op");
    if !LIST_OPERATIONS.contains(&operation) {
        writable_list(probe, text(action, "list_key"))?;
    }
    let key = text(action, "native_key");
    if key.is_empty() {
        return Ok(());
    }
    if LIST_EDITS.contains(&operation) {
        let list = writable_list(probe, key)?;
        return if equal(&list["data"], native_state) {
            Ok(())
        } else {
            Err(conflict(key))
        };
    }
    let current = read(probe, "reminders.read_reminder", key)?;
    if !present(&current) || !equal(&current["data"], native_state) {
        return Err(conflict(key));
    }
    if !writable(&current) {
        return Err(refuse("unsupported"));
    }
    Ok(())
}

pub(super) fn write(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    probe.call(
        "reminders.write",
        &json!({
            "op": action["op"],
            "native_key": action["native_key"],
            "list_key": action["list_key"],
            "args": action["args"],
            "command_id": command_id,
        }),
    )
}

pub(super) fn verified(operation: &str, written: &Value) -> bool {
    if text(written, "native_key").is_empty() {
        return false;
    }
    let observed = &written["observed"];
    match operation {
        "reminders.delete" | "reminders.delete_list" => {
            written["exists_after"] == false && observed.is_null()
        }
        "reminders.create_list" | "reminders.rename_list" => {
            observed.is_object() && observed["title"] == written["saved"]["title"]
        }
        _ => observed.is_object() && equal(observed, &written["saved"]),
    }
}

pub(super) fn absent(operation: &str) -> bool {
    matches!(operation, "reminders.delete" | "reminders.delete_list")
}

pub(super) fn resolved(probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let prepared = &evidence["prepared"];
    let action = &prepared["action"];
    let operation = text(action, "op");
    let key = match text(action, "native_key") {
        "" => text(evidence, "native_key"),
        key => key,
    };
    if key.is_empty() || !OPERATIONS.contains(&operation) {
        return None;
    }
    let args = &action["args"];
    let mut gone = false;
    match operation {
        "reminders.delete_list" => {
            if present(&read(probe, "reminders.read_list", key).ok()?) {
                return None;
            }
            gone = true;
        }
        "reminders.create_list" | "reminders.rename_list" => {
            let list = read(probe, "reminders.read_list", key).ok()?;
            if !present(&list) || list["data"]["title"] != args["title"] {
                return None;
            }
        }
        _ => {
            let current = read(probe, "reminders.read_reminder", key).ok()?;
            if operation == "reminders.delete" {
                if present(&current) {
                    return None;
                }
                gone = true;
            } else {
                if !present(&current) {
                    return None;
                }
                let actual = &current["data"];
                let completed = actual["completed"] == true;
                if (operation == "reminders.complete" && !completed)
                    || (operation == "reminders.uncomplete" && completed)
                {
                    return None;
                }
                if matches!(operation, "reminders.update" | "reminders.create") {
                    let fields: Vec<_> = args
                        .as_object()?
                        .iter()
                        .filter(|(name, _)| {
                            !matches!(name.as_str(), "id" | "list_id" | "due_all_day")
                        })
                        .collect();
                    if fields.is_empty()
                        || !fields
                            .iter()
                            .all(|(name, value)| field_matches(actual, name, value))
                    {
                        return None;
                    }
                }
            }
        }
    }
    Some(json!({
        "native_key": key,
        "observed": true,
        "absent": gone,
        "resolved": true,
        "fingerprint": prepared["request_hash"],
        "prepared": prepared,
    }))
}

fn field_matches(actual: &Value, name: &str, value: &Value) -> bool {
    let clears = matches!(name, "due" | "start" | "url" | "notes") && value == "";
    let expected = if clears { &Value::Null } else { value };
    if name == "notes" && expected.is_null() {
        return actual[name].is_null() || actual[name] == "";
    }
    equal(&actual[name], expected)
}

fn settle(operation: &str, written: &Value) -> Option<Settled> {
    verified(operation, written).then(|| Settled {
        disposition: "locally_observed",
        evidence: json!({
            "native_key": written["native_key"],
            "observed": true,
            "absent": absent(operation),
        }),
    })
}

fn reconcile_args(found: &Value, operation: &str) -> Value {
    if found["absent"] == true {
        json!({"absent_key": found["native_key"], "list": operation == "reminders.delete_list"})
    } else {
        json!({})
    }
}

pub(super) const DRIVER: Driver = Driver {
    name: "reminders",
    prepare,
    prepare_command: None,
    validate,
    write,
    settle,
    resolved,
    reconcile_args,
};

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        calls: RefCell<Vec<String>>,
        lists: Value,
        reminders: Value,
        can_create_list: bool,
        fail: Option<(&'static str, &'static str)>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                lists: json!({"work": {"title": "Work", "account": "a", "color": "c", "writable": true}}),
                reminders: json!({"r1": {"title": "milk", "notes": null, "priority": 0, "completed": false,
                    "list_id": {"$ref": {"collection": "reminders.list", "native_key": "work"}}}}),
                can_create_list: true,
                fail: None,
            }
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push(operation.into());
            if let Some((failing, code)) = self.fail
                && failing == operation
            {
                return Err(code.into());
            }
            let key = args["native_key"].as_str().unwrap_or_default();
            Ok(match operation {
                "reminders.read_list" => match self.lists.get(key) {
                    Some(data) => {
                        json!({"exists": true, "writable": data["writable"], "data": data})
                    }
                    None => json!({"exists": false}),
                },
                "reminders.read_reminder" => match self.reminders.get(key) {
                    Some(data) => {
                        json!({"exists": true, "writable": true, "list_key": "work", "data": data})
                    }
                    None => json!({"exists": false}),
                },
                "reminders.can_create_list" => json!({"allowed": self.can_create_list}),
                _ => Value::Null,
            })
        }
    }

    fn update(extra: &Value) -> Value {
        let mut action = json!({"op": "reminders.update", "native_key": "r1", "list_key": "work", "args": {"title": "oat milk"}});
        for (key, value) in extra.as_object().unwrap() {
            action[key] = value.clone();
        }
        action
    }

    fn expected() -> Value {
        json!({"title": "milk", "notes": null, "priority": 0, "completed": false,
            "list_id": {"$ref": {"collection": "reminders.list", "native_key": "other"}},
            "flagged": true, "tags": ["x"]})
    }

    #[test]
    fn a_matching_reminder_prepares_and_projection_fields_do_not_count() {
        let probe = Fake::new();
        let state = prepare(&probe, &update(&json!({})), &expected()).unwrap();
        assert_eq!(state["title"], "milk");
    }

    #[test]
    fn a_changed_reminder_is_a_conflict_that_names_the_key() {
        let probe = Fake::new();
        let mut stale = expected();
        stale["title"] = json!("bread");
        assert_eq!(
            prepare(&probe, &update(&json!({})), &stale),
            Err(Refusal {
                code: "conflict".into(),
                key: Some("r1".into()),
                message: None,
            })
        );
    }

    #[test]
    fn a_missing_reminder_is_a_conflict() {
        let probe = Fake::new();
        let action = update(&json!({"native_key": "gone"}));
        assert_eq!(
            prepare(&probe, &action, &expected()).unwrap_err().code,
            "conflict"
        );
    }

    #[test]
    fn a_moved_reminder_is_a_conflict_only_when_a_list_was_expected() {
        let probe = Fake::new();
        let moved = update(&json!({"expected_list_key": "elsewhere"}));
        assert_eq!(
            prepare(&probe, &moved, &expected()).unwrap_err().code,
            "conflict"
        );
        let same = update(&json!({"expected_list_key": "work"}));
        assert!(prepare(&probe, &same, &expected()).is_ok());
    }

    #[test]
    fn a_read_only_list_is_unsupported() {
        let mut probe = Fake::new();
        probe.lists = json!({"work": {"title": "Work", "writable": false}});
        assert_eq!(
            prepare(&probe, &update(&json!({})), &expected())
                .unwrap_err()
                .code,
            "unsupported"
        );
        let create = json!({"op": "reminders.create", "list_key": "work", "args": {"title": "x"}});
        assert_eq!(
            prepare(&probe, &create, &Value::Null).unwrap_err().code,
            "unsupported"
        );
    }

    #[test]
    fn an_unknown_operation_is_unsupported_before_any_read() {
        let probe = Fake::new();
        let action = json!({"op": "reminders.explode"});
        assert_eq!(
            prepare(&probe, &action, &Value::Null).unwrap_err().code,
            "unsupported"
        );
        assert!(probe.calls.borrow().is_empty());
    }

    #[test]
    fn list_edits_compare_the_list_and_creating_a_list_needs_a_default_account() {
        let probe = Fake::new();
        let rename =
            json!({"op": "reminders.rename_list", "native_key": "work", "args": {"title": "Home"}});
        let current = json!({"title": "Work", "account": "a", "color": "c", "writable": true, "sections": []});
        assert_eq!(prepare(&probe, &rename, &current).unwrap()["title"], "Work");
        let stale = json!({"title": "Old", "account": "a", "color": "c", "writable": true});
        assert_eq!(
            prepare(&probe, &rename, &stale).unwrap_err().code,
            "conflict"
        );
        let mut none = Fake::new();
        none.can_create_list = false;
        let create = json!({"op": "reminders.create_list", "args": {"title": "x"}});
        assert_eq!(
            prepare(&none, &create, &Value::Null).unwrap_err().code,
            "unsupported"
        );
        assert!(prepare(&probe, &create, &Value::Null).is_ok());
    }

    #[test]
    fn a_native_failure_never_becomes_a_decision() {
        let mut probe = Fake::new();
        probe.fail = Some(("reminders.read_reminder", "permission_missing"));
        assert_eq!(
            prepare(&probe, &update(&json!({})), &expected())
                .unwrap_err()
                .code,
            "unavailable"
        );
        probe.fail = Some(("reminders.read_reminder", "anything_else"));
        assert_eq!(
            prepare(&probe, &update(&json!({})), &expected())
                .unwrap_err()
                .code,
            "unavailable"
        );
    }

    #[test]
    fn dispatch_validation_rejects_drift_since_prepare() {
        let mut probe = Fake::new();
        let action = update(&json!({}));
        let state = prepare(&probe, &action, &expected()).unwrap();
        assert!(validate(&probe, &action, &state).is_ok());
        probe.reminders["r1"]["title"] = json!("edited meanwhile");
        assert_eq!(
            validate(&probe, &action, &state).unwrap_err().code,
            "conflict"
        );
        probe.reminders = json!({});
        assert_eq!(
            validate(&probe, &action, &state).unwrap_err().code,
            "conflict"
        );
        probe.lists = json!({"work": {"title": "Work", "writable": false}});
        probe.reminders = json!({"r1": {"title": "milk"}});
        assert_eq!(
            validate(&probe, &action, &json!({"title": "milk"}))
                .unwrap_err()
                .code,
            "unsupported"
        );
    }

    #[test]
    fn resolve_observes_only_what_the_framework_shows() {
        let mut probe = Fake::new();
        let evidence = |op: &str, args: Value| json!({"native_key": "r1", "prepared": {"request_hash": "h", "action": {"op": op, "native_key": "r1", "args": args}}});
        let done = evidence("reminders.update", json!({"title": "milk"}));
        let seen = resolved(&probe, &done).unwrap();
        assert_eq!(seen["observed"], true);
        assert_eq!(seen["fingerprint"], "h");
        assert_eq!(seen["absent"], false);
        let undone = evidence("reminders.update", json!({"title": "oat milk"}));
        assert!(resolved(&probe, &undone).is_none());
        let nothing = evidence("reminders.update", json!({}));
        assert!(resolved(&probe, &nothing).is_none());
        let complete = evidence("reminders.complete", json!({}));
        assert!(resolved(&probe, &complete).is_none());
        probe.reminders["r1"]["completed"] = json!(true);
        assert!(resolved(&probe, &complete).is_some());
        assert!(resolved(&probe, &evidence("reminders.uncomplete", json!({}))).is_none());
        let delete = evidence("reminders.delete", json!({}));
        assert!(resolved(&probe, &delete).is_none());
        probe.reminders = json!({});
        assert_eq!(resolved(&probe, &delete).unwrap()["absent"], true);
        assert!(resolved(&probe, &complete).is_none());
        let rename = evidence("reminders.rename_list", json!({"title": "Work"}));
        let mut rename = rename;
        rename["native_key"] = json!("work");
        rename["prepared"]["action"]["native_key"] = json!("work");
        assert!(resolved(&probe, &rename).is_some());
        rename["prepared"]["action"]["args"]["title"] = json!("Other");
        assert!(resolved(&probe, &rename).is_none());
        let mut gone = evidence("reminders.delete_list", json!({}));
        gone["prepared"]["action"]["native_key"] = json!("missing");
        assert_eq!(resolved(&probe, &gone).unwrap()["absent"], true);
        let keyless =
            json!({"prepared": {"action": {"op": "reminders.create", "args": {"title": "x"}}}});
        assert!(resolved(&probe, &keyless).is_none());
    }

    #[test]
    fn numbers_compare_by_value() {
        assert!(equal(
            &json!({"p": 0, "n": [1.0]}),
            &json!({"p": 0.0, "n": [1]})
        ));
        assert!(!equal(&json!({"p": 1}), &json!({"p": 2})));
    }

    #[test]
    fn read_back_decides_success() {
        let data = json!({"title": "oat milk"});
        let good =
            json!({"native_key": "r1", "saved": data, "observed": data, "exists_after": true});
        assert!(verified("reminders.update", &good));
        let differs = json!({"native_key": "r1", "saved": data, "observed": {"title": "milk"}, "exists_after": true});
        assert!(!verified("reminders.update", &differs));
        let unseen =
            json!({"native_key": "r1", "saved": data, "observed": null, "exists_after": false});
        assert!(!verified("reminders.update", &unseen));
        assert!(!verified(
            "reminders.update",
            &json!({"saved": data, "observed": data})
        ));
        let deleted =
            json!({"native_key": "r1", "saved": null, "observed": null, "exists_after": false});
        assert!(verified("reminders.delete", &deleted));
        let still =
            json!({"native_key": "r1", "saved": null, "observed": data, "exists_after": true});
        assert!(!verified("reminders.delete", &still));
        let listed = json!({"native_key": "l", "saved": {"title": "A", "x": 1}, "observed": {"title": "A", "x": 2}, "exists_after": true});
        assert!(verified("reminders.create_list", &listed));
        let retitled = json!({"native_key": "l", "saved": {"title": "A"}, "observed": {"title": "B"}, "exists_after": true});
        assert!(!verified("reminders.rename_list", &retitled));
        assert!(absent("reminders.delete_list") && !absent("reminders.update"));
    }
}
