use crate::capture::Result;
use crate::reminders_projection::{
    self as projection, ColumnSets, ManualOrder, Sections, base_data, base_observation, parse_uuid,
};
use crate::reminders_store::{ListRow, ReadResult, ReminderRow, Session, Snapshot};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

type Paths = Box<dyn Fn() -> Result<Vec<PathBuf>> + Send>;

pub fn default_paths(home: &Path) -> Result<Vec<PathBuf>> {
    paths_in(&home.join("Library/Group Containers/group.com.apple.reminders/Container_v1/Stores"))
}

pub fn paths_in(stores: &Path) -> Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(stores)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("Data-"))
                && path
                    .extension()
                    .is_some_and(|extension| extension == "sqlite")
                && path.is_file()
        })
        .collect();
    found.sort();
    Ok(found)
}

pub struct Enrichment {
    sessions: BTreeMap<PathBuf, Session>,
    manual_orders: BTreeMap<PathBuf, BTreeMap<i64, ManualOrder>>,
    section_states: BTreeMap<PathBuf, BTreeMap<i64, Sections>>,
    snapshots: BTreeMap<PathBuf, Snapshot>,
    fields: BTreeMap<PathBuf, BTreeMap<String, Value>>,
    base: BTreeMap<String, Value>,
    generation: String,
    paths: Paths,
}

impl std::fmt::Debug for Enrichment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Enrichment").finish_non_exhaustive()
    }
}

fn key(observation: &Value) -> String {
    format!(
        "{}/{}",
        observation["collection"].as_str().unwrap_or_default(),
        observation["native_key"].as_str().unwrap_or_default()
    )
}

fn identifier_of(row: &ReminderRow) -> Option<String> {
    row.identity
        .uuid
        .clone()
        .or_else(|| row.identity.text.as_deref().and_then(parse_uuid))
}

fn list_identifier(row: Option<&ListRow>) -> Option<String> {
    row.and_then(|row| row.identity.identifier())
}

fn names(prefix: &str, identifier: Option<String>) -> Option<String> {
    identifier.map(|identifier| format!("{prefix}{identifier}"))
}

impl Enrichment {
    #[must_use]
    pub fn new(paths: Paths) -> Self {
        Self {
            sessions: BTreeMap::new(),
            manual_orders: BTreeMap::new(),
            section_states: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            fields: BTreeMap::new(),
            base: BTreeMap::new(),
            generation: String::new(),
            paths,
        }
    }

    fn forget(&mut self, path: &Path, touched: &mut BTreeSet<String>) {
        if let Some(fields) = self.fields.remove(path) {
            touched.extend(fields.into_keys());
        }
        self.snapshots.remove(path);
        self.section_states.remove(path);
        self.manual_orders.remove(path);
        self.sessions.remove(path);
    }

    pub fn refresh(&mut self, observations: &[Value], generation: &str) -> Vec<Value> {
        if self.generation != generation {
            self.base.clear();
            generation.clone_into(&mut self.generation);
        }
        let mut touched: BTreeSet<String> = BTreeSet::new();
        if let Ok(paths) = (self.paths)() {
            let current: BTreeSet<PathBuf> = paths.iter().cloned().collect();
            let gone: Vec<PathBuf> = self
                .sessions
                .keys()
                .filter(|path| !current.contains(*path))
                .cloned()
                .collect();
            for path in gone {
                self.forget(&path, &mut touched);
            }
            for path in current {
                if self.refresh_path(&path, &mut touched).is_err() {
                    self.forget(&path, &mut touched);
                }
            }
        } else {
            touched.extend(self.base.keys().cloned());
            self.fields.clear();
            self.snapshots.clear();
            self.manual_orders.clear();
            self.section_states.clear();
            self.sessions.clear();
        }
        let mut outgoing: BTreeMap<String, Value> = BTreeMap::new();
        for observation in observations {
            let identity = key(observation);
            outgoing.insert(identity.clone(), observation.clone());
            match observation["kind"].as_str() {
                Some("upsert") => {
                    self.base.insert(identity, base_observation(observation));
                }
                Some("absent") => {
                    self.base.remove(&identity);
                }
                _ => {}
            }
        }
        for identity in &touched {
            if outgoing.contains_key(identity) {
                continue;
            }
            if let Some(base) = self.base.get(identity) {
                let mut observation = base.clone();
                observation["observed_at"] = Value::String(now());
                outgoing.insert(identity.clone(), observation);
            }
        }
        let ordered: Vec<Value> = outgoing.into_values().collect();
        self.apply(&ordered)
    }

