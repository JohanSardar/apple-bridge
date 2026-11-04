use crate::Error;
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub collection: String,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub checkpoint: String,
    pub sequence: u64,
    pub rust: bool,
    pub facts: bool,
}

#[derive(Debug)]
pub struct Flight<T> {
    pub generation: String,
    pub checkpoint: String,
    pub bytes: u64,
    pub keys: Vec<Row>,
    pub origin: T,
}

impl<T> Flight<T> {
    pub fn new(
        generation: String,
        checkpoint: String,
        bytes: u64,
        keys: Vec<Row>,
        origin: T,
    ) -> Self {
        Self {
            generation,
            checkpoint,
            bytes,
            keys,
            origin,
        }
    }

    #[must_use]
    pub fn matches(&self, checkpoint: &str, credit: Option<u64>) -> bool {
        self.checkpoint == checkpoint && credit == Some(self.bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Busy,
    TooLarge,
    Credit,
}

#[derive(Debug)]
pub struct Flights<T> {
    credit: u64,
    total: u64,
    sequence: u64,
    committed: BTreeMap<String, (String, String)>,
    inflight: BTreeMap<String, Flight<T>>,
    written: BTreeMap<(String, String), Seen>,
    by_key: BTreeMap<String, Seen>,
}

impl<T> Default for Flights<T> {
    fn default() -> Self {
        Self {
            credit: 0,
            total: 0,
            sequence: 0,
            committed: BTreeMap::new(),
            inflight: BTreeMap::new(),
            written: BTreeMap::new(),
            by_key: BTreeMap::new(),
        }
    }
}

impl<T> Flights<T> {
    pub fn open(&mut self, credit: u64, committed: BTreeMap<String, (String, String)>) {
        self.credit = credit;
        self.total = credit;
        self.committed.extend(committed);
    }

    #[must_use]
    pub fn credit(&self) -> u64 {
        self.credit
    }

    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn contains(&self, source: &str) -> bool {
        self.inflight.contains_key(source)
    }

    #[must_use]
    pub fn get(&self, source: &str) -> Option<&Flight<T>> {
        self.inflight.get(source)
    }

    #[must_use]
    pub fn committed(&self, source: &str) -> Option<&(String, String)> {
        self.committed.get(source)
    }

    #[must_use]
    pub fn predecessor(&self, source: &str, generation: &str) -> String {
        self.committed
            .get(source)
            .filter(|(committed, _)| committed == generation)
            .map(|(_, checkpoint)| checkpoint.clone())
            .unwrap_or_default()
    }

    pub fn track(&mut self, source: &str, flight: Flight<T>) -> Result<(), Error> {
        self.credit = self.credit.saturating_sub(flight.bytes);
        if self.inflight.insert(source.to_owned(), flight).is_some() {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    pub fn reserve(&mut self, source: &str, flight: Flight<T>) -> Result<(), Refused> {
        if flight.bytes > self.total {
            return Err(Refused::TooLarge);
        }
        if self.inflight.contains_key(source) {
            return Err(Refused::Busy);
        }
        if self.credit < flight.bytes {
            return Err(Refused::Credit);
        }
        self.credit -= flight.bytes;
        self.inflight.insert(source.to_owned(), flight);
        Ok(())
    }

    pub fn complete(&mut self, source: &str, rust: bool, facts: bool) -> Option<Flight<T>> {
        let flight = self.inflight.remove(source)?;
        self.sequence += 1;
        self.credit = self.credit.saturating_add(flight.bytes);
        self.committed.insert(
            source.to_owned(),
            (flight.generation.clone(), flight.checkpoint.clone()),
        );
        for row in &flight.keys {
            let seen = Seen {
                checkpoint: flight.checkpoint.clone(),
                sequence: self.sequence,
                rust,
                facts,
            };
            self.written
                .insert((row.collection.clone(), row.key.clone()), seen.clone());
            self.by_key.insert(row.key.clone(), seen);
        }
        Some(flight)
    }

    #[must_use]
    pub fn written(&self, collections: &[&str], key: &str, after: Option<u64>) -> Option<String> {
        collections.iter().find_map(|collection| {
            let seen = self
                .written
                .get(&((*collection).to_owned(), key.to_owned()))?;
            after
                .is_none_or(|after| seen.sequence > after)
                .then(|| seen.checkpoint.clone())
        })
    }

    #[must_use]
    pub fn latest(&self, key: &str) -> Option<&Seen> {
        self.by_key.get(key)
    }
}

pub trait Ledger: Sync {
    fn sequence(&self) -> Result<u64, Error>;
    fn written(
        &self,
        collections: &[&str],
        key: &str,
        after: Option<u64>,
        wait: Duration,
    ) -> Result<Option<String>, Error>;
}

#[must_use]
pub fn rows(observations: &[Value]) -> Vec<Row> {
    observations
        .iter()
        .filter(|row| row["kind"] == "upsert" || row["kind"] == "absent")
        .filter_map(|row| {
            Some(Row {
                collection: row["collection"].as_str()?.to_owned(),
                key: row["native_key"].as_str()?.to_owned(),
            })
        })
        .collect()
}

#[must_use]
pub fn typed_rows(observations: &[ab_wire::messages::Observation]) -> Vec<Row> {
    observations
        .iter()
        .filter(|row| row.kind == "upsert" || row.kind == "absent")
        .map(|row| Row {
            collection: row.collection.clone(),
            key: row.native_key.clone(),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn flight(checkpoint: &str, bytes: u64, keys: &[(&str, &str)]) -> Flight<()> {
        Flight::new(
            "g".into(),
            checkpoint.into(),
            bytes,
            keys.iter()
                .map(|(collection, key)| Row {
                    collection: (*collection).into(),
                    key: (*key).into(),
                })
                .collect(),
            (),
        )
    }

    #[test]
    fn credit_is_spent_and_returned_by_whole_frames() {
        let mut flights = Flights::default();
        flights.open(100, BTreeMap::new());
        flights.reserve("a", flight("c1", 60, &[])).expect("first");
        assert_eq!(flights.credit(), 40);
        assert_eq!(
            flights.reserve("b", flight("c2", 60, &[])),
            Err(Refused::Credit)
        );
        assert_eq!(
            flights.reserve("a", flight("c3", 10, &[])),
            Err(Refused::Busy)
        );
        assert_eq!(
            flights.reserve("b", flight("c4", 101, &[])),
            Err(Refused::TooLarge)
        );
        assert!(flights.get("a").expect("flight").matches("c1", Some(60)));
        assert!(!flights.get("a").expect("flight").matches("c1", Some(59)));
        assert!(!flights.get("a").expect("flight").matches("cx", Some(60)));
        flights.complete("a", true, false).expect("complete");
        assert_eq!(flights.credit(), 100);
        assert_eq!(flights.predecessor("a", "g"), "c1");
        assert_eq!(flights.predecessor("a", "other"), "");
    }

    #[test]
    fn writes_are_correlated_only_when_they_follow_the_mark() {
        let mut flights = Flights::default();
        flights.open(1000, BTreeMap::new());
        flights
            .reserve("calendar", flight("c1", 10, &[("calendar.event", "k")]))
            .expect("reserve");
        flights.complete("calendar", true, true);
        let mark = flights.sequence();
        assert_eq!(
            flights.written(&["calendar.event"], "k", None).as_deref(),
            Some("c1")
        );
        assert_eq!(flights.written(&["calendar.event"], "k", Some(mark)), None);
        flights
            .track("calendar", flight("c2", 10, &[("calendar.event", "k")]))
            .expect("track");
        flights.complete("calendar", false, true);
        assert_eq!(
            flights
                .written(&["calendar.calendar", "calendar.event"], "k", Some(mark))
                .as_deref(),
            Some("c2")
        );
        assert!(!flights.latest("k").expect("latest").rust);
    }

    #[test]
    fn tracking_a_second_flight_for_one_source_conflicts() {
        let mut flights = Flights::default();
        flights.open(1000, BTreeMap::new());
        flights.track("a", flight("c1", 10, &[])).expect("first");
        assert!(matches!(
            flights.track("a", flight("c2", 10, &[])),
            Err(Error::Conflict)
        ));
    }

    #[test]
    fn rows_keep_only_observations_of_records() {
        let rows = rows(&[
            json!({"kind":"upsert","collection":"c","native_key":"a"}),
            json!({"kind":"evict","collection":"c","native_key":"b"}),
            json!({"kind":"absent","collection":"c","native_key":"d"}),
            json!({"kind":"upsert","collection":"c"}),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].key, "d");
    }
}
