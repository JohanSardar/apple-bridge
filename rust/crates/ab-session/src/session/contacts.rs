use super::owned::{
    Driver, Probe, Refusal, Settled, conflict, equal, native_failure, refuse, text,
};
use serde_json::{Value, json};
use std::{thread, time::Duration};

const CONTACT_OPERATIONS: [&str; 3] = ["contacts.create", "contacts.update", "contacts.delete"];
const GROUP_OPERATIONS: [&str; 5] = [
    "contacts.create_group",
    "contacts.rename_group",
    "contacts.delete_group",
    "contacts.add_member",
    "contacts.remove_member",
];
const OBSERVATIONS: u32 = 20;
const OBSERVATION_GAP: Duration = Duration::from_millis(50);

fn is_group(operation: &str) -> bool {
    GROUP_OPERATIONS.contains(&operation)
}

fn present(read: &Value) -> bool {
    read["exists"].as_bool().unwrap_or(false)
}

fn call(probe: &dyn Probe, operation: &str, args: &Value) -> Result<Value, Refusal> {
    probe
        .call(operation, args)
        .map_err(|code| native_failure(&code))
}

fn read_contact(probe: &dyn Probe, key: &str) -> Result<Value, Refusal> {
    let contact = call(probe, "contacts.read_contact", &json!({"native_key": key}))?;
    if present(&contact) {
        Ok(contact)
    } else {
        Err(conflict(key))
    }
}

fn read_group(probe: &dyn Probe, key: &str, observer: bool) -> Result<Value, Refusal> {
    call(
        probe,
        "contacts.read_group",
        &json!({"native_key": key, "observer": observer}),
    )
}

fn existing_group(probe: &dyn Probe, key: &str) -> Result<Value, Refusal> {
    let group = read_group(probe, key, false)?;
    if present(&group) {
        Ok(group)
    } else {
        Err(conflict(key))
    }
}

fn default_container(probe: &dyn Probe) -> Result<(), Refusal> {
    let can = call(probe, "contacts.can_create", &json!({}))?;
    if can["allowed"] == true {
        Ok(())
    } else {
        Err(refuse("unsupported"))
    }
}

fn require_capability(probe: &dyn Probe, operation: &str) -> Result<(), Refusal> {
    let granted = call(probe, "contacts.group_capabilities", &json!({}))?;
    let proven = granted["operations"]
        .as_array()
        .is_some_and(|list| list.iter().any(|item| item == operation));
    if proven {
        Ok(())
    } else {
        Err(refuse("capability_unproven"))
    }
}

pub(super) fn prepare(
    probe: &dyn Probe,
    action: &Value,
    expected_state: &Value,
) -> Result<Value, Refusal> {
    let operation = text(action, "op");
    if !CONTACT_OPERATIONS.contains(&operation) && !is_group(operation) {
        return Err(refuse("unsupported"));
    }
    if is_group(operation) {
        return prepare_group(probe, action, expected_state);
    }
    let key = text(action, "native_key");
    let mut before = Value::Null;
    if operation == "contacts.create" {
        default_container(probe)?;
    } else {
        let contact = read_contact(probe, key)?;
        before = contact["data"].clone();
        if !equal(&before, expected_state) {
            return Err(conflict(key));
        }
        if before["writable"] != true {
            return Err(refuse("unsupported"));
        }
    }
    call(
        probe,
        "contacts.check_patch",
        &json!({"args": action["args"]}),
    )?;
    Ok(before)
}

fn prepare_group(
    probe: &dyn Probe,
    action: &Value,
    expected_state: &Value,
) -> Result<Value, Refusal> {
    let operation = text(action, "op");
    require_capability(probe, operation)?;
    let key = text(action, "native_key");
    let before = if operation == "contacts.create_group" {
        default_container(probe)?;
        Value::Null
    } else {
        let before = existing_group(probe, key)?["data"].clone();
        if !equal(&before, expected_state) {
            return Err(conflict(key));
        }
        if before["writable"] != true {
            return Err(refuse("unsupported"));
        }
        before
    };
    validate_group(probe, action, &before)?;
    Ok(before)
}

fn validate_group(probe: &dyn Probe, action: &Value, state: &Value) -> Result<(), Refusal> {
    let operation = text(action, "op");
    require_capability(probe, operation)?;
    let key = text(action, "native_key");
    if operation != "contacts.create_group" {
        let current = existing_group(probe, key)?["data"].clone();
        if !equal(&current, state) {
            return Err(conflict(key));
        }
        if current["writable"] != true {
            return Err(refuse("unsupported"));
        }
    }
    if matches!(operation, "contacts.create_group" | "contacts.rename_group") {
        let name = text(&action["args"], "name");
        if name.trim().is_empty() || name.len() > 256 {
            return Err(refuse("invalid"));
        }
    }
    if matches!(operation, "contacts.add_member" | "contacts.remove_member") {
        let member = read_contact(probe, text(action, "list_key"))?;
        if member["data"]["container_id"] != state["container_id"] {
            return Err(refuse("unsupported"));
        }
    }
    Ok(())
}

