use ab_state::{Gateway, ReadQuery, State, Visibility};
use ab_wire::messages::Capture;
use serde_json::{Value, json};

type Check = Result<(), Box<dyn std::error::Error>>;

fn observation(key: &str, text: &str) -> Value {
    json!({
        "kind": "upsert",
        "collection": "messages.message",
        "native_key": key,
        "observed_at": "2026-11-03T00:00:00Z",
        "revalidated": true,
        "data": {"text": text, "date": "2026-11-03T00:00:00Z"},
    })
}

fn commit(
    state: &mut State<Gateway>,
    predecessor: &str,
    checkpoint: &str,
    observations: &[Value],
) -> Check {
    let capture: Capture = serde_json::from_value(json!({
        "source": "messages",
        "generation": "search-update",
        "predecessor": predecessor,
        "checkpoint": checkpoint,
        "scan_id": checkpoint,
        "scope": ["messages.message"],
        "scan_started_at": "2026-11-03T00:00:00Z",
        "completion_evidence": "complete",
        "observations": observations,
    }))?;
    state.commit_capture(&capture)?;
    Ok(())
}

fn matches(state: &State<Gateway>, text: &str) -> Result<Vec<String>, ab_state::Error> {
    let visibility = Visibility::from([("messages".into(), true)]);
    let query = ReadQuery {
        text: text.into(),
        ..ReadQuery::default()
    };
    Ok(state
        .read(&visibility, &query)?
        .records
        .iter()
        .map(|record| record.id.as_str().to_owned())
        .collect())
}

#[test]
fn search_tracks_message_updates_without_stale_or_duplicate_matches() -> Check {
    let home = tempfile::tempdir()?;
    let mut state = State::<Gateway>::open(home.path())?;
    let mut message = observation("edited", "imaginary garden");
    let other = observation("unchanged", "paper boats");
    commit(&mut state, "", "seed", &[message.clone(), other])?;
    let original = matches(&state, "imaginary garden")?;
    let untouched = matches(&state, "paper boats")?;
    assert_eq!(original.len(), 1);
    assert_eq!(untouched.len(), 1);
    assert_ne!(original, untouched);

    message["data"]["read_at"] = json!("2026-11-03T00:01:00Z");
    commit(&mut state, "seed", "read", &[message.clone()])?;
    assert_eq!(matches(&state, "imaginary garden")?, original);

    message["data"]["text"] = json!("prototype");
    commit(&mut state, "read", "edit", &[message.clone()])?;
    assert_eq!(matches(&state, "imaginary garden")?, Vec::<String>::new());
    assert_eq!(matches(&state, "prototype")?, original);

    message["kind"] = json!("absent");
    commit(&mut state, "edit", "delete", &[message])?;
    assert_eq!(matches(&state, "prototype")?, Vec::<String>::new());
    assert_eq!(matches(&state, "paper boats")?, untouched);
    Ok(())
}
