use ab_native::calendar_store::Enrichment;
use rusqlite::Connection;
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::PathBuf};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

fn normalized(value: &Value) -> Value {
    match value {
        Value::Number(number) => number
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| value.clone(), Value::Number),
        Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), normalized(item)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn prepare(name: &str) -> Fallible<PathBuf> {
    let root = std::env::temp_dir().join(format!("ab-calstore-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("Attachments"))?;
    let connection = Connection::open(root.join("Calendar.sqlitedb"))?;
    connection.execute_batch(include_str!("parity/calendar-store/schema.sql"))?;
    fs::write(root.join("Attachments/a.pdf"), b"pdf-bytes-1")?;
    fs::write(root.join("Attachments/b.txt"), b"hello")?;
    Ok(root)
}

fn run(root: PathBuf) -> Fallible<(Vec<Value>, Vec<String>)> {
    let observations: Vec<Value> =
        serde_json::from_str(include_str!("parity/calendar-store/observations.json"))?;
    let lookups: Value = serde_json::from_str(include_str!("parity/calendar-store/lookups.json"))?;
    let mut keyed: BTreeMap<String, Value> = BTreeMap::new();
    for observation in observations {
        keyed.insert(
            format!(
                "{}/{}",
                observation["collection"].as_str().unwrap_or_default(),
                observation["native_key"].as_str().unwrap_or_default()
            ),
            observation,
        );
    }
    let ordered: Vec<Value> = keyed.values().cloned().collect();
    let mut enrichment = Enrichment::new(root);
    let enriched = enrichment.enrich(
        &ordered,
        &|native| lookups[native]["external"].as_str().unwrap_or_default().to_owned(),
        true,
    );
    Ok((enriched, enrichment.handles().keys().cloned().collect()))
}

#[test]
fn rust_enrichment_matches_the_swift_oracle_on_the_fixture_store() -> Fallible<()> {
    let expected: Value = serde_json::from_str(include_str!("parity/calendar-store/expected.json"))?;
    let (enriched, digests) = run(prepare("oracle")?)?;
    let want = expected["observations"].as_array().ok_or("observations")?;
    assert_eq!(enriched.len(), want.len());
    for (got, wanted) in enriched.iter().zip(want) {
        assert_eq!(normalized(got), normalized(wanted), "{}", got["native_key"]);
    }
    assert_eq!(
        serde_json::to_value(&digests)?,
        expected["blob_digests"],
        "attachment digests"
    );
    Ok(())
}

#[test]
fn a_missing_or_unreadable_store_is_unsupported_and_never_invents_fields() -> Fallible<()> {
    let root = std::env::temp_dir().join(format!("ab-calstore-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let (enriched, digests) = run(root)?;
    assert!(digests.is_empty());
    let event = enriched
        .iter()
        .find(|row| row["native_key"] == "ID-B")
        .ok_or("event")?;
    assert_eq!(event["data"]["store_status"], "unsupported");
    assert_eq!(
        event["data"]["store_coverage"]["travel_time"]["reason"],
        "calendar_store_unavailable_or_schema_unverified"
    );
    Ok(())
}

#[test]
fn attachment_bytes_are_served_in_bounded_confined_chunks() -> Fallible<()> {
    let root = prepare("blob")?;
    let observations: Vec<Value> =
        serde_json::from_str(include_str!("parity/calendar-store/observations.json"))?;
    let mut enrichment = Enrichment::new(root);
    enrichment.enrich(&observations, &|_| String::new(), true);
    let handle = enrichment
        .handles()
        .values()
        .next()
        .cloned()
        .ok_or("handle")?;
    let chunk = enrichment.read_blob(&handle, 0, 4)?;
    assert_eq!(chunk["eof"], false);
    assert!(enrichment.read_blob(&handle, 0, 0).is_err());
    assert!(enrichment.read_blob(&handle, 0, 1 << 20).is_err());
    assert!(enrichment.read_blob("calendar-attachment:unknown", 0, 4).is_err());
    Ok(())
}
