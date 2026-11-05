use crate::Error;
use jiff::{SignedDuration, Timestamp, Zoned, civil::Date, tz::TimeZone};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap, fs, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf, sync::Mutex,
};

const REFERENCE_EPOCH: i64 = 978_307_200;
const WINDOW_DAYS: i64 = 730;
const NEAR_DAYS: i64 = 30;

pub const PRESENT: &str = "present";
pub const MISSING: &str = "missing";
pub const OUTSIDE_SCOPE: &str = "outside_scope";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    Identifier(String),
    External(String),
    Window {
        start: Timestamp,
        end: Timestamp,
        calendar: Option<String>,
        key: Option<String>,
    },
}

impl Query {
    #[must_use]
    pub fn encode(&self) -> Value {
        match self {
            Self::Identifier(identifier) => json!({"via": "identifier", "identifier": identifier}),
            Self::External(identifier) => json!({"via": "external", "external": identifier}),
            Self::Window {
                start,
                end,
                calendar,
                key,
            } => json!({
                "via": "window",
                "start": start.as_second(),
                "end": end.as_second(),
                "calendar": calendar,
                "key": key,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub selector: Value,
    pub candidate: Value,
}

pub trait Candidates {
    fn candidates(&self, query: &Query) -> Result<Vec<Value>, String>;
    fn calendar_account(&self, calendar: &str) -> Result<Option<String>, String>;
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

fn civil_part(native: &str) -> Option<&str> {
    native.rsplit('|').next().filter(|part| part.len() == 10)
}

fn local_zone() -> TimeZone {
    TimeZone::system()
}

pub fn midnight(civil: &str, zone: &TimeZone) -> Option<Timestamp> {
    let date: Date = civil.parse().ok()?;
    if date.to_string() != civil {
        return None;
    }
    Some(date.to_zoned(zone.clone()).ok()?.timestamp())
}

fn reference_seconds(moment: Timestamp) -> i64 {
    moment.as_second() - REFERENCE_EPOCH
}

fn around(anchor: Timestamp) -> Option<(Timestamp, Timestamp)> {
    let second = SignedDuration::from_secs(1);
    Some((
        anchor.checked_sub(second).ok()?,
        anchor.checked_add(second).ok()?,
    ))
}

#[derive(Debug)]
pub struct Lookups {
    path: Option<PathBuf>,
    table: Mutex<BTreeMap<String, Value>>,
    saved: Mutex<BTreeMap<String, Value>>,
}

impl Lookups {
    pub fn open(path: Option<PathBuf>) -> Result<Self, Error> {
        let table: BTreeMap<String, Value> = match &path {
            Some(path) => match fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(error) => return Err(error.into()),
            },
            None => BTreeMap::new(),
        };
        Ok(Self {
            path,
            saved: Mutex::new(table.clone()),
            table: Mutex::new(table),
        })
    }

    pub fn remember(&self, native: &str, locator: &Value) -> Result<(), Error> {
        if locator.is_object() {
            self.table
                .lock()
                .map_err(|_| Error::WorkerPanicked)?
                .insert(native.to_owned(), locator.clone());
        }
        Ok(())
    }

    pub fn identity(&self, native: &str) -> Result<Option<Value>, Error> {
        Ok(self
            .table
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .get(native)
            .cloned())
    }

    pub fn save(&self) -> Result<(), Error> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let table = self.table.lock().map_err(|_| Error::WorkerPanicked)?;
        let mut saved = self.saved.lock().map_err(|_| Error::WorkerPanicked)?;
        if *table == *saved {
            return Ok(());
        }
        let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(&*table)?)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        saved.clone_from(&table);
        Ok(())
    }
}

fn matches(candidate: &Value, native: &str, identity: &Value) -> Result<bool, String> {
    if candidate["can_materialize"] != true {
        return Err("unavailable".to_owned());
    }
    if text(candidate, "calendar") != text(identity, "calendar")
        || text(candidate, "account") != text(identity, "account")
    {
        return Ok(false);
    }
    if text(candidate, "key") == native {
        return Ok(true);
    }
    let external = text(identity, "external");
    if external.is_empty() || text(candidate, "series") != external {
        return Ok(false);
    }
    if candidate["all_day"] == true
        && let (Some(civil), Some(original)) =
            (civil_part(native), candidate["occurrence_civil"].as_str())
    {
        return Ok(original == civil);
    }
    Ok(candidate["occurrence"] == identity["occurrence"])
}

fn selected(
    source: &dyn Candidates,
    query: &Query,
    native: &str,
    identity: &Value,
    detached_only: bool,
) -> Result<Option<Found>, String> {
    for candidate in source.candidates(query)? {
        if detached_only && candidate["detached"] != true {
            continue;
        }
        if matches(&candidate, native, identity)? {
            return Ok(Some(Found {
                selector: json!({"query": query.encode(), "key": candidate["key"]}),
                candidate,
            }));
        }
    }
    Ok(None)
}

pub fn lookup(
    source: &dyn Candidates,
    lookups: &Lookups,
    native: &str,
) -> Result<Option<Found>, String> {
    let identity = lookups
        .identity(native)
        .map_err(|_| "unavailable".to_owned())?;
    let Some(identity) = identity else {
        return Ok(None);
    };
    let identifier = text(&identity, "event_identifier");
    let external = text(&identity, "external");
    if native.contains('|')
        && let Some(original) = identity["occurrence"]
            .as_str()
            .and_then(|text| text.parse::<Timestamp>().ok())
    {
        let mut originals = vec![original];
        if let Some(civil) = civil_part(native) {
            originals.extend(midnight(civil, &local_zone()));
            originals.extend(midnight(civil, &TimeZone::UTC));
        }
        let anchor = originals.get(1).copied().unwrap_or(original);
        let mut seen = Vec::new();
        for moment in &originals {
            if seen.contains(moment) {
                continue;
            }
            seen.push(*moment);
            let alias = format!("{external}/RID={}", reference_seconds(*moment));
            if let Some(found) =
                selected(source, &Query::External(alias), native, &identity, false)?
            {
                return Ok(Some(found));
            }
        }
        let (start, end) = around(anchor).ok_or("unavailable")?;
        let window = Query::Window {
            start,
            end,
            calendar: None,
            key: None,
        };
        if let Some(found) = selected(source, &window, native, &identity, false)? {
            return Ok(Some(found));
        }
        return selected(
            source,
            &Query::Identifier(identifier.to_owned()),
            native,
            &identity,
            true,
        );
    }
    if let Some(found) = selected(
        source,
        &Query::Identifier(identifier.to_owned()),
        native,
        &identity,
        false,
    )? {
        return Ok(Some(found));
    }
    if !external.is_empty() {
        return selected(
            source,
            &Query::External(external.to_owned()),
            native,
            &identity,
            false,
        );
    }
    Ok(None)
}

fn start_of_today(now: &Zoned) -> Option<Zoned> {
    now.start_of_day().ok()
}

fn shifted(day: &Zoned, days: i64) -> Option<Zoned> {
    day.checked_add(jiff::Span::new().days(days)).ok()
}

#[must_use]
pub fn window(now: &Zoned) -> Option<(Timestamp, Timestamp)> {
    let today = start_of_today(now)?;
    Some((
        shifted(&today, -WINDOW_DAYS)?.timestamp(),
        shifted(&today, WINDOW_DAYS)?.timestamp(),
    ))
}

#[must_use]
pub fn chunks(now: &Zoned) -> Vec<(Timestamp, Timestamp)> {
    let Some(today) = start_of_today(now) else {
        return Vec::new();
    };
    let (Some(first), Some(last), Some(near_start), Some(near_end)) = (
        shifted(&today, -WINDOW_DAYS),
        shifted(&today, WINDOW_DAYS),
        shifted(&today, -NEAR_DAYS),
        shifted(&today, NEAR_DAYS),
    ) else {
        return Vec::new();
    };
    let mut result = vec![(near_start.timestamp(), near_end.timestamp())];
    for (from, to) in [(first, near_start), (near_end, last)] {
        let mut start = from;
        while start.timestamp() < to.timestamp() {
            let Ok(next) = start.checked_add(jiff::Span::new().years(1)) else {
                return result;
            };
            let end = if next.timestamp() < to.timestamp() {
                next
            } else {
                to.clone()
            };
            result.push((start.timestamp(), end.timestamp()));
            start = end;
        }
    }
    result
}

pub fn fresh(
    source: &dyn Candidates,
    lookups: &Lookups,
    native: &str,
) -> Result<Option<Found>, String> {
    if let Some(found) = lookup(source, lookups, native)? {
        return Ok(Some(found));
    }
    for (start, end) in chunks(&Zoned::now()) {
        let query = Query::Window {
            start,
            end,
            calendar: None,
            key: Some(native.to_owned()),
        };
        for candidate in source.candidates(&query)? {
            if candidate["can_materialize"] != true {
                return Err("unavailable".to_owned());
            }
            if text(&candidate, "key") == native {
                lookups
                    .remember(native, &candidate["locator"])
                    .map_err(|_| "unavailable".to_owned())?;
                return Ok(Some(Found {
                    selector: json!({"query": query.encode(), "key": native}),
                    candidate,
                }));
            }
        }
    }
    Ok(None)
}

pub fn known_absent(source: &dyn Candidates, lookups: &Lookups, native: &str) -> bool {
    let Ok(Some(identity)) = lookups.identity(native) else {
        return false;
    };
    let identifier = text(&identity, "event_identifier");
    if identifier.is_empty() {
        return false;
    }
    if !native.contains('|') {
        return source
            .candidates(&Query::Identifier(identifier.to_owned()))
            .is_ok_and(|rows| rows.is_empty());
    }
    let calendar = text(&identity, "calendar");
    let account = text(&identity, "account");
    let same_account = || {
        source
            .calendar_account(calendar)
            .is_ok_and(|found| found.as_deref() == Some(account))
    };
    if !same_account() {
        return false;
    }
    let original = match civil_part(native) {
        Some(civil) => midnight(civil, &local_zone()),
        None => identity["occurrence"]
            .as_str()
            .and_then(|text| text.parse().ok()),
    };
    let Some((start, end)) = original.and_then(around) else {
        return false;
    };
    let window = Query::Window {
        start,
        end,
        calendar: Some(calendar.to_owned()),
        key: None,
    };
    let Ok(captured) = source.candidates(&window) else {
        return false;
    };
    if captured.iter().any(|row| row["can_materialize"] != true) {
        return false;
    }
    let Ok(missing) = lookup(source, lookups, native).map(|found| found.is_none()) else {
        return false;
    };
    same_account() && missing && !captured.iter().any(|row| text(row, "key") == native)
}

fn date_of(value: &str, all_day: bool, zone: &TimeZone) -> Option<Timestamp> {
    if all_day {
        return midnight(value, zone);
    }
    value.parse().ok()
}

fn zone_of(name: &str) -> TimeZone {
    TimeZone::get(name).unwrap_or_else(|_| local_zone())
}

pub fn revalidate(
    source: &dyn Candidates,
    lookups: &Lookups,
    native: &str,
    data: &Value,
    now: &Zoned,
) -> Result<&'static str, String> {
    let (first, last) = window(now).ok_or("unavailable")?;
    if let Some(found) = lookup(source, lookups, native)? {
        let value = &found.candidate;
        let all_day = value["all_day"] == true;
        let zone = if all_day {
            local_zone()
        } else {
            zone_of(text(value, "time_zone"))
        };
        let (Some(start), Some(end)) = (
            date_of(text(value, "start"), all_day, &zone),
            date_of(text(value, "end"), all_day, &zone),
        ) else {
            return Ok(PRESENT);
        };
        return Ok(if start >= last || end <= first {
            OUTSIDE_SCOPE
        } else {
            PRESENT
        });
    }
    let captured = match text(data, "occurrence_date") {
        "" => text(data, "start"),
        occurrence => occurrence,
    };
    let Some(moment) = date_of(captured, data["all_day"] == true, &local_zone()) else {
        return Ok(PRESENT);
    };
    Ok(if moment < first || moment >= last {
        OUTSIDE_SCOPE
    } else {
        MISSING
    })
}

pub fn take_locator(row: &mut Map<String, Value>) -> Option<Value> {
    row.remove("locator")
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Store {
        identifier: BTreeMap<String, Vec<Value>>,
        external: BTreeMap<String, Vec<Value>>,
        window: Vec<Value>,
        accounts: BTreeMap<String, String>,
        asked: RefCell<Vec<Query>>,
        failing: bool,
    }

    impl Candidates for Store {
        fn candidates(&self, query: &Query) -> Result<Vec<Value>, String> {
            self.asked.borrow_mut().push(query.clone());
            if self.failing {
                return Err("unavailable".into());
            }
            Ok(match query {
                Query::Identifier(id) => self.identifier.get(id).cloned().unwrap_or_default(),
                Query::External(id) => self.external.get(id).cloned().unwrap_or_default(),
                Query::Window { key, .. } => self
                    .window
                    .iter()
                    .filter(|row| key.as_ref().is_none_or(|key| text(row, "key") == key))
                    .cloned()
                    .collect(),
            })
        }
        fn calendar_account(&self, calendar: &str) -> Result<Option<String>, String> {
            Ok(self.accounts.get(calendar).cloned())
        }
    }

    fn event(key: &str) -> Value {
        json!({"key": key, "can_materialize": true, "calendar": "c", "account": "a", "series": "ext",
            "occurrence": "2026-03-08T16:00:00.000Z", "all_day": false, "detached": false,
            "start": "2026-03-08T16:00:00Z", "end": "2026-03-08T17:00:00Z", "time_zone": "UTC"})
    }

    fn identity() -> Value {
        json!({"event_identifier": "id1", "calendar": "c", "account": "a", "external": "ext", "occurrence": "2026-03-08T16:00:00.000Z"})
    }

    fn table(native: &str) -> Lookups {
        let lookups = Lookups::open(None).unwrap();
        lookups.remember(native, &identity()).unwrap();
        lookups
    }

    #[test]
    fn an_unknown_record_finds_nothing_and_asks_nothing() {
        let store = Store::default();
        let lookups = Lookups::open(None).unwrap();
        assert!(lookup(&store, &lookups, "k").unwrap().is_none());
        assert!(store.asked.borrow().is_empty());
    }

    #[test]
    fn a_plain_event_is_found_by_id_then_by_its_external_id() {
        let mut store = Store::default();
        store.identifier.insert("id1".into(), vec![event("k")]);
        let found = lookup(&store, &table("k"), "k").unwrap().unwrap();
        assert_eq!(found.selector["key"], "k");
        assert_eq!(found.selector["query"]["via"], "identifier");
        let mut moved = Store::default();
        moved
            .external
            .insert("ext".into(), vec![event("other-key")]);
        let found = lookup(&moved, &table("k"), "k").unwrap().unwrap();
        assert_eq!(found.selector["query"]["via"], "external");
    }

    #[test]
    fn another_calendar_or_account_is_never_the_event() {
        for (field, value) in [("calendar", "z"), ("account", "z")] {
            let mut wrong = event("k");
            wrong[field] = json!(value);
            let mut store = Store::default();
            store.identifier.insert("id1".into(), vec![wrong]);
            assert!(
                lookup(&store, &table("k"), "k").unwrap().is_none(),
                "{field}"
            );
        }
    }

    #[test]
    fn a_partial_event_is_a_failure_not_an_absence() {
        let mut broken = event("k");
        broken["can_materialize"] = json!(false);
        let mut store = Store::default();
        store.identifier.insert("id1".into(), vec![broken]);
        assert!(lookup(&store, &table("k"), "k").is_err());
    }

    #[test]
    fn an_occurrence_goes_alias_then_window_then_detached_id() {
        let native = "k|2026-03-08T16:00:00.000Z";
        let original: Timestamp = "2026-03-08T16:00:00Z".parse().unwrap();
        let alias = format!("ext/RID={}", original.as_second() - REFERENCE_EPOCH);
        let mut store = Store::default();
        store.external.insert(alias.clone(), vec![event(native)]);
        let found = lookup(&store, &table(native), native).unwrap().unwrap();
        assert_eq!(found.selector["query"]["external"], alias);

        let by_window = Store {
            window: vec![event(native)],
            ..Store::default()
        };
        let found = lookup(&by_window, &table(native), native).unwrap().unwrap();
        assert_eq!(found.selector["query"]["via"], "window");
        assert_eq!(found.selector["query"]["start"], original.as_second() - 1);
        assert_eq!(found.selector["query"]["end"], original.as_second() + 1);

        let mut detached = event(native);
        detached["detached"] = json!(true);
        let mut by_id = Store::default();
        by_id
            .identifier
            .insert("id1".into(), vec![event("the-series"), detached]);
        let found = lookup(&by_id, &table(native), native).unwrap().unwrap();
        assert_eq!(found.candidate["detached"], true);
        let mut not_detached = Store::default();
        not_detached
            .identifier
            .insert("id1".into(), vec![event(native)]);
        assert!(
            lookup(&not_detached, &table(native), native)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn an_all_day_occurrence_tries_local_and_utc_midnight_and_compares_the_civil_day() {
        let native = "k|2026-03-08";
        let utc = midnight("2026-03-08", &TimeZone::UTC).unwrap();
        let alias = format!("ext/RID={}", utc.as_second() - REFERENCE_EPOCH);
        let mut moved = event("other");
        moved["all_day"] = json!(true);
        moved["occurrence_civil"] = json!("2026-03-08");
        let mut store = Store::default();
        store.external.insert(alias, vec![moved.clone()]);
        assert!(lookup(&store, &table(native), native).unwrap().is_some());
        moved["occurrence_civil"] = json!("2026-03-09");
        let next = Store {
            window: vec![moved],
            ..Store::default()
        };
        assert!(lookup(&next, &table(native), native).unwrap().is_none());
    }

    #[test]
    fn a_scan_finds_and_remembers_what_the_table_did_not_know() {
        let mut row = event("k");
        row["locator"] = identity();
        let store = Store {
            window: vec![row],
            ..Store::default()
        };
        let lookups = Lookups::open(None).unwrap();
        let found = fresh(&store, &lookups, "k").unwrap().unwrap();
        assert_eq!(found.selector["key"], "k");
        assert_eq!(lookups.identity("k").unwrap().unwrap(), identity());
        let nothing = Store::default();
        assert!(
            fresh(&nothing, &Lookups::open(None).unwrap(), "gone")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn known_absent_needs_positive_proof() {
        let native = "k|2026-03-08T16:00:00.000Z";
        let mut store = Store::default();
        store.accounts.insert("c".into(), "a".into());
        assert!(known_absent(&store, &table(native), native));
        assert!(
            !known_absent(&store, &Lookups::open(None).unwrap(), native),
            "never captured"
        );
        let mut other_account = Store::default();
        other_account.accounts.insert("c".into(), "z".into());
        assert!(!known_absent(&other_account, &table(native), native));
        let gone = Store {
            accounts: BTreeMap::new(),
            ..Store::default()
        };
        assert!(
            !known_absent(&gone, &table(native), native),
            "calendar itself gone"
        );
        let mut there = Store::default();
        there.accounts.insert("c".into(), "a".into());
        there.window = vec![event(native)];
        assert!(!known_absent(&there, &table(native), native));
        let mut failing = Store::default();
        failing.accounts.insert("c".into(), "a".into());
        failing.failing = true;
        assert!(!known_absent(&failing, &table(native), native));
        let mut partial = event("x");
        partial["can_materialize"] = json!(false);
        let mut unreadable = Store::default();
        unreadable.accounts.insert("c".into(), "a".into());
        unreadable.window = vec![partial];
        assert!(!known_absent(&unreadable, &table(native), native));
        let plain = Store::default();
        assert!(known_absent(&plain, &table("p"), "p"));
        let mut still = Store::default();
        still.identifier.insert("id1".into(), vec![event("p")]);
        assert!(!known_absent(&still, &table("p"), "p"));
    }

    fn now() -> Zoned {
        "2026-06-01T12:00:00+00:00[UTC]".parse().unwrap()
    }

    #[test]
    fn revalidation_only_says_missing_when_a_readable_date_is_in_range() {
        let store = Store::default();
        let lookups = Lookups::open(None).unwrap();
        let verdict = |data: Value| revalidate(&store, &lookups, "k", &data, &now()).unwrap();
        assert_eq!(verdict(json!({"start": "2026-03-08T16:00:00Z"})), MISSING);
        assert_eq!(
            verdict(json!({"start": "2001-01-01T00:00:00Z"})),
            OUTSIDE_SCOPE
        );
        assert_eq!(
            verdict(json!({"start": "2099-01-01T00:00:00Z"})),
            OUTSIDE_SCOPE
        );
        assert_eq!(verdict(json!({"start": "not a date"})), PRESENT);
        assert_eq!(verdict(json!({})), PRESENT);
        assert_eq!(
            verdict(json!({"start": "2026-03-08", "all_day": true})),
            MISSING
        );
        assert_eq!(
            verdict(json!({"start": "2026-99-08", "all_day": true})),
            PRESENT
        );
        assert_eq!(
            verdict(
                json!({"occurrence_date": "2001-01-01T00:00:00Z", "start": "2026-03-08T16:00:00Z"})
            ),
            OUTSIDE_SCOPE
        );
    }

    #[test]
    fn a_found_event_is_present_or_moved_out_of_range() {
        let mut store = Store::default();
        store.identifier.insert("id1".into(), vec![event("k")]);
        assert_eq!(
            revalidate(&store, &table("k"), "k", &json!({}), &now()).unwrap(),
            PRESENT
        );
        let mut old = event("k");
        old["start"] = json!("2001-01-01T00:00:00Z");
        old["end"] = json!("2001-01-01T01:00:00Z");
        let mut moved = Store::default();
        moved.identifier.insert("id1".into(), vec![old]);
        assert_eq!(
            revalidate(&moved, &table("k"), "k", &json!({}), &now()).unwrap(),
            OUTSIDE_SCOPE
        );
        let failing = Store {
            failing: true,
            ..Store::default()
        };
        assert!(revalidate(&failing, &table("k"), "k", &json!({}), &now()).is_err());
    }

    #[test]
    fn the_window_and_its_chunks_cover_the_same_range() {
        let (first, last) = window(&now()).unwrap();
        let pieces = chunks(&now());
        assert_eq!(
            pieces[0].1.as_second() - pieces[0].0.as_second(),
            60 * 86_400
        );
        assert_eq!(pieces.iter().map(|piece| piece.0).min().unwrap(), first);
        assert_eq!(pieces.iter().map(|piece| piece.1).max().unwrap(), last);
        let total: i64 = pieces
            .iter()
            .map(|piece| piece.1.as_second() - piece.0.as_second())
            .sum();
        assert_eq!(total, last.as_second() - first.as_second());
    }

    #[test]
    fn the_table_survives_a_restart_in_the_same_file() {
        let path = std::env::temp_dir().join(format!("ab-lookups-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        let lookups = Lookups::open(Some(path.clone())).unwrap();
        lookups.remember("k", &identity()).unwrap();
        lookups.save().unwrap();
        let again = Lookups::open(Some(path.clone())).unwrap();
        assert_eq!(again.identity("k").unwrap().unwrap(), identity());
        fs::remove_file(&path).unwrap();
    }
}
