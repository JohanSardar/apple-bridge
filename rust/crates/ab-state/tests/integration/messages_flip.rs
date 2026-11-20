#![allow(clippy::expect_used)]
use ab_state::{Gateway, State};
use ab_wire::messages::Capture;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::path::Path;

const SOURCE_DATA: &str = "2026-11-03T00:00:00Z";
const REPLACED: &str = "2026-11-03T01:00:00Z";

fn capture(
    generation: &str,
    predecessor: &str,
    checkpoint: &str,
    at: &str,
    rows: &[Value],
) -> Capture {
    let observations: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            row["observed_at"] = json!(at);
            row["revalidated"] = json!(true);
            row["scoped_alias"] = json!(format!(
                "{}/{}",
                row["collection"].as_str().unwrap_or(""),
                row["native_key"].as_str().unwrap_or("")
            ));
            row
        })
        .collect();
    serde_json::from_value(json!({
        "source": "messages",
        "generation": generation,
        "predecessor": predecessor,
        "checkpoint": checkpoint,
        "observations": observations,
        "scan_id": checkpoint,
        "scope": ["messages.attachment"],
        "scan_started_at": at,
        "completion_evidence": "complete",
    }))
    .expect("capture")
}

fn upsert(key: &str, data: &Value) -> Value {
    json!({"kind":"upsert","collection":"messages.attachment","native_key":key,"data":data})
}

fn rows(home: &Path) -> Vec<(String, bool, Value)> {
    let db = Connection::open(home.join("state.db")).expect("open");
    let mut statement = db
        .prepare("SELECT a.native_key,r.deleted,r.payload FROM records r JOIN aliases a ON a.record_id=r.id AND a.collection=r.collection WHERE r.collection='messages.attachment' GROUP BY a.native_key ORDER BY a.native_key")
        .expect("prepare");
    statement
        .query_map([], |row| {
            let payload: Vec<u8> = row.get(2)?;
            let record: Value = serde_json::from_slice(&payload).expect("payload");
            Ok((
                row.get(0)?,
                row.get::<_, i64>(1)? != 0,
                record["data"].clone(),
            ))
        })
        .expect("query")
        .map(|row| row.expect("row"))
        .collect()
}

#[test]
fn rust_generation_flip_keeps_digests_and_derivatives() {
    let home = std::env::temp_dir().join(format!("ab-flip-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("home");
    let derived = json!({
        "blob": {"state":"present","digest":"d1"},
        "mime_type": "image/jpeg",
        "media": {"width": 640, "height": 480, "thumbhash": "th"},
        "derivatives": {"thumb": {"digest":"t1","size":9,"mime":"image/jpeg","width":64,"height":48,"state":"present"}}
    });
    let offloaded = json!({
        "blob": {"state":"offloaded","digest":"d2"},
        "media": {"duration_ms": 5},
        "derivatives": {"video": {"digest":"v2","size":9,"mime":"video/mp4","state":"present"}}
    });
    let plain = json!({"blob": {"state":"present","digest":"d3"}, "mime_type":"text/plain"});
    let gone = json!({"blob": {"state":"present","digest":"d4"}});
    {
        let mut store = State::<Gateway>::open(&home).expect("state");
        store
            .commit_capture(&capture(
                "swift",
                "",
                "1",
                SOURCE_DATA,
                &[
                    upsert("derived", &derived),
                    upsert("offloaded", &offloaded),
                    upsert("plain", &plain),
                    upsert("gone", &gone),
                ],
            ))
            .expect("seed");
        store
            .commit_capture(&capture(
                "rust",
                "",
                "r1",
                REPLACED,
                &[
                    upsert("derived", &json!({"blob":{"state":"present","digest":"d1"},"mime_type":"image/jpeg"}),
                    ),
                    upsert("offloaded", &json!({"blob":{"state":"offloaded"}})),
                    upsert("plain", &plain),
                    json!({"kind":"absent","collection":"messages.attachment","native_key":"gone","data":{}}),
                ],
            ))
            .expect("flip");
    }
    let after = rows(&home);
    let by = |key: &str| after.iter().find(|row| row.0 == key).expect("row");
    assert_eq!(by("derived").2, derived);
    assert_eq!(by("offloaded").2, offloaded);
    assert_eq!(by("plain").2, plain);
    assert!(!by("derived").1 && !by("offloaded").1 && !by("plain").1);
    assert!(by("gone").1);
    std::fs::remove_dir_all(home).expect("cleanup");
}

#[test]
fn changed_file_replaces_known_derivatives() {
    let home = std::env::temp_dir().join(format!("ab-flip-changed-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("home");
    {
        let mut store = State::<Gateway>::open(&home).expect("state");
        store
            .commit_capture(&capture(
                "swift",
                "",
                "1",
                SOURCE_DATA,
                &[upsert("a", &json!({"blob":{"state":"present","digest":"d1"},"derivatives":{"thumb":{"digest":"t1","state":"present"}}}),
                )],
            ))
            .expect("seed");
        store
            .commit_capture(&capture(
                "rust",
                "",
                "r1",
                REPLACED,
                &[upsert(
                    "a",
                    &json!({"blob":{"state":"present","digest":"d9"}}),
                )],
            ))
            .expect("flip");
    }
    let after = rows(&home);
    assert_eq!(
        after[0].2,
        json!({"blob":{"state":"present","digest":"d9"}})
    );
    std::fs::remove_dir_all(home).expect("cleanup");
}

#[test]
fn a_failed_media_answer_does_not_empty_known_derivatives() {
    let home = std::env::temp_dir().join(format!("ab-flip-failed-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("home");
    let known = json!({
        "blob": {"state":"present","digest":"d1"},
        "media": {"width": 640},
        "derivatives": {"thumb": {"digest":"t1","state":"present"}}
    });
    {
        let mut store = State::<Gateway>::open(&home).expect("state");
        store
            .commit_capture(&capture(
                "swift",
                "",
                "1",
                SOURCE_DATA,
                &[upsert("a", &known)],
            ))
            .expect("seed");
        store
            .commit_capture(&capture(
                "rust",
                "",
                "r1",
                REPLACED,
                &[upsert(
                    "a",
                    &json!({"blob":{"state":"present","digest":"d1"},"media":{},"derivatives":{}}),
                )],
            ))
            .expect("failed answer");
    }
    assert_eq!(rows(&home)[0].2, known);
    std::fs::remove_dir_all(home).expect("cleanup");
}
