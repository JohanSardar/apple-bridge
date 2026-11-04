use crate::capture::{self, Context, Error, Observation, Result, ScanInput, State};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const SOURCE: &str = "files";
pub const COLLECTION: &str = "files.item";
pub const GENERATION: &str = "native/1";
pub const MAX_ITEMS: usize = 10_000;
pub const MAX_DEPTH: usize = 16;
pub const MAX_ROOTS: usize = 16;
pub const TAGS_ATTRIBUTE: &str = "com.apple.metadata:_kMDItemUserTags";
pub const LOCAL: &str = "local";
pub const NOT_DOWNLOADED: &str = "notDownloaded";

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub path: String,
    pub name: String,
    pub directory: bool,
    pub extension: String,
    pub size: u64,
    pub created: Option<f64>,
    pub modified: Option<f64>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Walk {
    pub root: String,
    pub entries: Vec<Entry>,
    pub complete: bool,
}

fn seconds(time: std::io::Result<std::time::SystemTime>) -> Option<f64> {
    let duration = time.ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(duration.as_secs_f64())
}

#[must_use]
pub fn parse_tags(bytes: &[u8]) -> Vec<String> {
    let Ok(document) = ::plist::Value::from_reader(std::io::Cursor::new(bytes)) else {
        return Vec::new();
    };
    document
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.as_string())
        .map(|tag| tag.split('\n').next().unwrap_or_default().to_owned())
        .collect()
}

pub fn walk(root: &Path, attribute: &dyn Fn(&Path, &str) -> Option<Vec<u8>>) -> Result<Walk> {
    let root = root.canonicalize()?;
    let root_text = root.to_str().ok_or(Error::Corrupt)?.to_owned();
    let mut walk = Walk {
        root: root_text.clone(),
        entries: Vec::new(),
        complete: true,
    };
    let mut visited = 0_usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root, 0)];
    while let Some((directory, depth)) = stack.pop() {
        let Ok(listing) = std::fs::read_dir(&directory) else {
            walk.complete = false;
            continue;
        };
        let mut listing: Vec<_> = listing.filter_map(std::result::Result::ok).collect();
        listing.sort_by_key(std::fs::DirEntry::file_name);
        let mut children = Vec::new();
        for item in listing {
            visited += 1;
            if visited > MAX_ITEMS {
                walk.complete = false;
                return Ok(walk);
            }
            if depth + 1 > MAX_DEPTH {
                walk.complete = false;
                continue;
            }
            let path = item.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                walk.complete = false;
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            let key = path.to_str().ok_or(Error::Corrupt)?.to_owned();
            let relative = key
                .strip_prefix(&root_text)
                .and_then(|rest| rest.strip_prefix('/'))
                .ok_or(Error::Corrupt)?
                .to_owned();
            let directory_item = metadata.is_dir();
            walk.entries.push(Entry {
                key,
                path: relative,
                directory: directory_item,
                extension: path
                    .extension()
                    .map(|extension| extension.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                size: if directory_item { 0 } else { metadata.len() },
                created: seconds(metadata.created()),
                modified: seconds(metadata.modified()),
                tags: attribute(&path, TAGS_ATTRIBUTE)
                    .map(|bytes| parse_tags(&bytes))
                    .unwrap_or_default(),
                name,
            });
            if directory_item {
                children.push((path, depth + 1));
            }
        }
        stack.extend(children.into_iter().rev());
    }
    Ok(walk)
}

fn stamp(value: Option<f64>) -> Result<Value> {
    value
        .map(|time| capture::timestamp(time).map(Value::String))
        .transpose()
        .map(|stamp| stamp.unwrap_or(Value::Null))
}

pub fn observations(
    walk: &Walk,
    statuses: &BTreeMap<String, String>,
    context: &Context,
) -> Result<Vec<Observation>> {
    walk.entries
        .iter()
        .map(|entry| {
            let status = statuses
                .get(&entry.key)
                .map_or(LOCAL, std::string::String::as_str);
            let data = json!({
                "root": walk.root,
                "path": entry.path,
                "name": entry.name,
                "kind": if entry.directory { "directory" } else { entry.extension.as_str() },
                "size": entry.size,
                "created": stamp(entry.created)?,
                "modified": stamp(entry.modified)?,
                "tags": entry.tags,
                "downloaded": status != NOT_DOWNLOADED,
                "icloud_status": status,
            });
            capture::upsert(COLLECTION, &entry.key, None, data, context)
        })
        .collect()
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Reader {
    roots: BTreeMap<String, State>,
}

#[derive(Debug)]
pub struct Pending {
    pub root: String,
    pub batch: capture::Pending,
}

impl Reader {
    pub fn prepare(
        &self,
        walk: &Walk,
        statuses: &BTreeMap<String, String>,
        context: &Context,
    ) -> Result<Pending> {
        let records = observations(walk, statuses, context)?;
        let state = self.roots.get(&walk.root).cloned().unwrap_or_default();
        let input = ScanInput {
            source: SOURCE,
            generation: GENERATION,
            records,
            scope: vec![COLLECTION.into()],
            coverage: Value::Null,
            context,
            live: true,
        };
        let batch = if walk.complete {
            state.prepare(input)?
        } else {
            state.prepare_incomplete(input)?
        };
        Ok(Pending {
            root: walk.root.clone(),
            batch,
        })
    }

    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        let mut state = self.roots.get(&pending.root).cloned().unwrap_or_default();
        state.commit(pending.batch)?;
        self.roots.insert(pending.root, state);
        Ok(())
    }
}
