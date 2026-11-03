use ab_native::agenda::{self, Revalidation, Snapshot};
use ab_native::capture::Context;
use ab_native::contacts;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[test]
fn swift_oracle_matches_current_sources() -> Result<(), Box<dyn std::error::Error>> {
    let evidence: Value =
        serde_json::from_str(include_str!("../parity/agenda-contacts/evidence.json"))?;
    let sources = [
        (
            "native/Sources/CalendarSource/CalendarSource.swift",
            include_str!("../../../../native/Sources/CalendarSource/CalendarSource.swift"),
        ),
        (
            "native/Sources/ContactsSource/ContactsSource.swift",
            include_str!("../../../../native/Sources/ContactsSource/ContactsSource.swift"),
        ),
        (
            "native/Sources/SourceSupport/Source.swift",
            include_str!("../../../../native/Sources/SourceSupport/Source.swift"),
        ),
        (
            "native/Sources/CalendarSource/CalendarTime.swift",
            include_str!("../../../../native/Sources/CalendarSource/CalendarTime.swift"),
        ),
        (
            "native/Sources/CalendarSource/CalendarScanScope.swift",
            include_str!("../../../../native/Sources/CalendarSource/CalendarScanScope.swift"),
        ),
    ];
    for (path, source) in sources {
        assert_eq!(
            ab_native::body_digest(source.as_bytes()),
            evidence["source_sha256"][path]
        );
    }
    Ok(())
}

#[test]
fn swift_synthetic_shadow_replay_has_zero_differences() -> Result<(), Box<dyn std::error::Error>> {
    let stages: Vec<Value> =
        serde_json::from_str(include_str!("../parity/agenda-contacts/replay.json"))?;
    let mut calendar = agenda::Reader::default();
    let mut contacts = contacts::Reader::default();
    let mut counts = BTreeMap::new();
    for stage in stages {
        let snapshot: Snapshot = serde_json::from_value(stage["snapshot"].clone())?;
        let context: Context = serde_json::from_value(stage["context"].clone())?;
        let source = stage["source"].as_str().ok_or("missing source")?;
        let pending = if source == "contacts" {
            contacts.prepare(snapshot, &context)?
        } else {
            calendar.prepare(snapshot, &context)?
        };
        assert_eq!(
            serde_json::to_value(&pending.observations)?,
            stage["expected"],
            "{} {}",
            source,
            stage["name"]
        );
        for (index, observation) in pending.observations.iter().enumerate() {
            let swift_data = stage["expected_data"][index]
                .as_str()
                .ok_or("missing Swift data bytes")?;
            let expected = ab_wire::gojson::record_data(
                swift_data.as_bytes(),
                ab_wire::gojson::EscapeMode::Html,
            )?;
            let actual = observation.data.as_ref().ok_or("missing Rust data")?;
            assert_eq!(actual.get().as_bytes(), expected, "{source} data bytes");
        }
        if source == "contacts" {
            contacts.commit(pending)?;
        } else {
            calendar.commit(pending)?;
        }
        *counts.entry(source.to_owned()).or_insert(0) += 1;
    }
    assert_eq!(counts["calendar"], 9);
    assert_eq!(counts["contacts"], 10);
    Ok(())
}

fn context(checkpoint: &str) -> Context {
    Context {
        observed_at: "2026-11-03T00:00:00.000Z".into(),
        checkpoint: checkpoint.into(),
        scan_id: checkpoint.into(),
    }
}

fn calendar_snapshot(generation: &str, records: Vec<Value>) -> Snapshot {
    Snapshot {
        generation: generation.into(),
        scope: vec!["calendar.calendar".into(), "calendar.event".into()],
        complete: true,
        records,
        revalidated: BTreeMap::new(),
        force: false,
    }
}

fn calendar_record(key: &str) -> Value {
    json!({"kind": "upsert", "collection": "calendar.event", "native_key": key,
           "scoped_alias": key, "data": {"title": key}})
}

#[test]
fn incomplete_scope_duplicate_and_stale_ack_cannot_mutate_state()
-> Result<(), Box<dyn std::error::Error>> {
    let mut reader = agenda::Reader::default();
    let initial = calendar_snapshot("g1", vec![calendar_record("a")]);
    let first = reader.prepare(initial.clone(), &context("one"))?;
    let stale = reader.prepare(initial.clone(), &context("two"))?;
    reader.commit(first)?;
    assert!(reader.commit(stale).is_err());
    let mut incomplete = calendar_snapshot("g1", vec![]);
    incomplete.complete = false;
    assert!(reader.prepare(incomplete, &context("three")).is_err());
    let mut scope = initial.clone();
    scope.scope.pop();
    assert!(reader.prepare(scope, &context("three")).is_err());
    let duplicate = calendar_snapshot("g1", vec![calendar_record("a"), calendar_record("a")]);
    assert!(reader.prepare(duplicate, &context("three")).is_err());
    let unchanged = reader.prepare(initial, &context("three"))?;
    assert_eq!(unchanged.observations.len(), 0);
    Ok(())
}

#[test]
fn generation_change_retains_candidates_until_later_revalidation()
-> Result<(), Box<dyn std::error::Error>> {
    let mut reader = agenda::Reader::default();
    let first = reader.prepare(
        calendar_snapshot("g1", vec![calendar_record("a")]),
        &context("one"),
    )?;
    reader.commit(first)?;
    let changed = reader.prepare(calendar_snapshot("g2", vec![]), &context("two"))?;
    assert_eq!(changed.observations.len(), 0);
    reader.commit(changed)?;
    let unconfirmed = reader.prepare(calendar_snapshot("g2", vec![]), &context("three"))?;
    assert_eq!(unconfirmed.observations.len(), 0);
    reader.commit(unconfirmed)?;
    let mut revalidated = calendar_snapshot("g2", vec![]);
    revalidated
        .revalidated
        .insert("calendar.event/a".into(), Revalidation::Absent);
    let absent = reader.prepare(revalidated, &context("four"))?;
    assert_eq!(absent.observations.len(), 1);
    assert_eq!(absent.observations[0].kind, "absent");
    let value = serde_json::to_value(&absent.observations[0])?;
    assert_eq!(value["data"]["title"], "a");
    Ok(())
}

#[test]
fn ack_from_different_checkpoint_state_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let mut left = agenda::Reader::default();
    let mut right = agenda::Reader::default();
    let first = left.prepare(
        calendar_snapshot("g1", vec![calendar_record("a")]),
        &context("one"),
    )?;
    left.commit(first)?;
    let other = right.prepare(
        calendar_snapshot("g1", vec![calendar_record("b")]),
        &context("two"),
    )?;
    right.commit(other)?;
    let foreign = right.prepare(calendar_snapshot("g1", vec![]), &context("three"))?;
    assert!(left.commit(foreign).is_err());
    Ok(())
}