pub(super) fn validate(
    probe: &dyn Probe,
    action: &Value,
    native_state: &Value,
) -> Result<(), Refusal> {
    let operation = text(action, "op");
    if is_group(operation) {
        return validate_group(probe, action, native_state);
    }
    if operation != "contacts.create" {
        let key = text(action, "native_key");
        let current = read_contact(probe, key)?["data"].clone();
        if !equal(&current, native_state) {
            return Err(conflict(key));
        }
        if current["writable"] != true {
            return Err(refuse("unsupported"));
        }
    }
    Ok(())
}

fn group_effect(probe: &dyn Probe, action: &Value, key: &str) -> Result<bool, String> {
    let operation = text(action, "op");
    let group = probe.call(
        "contacts.read_group",
        &json!({"native_key": key, "observer": true}),
    )?;
    if operation == "contacts.delete_group" {
        return Ok(!present(&group));
    }
    if !present(&group) {
        return Ok(false);
    }
    if matches!(operation, "contacts.create_group" | "contacts.rename_group") {
        return Ok(group["data"]["name"] == action["args"]["name"]);
    }
    let member = text(action, "list_key");
    let listed = group["data"]["member_ids"].as_array().is_some_and(|ids| {
        ids.iter()
            .any(|id| id["$ref"]["native_key"].as_str() == Some(member))
    });
    Ok(listed == (operation == "contacts.add_member"))
}

pub(super) fn write(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    let operation = text(action, "op");
    if !is_group(operation) {
        return probe.call(
            "contacts.write",
            &json!({
                "op": action["op"],
                "native_key": action["native_key"],
                "args": action["args"],
                "command_id": command_id,
            }),
        );
    }
    let written = probe.call(
        "contacts.write_group",
        &json!({
            "op": action["op"],
            "native_key": action["native_key"],
            "list_key": action["list_key"],
            "args": action["args"],
            "command_id": command_id,
        }),
    )?;
    let key = text(&written, "native_key").to_owned();
    let mut observed = false;
    for attempt in 0..OBSERVATIONS {
        if group_effect(probe, action, &key)? {
            observed = true;
            break;
        }
        if attempt + 1 < OBSERVATIONS {
            thread::sleep(OBSERVATION_GAP);
        }
    }
    if !observed {
        return Err("unavailable".into());
    }
    probe.call("contacts.reconcile", &json!({}))?;
    Ok(json!({"native_key": key, "observed": true}))
}

fn settle(operation: &str, written: &Value) -> Option<Settled> {
    if text(written, "native_key").is_empty() {
        return None;
    }
    let confirmed = if is_group(operation) {
        written["observed"] == true
    } else if operation == "contacts.delete" {
        written["exists_after"] == false
    } else {
        written["observed"].is_object() && equal(&written["observed"], &written["saved"])
    };
    confirmed.then(|| Settled {
        disposition: "locally_observed",
        evidence: json!({
            "native_key": written["native_key"],
            "observed": true,
            "absent": matches!(operation, "contacts.delete" | "contacts.delete_group"),
        }),
    })
}

pub(super) fn resolved(probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let prepared = &evidence["prepared"];
    let action = &prepared["action"];
    let operation = text(action, "op");
    let key = match text(action, "native_key") {
        "" => text(evidence, "native_key"),
        key => key,
    };
    if key.is_empty() || !(CONTACT_OPERATIONS.contains(&operation) || is_group(operation)) {
        return None;
    }
    if is_group(operation) {
        if !group_effect(probe, action, key).ok()? {
            return None;
        }
    } else if operation == "contacts.delete" {
        let contact = probe
            .call("contacts.read_contact", &json!({"native_key": key}))
            .ok()?;
        if present(&contact) {
            return None;
        }
    } else {
        let compared = probe
            .call(
                "contacts.goal",
                &json!({"native_key": key, "args": action["args"]}),
            )
            .ok()?;
        if !compared["actual"].is_object() || !equal(&compared["actual"], &compared["desired"]) {
            return None;
        }
    }
    Some(json!({
        "native_key": key,
        "observed": true,
        "absent": matches!(operation, "contacts.delete" | "contacts.delete_group"),
        "resolved": true,
        "fingerprint": prepared["request_hash"],
        "prepared": prepared,
    }))
}

