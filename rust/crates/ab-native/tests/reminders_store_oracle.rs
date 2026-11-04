use ab_native::reminders_enrichment::Enrichment;
use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;

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
                .filter(|(key, _)| key.as_str() != "observed_at")
                .map(|(key, item)| (key.clone(), normalized(item)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn replay(name: &str, schema: &str, steps: &str) -> Fallible<Vec<Value>> {
    let path = std::env::temp_dir().join(format!("ab-remstore-{name}-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let database = Connection::open(&path)?;
    database.execute_batch(schema)?;
    let steps: Vec<Value> = serde_json::from_str(steps)?;
    let location: PathBuf = path.clone();
    let mut enrichment = Enrichment::new(Box::new(move || Ok(vec![location.clone()])));
    let mut results = Vec::new();
    for step in steps {
        let sql = step["sql"].as_str().unwrap_or_default();
        if !sql.is_empty() {
            database.execute_batch(sql)?;
        }
        let observations: Vec<Value> = step["observations"].as_array().cloned().unwrap_or_default();
        let refreshed = enrichment.refresh(&observations, "oracle");
        results.push(serde_json::json!({"label": step["label"], "observations": refreshed}));
    }
    let _ = std::fs::remove_file(&path);
    Ok(results)
}

fn compare(got: &[Value], expected: &str) -> Fallible<()> {
    let expected: Vec<Value> = serde_json::from_str(expected)?;
    assert_eq!(got.len(), expected.len());
    for (got, wanted) in got.iter().zip(&expected) {
        let label = wanted["label"].as_str().unwrap_or_default();
        let left = normalized(&got["observations"]);
        let right = normalized(&wanted["observations"]);
        let (left, right) = (left.as_array().ok_or("rows")?, right.as_array().ok_or("rows")?);
        assert_eq!(left.len(), right.len(), "{label}: row count");
        for (a, b) in left.iter().zip(right) {
            assert_eq!(a, b, "{label}: {} {}", a["collection"], a["native_key"]);
        }
    }
    Ok(())
}

#[test]
fn history_mode_matches_the_swift_oracle_step_by_step() -> Fallible<()> {
    let got = replay(
        "history",
        include_str!("parity/reminders-store/schema.sql"),
        include_str!("parity/reminders-store/steps.json"),
    )?;
    compare(&got, include_str!("parity/reminders-store/expected-history.json"))
}

#[test]
fn version_mode_matches_the_swift_oracle_step_by_step() -> Fallible<()> {
    let got = replay(
        "versions",
        include_str!("parity/reminders-store/schema-versions.sql"),
        include_str!("parity/reminders-store/steps-versions.json"),
    )?;
    compare(&got, include_str!("parity/reminders-store/expected-versions.json"))
}

#[test]
fn a_missing_store_reports_unavailable_and_never_invents_fields() -> Fallible<()> {
    let mut enrichment = Enrichment::new(Box::new(|| Ok(Vec::new())));
    let observation = serde_json::json!({"kind":"upsert","collection":"reminders.reminder","native_key":"x","data":{"title":"t"}});
    let rows = enrichment.refresh(&[observation], "g");
    assert_eq!(rows[0]["data"]["store_status"], "unavailable");
    assert_eq!(rows[0]["data"]["title"], "t");
    assert_eq!(rows[0]["data"]["flagged"], Value::Null);
    Ok(())
}