    #[must_use]
    pub fn apply(&self, observations: &[Value]) -> Vec<Value> {
        observations
            .iter()
            .map(|observation| {
                let collection = observation["collection"].as_str().unwrap_or_default();
                if observation["kind"] != "upsert"
                    || !["reminders.list", "reminders.reminder"].contains(&collection)
                {
                    return observation.clone();
                }
                let identity = key(observation);
                let matches: Vec<&Value> = self
                    .fields
                    .values()
                    .filter_map(|fields| fields.get(&identity))
                    .collect();
                let enrichment = if matches.len() == 1 {
                    matches[0].clone()
                } else {
                    projection::unavailable(collection)
                };
                let mut data = base_data(&observation["data"])
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                if let Some(extra) = enrichment.as_object() {
                    for (name, value) in extra {
                        data.insert(name.clone(), value.clone());
                    }
                }
                let mut result = observation.clone();
                result["data"] = Value::Object(data);
                result
            })
            .collect()
    }

    fn update(
        &mut self,
        path: &Path,
        old: Option<String>,
        native: Option<String>,
        value: Option<Value>,
        touched: &mut BTreeSet<String>,
    ) {
        if let Some(old) = old
            && Some(&old) != native.as_ref()
        {
            if let Some(fields) = self.fields.get_mut(path) {
                fields.remove(&old);
            }
            touched.insert(old);
        }
        if let (Some(native), Some(value)) = (native, value) {
            let fields = self.fields.entry(path.to_path_buf()).or_default();
            if fields.get(&native) != Some(&value) {
                fields.insert(native.clone(), value);
                touched.insert(native);
            }
        }
    }

    fn read(&mut self, path: &Path) -> Result<ReadResult> {
        if !self.sessions.contains_key(path) {
            self.sessions
                .insert(path.to_path_buf(), Session::new(path)?);
        }
        self.sessions
            .get_mut(path)
            .ok_or(crate::capture::Error::Incomplete)?
            .read(false)
    }

