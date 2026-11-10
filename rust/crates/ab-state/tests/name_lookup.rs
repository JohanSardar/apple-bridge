#![allow(clippy::expect_used)]
use ab_state::{Gateway, State, Visibility};
use ab_wire::messages::Capture;
use serde_json::{Value, json};

fn person(key: &str, given: &str, middle: &str, family: &str) -> Value {
    json!({
        "kind": "upsert", "collection": "contacts.contact", "native_key": key,
        "observed_at": "2026-11-05T00:00:00Z",
        "data": {"given_name": given, "middle_name": middle, "family_name": family,
                 "phones": [{"value": format!("+1555000{}", key.len())}], "emails": []},
    })
}

fn names(state: &State<Gateway>, visibility: &Visibility, query: &str) -> Vec<String> {
    let found = state
        .read_model(
            visibility,
            "contacts.lookup",
            &json!({"q": query, "limit": 10}),
        )
        .expect("lookup");
    found["records"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            format!(
                "{} {}",
                r["data"]["given_name"].as_str().unwrap_or_default(),
                r["data"]["family_name"].as_str().unwrap_or_default()
            )
        })
        .collect()
}

#[test]
fn full_names_match_in_any_order_ignoring_case_spacing_and_accents() {
    let home = tempfile::tempdir().expect("home");
    let mut state = State::<Gateway>::open(home.path()).expect("state");
    let capture: Capture = serde_json::from_value(json!({
        "source": "contacts", "generation": "g", "predecessor": "", "checkpoint": "1", "scan_id": "1",
        "scope": ["contacts.contact"], "scan_started_at": "2026-11-05T00:00:00Z",
        "completion_evidence": "complete",
        "observations": [
            person("synthetic-a", "Ana", "", "Novak"),
            person("synthetic-bb", "Ana", "", "Kovač"),
            person("synthetic-ccc", "Marko", "", "Horvat"),
        ],
    }))
    .expect("capture");
    state.commit_capture(&capture).expect("commit");
    let visibility: Visibility = [("contacts".to_owned(), true)].into_iter().collect();
    assert_eq!(names(&state, &visibility, "Ana Novak"), ["Ana Novak"]);
    assert_eq!(names(&state, &visibility, "novak  ana"), ["Ana Novak"]);
    assert_eq!(names(&state, &visibility, "Marko Horvat"), ["Marko Horvat"]);
    assert_eq!(names(&state, &visibility, "Ana Kovac"), ["Ana Kovač"]);
    assert_eq!(names(&state, &visibility, "ANA").len(), 2);
    assert!(names(&state, &visibility, "Ana Horvat").is_empty());
}

#[test]
fn calendar_ranges_accept_dates_in_the_given_zone() {
    let home = tempfile::tempdir().expect("home");
    let state = State::<Gateway>::open(home.path()).expect("state");
    let visibility: Visibility = [("calendar".to_owned(), true)].into_iter().collect();
    let found = state.read_model(
        &visibility,
        "calendar.range",
        &json!({"start": "2026-11-10", "end": "2026-11-11", "time_zone": "America/Chicago"}),
    );
    assert!(found.is_ok(), "{found:?}");
    assert!(
        state
            .read_model(
                &visibility,
                "calendar.range",
                &json!({"start": "next week", "end": "2026-11-11", "time_zone": "America/Chicago"})
            )
            .is_err()
    );
}
