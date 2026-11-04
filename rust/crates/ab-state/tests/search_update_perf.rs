#![allow(clippy::expect_used)]
use ab_state::{Gateway, State, Visibility};
use ab_wire::messages::Capture;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

const QUERIES: [&str; 5] = [
    "imaginary garden",
    "paper boats",
    "prototype",
    "Synthetic Studio",
    "ftsDefinitelyAbsent",
];

struct Report {
    checks: Vec<(String, bool)>,
    numbers: BTreeMap<String, f64>,
    counts: BTreeMap<String, usize>,
}
impl Report {
    fn check(&mut self, name: &str, passed: bool) {
        self.checks.push((name.to_owned(), passed));
    }
    fn time(&mut self, name: &str, started: Instant) {
        self.numbers
            .insert(name.to_owned(), started.elapsed().as_secs_f64() * 1000.0);
    }
}

struct Harness {
    home: std::path::PathBuf,
    store: Option<State<Gateway>>,
    visibility: Visibility,
    checkpoints: BTreeMap<String, String>,
    number: u64,
    through: (u64, u64),
}
impl Harness {
    fn store(&mut self) -> &mut State<Gateway> {
        self.store.as_mut().expect("store open")
    }
    fn commit(&mut self, source: &str, mut observations: Vec<Value>) -> Result<(), String> {
        self.number += 1;
        let checkpoint = self.number.to_string();
        let now = "2026-11-03T00:00:00Z";
        let mut collections: Vec<String> = vec![];
        for observation in &mut observations {
            observation["observed_at"] = json!(now);
            observation["revalidated"] = json!(true);
            let collection = observation["collection"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if !collections.contains(&collection) {
                collections.push(collection);
            }
        }
        let capture: Capture = serde_json::from_value(json!({
            "source": source,
            "generation": "fts-demo",
            "predecessor": self.checkpoints.get(source).cloned().unwrap_or_default(),
            "checkpoint": checkpoint,
            "observations": observations,
            "scan_id": checkpoint,
            "scope": collections,
            "scan_started_at": now,
            "completion_evidence": "complete",
        }))
        .map_err(|e| e.to_string())?;
        let result = self
            .store()
            .commit_capture(&capture)
            .map_err(|e| e.to_string())?;
        self.through = (result.after, result.through);
        self.checkpoints.insert(source.to_owned(), checkpoint);
        Ok(())
    }
    fn matches(&mut self, query: &str) -> Result<Vec<String>, String> {
        let visibility = self.visibility.clone();
        let mut ids = vec![];
        let mut after = String::new();
        loop {
            let page = self
                .store()
                .read(
                    &visibility,
                    &ab_state::ReadQuery {
                        text: query.to_owned(),
                        after: after.clone(),
                        limit: 1000,
                        ..ab_state::ReadQuery::default()
                    },
                )
                .map_err(|e| e.to_string())?;
            for record in &page.records {
                ids.push(format!(
                    "{}/{}",
                    serde_json::to_value(record.collection)
                        .map_err(|e| e.to_string())?
                        .as_str()
                        .unwrap_or_default(),
                    record.id.as_str()
                ));
            }
            if page.next.is_empty() {
                return Ok(ids);
            }
            after = page.next;
        }
    }
    fn reopen(&mut self) -> Result<(), String> {
        self.store = None;
        self.store = Some(State::<Gateway>::open(&self.home).map_err(|e| e.to_string())?);
        Ok(())
    }
}

fn database(home: &Path) -> Connection {
    let database = Connection::open(home.join("state.db")).expect("maintenance connection");
    database
        .busy_timeout(Duration::from_secs(5))
        .expect("busy timeout");
    database
}
fn count(database: &Connection, sql: &str) -> i64 {
    database.query_row(sql, [], |row| row.get(0)).unwrap_or(-1)
}
fn identities(database: &Connection) -> BTreeMap<String, String> {
    let mut statement = database
        .prepare("SELECT collection,record_id,native_key FROM aliases WHERE generation='fts-demo'")
        .expect("alias query");
    statement
        .query_map([], |row| {
            Ok((
                format!("{}/{}", row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                format!("{}/{}", row.get::<_, String>(0)?, row.get::<_, String>(2)?),
            ))
        })
        .expect("aliases")
        .filter_map(Result::ok)
        .collect()
}
fn snapshot(
    harness: &mut Harness,
    identities: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut out = BTreeMap::new();
    for query in QUERIES {
        let mut ids: Vec<String> = harness
            .matches(query)?
            .into_iter()
            .map(|id| identities.get(&id).cloned().unwrap_or(id))
            .collect();
        ids.sort();
        out.insert(query.to_owned(), ids);
    }
    Ok(out)
}
fn updates(targets: &[Value], iteration: i64) -> Vec<Value> {
    targets
        .iter()
        .map(|original| {
            let mut next = original.clone();
            next["data"]["read_at"] = json!(lab_demo::ts(lab_demo::EPOCH + iteration + 1));
            next
        })
        .collect()
}

#[test]
#[ignore = "104,900-message dataset; run by `make rust-search-perf` in release mode"]
fn search_stays_correct_while_one_hundred_messages_change() {
    let report = run();
    for (name, value) in &report.numbers {
        eprintln!("speed {name}: {value:.1} ms");
    }
    for (query, matches) in &report.counts {
        eprintln!("matches {query}: {matches}");
    }
    if let Ok(path) = std::env::var("AB_SEARCH_PERF_OUT") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&json!({"speed_ms": report.numbers, "matches": report.counts, "checks": report.checks}))
                .expect("report"),
        )
        .expect("write report");
    }
    let failed: Vec<_> = report
        .checks
        .iter()
        .filter(|(_, passed)| !passed)
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(failed.is_empty(), "failed assertions: {failed:?}");
}

