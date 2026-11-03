use crate::capture::{Context, Error, Observation, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Revalidation {
    Absent,
    Evict,
    Present,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub generation: String,
    pub scope: Vec<String>,
    pub complete: bool,
    pub records: Vec<Value>,
    #[serde(default)]
    pub revalidated: BTreeMap<String, Revalidation>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reader {
    source: String,
    scope: Vec<String>,
    generation: String,
    checkpoint: String,
    revision: u64,
    records: BTreeMap<String, Value>,
}

#[derive(Debug, serde::Serialize)]
pub struct Pending {
    pub source: String,
    pub generation: String,
    pub checkpoint: String,
    pub scan_id: String,
    pub scan_started_at: String,
    pub scope: Vec<String>,
    pub completion_evidence: &'static str,
    pub observations: Vec<Observation>,
    #[serde(skip)]
    previous: Reader,
    #[serde(skip)]
    next: Reader,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new("calendar", &["calendar.calendar", "calendar.event"])
    }
}

impl Reader {
    pub(crate) fn new(source: &str, scope: &[&str]) -> Self {
        Self {
            source: source.into(),
            scope: scope.iter().map(|value| (*value).into()).collect(),
            generation: String::new(),
            checkpoint: String::new(),
            revision: 0,
            records: BTreeMap::new(),
        }
    }

    pub fn prepare(&self, snapshot: Snapshot, context: &Context) -> Result<Pending> {
        self.validate(&snapshot, context)?;
        let same_generation = self.generation == snapshot.generation;
        let mut current = BTreeMap::new();
        let mut changes = Vec::new();
        for mut row in snapshot.records {
            let collection = row["collection"].as_str().ok_or(Error::Corrupt)?;
            let key = row["native_key"].as_str().ok_or(Error::Corrupt)?;
            if !self.scope.iter().any(|scope| scope == collection)
                || key.is_empty()
                || row["kind"] != "upsert"
                || row.get("data").is_none()
                || !row["data"].is_object()
                || !row["scoped_alias"].is_string()
            {
                return Err(Error::Corrupt);
            }
            let identity = format!("{collection}/{key}");
            row["observed_at"] = Value::String(context.observed_at.clone());
            let differs = self.records.get(&identity).is_none_or(|previous| {
                !crate::capture::data_equal(&previous["data"].to_string(), &row["data"].to_string())
                    || (self.source == "calendar"
                        && previous["scoped_alias"] != row["scoped_alias"])
            });
            if snapshot.force || !same_generation || differs {
                changes.push(row.clone());
            }
            if current.insert(identity, row).is_some() {
                return Err(Error::Corrupt);
            }
        }
        self.reconcile_missing(
            &mut current,
            &mut changes,
            &snapshot.revalidated,
            context,
            same_generation,
        )?;
        changes.sort_by(|left, right| {
            (&left["collection"].as_str(), &left["native_key"].as_str())
                .cmp(&(&right["collection"].as_str(), &right["native_key"].as_str()))
        });
        let next = Self {
            source: self.source.clone(),
            scope: self.scope.clone(),
            generation: snapshot.generation.clone(),
            checkpoint: context.checkpoint.clone(),
            revision: self.revision.checked_add(1).ok_or(Error::Conflict)?,
            records: current,
        };
        Ok(Pending {
            source: self.source.clone(),
            generation: snapshot.generation,
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
            scan_started_at: context.observed_at.clone(),
            scope: self.scope.clone(),
            completion_evidence: "complete",
            observations: changes
                .iter()
                .map(encode_observation)
                .collect::<Result<_>>()?,
            previous: self.clone(),
            next,
        })
    }

    fn validate(&self, snapshot: &Snapshot, context: &Context) -> Result<()> {
        if !snapshot.complete {
            return Err(Error::Incomplete);
        }
        let expected: BTreeSet<_> = self.scope.iter().collect();
        let actual: BTreeSet<_> = snapshot.scope.iter().collect();
        if snapshot.generation.is_empty()
            || context.checkpoint.is_empty()
            || context.checkpoint == self.checkpoint
            || expected != actual
            || actual.len() != snapshot.scope.len()
            || snapshot.records.len() > 100_000
        {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    fn reconcile_missing(
        &self,
        current: &mut BTreeMap<String, Value>,
        changes: &mut Vec<Value>,
        revalidated: &BTreeMap<String, Revalidation>,
        context: &Context,
        same_generation: bool,
    ) -> Result<()> {
        if revalidated
            .keys()
            .any(|key| !self.records.contains_key(key) || current.contains_key(key))
        {
            return Err(Error::Corrupt);
        }
        for (identity, previous) in &self.records {
            if current.contains_key(identity) {
                continue;
            }
            let alias = previous["scoped_alias"].as_str().unwrap_or_default();
            if self.source == "calendar"
                && !alias.is_empty()
                && current.values().any(|row| row["scoped_alias"] == alias)
            {
                continue;
            }
            let disposition = revalidated.get(identity);
            if disposition == Some(&Revalidation::Evict)
                && previous["collection"] != "calendar.event"
            {
                return Err(Error::Corrupt);
            }
            let container_absence =
                self.source == "contacts" && previous["collection"] == "contacts.container";
            if !same_generation
                || disposition == Some(&Revalidation::Present)
                || (disposition.is_none() && !container_absence)
            {
                current.insert(identity.clone(), previous.clone());
                continue;
            }
            if self.source == "contacts" && disposition == Some(&Revalidation::Evict) {
                return Err(Error::Corrupt);
            }
            let mut absent = previous.clone();
            absent["kind"] = Value::String(
                if disposition == Some(&Revalidation::Evict) {
                    "evict"
                } else {
                    "absent"
                }
                .into(),
            );
            absent["observed_at"] = Value::String(context.observed_at.clone());
            absent["revalidated"] = Value::Bool(true);
            changes.push(absent);
        }
        Ok(())
    }

    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        if pending.previous != *self
            || pending.next.source != self.source
            || pending.next.scope != self.scope
            || pending.next.checkpoint == self.checkpoint
        {
            return Err(Error::Conflict);
        }
        *self = pending.next;
        Ok(())
    }
}

fn encode_observation(row: &Value) -> Result<Observation> {
    let kind = match row["kind"].as_str() {
        Some("upsert") => "upsert",
        Some("absent") => "absent",
        Some("evict") => "evict",
        _ => return Err(Error::Corrupt),
    };
    let bytes = crate::native_json::record_data(&row["data"])?;
    Ok(Observation {
        kind,
        collection: row["collection"].as_str().ok_or(Error::Corrupt)?.into(),
        native_key: row["native_key"].as_str().ok_or(Error::Corrupt)?.into(),
        scoped_alias: row["scoped_alias"].as_str().map(str::to_owned),
        observed_at: row["observed_at"].as_str().ok_or(Error::Corrupt)?.into(),
        data: Some(serde_json::from_slice(&bytes)?),
        revalidated: (kind != "upsert").then_some(true),
    })
}
