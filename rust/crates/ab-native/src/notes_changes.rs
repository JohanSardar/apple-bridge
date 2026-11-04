use crate::{
    capture::{Context, Error, Result},
    native_json,
    notes_store::{self, Database, Facts, Row, StoreSnapshot, integer, number, text},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Debug, Clone)]
pub(crate) struct Checkpoint {
    schema: String,
    objects: BTreeMap<i64, Row>,
    bodies: BTreeMap<i64, Row>,
}
#[derive(Debug)]
pub(crate) struct ChangeScan {
    pub snapshot: StoreSnapshot,
    pub removed: BTreeSet<String>,
    pub checkpoint: Checkpoint,
    pub changed_notes: usize,
    pub bootstrap: bool,
}
fn owner(
    id: i64,
    historical: bool,
    all: &BTreeMap<i64, Row>,
    prior: &BTreeMap<i64, Row>,
    note_entity: i64,
    seen: &BTreeSet<i64>,
) -> Option<i64> {
    let rows = if historical { prior } else { all };
    if seen.len() >= 32 || seen.contains(&id) {
        return None;
    }
    let row = rows.get(&id)?;
    if integer(row, "Z_ENT") == note_entity {
        return Some(id);
    }
    let mut seen = seen.clone();
    seen.insert(id);
    for field in ["ZNOTE", "ZNOTE1", "ZPARENTATTACHMENT"] {
        let parent = integer(row, field);
        if parent > 0
            && let Some(note) = owner(parent, historical, all, prior, note_entity, &seen)
        {
            return Some(note);
        }
    }
    None
}
#[derive(Debug, Clone, Copy)]
struct Entities {
    note: i64,
    folder: i64,
    account: i64,
}
fn load(db: &Database) -> Result<(Checkpoint, Entities)> {
    let columns = db.columns("ZICCLOUDSYNCINGOBJECT")?;
    if !columns.contains("Z_OPT") || !db.columns("ZICNOTEDATA")?.contains("Z_OPT") {
        return Err(Error::Unsupported);
    }
    let entities = db.query("SELECT Z_ENT,Z_NAME FROM Z_PRIMARYKEY", &[])?;
    let entity = |name: &str| {
        entities
            .iter()
            .find(|r| text(r, "Z_NAME") == name)
            .map(|r| integer(r, "Z_ENT"))
            .ok_or(Error::Unsupported)
    };
    let entities = Entities {
        note: entity("ICNote")?,
        folder: entity("ICFolder")?,
        account: entity("ICAccount")?,
    };
    let uuid = db
        .query("SELECT Z_UUID FROM Z_METADATA", &[])?
        .first()
        .map(|r| text(r, "Z_UUID").to_owned())
        .ok_or(Error::Unsupported)?;
    let version = db.query("PRAGMA schema_version", &[])?;
    let schema = format!(
        "{uuid}:{}",
        native_json::description(version.first().map_or(0.0, |r| number(r, "schema_version")))?
    );
    let fields = [
        "Z_PK",
        "Z_ENT",
        "Z_OPT",
        "ZIDENTIFIER",
        "ZNOTEDATA",
        "ZFOLDER",
        "ZFOLDERTYPE",
        "ZPARENT",
        "ZACCOUNT8",
        "ZNOTE",
        "ZNOTE1",
        "ZPARENTATTACHMENT",
        "ZMEDIA",
        "ZMARKEDFORDELETION",
    ]
    .iter()
    .filter(|key| columns.contains(**key))
    .copied()
    .collect::<Vec<_>>()
    .join(",");
    let objects = db
        .query(&format!("SELECT {fields} FROM ZICCLOUDSYNCINGOBJECT"), &[])?
        .into_iter()
        .map(|row| (integer(&row, "Z_PK"), row))
        .collect();
    let bodies = db
        .query("SELECT Z_PK,Z_OPT FROM ZICNOTEDATA", &[])?
        .into_iter()
        .map(|row| (integer(&row, "Z_PK"), row))
        .collect();
    Ok((
        Checkpoint {
            schema,
            objects,
            bodies,
        },
        entities,
    ))
}
#[derive(Debug)]
struct Difference<'a> {
    current: &'a Checkpoint,
    previous: &'a Checkpoint,
    entities: Entities,
    all: BTreeMap<i64, Row>,
    objects: BTreeSet<i64>,
    bodies: BTreeSet<i64>,
}
impl<'a> Difference<'a> {
    fn new(current: &'a Checkpoint, previous: &'a Checkpoint, entities: Entities) -> Self {
        let objects = current
            .objects
            .keys()
            .chain(previous.objects.keys())
            .copied()
            .filter(|id| current.objects.get(id) != previous.objects.get(id))
            .collect();
        let bodies = current
            .bodies
            .keys()
            .chain(previous.bodies.keys())
            .copied()
            .filter(|id| current.bodies.get(id) != previous.bodies.get(id))
            .collect();
        let mut all = previous.objects.clone();
        all.extend(current.objects.clone());
        Self {
            current,
            previous,
            entities,
            all,
            objects,
            bodies,
        }
    }
    fn removed_identities(&self) -> BTreeSet<String> {
        let mut removed = BTreeSet::new();
        for id in &self.objects {
            if let Some(old) = self.previous.objects.get(id)
                && old.get("ZIDENTIFIER")
                    != self
                        .current
                        .objects
                        .get(id)
                        .and_then(|r| r.get("ZIDENTIFIER"))
            {
                let entity = integer(old, "Z_ENT");
                let collection = if entity == self.entities.note {
                    "notes.note"
                } else if entity == self.entities.folder {
                    "notes.folder"
                } else {
                    ""
                };
                if !collection.is_empty()
                    && let Some(key) = old.get("ZIDENTIFIER").and_then(serde_json::Value::as_str)
                {
                    removed.insert(format!("{collection}/{key}"));
                }
            }
            if self
                .current
                .objects
                .get(id)
                .is_none_or(|row| number(row, "ZMARKEDFORDELETION") != 0.0)
                && let Some(old) = self
                    .all
                    .get(id)
                    .filter(|row| integer(row, "Z_ENT") == self.entities.folder)
                && let Some(key) = old.get("ZIDENTIFIER").and_then(serde_json::Value::as_str)
            {
                removed.insert(format!("notes.folder/{key}"));
            }
        }
        removed
    }
    fn folder_notes(&self, id: i64) -> BTreeSet<i64> {
        let mut affected = BTreeSet::from([id]);
        loop {
            let mut expanded = false;
            for (pk, row) in &self.all {
                if integer(row, "Z_ENT") == self.entities.folder
                    && (affected.contains(&integer(row, "ZPARENT"))
                        || affected.contains(&integer(row, "ZACCOUNT8")))
                {
                    expanded |= affected.insert(*pk);
                }
            }
            if !expanded {
                break;
            }
        }
        self.all
            .iter()
            .filter(|(_, row)| {
                integer(row, "Z_ENT") == self.entities.note
                    && affected.contains(&integer(row, "ZFOLDER"))
            })
            .map(|(pk, _)| *pk)
            .collect()
    }
    fn changed_notes(&self) -> BTreeSet<i64> {
        let mut changed = BTreeSet::new();
        let mut media_owners = BTreeMap::<i64, BTreeSet<i64>>::new();
        for rows in [&self.all, &self.previous.objects] {
            for (pk, row) in rows {
                let media = integer(row, "ZMEDIA");
                if media > 0 {
                    media_owners.entry(media).or_default().insert(*pk);
                }
            }
        }
        for id in &self.objects {
            for historical in [false, true] {
                if let Some(note) = owner(
                    *id,
                    historical,
                    &self.all,
                    &self.previous.objects,
                    self.entities.note,
                    &BTreeSet::new(),
                ) {
                    changed.insert(note);
                }
                if let Some(owners) = media_owners.get(id) {
                    for pk in owners {
                        if let Some(note) = owner(
                            *pk,
                            historical,
                            &self.all,
                            &self.previous.objects,
                            self.entities.note,
                            &BTreeSet::new(),
                        ) {
                            changed.insert(note);
                        }
                    }
                }
            }
            if self.all.get(id).is_some_and(|row| {
                [self.entities.folder, self.entities.account].contains(&integer(row, "Z_ENT"))
            }) {
                changed.extend(self.folder_notes(*id));
            }
        }
        for (pk, note) in &self.all {
            if integer(note, "Z_ENT") == self.entities.note
                && self.bodies.contains(&integer(note, "ZNOTEDATA"))
            {
                changed.insert(*pk);
            }
        }
        changed
    }
    fn keys(&self, removed: &mut BTreeSet<String>) -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        for pk in self.changed_notes() {
            if let Some(row) = self
                .current
                .objects
                .get(&pk)
                .filter(|row| number(row, "ZMARKEDFORDELETION") == 0.0)
            {
                keys.insert(text(row, "ZIDENTIFIER").to_owned());
            } else if let Some(row) = self.all.get(&pk)
                && let Some(key) = row.get("ZIDENTIFIER").and_then(serde_json::Value::as_str)
            {
                removed.insert(format!("notes.note/{key}"));
            }
        }
        keys
    }
}
pub(crate) fn scan(
    path: &Path,
    previous: Option<&Checkpoint>,
    facts: &Facts,
    context: &Context,
) -> Result<ChangeScan> {
    let db = Database::open(path)?;
    let (checkpoint, entities) = load(&db)?;
    let previous = previous.filter(|p| p.schema == checkpoint.schema);
    let empty = Checkpoint {
        schema: String::new(),
        objects: BTreeMap::new(),
        bodies: BTreeMap::new(),
    };
    let difference = Difference::new(&checkpoint, previous.unwrap_or(&empty), entities);
    let mut removed = difference.removed_identities();
    let keys = difference.keys(&mut removed);
    let snapshot = notes_store::read(&db, path, previous.map(|_| &keys), true, facts, context)?;
    let projected = snapshot
        .observations
        .iter()
        .filter(|row| row.collection == "notes.note")
        .map(|row| row.native_key.clone())
        .collect::<BTreeSet<_>>();
    removed.extend(
        keys.difference(&projected)
            .map(|key| format!("notes.note/{key}")),
    );
    for row in &snapshot.observations {
        removed.remove(&format!("{}/{}", row.collection, row.native_key));
    }
    db.finish()?;
    Ok(ChangeScan {
        snapshot,
        removed,
        checkpoint,
        changed_notes: projected.len(),
        bootstrap: previous.is_none(),
    })
}