#[allow(clippy::too_many_lines)]
fn run() -> Report {
    let mut report = Report {
        checks: vec![],
        numbers: BTreeMap::new(),
        counts: BTreeMap::new(),
    };
    let directory = tempfile::tempdir().expect("private store");
    let mut harness = Harness {
        home: directory.path().to_owned(),
        store: None,
        visibility: Visibility::new(),
        checkpoints: BTreeMap::new(),
        number: 0,
        through: (0, 0),
    };
    let opened = Instant::now();
    harness.store = Some(State::<Gateway>::open(&harness.home).expect("open"));
    report.time("fixture_startup", opened);
    let dataset = lab_demo::generate(lab_demo::Options {
        seed: 1,
        huge: true,
    });
    report.check(
        "unchanged_huge_demo_104900_messages",
        dataset.counts.get("messages.message") == Some(&104_900),
    );
    let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut targets = vec![];
    for record in &dataset.records {
        let collection = record["collection"].as_str().unwrap_or_default();
        let source = collection.split('.').next().unwrap_or_default().to_owned();
        harness.visibility.insert(source.clone(), true);
        grouped.entry(source).or_default().push(record.clone());
        if collection == "messages.message"
            && record["native_key"]
                .as_str()
                .is_some_and(|key| key.starts_with("demo-message-39-"))
            && targets.len() < 100
        {
            targets.push(record.clone());
        }
    }
    harness.visibility.insert("calendar".into(), true);
    let seeding = Instant::now();
    for (source, records) in &grouped {
        for chunk in records.chunks(2048) {
            if let Err(error) = harness.commit(source, chunk.to_vec()) {
                eprintln!("huge demo capture failed: {error}");
                report.check("huge_demo_captured", false);
                return report;
            }
        }
    }
    report.time("huge_demo_seed", seeding);
    let database = database(&harness.home);
    report.check(
        "persisted_104900_messages",
        count(
            &database,
            "SELECT count(*) FROM records WHERE collection='messages.message' AND deleted=0",
        ) == 104_900,
    );
    let identities = identities(&database);
    let Ok(before) = snapshot(&mut harness, &identities) else {
        report.check("baseline_search_sets", false);
        return report;
    };
    for (query, ids) in &before {
        report.counts.insert(query.clone(), ids.len());
        let digest = Sha256::digest(ids.join("\n"))
            .iter()
            .fold(String::new(), |text, byte| text + &format!("{byte:02x}"));
        eprintln!("query_set_sha256_{query}: {digest}");
    }
    let mut timings = vec![];
    let mut all_hundred = true;
    for iteration in 0..5 {
        let started = Instant::now();
        if let Err(error) = harness.commit("messages", updates(&targets, iteration)) {
            eprintln!("hundred record capture failed: {error}");
            report.check("hundred_record_capture", false);
            return report;
        }
        timings.push(started.elapsed().as_secs_f64() * 1000.0);
        all_hundred &= harness.through.1 - harness.through.0 == 100;
    }
    timings.sort_by(f64::total_cmp);
    report.numbers.insert(
        "hundred_record_capture_commit_max".into(),
        timings.last().copied().unwrap_or(0.0),
    );
    report.check(
        "five_samples_commit_exactly_100_updates",
        timings.len() == 5 && all_hundred && targets.len() == 100,
    );
    report.check(
        "fixed_search_sets_equal_after_updates",
        snapshot(&mut harness, &identities).is_ok_and(|after| after == before),
    );
    let target_native = targets[0]["native_key"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let target_id: Option<String> = database
        .query_row(
            "SELECT record_id FROM aliases WHERE source='messages' AND generation='fts-demo' AND collection='messages.message' AND native_key=?",
            [&target_native],
            |row| row.get(0),
        )
        .ok();
    let Some(target_id) = target_id else {
        report.check("target_identity", false);
        return report;
    };
    harness.store = None;
    let legacy = database.execute_batch(
        "DROP TRIGGER IF EXISTS records_search_insert; DROP TRIGGER IF EXISTS records_search_update; DROP TRIGGER IF EXISTS records_search_delete; DROP TABLE IF EXISTS records_fts_map; UPDATE meta SET value='2' WHERE key='fts_version'",
    );
    report.check("legacy_v2_fixture", legacy.is_ok());
    let started = Instant::now();
    let reopened = harness.reopen();
    report.time("migration_startup_and_rebuild", started);
    report.check("migration_open", reopened.is_ok());
    if reopened.is_err() {
        return report;
    }
    report.check(
        "search_ready_when_open_returns",
        harness
            .matches("prototype")
            .is_ok_and(|ids| !ids.is_empty()),
    );
    let visibility = harness.visibility.clone();
    report.check(
        "ordinary_reads_available_after_migration",
        harness
            .store()
            .read(
                &visibility,
                &ab_state::ReadQuery {
                    limit: 1,
                    ..ab_state::ReadQuery::default()
                },
            )
            .is_ok(),
    );
    let started = Instant::now();
    report.check(
        "capture_progress_after_migration",
        harness.commit("messages", updates(&targets, 30)).is_ok(),
    );
    report.time("capture_after_migration", started);
    let version: Option<String> = database
        .query_row(
            "SELECT value FROM meta WHERE key='fts_version'",
            [],
            |row| row.get(0),
        )
        .ok();
    report.check("fts_version_five", version.as_deref() == Some("5"));
    report.check(
        "fixed_search_sets_equal_after_rebuild",
        snapshot(&mut harness, &identities).is_ok_and(|after| after == before),
    );
    let stable_id: i64 = database
        .query_row(
            "SELECT fts_rowid FROM records_fts_map WHERE collection='messages.message' AND id=?",
            [&target_id],
            |row| row.get(0),
        )
        .unwrap_or(-1);
    harness.store = None;
    let moved = database
        .execute(
            "UPDATE records SET rowid=(SELECT max(rowid)+1024 FROM records) WHERE collection='messages.message' AND id=?",
            [&target_id],
        )
        .and_then(|_| database.execute_batch("VACUUM"));
    report.check("vacuum_completed", moved.is_ok());
    if harness.reopen().is_err() {
        report.check("vacuum_reopen", false);
        return report;
    }
    if harness.commit("messages", updates(&targets, 31)).is_err() {
        report.check("capture_after_vacuum", false);
        return report;
    }
    let stable_after: i64 = database
        .query_row(
            "SELECT fts_rowid FROM records_fts_map WHERE collection='messages.message' AND id=?",
            [&target_id],
            |row| row.get(0),
        )
        .unwrap_or(-2);
    report.check(
        "stable_fts_key_after_record_rowid_change_and_vacuum",
        stable_id > 0 && stable_id == stable_after,
    );
    report.check(
        "fixed_search_sets_equal_after_vacuum_update",
        snapshot(&mut harness, &identities).is_ok_and(|after| after == before),
    );
    let mut changed = updates(&targets, 32)
        .into_iter()
        .take(1)
        .collect::<Vec<_>>();
    changed[0]["data"]["text"] = json!("ftsUniqueReplacement");
    let committed = harness.commit("messages", changed.clone());
    let ids = harness.matches("ftsUniqueReplacement");
    report.check(
        "changed_text_indexed_once",
        committed.is_ok()
            && ids.is_ok_and(|ids| ids == vec![format!("messages.message/{target_id}")]),
    );
    changed[0]["kind"] = json!("absent");
    let committed = harness.commit("messages", changed);
    report.check(
        "delete_removes_search_match",
        committed.is_ok()
            && harness
                .matches("ftsUniqueReplacement")
                .is_ok_and(|ids| ids.is_empty()),
    );
    let mut calendar = json!({"kind":"upsert","collection":"calendar.event","native_key":"fts-evict","data":{"title":"ftsUniqueEviction"}});
    let committed = harness.commit("calendar", vec![calendar.clone()]);
    report.check(
        "calendar_search_match",
        committed.is_ok()
            && harness
                .matches("ftsUniqueEviction")
                .is_ok_and(|ids| ids.len() == 1),
    );
    calendar["kind"] = json!("evict");
    let committed = harness.commit("calendar", vec![calendar]);
    report.check(
        "eviction_removes_search_match",
        committed.is_ok()
            && harness
                .matches("ftsUniqueEviction")
                .is_ok_and(|ids| ids.is_empty()),
    );
    harness.visibility.remove("calendar");
    let visibility = harness.visibility.clone();
    let restricted = harness.store().restrict(&visibility);
    report.check(
        "deleted_and_evicted_fts_rows_absent",
        restricted.is_ok()
            && count(
                &database,
                "SELECT count(*) FROM records_fts f LEFT JOIN records r ON r.collection=f.collection AND r.id=f.id WHERE r.id IS NULL OR r.deleted!=0",
            ) == 0,
    );
    report.check(
        "hard_delete_removes_mapping",
        count(
            &database,
            "SELECT count(*) FROM records_fts_map m LEFT JOIN records r ON r.collection=m.collection AND r.id=m.id WHERE r.id IS NULL",
        ) == 0,
    );
    report
}
