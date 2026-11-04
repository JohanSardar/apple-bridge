use super::owned::{Driver, Probe, Refusal, Settled, equal, native_failure, refuse, text};
use serde_json::{Value, json};
use std::{thread, time::Duration};

const OPERATIONS: [&str; 6] = [
    "notes.create",
    "notes.update",
    "notes.delete",
    "notes.create_folder",
    "notes.move",
    "notes.rename_folder",
];
const FOLDER_PREFIX: &str = "Bridge Test [ab-test";
const FORBIDDEN_ORIGINAL: [&str; 6] = [
    "data-notes-read-only",
    "<table",
    "<ul",
    "<ol",
    "<pre",
    "<blockquote",
];
const HTML_LIMIT: usize = 1_000_000;
const OBSERVATIONS: u32 = 100;
const BODY_READS: u32 = 60;
const OBSERVATION_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(50)
};

fn truthy(data: &Value, field: &str) -> bool {
    data[field] == true
}

fn structure(html: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            let end = rest.find('>')?;
            tokens.push(rest[..=end].to_owned());
            rest = &rest[end + 1..];
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            tokens.push("text".to_owned());
            rest = &rest[end..];
        }
    }
    Some(tokens)
}

pub(super) fn rich_edit_allowed(before: &Value, html: &str) -> bool {
    let attachments_empty = before["attachments"]
        .as_array()
        .is_none_or(std::vec::Vec::is_empty);
    if before["content_status"] != "complete"
        || before["block_status"] != "complete"
        || before["round_trip_status"] != "unproven"
        || !attachments_empty
        || truthy(before, "locked")
        || truthy(before, "read_only")
        || truthy(before, "recently_deleted")
        || html.is_empty()
        || html.len() > HTML_LIMIT
    {
        return false;
    }
    let original = text(before, "html");
    if FORBIDDEN_ORIGINAL
        .iter()
        .any(|marker| original.contains(marker))
    {
        return false;
    }
    if original.is_empty() || html.contains('\0') {
        return false;
    }
    match (structure(original), structure(html)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn collection(operation: &str) -> &'static str {
    if operation == "notes.rename_folder" {
        "notes.folder"
    } else {
        "notes.note"
    }
}

fn read(probe: &dyn Probe, collection: &str, key: &str) -> Result<Value, Refusal> {
    let found = probe
        .call(
            "notes.read",
            &json!({"native_key": key, "collection": collection}),
        )
        .map_err(|code| native_failure(&code))?;
    if found["exists"] == true {
        Ok(found)
    } else {
        Err(refuse("conflict"))
    }
}

fn body_pending(data: &Value) -> bool {
    data["content_status"]
        .as_str()
        .is_some_and(|status| status != "complete")
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
    let mut state = Value::Null;
    if !key.is_empty() {
        let mut found = read(probe, collection(operation), key)?;
        for _ in 0..BODY_READS {
            if operation != "notes.update" || !body_pending(&found["data"]) {
                break;
            }
            thread::sleep(OBSERVATION_GAP);
            found = read(probe, collection(operation), key)?;
        }
        let before = found["data"].clone();
        if operation == "notes.update" {
            let html = &action["note_args"]["html"];
            if html.is_null() {
                if before["editable"] != "plain" {
                    return Err(refuse("unsupported"));
                }
            } else if !rich_edit_allowed(&before, html.as_str().unwrap_or_default()) {
                return Err(refuse("unsupported"));
            }
        }
        if truthy(&before, "locked")
            || truthy(&before, "read_only")
            || truthy(&before, "recently_deleted")
        {
            return Err(refuse("unsupported"));
        }
        if operation == "notes.rename_folder" {
            let title = text(&before, "title");
            if !title.starts_with(FOLDER_PREFIX)
                || !text(&action["note_args"], "title").starts_with(FOLDER_PREFIX)
                || !equal(&expected_state["title"], &before["title"])
            {
                return Err(refuse("conflict"));
            }
            return Ok(json!({"native_fingerprint": found["folder_fingerprint"]}));
        }
        if !equal(
            &expected_state["native_fingerprint"],
            &before["native_fingerprint"],
        ) {
            return Err(refuse("conflict"));
        }
        state = json!({"native_fingerprint": before["native_fingerprint"]});
        if operation == "notes.update" {
            state["plain_text"] = before["plain_text"].clone();
        }
    }
    if matches!(operation, "notes.create" | "notes.move") {
        let folder = read(probe, "notes.folder", text(action, "folder_key"))?;
        if truthy(&folder["data"], "read_only") {
            return Err(refuse("unsupported"));
        }
    }
    Ok(state)
}

pub(super) fn validate(
    probe: &dyn Probe,
    action: &Value,
    native_state: &Value,
) -> Result<(), Refusal> {
    let operation = text(action, "op");
    let key = text(action, "native_key");
    if operation == "notes.rename_folder" {
        let current = read(probe, "notes.folder", key)?;
        let fingerprint = text(&current, "folder_fingerprint");
        return if !fingerprint.is_empty()
            && equal(&json!(fingerprint), &native_state["native_fingerprint"])
        {
            Ok(())
        } else {
            Err(refuse("conflict"))
        };
    }
    if !OPERATIONS.contains(&operation) {
        return Err(refuse("unsupported"));
    }
    if !key.is_empty() {
        let current = read(probe, "notes.note", key)?;
        if !equal(
            &current["data"]["native_fingerprint"],
            &native_state["native_fingerprint"],
        ) {
            return Err(refuse("conflict"));
        }
    }
    Ok(())
}

fn effect_visible(operation: &str, action: &Value, state: &Value, seen: &Value) -> bool {
    let args = &action["note_args"];
    if operation == "notes.delete" {
        return matches!(
            seen["deletion_state"].as_str(),
            Some("gone" | "marked" | "trashed")
        );
    }
    if seen["observed"] != true {
        return false;
    }
    let data = &seen["data"];
    match operation {
        "notes.rename_folder" => equal(&data["title"], &args["title"]),
        "notes.move" => {
            data["folder_id"]["$ref"]["native_key"].as_str() == Some(text(action, "folder_key"))
        }
        "notes.update" if !args["html"].is_null() => equal(&data["html"], &args["html"]),
        "notes.update" => !equal(&data["plain_text"], &state["plain_text"]),
        _ => true,
    }
}

pub(super) fn write(
    probe: &dyn Probe,
    command_id: &str,
    action: &Value,
    state: &Value,
) -> Result<Value, String> {
    let operation = text(action, "op");
    let written = probe.call(
        "notes.write",
        &json!({
            "op": action["op"],
            "native_key": action["native_key"],
            "folder_key": action["folder_key"],
            "note_args": action["note_args"],
            "command_id": command_id,
        }),
    )?;
    let script_id = text(&written, "script_id").to_owned();
    for attempt in 0..OBSERVATIONS {
        let seen = probe.call(
            "notes.observe",
            &json!({"script_id": script_id, "native_key": action["native_key"]}),
        )?;
        if effect_visible(operation, action, state, &seen) {
            probe.call("notes.reconcile", &json!({}))?;
            let key = if operation == "notes.delete" {
                action["native_key"].clone()
            } else {
                seen["native_key"].clone()
            };
            return Ok(
                json!({"native_key": key, "observed": true, "absent": seen["observed"] != true}),
            );
        }
        if attempt + 1 < OBSERVATIONS {
            thread::sleep(OBSERVATION_GAP);
        }
    }
    Err(written["failure"]
        .as_str()
        .unwrap_or("unavailable")
        .to_owned())
}

fn settle(operation: &str, written: &Value) -> Option<Settled> {
    if text(written, "native_key").is_empty() || written["observed"] != true {
        return None;
    }
    let mut evidence = json!({"native_key": written["native_key"], "observed": true});
    if operation == "notes.delete" {
        evidence["absent"] = written["absent"].clone();
    }
    Some(Settled {
        disposition: "locally_observed",
        evidence,
    })
}

fn resolved(_probe: &dyn Probe, _evidence: &Value) -> Option<Value> {
    None
}

fn reconcile_args(_found: &Value, _operation: &str) -> Value {
    json!({})
}

pub(super) const DRIVER: Driver = Driver {
    name: "notes",
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
        pending_reads: RefCell<u32>,
        note: Value,
        folder: Value,
        seen: RefCell<Vec<Value>>,
    }

    fn note(html: &str) -> Value {
        json!({"native_fingerprint": "f1", "plain_text": "hello", "editable": "plain",
            "content_status": "complete", "block_status": "complete", "round_trip_status": "unproven",
            "attachments": [], "html": html})
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                pending_reads: RefCell::new(0),
                note: note("<div>hello</div>"),
                folder: json!({"title": "Bridge Test [ab-test x]", "read_only": false}),
                seen: RefCell::new(Vec::new()),
            }
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push(operation.into());
            let key = args["native_key"].as_str().unwrap_or_default();
            Ok(match (operation, args["collection"].as_str()) {
                ("notes.read", Some("notes.note")) if key == "n1" => {
                    let mut pending = self.pending_reads.borrow_mut();
                    let mut data = self.note.clone();
                    if *pending > 0 {
                        *pending -= 1;
                        data["editable"] = json!("none");
                        data["content_status"] = json!("partial");
                    }
                    json!({"exists": true, "data": data})
                }
                ("notes.read", Some("notes.folder")) if key == "d1" => {
                    json!({"exists": true, "data": self.folder, "folder_fingerprint": "ff1"})
                }
                ("notes.read", _) => json!({"exists": false}),
                ("notes.write", _) => json!({"script_id": "s1"}),
                ("notes.observe", _) => {
                    let mut seen = self.seen.borrow_mut();
                    if seen.is_empty() {
                        json!({"observed": false})
                    } else {
                        seen.remove(0)
                    }
                }
                _ => json!({}),
            })
        }
    }

    fn update(extra: &Value) -> Value {
        let mut action =
            json!({"op": "notes.update", "native_key": "n1", "note_args": {"plain_text": "bye"}});
        for (key, value) in extra.as_object().unwrap() {
            action[key] = value.clone();
        }
        action
    }

    fn expected() -> Value {
        json!({"native_fingerprint": "f1"})
    }

    #[test]
    fn an_update_waits_for_a_body_still_arriving_and_refuses_one_that_never_does() {
        let arriving = Fake::new();
        *arriving.pending_reads.borrow_mut() = 3;
        assert!(prepare(&arriving, &update(&json!({})), &expected()).is_ok());
        assert_eq!(*arriving.pending_reads.borrow(), 0);

        let stuck = Fake::new();
        *stuck.pending_reads.borrow_mut() = BODY_READS + 5;
        assert_eq!(
            prepare(&stuck, &update(&json!({})), &expected())
                .unwrap_err()
                .code,
            "unsupported"
        );
        assert_eq!(*stuck.pending_reads.borrow(), 4);

        let rich = {
            let mut probe = Fake::new();
            probe.note["editable"] = json!("rich");
            probe
        };
        let _ = prepare(&rich, &update(&json!({})), &expected());
        let reads = rich
            .calls
            .borrow()
            .iter()
            .filter(|call| *call == "notes.read")
            .count();
        assert_eq!(reads, 1, "a finished body is decided at once");
    }

    #[test]
    fn a_matching_plain_edit_prepares_with_the_fingerprint_and_text() {
        let state = prepare(&Fake::new(), &update(&json!({})), &expected()).unwrap();
        assert_eq!(
            state,
            json!({"native_fingerprint": "f1", "plain_text": "hello"})
        );
    }

    #[test]
    fn every_refusal_has_a_negative_control() {
        let stale = json!({"native_fingerprint": "other"});
        let refused = prepare(&Fake::new(), &update(&json!({})), &stale).unwrap_err();
        assert_eq!((refused.code.as_str(), refused.key), ("conflict", None));
        let gone = update(&json!({"native_key": "gone"}));
        assert_eq!(
            prepare(&Fake::new(), &gone, &expected()).unwrap_err().code,
            "conflict"
        );
        for field in ["locked", "read_only", "recently_deleted"] {
            let mut probe = Fake::new();
            probe.note[field] = json!(true);
            assert_eq!(
                prepare(&probe, &update(&json!({})), &expected())
                    .unwrap_err()
                    .code,
                "unsupported",
                "{field}"
            );
        }
        let mut rich = Fake::new();
        rich.note["editable"] = json!("rich");
        assert_eq!(
            prepare(&rich, &update(&json!({})), &expected())
                .unwrap_err()
                .code,
            "unsupported"
        );
        let unknown = json!({"op": "notes.delete_folder"});
        assert_eq!(
            prepare(&Fake::new(), &unknown, &Value::Null)
                .unwrap_err()
                .code,
            "unsupported"
        );
        let create = json!({"op": "notes.create", "folder_key": "d1", "note_args": {}});
        assert!(prepare(&Fake::new(), &create, &Value::Null).is_ok());
        let mut readonly = Fake::new();
        readonly.folder["read_only"] = json!(true);
        assert_eq!(
            prepare(&readonly, &create, &Value::Null).unwrap_err().code,
            "unsupported"
        );
        let missing =
            json!({"op": "notes.move", "native_key": "n1", "folder_key": "nope", "note_args": {}});
        assert_eq!(
            prepare(&Fake::new(), &missing, &expected())
                .unwrap_err()
                .code,
            "conflict"
        );
    }

    #[test]
    fn rich_edits_keep_the_structure_and_refuse_everything_else() {
        let before = note("<div>hello</div><div><br></div>");
        assert!(rich_edit_allowed(
            &before,
            "<div>other words</div><div><br></div>"
        ));
        assert!(!rich_edit_allowed(&before, "<div>hello</div>"));
        assert!(!rich_edit_allowed(
            &before,
            "<div>hello</div><div><br></div><div>x</div>"
        ));
        assert!(!rich_edit_allowed(&before, ""));
        assert!(!rich_edit_allowed(&before, "<div>a\0</div><div><br></div>"));
        assert!(!rich_edit_allowed(
            &before,
            "<div>a < b</div><div><br></div>"
        ));
        assert!(!rich_edit_allowed(
            &before,
            &format!("<div>{}</div><div><br></div>", "x".repeat(1_000_001))
        ));
        for marker in FORBIDDEN_ORIGINAL {
            let marked = note(&format!("<div>hello</div>{marker}>"));
            assert!(
                !rich_edit_allowed(&marked, &format!("<div>x</div>{marker}>")),
                "{marker}"
            );
        }
        for (field, value) in [
            ("content_status", "partial"),
            ("block_status", "partial"),
            ("round_trip_status", "proven"),
        ] {
            let mut changed = before.clone();
            changed[field] = json!(value);
            assert!(
                !rich_edit_allowed(&changed, "<div>x</div><div><br></div>"),
                "{field}"
            );
        }
        let mut attached = before.clone();
        attached["attachments"] = json!([{"id": "a"}]);
        assert!(!rich_edit_allowed(&attached, "<div>x</div><div><br></div>"));
        for field in ["locked", "read_only", "recently_deleted"] {
            let mut flagged = before.clone();
            flagged[field] = json!(true);
            assert!(
                !rich_edit_allowed(&flagged, "<div>x</div><div><br></div>"),
                "{field}"
            );
        }
        let mut empty = before.clone();
        empty["html"] = json!("");
        assert!(!rich_edit_allowed(&empty, "<div>x</div>"));
    }

    #[test]
    fn folder_renames_are_limited_to_test_folders_and_the_expected_title() {
        let rename = json!({"op": "notes.rename_folder", "native_key": "d1", "note_args": {"title": "Bridge Test [ab-test y]"}});
        let current = json!({"title": "Bridge Test [ab-test x]"});
        assert_eq!(
            prepare(&Fake::new(), &rename, &current).unwrap()["native_fingerprint"],
            "ff1"
        );
        let stale = json!({"title": "Bridge Test [ab-test z]"});
        assert_eq!(
            prepare(&Fake::new(), &rename, &stale).unwrap_err().code,
            "conflict"
        );
        let outside = json!({"op": "notes.rename_folder", "native_key": "d1", "note_args": {"title": "Personal"}});
        assert_eq!(
            prepare(&Fake::new(), &outside, &current).unwrap_err().code,
            "conflict"
        );
        let mut real = Fake::new();
        real.folder["title"] = json!("Personal");
        assert_eq!(
            prepare(&real, &rename, &json!({"title": "Personal"}))
                .unwrap_err()
                .code,
            "conflict"
        );
        let prepared = prepare(&Fake::new(), &rename, &current).unwrap();
        assert!(validate(&Fake::new(), &rename, &prepared).is_ok());
        assert_eq!(
            validate(&Fake::new(), &rename, &json!({"native_fingerprint": "old"}))
                .unwrap_err()
                .code,
            "conflict"
        );
    }

    #[test]
    fn drift_between_prepare_and_dispatch_is_a_conflict() {
        let mut probe = Fake::new();
        let state = prepare(&probe, &update(&json!({})), &expected()).unwrap();
        assert!(validate(&probe, &update(&json!({})), &state).is_ok());
        probe.note["native_fingerprint"] = json!("f2");
        assert_eq!(
            validate(&probe, &update(&json!({})), &state)
                .unwrap_err()
                .code,
            "conflict"
        );
    }

    #[test]
    fn a_write_succeeds_only_when_the_store_shows_the_effect() {
        let state = json!({"native_fingerprint": "f1", "plain_text": "hello"});
        let probe = Fake::new();
        probe.seen.borrow_mut().extend([
            json!({"observed": true, "native_key": "n1", "data": {"plain_text": "hello"}}),
            json!({"observed": true, "native_key": "n1", "data": {"plain_text": "bye"}}),
        ]);
        let written = write(&probe, "c", &update(&json!({})), &state).unwrap();
        assert_eq!(written["native_key"], "n1");
        assert!(
            probe
                .calls
                .borrow()
                .iter()
                .any(|call| call == "notes.reconcile")
        );
        let rich = update(&json!({"note_args": {"html": "<div>x</div>"}}));
        let html = Fake::new();
        html.seen
            .borrow_mut()
            .push(json!({"observed": true, "native_key": "n1", "data": {"html": "<div>x</div>"}}));
        assert!(write(&html, "c", &rich, &state).is_ok());
        let moved =
            json!({"op": "notes.move", "native_key": "n1", "folder_key": "d2", "note_args": {}});
        let elsewhere = Fake::new();
        elsewhere.seen.borrow_mut().extend(std::iter::repeat_n(json!({"observed": true, "native_key": "n1", "data": {"folder_id": {"$ref": {"native_key": "d1"}}}}), 100));
        assert_eq!(
            write(&elsewhere, "c", &moved, &state).unwrap_err(),
            "unavailable"
        );
        let delete = json!({"op": "notes.delete", "native_key": "n1", "note_args": {}});
        let trashed = Fake::new();
        trashed
            .seen
            .borrow_mut()
            .push(json!({"observed": false, "deletion_state": "trashed"}));
        let written = write(&trashed, "c", &delete, &state).unwrap();
        assert_eq!(
            (written["native_key"].as_str(), written["absent"].as_bool()),
            (Some("n1"), Some(true))
        );
        let active = Fake::new();
        active.seen.borrow_mut().extend(std::iter::repeat_n(
            json!({"observed": true, "deletion_state": "active", "native_key": "n1"}),
            100,
        ));
        assert!(write(&active, "c", &delete, &state).is_err());
    }

    #[test]
    fn outcomes_need_an_observed_key() {
        assert!(
            settle(
                "notes.update",
                &json!({"native_key": "n1", "observed": true})
            )
            .is_some()
        );
        assert!(settle("notes.update", &json!({"observed": true})).is_none());
        assert!(settle("notes.update", &json!({"native_key": "n1"})).is_none());
        let deleted = settle(
            "notes.delete",
            &json!({"native_key": "n1", "observed": true, "absent": true}),
        )
        .unwrap();
        assert_eq!(deleted.evidence["absent"], true);
    }
}
