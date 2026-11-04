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
    #[must_use]
    pub fn reminders() -> Self {
        Self::new("reminders", &["reminders.list", "reminders.reminder"])
    }

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

    #[must_use]
    pub fn knows(&self, identity: &str) -> bool {
        self.records.contains_key(identity)
    }

    #[must_use]
    pub fn generation(&self) -> &str {
        &self.generation
    }

    #[must_use]
    pub fn same_shape(&self, other: &Self) -> bool {
        self.source == other.source && self.scope == other.scope
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
                    || (self.aliased() && previous["scoped_alias"] != row["scoped_alias"])
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
                .map(|row| encode_observation(row, self.source == "reminders"))
                .collect::<Result<_>>()?,
            previous: self.clone(),
            next,
        })
    }

    fn alias_covered<'a>(
        &self,
        previous: &Value,
        mut current: impl Iterator<Item = &'a Value>,
    ) -> bool {
        let alias = previous["scoped_alias"].as_str().unwrap_or_default();
        !alias.is_empty()
            && match self.source.as_str() {
                "calendar" => current.any(|row| row["scoped_alias"] == alias),
                "reminders" => current.any(|row| {
                    row["collection"] == previous["collection"] && row["scoped_alias"] == alias
                }),
                _ => false,
            }
    }

    #[must_use]
    pub fn unaccounted(&self, rows: &[Value]) -> Vec<Value> {
        let present: BTreeSet<String> = rows
            .iter()
            .filter_map(|row| {
                Some(format!(
                    "{}/{}",
                    row["collection"].as_str()?,
                    row["native_key"].as_str()?
                ))
            })
            .collect();
        self.records
            .iter()
            .filter(|(identity, previous)| {
                !present.contains(*identity)
                    && !(self.source == "contacts" && previous["collection"] == "contacts.container")
                    && !self.alias_covered(previous, rows.iter())
            })
            .map(|(_, previous)| previous.clone())
            .collect()
    }

    fn aliased(&self) -> bool {
        matches!(self.source.as_str(), "calendar" | "reminders")
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
            if self.alias_covered(previous, current.values()) {
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

fn encode_observation(row: &Value, omit_empty_alias: bool) -> Result<Observation> {
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
        scoped_alias: row["scoped_alias"]
            .as_str()
            .filter(|alias| !omit_empty_alias || !alias.is_empty())
            .map(str::to_owned),
        observed_at: row["observed_at"].as_str().ok_or(Error::Corrupt)?.into(),
        data: Some(serde_json::from_slice(&bytes)?),
        revalidated: (kind != "upsert").then_some(true),
    })
}
