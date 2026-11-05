use crate::capture::{self, Context, Error, Pending, Result, ScanInput, State};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const SOURCE: &str = "shortcuts";
pub const FOLDER: &str = "shortcuts.folder";
pub const SHORTCUT: &str = "shortcuts.shortcut";
pub const GENERATION: &str = "native/1";
pub const PACK: [&str; 4] = ["AB Echo", "AB Clock", "AB Focus", "AB Pack v1"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub identifier: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub folders: Vec<Entry>,
    pub shortcuts: Vec<Entry>,
    pub membership: BTreeMap<String, String>,
}

fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, size)| {
            group.len() == size && group.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

pub fn parse_listing(text: &str) -> Result<Vec<Entry>> {
    let mut seen = BTreeSet::new();
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, tail) = line.rsplit_once(" (").ok_or(Error::Corrupt)?;
            let identifier = tail.strip_suffix(')').ok_or(Error::Corrupt)?;
            if name.is_empty()
                || !is_uuid(identifier)
                || !seen.insert(identifier.to_ascii_lowercase())
            {
                return Err(Error::Corrupt);
            }
            Ok(Entry {
                name: name.to_owned(),
                identifier: identifier.to_owned(),
            })
        })
        .collect()
}

impl Inventory {
    pub fn add_members(&mut self, folder: &Entry, members: Vec<Entry>) -> Result<()> {
        for member in members {
            if self
                .membership
                .insert(member.identifier, folder.identifier.clone())
                .is_some()
            {
                return Err(Error::Corrupt);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn pack_version(&self) -> Option<&'static str> {
        PACK.iter()
            .all(|name| {
                self.shortcuts
                    .iter()
                    .filter(|entry| entry.name == *name)
                    .count()
                    == 1
            })
            .then_some("1")
    }
}

fn alias(collection: &str, key: &str) -> String {
    format!("{SOURCE}/{collection}/{key}")
}

pub fn prepare(
    state: &State,
    inventory: &Inventory,
    context: &Context,
    live: bool,
) -> Result<Pending> {
    let mut records = Vec::new();
    for folder in &inventory.folders {
        records.push(capture::upsert(
            FOLDER,
            &folder.identifier,
            Some(alias(FOLDER, &folder.identifier)),
            json!({"name": folder.name, "identifier": folder.identifier}),
            context,
        )?);
    }
    for shortcut in &inventory.shortcuts {
        let folder = inventory.membership.get(&shortcut.identifier).map_or(
            Value::Null,
            |key| json!({"$ref": {"collection": FOLDER, "native_key": key}}),
        );
        records.push(capture::upsert(
            SHORTCUT,
            &shortcut.identifier,
            Some(alias(SHORTCUT, &shortcut.identifier)),
            json!({
                "name": shortcut.name,
                "identifier": shortcut.identifier,
                "accepts_input": null,
                "folder_id": folder,
            }),
            context,
        )?);
    }
    state.prepare(ScanInput {
        source: SOURCE,
        generation: GENERATION,
        records,
        scope: vec![FOLDER.into(), SHORTCUT.into()],
        coverage: Value::Null,
        context,
        live,
    })
}

#[cfg(test)]
mod tests {
    use super::{Entry, Inventory, parse_listing};

    const ONE: &str = "11111111-1111-1111-1111-111111111111";
    const TWO: &str = "22222222-2222-2222-2222-222222222222";

    #[test]
    fn a_listing_is_all_or_nothing() {
        let good = format!("Echo (one) ({ONE})\nOther ({TWO})\n");
        let parsed = parse_listing(&good).unwrap_or_default();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "Echo (one)");
        assert_eq!(parsed[0].identifier, ONE);
        assert!(parse_listing("").is_ok_and(|rows| rows.is_empty()));
        assert!(parse_listing("no identifier").is_err());
        assert!(parse_listing(&format!("A (not-a-uuid)\nB ({ONE})")).is_err());
        assert!(parse_listing(&format!("A ({ONE})\nB ({ONE})")).is_err());
        assert!(parse_listing(&format!(" ({ONE})")).is_err());
    }

    #[test]
    fn the_pack_needs_each_name_once() {
        let entry = |name: &str, id: u32| Entry {
            name: name.into(),
            identifier: format!("{id}"),
        };
        let mut inventory = Inventory {
            shortcuts: vec![
                entry("AB Echo", 1),
                entry("AB Clock", 2),
                entry("AB Focus", 3),
                entry("AB Pack v1", 4),
            ],
            ..Inventory::default()
        };
        assert_eq!(inventory.pack_version(), Some("1"));
        inventory.shortcuts.push(entry("AB Echo", 5));
        assert_eq!(inventory.pack_version(), None);
    }

    #[test]
    fn a_shortcut_in_two_folders_is_refused() {
        let folder = |id: &str| Entry {
            name: id.into(),
            identifier: id.into(),
        };
        let member = || {
            vec![Entry {
                name: "s".into(),
                identifier: ONE.into(),
            }]
        };
        let mut inventory = Inventory::default();
        assert!(inventory.add_members(&folder("a"), member()).is_ok());
        assert!(inventory.add_members(&folder("b"), member()).is_err());
    }
}