    #[allow(clippy::too_many_lines)]
    fn refresh_path(&mut self, path: &Path, touched: &mut BTreeSet<String>) -> Result<()> {
        let result = self.read(path)?;
        let previous = self.snapshots.get(path).cloned().unwrap_or_default();
        let snapshot = result.snapshot.clone();
        let mut changed_reminders = result.reminder_keys.clone();
        let changed_order_lists: Vec<i64> = result
            .list_keys
            .iter()
            .copied()
            .filter(|key| {
                !snapshot.lists.contains_key(key)
                    || previous
                        .lists
                        .get(key)
                        .and_then(|list| list.ordering_json.clone())
                        != snapshot
                            .lists
                            .get(key)
                            .and_then(|list| list.ordering_json.clone())
                    || self
                        .manual_orders
                        .get(path)
                        .is_none_or(|orders| !orders.contains_key(key))
            })
            .collect();
        let mut changed_positions: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
        let mut changed_order_coverage: BTreeSet<i64> = BTreeSet::new();
        for key in changed_order_lists {
            let before = self
                .manual_orders
                .get(path)
                .and_then(|orders| orders.get(&key))
                .cloned()
                .unwrap_or(ManualOrder::Unsupported);
            let after = ManualOrder::decode(
                snapshot
                    .lists
                    .get(&key)
                    .and_then(|list| list.ordering_json.as_deref()),
            );
            match after.changed_identifiers(&before) {
                Some(changed) => {
                    if !changed.is_empty() {
                        changed_positions.insert(key, changed);
                    }
                }
                None => {
                    changed_order_coverage.insert(key);
                }
            }
            let orders = self.manual_orders.entry(path.to_path_buf()).or_default();
            if snapshot.lists.contains_key(&key) {
                orders.insert(key, after);
            } else {
                orders.remove(&key);
            }
        }
        if !changed_positions.is_empty() || !changed_order_coverage.is_empty() {
            for row in snapshot.reminders.values() {
                let Some(list) = row.list else {
                    continue;
                };
                if changed_order_coverage.contains(&list) {
                    changed_reminders.insert(row.identity.key);
                } else if let (Some(changed), Some(identifier)) =
                    (changed_positions.get(&list), identifier_of(row))
                    && changed.contains(&identifier)
                {
                    changed_reminders.insert(row.identity.key);
                }
            }
        }
        let mut section_lists: BTreeSet<i64> = result
            .list_keys
            .iter()
            .copied()
            .filter(|key| {
                let before = previous.lists.get(key);
                let after = snapshot.lists.get(key);
                before.and_then(|list| list.section_order.clone())
                    != after.and_then(|list| list.section_order.clone())
                    || before.and_then(|list| list.section_membership.clone())
                        != after.and_then(|list| list.section_membership.clone())
            })
            .collect();
        for key in &result.reminder_keys {
            let before = previous.reminders.get(key);
            let after = snapshot.reminders.get(key);
            if before.and_then(|row| row.list) != after.and_then(|row| row.list)
                || projection::reminder_identifier(before) != projection::reminder_identifier(after)
            {
                section_lists.extend(before.and_then(|row| row.list));
                section_lists.extend(after.and_then(|row| row.list));
            }
        }
        for key in &result.section_keys {
            section_lists.extend(previous.sections.get(key).and_then(|row| row.list));
            section_lists.extend(snapshot.sections.get(key).and_then(|row| row.list));
        }
        let states: Vec<i64> = result.list_keys.union(&section_lists).copied().collect();
        for key in states {
            let validated = snapshot
                .lists
                .get(&key)
                .and_then(|list| Sections::validated(list, &snapshot));
            let map = self.section_states.entry(path.to_path_buf()).or_default();
            match validated {
                Some(value) => {
                    map.insert(key, value);
                }
                None => {
                    map.remove(&key);
                }
            }
        }
        changed_reminders.extend(
            snapshot
                .reminders
                .values()
                .filter(|row| row.list.is_some_and(|list| section_lists.contains(&list)))
                .map(|row| row.identity.key),
        );
        let changed_parents: BTreeSet<i64> = result
            .reminder_keys
            .iter()
            .copied()
            .filter(|key| {
                projection::reminder_identifier(previous.reminders.get(key))
                    != projection::reminder_identifier(snapshot.reminders.get(key))
            })
            .collect();
        if !changed_parents.is_empty() {
            changed_reminders.extend(
                snapshot
                    .reminders
                    .values()
                    .filter(|row| {
                        row.parent
                            .is_some_and(|parent| changed_parents.contains(&parent))
                    })
                    .map(|row| row.identity.key),
            );
        }
        if !result.object_keys.is_empty() || !result.label_keys.is_empty() {
            let mut keys = result.object_keys.clone();
            keys.extend(
                snapshot
                    .objects
                    .values()
                    .filter(|object| {
                        object
                            .hashtag_label
                            .is_some_and(|label| result.label_keys.contains(&label))
                    })
                    .map(|object| object.identity.key),
            );
            for key in keys {
                for object in [previous.objects.get(&key), snapshot.objects.get(&key)]
                    .into_iter()
                    .flatten()
                {
                    changed_reminders.extend(object.reminders.iter().flatten().copied());
                }
            }
        }
        let reminder_columns = result
            .columns
            .get("ZREMCDREMINDER")
            .cloned()
            .unwrap_or_default();
        let list_columns = result
            .columns
            .get("ZREMCDBASELIST")
            .cloned()
            .unwrap_or_default();
        let tag_columns: ColumnSets = result.columns.clone();
        for key in changed_reminders {
            let old = names(
                "reminders.reminder/",
                projection::reminder_identifier(previous.reminders.get(&key)),
            );
            let native = names(
                "reminders.reminder/",
                projection::reminder_identifier(snapshot.reminders.get(&key)),
            );
            let value = snapshot.reminders.get(&key).map(|row| {
                let order = row
                    .list
                    .and_then(|list| {
                        self.manual_orders
                            .get(path)
                            .and_then(|orders| orders.get(&list))
                    })
                    .cloned()
                    .unwrap_or(ManualOrder::Unsupported);
                let sections = row.list.and_then(|list| {
                    self.section_states
                        .get(path)
                        .and_then(|states| states.get(&list))
                });
                projection::reminder(
                    row,
                    &snapshot,
                    &reminder_columns,
                    &order,
                    sections,
                    &tag_columns,
                )
            });
            self.update(path, old, native, value, touched);
        }
        let mut changed_lists: BTreeSet<i64> =
            result.list_keys.union(&section_lists).copied().collect();
        let changed_groups: BTreeSet<i64> = result
            .list_keys
            .iter()
            .copied()
            .filter(|key| {
                list_identifier(previous.lists.get(key)) != list_identifier(snapshot.lists.get(key))
            })
            .collect();
        if !changed_groups.is_empty() {
            changed_lists.extend(
                snapshot
                    .lists
                    .values()
                    .filter(|list| {
                        list.parent
                            .is_some_and(|parent| changed_groups.contains(&parent))
                            || list
                                .alternate_parent
                                .is_some_and(|parent| changed_groups.contains(&parent))
                    })
                    .map(|list| list.identity.key),
            );
        }
        for key in changed_lists {
            let old = names("reminders.list/", list_identifier(previous.lists.get(&key)));
            let native = names("reminders.list/", list_identifier(snapshot.lists.get(&key)));
            let value = snapshot
                .lists
                .get(&key)
                .map(|row| projection::list(row, &snapshot, &list_columns));
            self.update(path, old, native, value, touched);
        }
        self.snapshots.insert(path.to_path_buf(), snapshot);
        Ok(())
    }
}

fn now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    crate::capture::timestamp(seconds).unwrap_or_default()
}
