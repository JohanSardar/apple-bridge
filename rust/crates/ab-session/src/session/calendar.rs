use super::owned::{
    Driver, Probe, Refusal, Settled, conflict, equal, native_failure, refuse, text,
};
use serde_json::{Map, Value, json};

const OPERATIONS: [&str; 4] = [
    "calendar.create_event",
    "calendar.update_event",
    "calendar.delete_event",
    "calendar.create_calendar",
];
const ENRICHMENT: [&str; 16] = [
    "store_status",
    "store_coverage",
    "store_identifier_match",
    "travel_time",
    "travel_advisory_behavior",
    "structured_location",
    "invitation_status",
    "owner_participation_status",
    "privacy_level",
    "priority",
    "conference_url",
    "attachments",
    "start_location",
    "attendee_proposals",
    "participant_comments",
    "store_alarm_details",
];

fn request(action: &Value) -> &Value {
    &action["calendar_args"]
}

fn present(read: &Value) -> bool {
    read["exists"].as_bool().unwrap_or(false)
}

fn read(probe: &dyn Probe, operation: &str, key: &str) -> Result<Value, Refusal> {
    probe
        .call(operation, &json!({"native_key": key}))
        .map_err(|code| native_failure(&code))
}

fn fields_without(data: &Value, removed: &[&str]) -> Value {
    let fields: Map<String, Value> = data
        .as_object()
        .map(|fields| {
            fields
                .iter()
                .filter(|(key, _)| !removed.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    Value::Object(fields)
}

fn writable_calendar(probe: &dyn Probe, key: &str) -> Result<(), Refusal> {
    let calendar = read(probe, "calendar.read_calendar", key)?;
    if present(&calendar) && calendar["writable"] == true {
        Ok(())
    } else {
        Err(refuse("unsupported"))
    }
}

fn goal(probe: &dyn Probe, action: &Value, baseline: &Value) -> Result<Value, Refusal> {
    probe
        .call(
            "calendar.goal",
            &json!({
                "native_key": action["native_key"],
                "list_key": action["list_key"],
                "calendar_args": request(action),
                "baseline": baseline,
            }),
        )
        .map_err(|code| native_failure(&code))
}

fn event_read(probe: &dyn Probe, key: &str) -> Result<Value, Refusal> {
    let event = read(probe, "calendar.read_event", key)?;
    if present(&event) {
        Ok(event)
    } else {
        Err(conflict(key))
    }
}

pub(super) fn prepare(
    probe: &dyn Probe,
    action: &Value,
    expected_state: &Value,
) -> Result<Value, Refusal> {
    let operation = text(action, "op");
    if !OPERATIONS.contains(&operation) || request(action)["span"] == "all" {
        return Err(refuse("unsupported"));
    }
    let key = text(action, "native_key");
    let mut before = Value::Null;
    if !key.is_empty() {
        let event = event_read(probe, key)?;
        before = event["data"].clone();
        if event["writable"] != true {
            return Err(refuse("unsupported"));
        }
        if text(action, "expected_list_key") != text(&event, "calendar_key") {
            return Err(conflict(key));
        }
        let mut removed = vec!["calendar_id"];
        removed.extend(ENRICHMENT);
        if !equal(
            &fields_without(expected_state, &removed),
            &fields_without(&before, &["calendar_id"]),
        ) {
            return Err(conflict(key));
        }
    }
    if operation == "calendar.create_calendar" {
        let can = probe
            .call("calendar.can_create_calendar", &json!({}))
            .map_err(|code| native_failure(&code))?;
        if can["allowed"] != true {
            return Err(refuse("unsupported"));
        }
    } else {
        writable_calendar(probe, text(action, "list_key"))?;
        if operation != "calendar.delete_event" {
            goal(probe, action, &Value::Null)?;
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
    if !OPERATIONS.contains(&operation) {
        return Err(refuse("unsupported"));
    }
    let key = text(action, "native_key");
    if !key.is_empty() {
        let event = event_read(probe, key)?;
        if !equal(&event["data"], native_state) {
            return Err(conflict(key));
        }
        if event["writable"] != true {
            return Err(refuse("unsupported"));
        }
    }
    if operation != "calendar.create_calendar" {
        writable_calendar(probe, text(action, "list_key"))?;
        if operation != "calendar.delete_event" {
            goal(probe, action, &Value::Null)?;
        }
    }
    Ok(())
}

pub(super) fn write(probe: &dyn Probe, command_id: &str, action: &Value) -> Result<Value, String> {
    probe.call(
        "calendar.write",
        &json!({
            "op": action["op"],
            "native_key": action["native_key"],
            "list_key": action["list_key"],
            "calendar_args": request(action),
            "command_id": command_id,
        }),
    )
}

fn blank(value: &Value) -> bool {
    value.is_null() || value == ""
}

fn unordered(actual: &Value, expected: &Value, same: impl Fn(&Value, &Value) -> bool) -> bool {
    let empty = Vec::new();
    let mut remaining: Vec<&Value> = actual.as_array().unwrap_or(&empty).iter().collect();
    let wanted = expected.as_array().unwrap_or(&empty);
    if remaining.len() != wanted.len() {
        return false;
    }
    for item in wanted {
        let Some(index) = remaining.iter().position(|candidate| same(candidate, item)) else {
            return false;
        };
        remaining.remove(index);
    }
    true
}

fn civil_utc(value: &str) -> Option<String> {
    let timestamp: jiff::Timestamp = value.parse().ok()?;
    let date = timestamp.to_zoned(jiff::tz::TimeZone::UTC).date();
    Some(format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month(),
        date.day()
    ))
}

fn same_rule(actual: &Value, expected: &Value, all_day: bool) -> bool {
    let mut left = actual.as_object().cloned().unwrap_or_default();
    let mut right = expected.as_object().cloned().unwrap_or_default();
    let no_days = |rule: &Value| {
        rule["days_of_week"]
            .as_array()
            .is_none_or(std::vec::Vec::is_empty)
    };
    if no_days(actual) && no_days(expected) {
        left.remove("first_day_of_week");
        right.remove("first_day_of_week");
    }
    if all_day {
        for rule in [&mut left, &mut right] {
            if let Some(end) = rule.get("end").and_then(Value::as_str).and_then(civil_utc) {
                rule.insert("end".into(), Value::String(end));
            }
        }
    }
    equal(&Value::Object(left), &Value::Object(right))
}

pub(super) fn matches(actual: &Value, expected: &Value, all_day: bool) -> bool {
    let (Some(_), Some(wanted)) = (actual.as_object(), expected.as_object()) else {
        return false;
    };
    wanted.iter().all(|(field, requested)| {
        let stored = &actual[field.as_str()];
        if matches!(field.as_str(), "notes" | "location" | "url")
            && blank(stored)
            && blank(requested)
        {
            return true;
        }
        match field.as_str() {
            "recurrence" => unordered(stored, requested, |a, b| same_rule(a, b, all_day)),
            "alarms" => unordered(stored, requested, equal),
            _ => equal(stored, requested),
        }
    })
}

fn settle(operation: &str, written: &Value) -> Option<Settled> {
    let key = written["native_key"].clone();
    if text(written, "native_key").is_empty() {
        return None;
    }
    match operation {
        "calendar.delete_event" => (written["exists_after"] == false).then(|| Settled {
            disposition: "locally_observed",
            evidence: json!({"native_key": key, "observed": true, "native_result": null, "absent": true}),
        }),
        "calendar.create_calendar" => {
            let saved = &written["saved_title"];
            (saved.is_string() && written["observed_title"] == *saved).then(|| Settled {
                disposition: "locally_observed",
                evidence: json!({"native_key": key, "observed": true, "native_result": written["actual"], "absent": false}),
            })
        }
        _ => {
            let all_day = written["all_day"] == true;
            let observed = matches(&written["actual"], &written["goal"], all_day);
            let mut evidence = json!({
                "native_key": key,
                "observed": observed,
                "native_result": written["actual"],
                "native_expected": written["goal"],
                "native_expected_all_day": all_day,
                "absent": false,
            });
            if observed {
                Some(Settled {
                    disposition: "locally_observed",
                    evidence,
                })
            } else {
                evidence["reason"] = json!("native_postcondition_mismatch");
                Some(Settled {
                    disposition: "indeterminate",
                    evidence,
                })
            }
        }
    }
}

pub(super) fn resolved(probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let prepared = &evidence["prepared"];
    let action = &prepared["action"];
    let operation = text(action, "op");
    let key = match text(evidence, "native_key") {
        "" => text(action, "native_key"),
        key => key,
    };
    if key.is_empty() || !OPERATIONS.contains(&operation) {
        return None;
    }
    let mut result = evidence.as_object().cloned().unwrap_or_default();
    match operation {
        "calendar.create_calendar" => {
            let calendar = read(probe, "calendar.read_calendar", key).ok()?;
            if !present(&calendar) || calendar["data"]["title"] != request(action)["title"] {
                return None;
            }
        }
        "calendar.delete_event" => {
            let event = read(probe, "calendar.read_event", key).ok()?;
            if present(&event) || event["known_absent"] != true {
                return None;
            }
        }
        _ => {
            let current = read(probe, "calendar.read_event", key).ok()?;
            if !present(&current) {
                return None;
            }
            let (expected, all_day) = if evidence["native_expected"].is_null() {
                let answer = goal(probe, action, &prepared["native_state"]).ok()?;
                (answer["goal"].clone(), current["all_day"] == true)
            } else {
                (
                    evidence["native_expected"].clone(),
                    if evidence["native_expected_all_day"].is_null() {
                        current["all_day"] == true
                    } else {
                        evidence["native_expected_all_day"] == true
                    },
                )
            };
            let observed = matches(&current["native_result"], &expected, all_day);
            result.insert("native_key".into(), json!(key));
            result.insert("observed".into(), json!(observed));
            result.insert("native_result".into(), current["native_result"].clone());
            result.insert("native_expected".into(), expected);
            result.insert("native_expected_all_day".into(), json!(all_day));
            result.insert("absent".into(), json!(false));
            result.insert("prepared".into(), prepared.clone());
            if !observed {
                return None;
            }
            return Some(Value::Object(result));
        }
    }
    result.insert("native_key".into(), json!(key));
    result.insert("observed".into(), json!(true));
    result.insert("absent".into(), json!(operation == "calendar.delete_event"));
    Some(Value::Object(result))
}

fn reconcile_args(found: &Value, _operation: &str) -> Value {
    if found["absent"] == true {
        json!({"absent_key": found["native_key"]})
    } else {
        json!({})
    }
}

pub(super) const DRIVER: Driver = Driver {
    name: "calendar",
    prepare,
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
        event: Value,
        calendar_writable: bool,
        goal: Result<Value, &'static str>,
        known_absent: bool,
    }

    fn event_data(title: &str) -> Value {
        json!({"calendar_id": {"$ref": {"collection": "calendar.calendar", "native_key": "work"}},
            "title": title, "notes": null, "all_day": false, "recurrence": []})
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                event: event_data("standup"),
                calendar_writable: true,
                goal: Ok(json!({"goal": {"title": "retro"}, "all_day": false})),
                known_absent: true,
            }
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push(operation.into());
            let key = args["native_key"].as_str().unwrap_or_default();
            Ok(match operation {
                "calendar.read_event" if key == "e1" => {
                    json!({"exists": true, "writable": self.calendar_writable,
                    "calendar_key": "work", "all_day": self.event["all_day"], "data": self.event,
                    "native_result": self.event})
                }
                "calendar.read_event" => {
                    json!({"exists": false, "known_absent": self.known_absent})
                }
                "calendar.read_calendar" if key == "work" => {
                    json!({"exists": true, "writable": self.calendar_writable,
                    "data": {"title": "Work"}})
                }
                "calendar.read_calendar" => json!({"exists": false}),
                "calendar.can_create_calendar" => json!({"allowed": true}),
                "calendar.goal" => return self.goal.clone().map_err(str::to_owned),
                _ => Value::Null,
            })
        }
    }

    fn update(extra: &Value) -> Value {
        let mut action = json!({"op": "calendar.update_event", "native_key": "e1", "list_key": "work",
            "expected_list_key": "work", "calendar_args": {"title": "retro", "span": "this"}});
        for (key, value) in extra.as_object().unwrap() {
            action[key] = value.clone();
        }
        action
    }

    fn expected() -> Value {
        let mut state = event_data("standup");
        state["calendar_id"] =
            json!({"$ref": {"collection": "calendar.calendar", "native_key": "elsewhere"}});
        state["travel_time"] = json!(300);
        state
    }

    #[test]
    fn a_matching_event_prepares_and_enrichment_fields_do_not_count() {
        let state = prepare(&Fake::new(), &update(&json!({})), &expected()).unwrap();
        assert_eq!(state["title"], "standup");
    }

    #[test]
    fn every_conflict_names_the_key() {
        let mut stale = expected();
        stale["title"] = json!("other");
        let refusal = prepare(&Fake::new(), &update(&json!({})), &stale).unwrap_err();
        assert_eq!(refusal.code, "conflict");
        assert_eq!(refusal.key.as_deref(), Some("e1"));
        let moved = update(&json!({"expected_list_key": "elsewhere"}));
        assert_eq!(
            prepare(&Fake::new(), &moved, &expected()).unwrap_err().code,
            "conflict"
        );
        let gone = update(&json!({"native_key": "gone"}));
        assert_eq!(
            prepare(&Fake::new(), &gone, &expected()).unwrap_err().code,
            "conflict"
        );
    }

    #[test]
    fn unsupported_requests_are_refused_before_any_read() {
        let probe = Fake::new();
        let all = update(&json!({"calendar_args": {"title": "x", "span": "all"}}));
        assert_eq!(
            prepare(&probe, &all, &expected()).unwrap_err().code,
            "unsupported"
        );
        let unknown = json!({"op": "calendar.explode"});
        assert_eq!(
            prepare(&probe, &unknown, &Value::Null).unwrap_err().code,
            "unsupported"
        );
        assert!(probe.calls.borrow().is_empty());
    }

    #[test]
    fn read_only_calendars_and_invalid_goals_are_refused() {
        let mut probe = Fake::new();
        probe.calendar_writable = false;
        assert_eq!(
            prepare(&probe, &update(&json!({})), &expected())
                .unwrap_err()
                .code,
            "unsupported"
        );
        let create = json!({"op": "calendar.create_event", "list_key": "work", "calendar_args": {"title": "x"}});
        assert_eq!(
            prepare(&probe, &create, &Value::Null).unwrap_err().code,
            "unsupported"
        );
        let mut invalid = Fake::new();
        invalid.goal = Err("invalid");
        assert_eq!(
            prepare(&invalid, &update(&json!({})), &expected())
                .unwrap_err()
                .code,
            "invalid"
        );
        let delete = update(&json!({"op": "calendar.delete_event"}));
        let deleting = Fake::new();
        assert!(prepare(&deleting, &delete, &expected()).is_ok());
        assert!(
            !deleting
                .calls
                .borrow()
                .iter()
                .any(|call| call == "calendar.goal")
        );
    }

    #[test]
    fn drift_between_prepare_and_dispatch_is_a_conflict() {
        let mut probe = Fake::new();
        let action = update(&json!({}));
        let state = prepare(&probe, &action, &expected()).unwrap();
        assert!(validate(&probe, &action, &state).is_ok());
        probe.event["title"] = json!("edited");
        assert_eq!(
            validate(&probe, &action, &state).unwrap_err().code,
            "conflict"
        );
        probe.event = event_data("standup");
        probe.calendar_writable = false;
        assert_eq!(
            validate(&probe, &action, &state).unwrap_err().code,
            "unsupported"
        );
    }

    #[test]
    fn the_postcondition_decides_observed_or_indeterminate() {
        let good = json!({"native_key": "e1", "actual": {"title": "retro", "notes": ""}, "goal": {"title": "retro", "notes": null}, "all_day": false});
        let settled = settle("calendar.update_event", &good).unwrap();
        assert_eq!(settled.disposition, "locally_observed");
        assert_eq!(settled.evidence["observed"], true);
        let bad = json!({"native_key": "e1", "actual": {"title": "standup"}, "goal": {"title": "retro"}, "all_day": false});
        let settled = settle("calendar.update_event", &bad).unwrap();
        assert_eq!(settled.disposition, "indeterminate");
        assert_eq!(settled.evidence["observed"], false);
        assert_eq!(settled.evidence["reason"], "native_postcondition_mismatch");
        let unseen = json!({"native_key": "e1", "actual": null, "goal": {"title": "retro"}, "all_day": false});
        assert_eq!(
            settle("calendar.update_event", &unseen)
                .unwrap()
                .disposition,
            "indeterminate"
        );
        assert!(settle("calendar.update_event", &json!({"actual": {}, "goal": {}})).is_none());
    }

    #[test]
    fn deletes_and_new_calendars_need_their_own_read_back() {
        let gone = json!({"native_key": "e1", "exists_after": false});
        assert_eq!(
            settle("calendar.delete_event", &gone).unwrap().evidence["absent"],
            true
        );
        let still = json!({"native_key": "e1", "exists_after": true});
        assert!(settle("calendar.delete_event", &still).is_none());
        let made = json!({"native_key": "c1", "saved_title": "Trips", "observed_title": "Trips", "actual": {"title": "Trips"}});
        assert!(settle("calendar.create_calendar", &made).is_some());
        let renamed =
            json!({"native_key": "c1", "saved_title": "Trips", "observed_title": "Other"});
        assert!(settle("calendar.create_calendar", &renamed).is_none());
        let unseen = json!({"native_key": "c1", "saved_title": "Trips", "observed_title": null});
        assert!(settle("calendar.create_calendar", &unseen).is_none());
    }

    #[test]
    fn recurrence_and_alarms_compare_unordered_and_all_day_ends_by_utc_date() {
        let rule = |end: &str| json!({"frequency": "daily", "interval": 1, "count": 0, "end": end, "days_of_week": [], "first_day_of_week": 1});
        let other = json!({"frequency": "daily", "interval": 1, "count": 0, "end": "2026-03-04T05:00:00Z", "days_of_week": [], "first_day_of_week": 2});
        assert!(matches(
            &json!({"recurrence": [rule("2026-03-04T00:00:00Z")]}),
            &json!({"recurrence": [other]}),
            true
        ));
        assert!(!matches(
            &json!({"recurrence": [rule("2026-03-05T00:00:00Z")]}),
            &json!({"recurrence": [other]}),
            true
        ));
        assert!(!matches(
            &json!({"recurrence": [rule("2026-03-04T00:00:00Z")]}),
            &json!({"recurrence": [other]}),
            false
        ));
        let a = json!({"relative_offset": -60});
        let b = json!({"relative_offset": -600});
        assert!(matches(
            &json!({"alarms": [a, b]}),
            &json!({"alarms": [b, a]}),
            false
        ));
        assert!(!matches(
            &json!({"alarms": [a]}),
            &json!({"alarms": [a, b]}),
            false
        ));
        assert!(!matches(&json!(null), &json!({}), false));
    }

    #[test]
    fn resolve_observes_only_what_the_framework_shows() {
        let mut probe = Fake::new();
        let evidence = json!({"native_key": "e1", "native_expected": {"title": "standup"}, "native_expected_all_day": false,
            "prepared": {"action": update(&json!({})), "native_state": event_data("standup"), "request_hash": "h"}});
        assert_eq!(resolved(&probe, &evidence).unwrap()["observed"], true);
        probe.event["title"] = json!("changed");
        assert!(resolved(&probe, &evidence).is_none());
        let mut delete = evidence.clone();
        delete["prepared"]["action"] =
            update(&json!({"op": "calendar.delete_event", "native_key": "gone"}));
        delete["native_key"] = json!("gone");
        assert_eq!(resolved(&probe, &delete).unwrap()["absent"], true);
        probe.known_absent = false;
        assert!(resolved(&probe, &delete).is_none());
        assert!(
            resolved(
                &probe,
                &json!({"prepared": {"action": {"op": "calendar.update_event"}}})
            )
            .is_none()
        );
    }
}
