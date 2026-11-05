#![allow(clippy::expect_used)]
use ab_state::{Gateway, State, Visibility};
use serde_json::{Value, json};

fn state_with_replica_marker(home: &std::path::Path) -> State<Gateway> {
    let mut state = State::<Gateway>::open(home).expect("state");
    state
        .set_replica_status(json!({
            "role": "gateway",
            "cursor": {"epoch": "synthetic-private-epoch", "sequence": 4242},
            "gateway_head": 4242,
            "blobs": {"entries": {}},
        }))
        .expect("status");
    state
}

fn visibility(names: &[&str]) -> Visibility {
    names.iter().map(|name| ((*name).to_owned(), true)).collect()
}

fn sources_status(state: &State<Gateway>, names: &[&str]) -> Value {
    state
        .read_model(&visibility(names), "sources.status", &json!({}))
        .expect("sources.status")
}

#[test]
fn sources_status_hides_replica_state_from_a_connector_without_system_visibility() {
    let home = tempfile::tempdir().expect("home");
    let state = state_with_replica_marker(home.path());
    let status = sources_status(&state, &["messages"]);
    assert!(status.get("replica").is_none(), "{status}");
    assert!(!status.to_string().contains("synthetic-private-epoch"));
}

#[test]
fn sources_status_keeps_replica_state_for_system_and_admin_visibility() {
    let home = tempfile::tempdir().expect("home");
    let state = state_with_replica_marker(home.path());
    for names in [&["system"][..], &["admin"][..]] {
        assert_eq!(
            sources_status(&state, names)["replica"]["gateway_head"],
            4242,
            "{names:?}"
        );
    }
}