fn reconcile_args(found: &Value, _operation: &str) -> Value {
    if found["absent"] == true {
        json!({"absent_key": found["native_key"]})
    } else {
        json!({})
    }
}

pub(super) const DRIVER: Driver = Driver {
    name: "contacts",
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
        contact: Value,
        group: Value,
        proven: Vec<&'static str>,
        patch: Result<Value, &'static str>,
        group_after: RefCell<Vec<Value>>,
    }

    fn contact_data(given: &str) -> Value {
        json!({"given_name": given, "writable": true, "container_id": "main"})
    }

    fn group_data(name: &str) -> Value {
        json!({"name": name, "writable": true, "container_id": "main", "member_ids": []})
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                contact: contact_data("Ada"),
                group: group_data("Team"),
                proven: GROUP_OPERATIONS.to_vec(),
                patch: Ok(json!({})),
                group_after: RefCell::new(Vec::new()),
            }
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push(operation.into());
            let key = args["native_key"].as_str().unwrap_or_default();
            Ok(match operation {
                "contacts.read_contact" if key == "c1" || key == "m1" => {
                    json!({"exists": true, "data": self.contact})
                }
                "contacts.read_group" if args["observer"] == true => {
                    let mut later = self.group_after.borrow_mut();
                    if later.is_empty() {
                        json!({"exists": true, "data": self.group})
                    } else {
                        later.remove(0)
                    }
                }
                "contacts.read_group" if key == "g1" => json!({"exists": true, "data": self.group}),
                "contacts.read_contact" | "contacts.read_group" => json!({"exists": false}),
                "contacts.can_create" => json!({"allowed": true}),
                "contacts.check_patch" => return self.patch.clone().map_err(str::to_owned),
                "contacts.group_capabilities" => json!({"operations": self.proven}),
                "contacts.write_group" => json!({"native_key": "g1"}),
                _ => Value::Null,
            })
        }
    }

    fn update() -> Value {
        json!({"op": "contacts.update", "native_key": "c1", "args": {"given_name": "Grace"}})
    }

    #[test]
    fn a_matching_contact_prepares() {
        let state = prepare(&Fake::new(), &update(), &contact_data("Ada")).unwrap();
        assert_eq!(state["given_name"], "Ada");
    }

    #[test]
    fn stale_missing_read_only_and_invalid_requests_are_refused() {
        let stale = prepare(&Fake::new(), &update(), &contact_data("Other")).unwrap_err();
        assert_eq!(
            (stale.code.as_str(), stale.key.as_deref()),
            ("conflict", Some("c1"))
        );
        let gone = json!({"op": "contacts.update", "native_key": "gone", "args": {}});
        assert_eq!(
            prepare(&Fake::new(), &gone, &Value::Null).unwrap_err().code,
            "conflict"
        );
        let mut readonly = Fake::new();
        readonly.contact["writable"] = json!(false);
        assert_eq!(
            prepare(&readonly, &update(), &readonly.contact)
                .unwrap_err()
                .code,
            "unsupported"
        );
        let mut invalid = Fake::new();
        invalid.patch = Err("invalid");
        assert_eq!(
            prepare(&invalid, &update(), &contact_data("Ada"))
                .unwrap_err()
                .code,
            "invalid"
        );
        let unknown = json!({"op": "contacts.explode"});
        assert_eq!(
            prepare(&Fake::new(), &unknown, &Value::Null)
                .unwrap_err()
                .code,
            "unsupported"
        );
        let create = json!({"op": "contacts.create", "args": {"given_name": "A"}});
        assert!(prepare(&Fake::new(), &create, &Value::Null).is_ok());
    }

    #[test]
    fn drift_between_prepare_and_dispatch_is_a_conflict() {
        let mut probe = Fake::new();
        let state = prepare(&probe, &update(), &contact_data("Ada")).unwrap();
        assert!(validate(&probe, &update(), &state).is_ok());
        probe.contact["given_name"] = json!("Edited");
        assert_eq!(
            validate(&probe, &update(), &state).unwrap_err().code,
            "conflict"
        );
    }

    #[test]
    fn group_operations_need_a_proven_capability() {
        let mut probe = Fake::new();
        probe.proven = vec!["contacts.rename_group"];
        let rename =
            json!({"op": "contacts.rename_group", "native_key": "g1", "args": {"name": "Crew"}});
        assert!(prepare(&probe, &rename, &group_data("Team")).is_ok());
        let delete = json!({"op": "contacts.delete_group", "native_key": "g1"});
        assert_eq!(
            prepare(&probe, &delete, &group_data("Team"))
                .unwrap_err()
                .code,
            "capability_unproven"
        );
        let state = group_data("Team");
        probe.proven = Vec::new();
        assert_eq!(
            validate(&probe, &rename, &state).unwrap_err().code,
            "capability_unproven"
        );
    }

    #[test]
    fn group_names_members_and_state_are_validated() {
        let probe = Fake::new();
        for name in ["", "   ", &"x".repeat(257)] {
            let create = json!({"op": "contacts.create_group", "args": {"name": name}});
            assert_eq!(
                prepare(&probe, &create, &Value::Null).unwrap_err().code,
                "invalid"
            );
        }
        let create = json!({"op": "contacts.create_group", "args": {"name": "Crew"}});
        assert!(prepare(&probe, &create, &Value::Null).is_ok());
        let add = json!({"op": "contacts.add_member", "native_key": "g1", "list_key": "m1"});
        assert!(prepare(&probe, &add, &group_data("Team")).is_ok());
        let mut other = Fake::new();
        other.contact["container_id"] = json!("elsewhere");
        assert_eq!(
            prepare(&other, &add, &group_data("Team")).unwrap_err().code,
            "unsupported"
        );
        let missing =
            json!({"op": "contacts.add_member", "native_key": "g1", "list_key": "nobody"});
        assert_eq!(
            prepare(&probe, &missing, &group_data("Team"))
                .unwrap_err()
                .code,
            "conflict"
        );
        assert_eq!(
            prepare(&probe, &add, &group_data("Stale"))
                .unwrap_err()
                .code,
            "conflict"
        );
        let mut readonly = Fake::new();
        readonly.group["writable"] = json!(false);
        assert_eq!(
            prepare(&readonly, &add, &readonly.group).unwrap_err().code,
            "unsupported"
        );
    }

    #[test]
    fn a_group_write_is_confirmed_only_by_an_independent_read() {
        let probe = Fake::new();
        let rename =
            json!({"op": "contacts.rename_group", "native_key": "g1", "args": {"name": "Team"}});
        let written = write(&probe, "c", &rename, &Value::Null).unwrap();
        assert_eq!(written["observed"], true);
        assert!(
            probe
                .calls
                .borrow()
                .iter()
                .any(|call| call == "contacts.reconcile")
        );
        let other = Fake::new();
        let never = json!({"op": "contacts.rename_group", "native_key": "g1", "args": {"name": "Elsewhere"}});
        assert_eq!(
            write(&other, "c", &never, &Value::Null).unwrap_err(),
            "unavailable"
        );
        let settling = Fake::new();
        settling
            .group_after
            .borrow_mut()
            .push(json!({"exists": true, "data": group_data("Old")}));
        assert!(write(&settling, "c", &json!({"op": "contacts.rename_group", "native_key": "g1", "args": {"name": "Team"}}), &Value::Null).is_ok());
        let add = json!({"op": "contacts.add_member", "native_key": "g1", "list_key": "m1"});
        let mut member = Fake::new();
        member.group["member_ids"] =
            json!([{"$ref": {"collection": "contacts.contact", "native_key": "m1"}}]);
        assert!(write(&member, "c", &add, &Value::Null).is_ok());
        let remove = json!({"op": "contacts.remove_member", "native_key": "g1", "list_key": "m1"});
        assert_eq!(
            write(&member, "c", &remove, &Value::Null).unwrap_err(),
            "unavailable"
        );
    }

    #[test]
    fn the_read_back_decides_contact_outcomes() {
        let saved = contact_data("Grace");
        let good =
            json!({"native_key": "c1", "saved": saved, "observed": saved, "exists_after": true});
        assert!(settle("contacts.update", &good).is_some());
        let differs = json!({"native_key": "c1", "saved": saved, "observed": contact_data("Ada"), "exists_after": true});
        assert!(settle("contacts.update", &differs).is_none());
        let unseen =
            json!({"native_key": "c1", "saved": saved, "observed": null, "exists_after": false});
        assert!(settle("contacts.update", &unseen).is_none());
        assert!(
            settle(
                "contacts.update",
                &json!({"saved": saved, "observed": saved})
            )
            .is_none()
        );
        let deleted = json!({"native_key": "c1", "exists_after": false});
        assert_eq!(
            settle("contacts.delete", &deleted).unwrap().evidence["absent"],
            true
        );
        assert!(
            settle(
                "contacts.delete",
                &json!({"native_key": "c1", "exists_after": true})
            )
            .is_none()
        );
        assert!(
            settle(
                "contacts.add_member",
                &json!({"native_key": "g1", "observed": true})
            )
            .is_some()
        );
        assert!(settle("contacts.add_member", &json!({"native_key": "g1"})).is_none());
    }
}
