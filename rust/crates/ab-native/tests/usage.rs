#![allow(clippy::unwrap_used)]

use ab_native::{
    infocus::InFocusEvent,
    usage::{self, Interval},
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    events: Vec<InFocusEvent>,
    zone: String,
    mac: bool,
    blocked: Vec<Interval>,
}

#[test]
fn swift_rollup_parity() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../parity/usage/rollup-input.json")).unwrap();
    let vectors = include_str!("../parity/usage/rollup-expected.jsonl")
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), vectors.len());
    for (case, expected) in cases.into_iter().zip(vectors) {
        let records = usage::records(
            &case.events,
            "synthetic",
            case.mac,
            &case.zone,
            &case.blocked,
        )
        .unwrap();
        assert_eq!(expected["name"], case.name);
        let actual = serde_json::to_value(records).unwrap();
        assert!(
            equal(&actual, &expected["records"]),
            "{}: actual {actual}, expected {}",
            case.name,
            expected["records"]
        );
    }
}

fn equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => a.as_f64() == b.as_f64(),
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|b| equal(v, b)))
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        _ => a == b,
    }
}

#[test]
fn concurrent_checkpoint_preparations_cannot_commit_out_of_order() {
    use ab_native::usage::{Context, Reader};
    let root = std::env::temp_dir().join(format!(
        "ab-usage-race-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("sync")).unwrap();
    std::fs::create_dir_all(root.join("streams/restricted/App.InFocus/local")).unwrap();
    let db = rusqlite::Connection::open(root.join("sync/sync.db")).unwrap();
    db.execute_batch("CREATE TABLE DevicePeer(device_identifier TEXT,name TEXT,platform INTEGER,me INTEGER,model TEXT); INSERT INTO DevicePeer VALUES('synthetic','Synthetic Mac',3,1,'Mac');").unwrap();
    let mut context = Context {
        generation: "synthetic".into(),
        checkpoint: "a".into(),
        scan_id: "a".into(),
        observed_at: "2027-01-16T12:00:00.000Z".into(),
        zone: "America/New_York".into(),
        local_name: "Synthetic Mac".into(),
        blocked: Vec::new(),
        power: Vec::new(),
        history: None,
    };
    let mut reader = Reader::default();
    let first = reader.scan(&root, &context, false).unwrap();
    context.checkpoint = "b".into();
    let second = reader.scan(&root, &context, false).unwrap();
    reader.commit(second).unwrap();
    assert!(reader.commit(first).is_err());
    let duplicate = reader.scan(&root, &context, true).unwrap();
    assert!(reader.commit(duplicate).is_err());
    std::fs::remove_dir(root.join("streams/restricted/App.InFocus/local")).unwrap();
    context.checkpoint = "c".into();
    assert!(reader.scan(&root, &context, true).is_err());
    std::fs::create_dir(root.join("streams/restricted/App.InFocus/local")).unwrap();
    let recovered = reader.scan(&root, &context, true).unwrap();
    assert!(recovered.observations.is_empty());
    reader.commit(recovered).unwrap();
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
