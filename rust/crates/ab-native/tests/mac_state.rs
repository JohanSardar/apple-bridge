#![allow(clippy::unwrap_used)]

use ab_native::{
    capture::{Context, State},
    mac,
};
use serde_json::{Value, json};

fn context(checkpoint: &str) -> Context {
    Context {
        observed_at: "2026-11-03T00:00:00.000Z".into(),
        checkpoint: checkpoint.into(),
        scan_id: format!("scan-{checkpoint}"),
    }
}

fn row(collection: &str, key: &str, data: &Value) -> Value {
    json!({"kind":"upsert","collection":collection,"native_key":key,"data":data})
}

fn kinds(pending: &ab_native::capture::Pending) -> Vec<(String, String)> {
    pending
        .batch
        .observations
        .iter()
        .map(|o| (o.kind.to_owned(), o.native_key.clone()))
        .collect()
}

#[test]
fn rust_owns_the_mac_diff_and_every_deletion() {
    let rows = vec![
        row("mac.power", "system", &json!({"charging":false})),
        row("mac.app", "1", &json!({"name":"Mail"})),
        row("mac.app", "2", &json!({"name":"Notes"})),
    ];
    let mut state = State::default();
    let first = mac::prepare(&state, &rows, &context("c1"), false).unwrap();
    assert_eq!(first.batch.observations.len(), 3);
    assert!(!first.batch.live);
    let app = first
        .batch
        .observations
        .iter()
        .find(|o| o.native_key == "1")
        .unwrap();
    assert_eq!(app.scoped_alias.as_deref(), Some("mac/mac.app/1"));
    state.commit(first).unwrap();

    let unchanged = mac::prepare(&state, &rows, &context("c2"), true).unwrap();
    assert!(unchanged.batch.observations.is_empty());

    let rows = vec![
        row("mac.power", "system", &json!({"charging":true})),
        row("mac.app", "1", &json!({"name":"Mail"})),
    ];
    let second = mac::prepare(&state, &rows, &context("c3"), true).unwrap();
    assert!(second.batch.live);
    assert_eq!(
        kinds(&second),
        [
            ("upsert".to_owned(), "system".to_owned()),
            ("absent".to_owned(), "2".to_owned())
        ]
    );
}

#[test]
fn rows_outside_the_mac_scope_or_malformed_are_refused() {
    let state = State::default();
    for bad in [
        row("mac.bluetooth_device", "x", &json!({})),
        row("mac.app", "", &json!({})),
        row("mac.app", "1", &json!("not an object")),
        json!({"kind":"absent","collection":"mac.app","native_key":"1","data":{}}),
        json!({"kind":"upsert","native_key":"1","data":{}}),
    ] {
        assert!(
            mac::prepare(&state, std::slice::from_ref(&bad), &context("c"), false).is_err(),
            "{bad}"
        );
    }
    let duplicate = [
        row("mac.app", "1", &json!({})),
        row("mac.app", "1", &json!({})),
    ];
    assert!(mac::prepare(&state, &duplicate, &context("c"), false).is_err());
}
