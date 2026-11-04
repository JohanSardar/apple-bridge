use crate::{assets, capture::Error};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_VISITED: usize = 50_000;
const MAX_ITEMS: usize = 2_000;
const MAX_DEPTH: usize = 12;
const MAX_ACTIONS_FILE: u64 = 4 << 20;
const MAX_INFO_FILE: u64 = 1 << 20;
const MAX_TOTAL_BYTES: u64 = 32 << 20;
const MAX_RESULT_BYTES: usize = 900_000;
const DEADLINE: Duration = Duration::from_secs(10);
const ACTIONS_FILE: &str = "extract.actionsdata";
const METADATA_DIRECTORY: &str = "Metadata.appintents";

#[must_use]
pub fn default_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        home.join("Applications"),
    ]
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, Error> {
    let file = assets::open_regular(path)?;
    if file.metadata()?.len() > limit {
        return Err(Error::Incomplete);
    }
    let mut data = Vec::new();
    file.take(limit + 1).read_to_end(&mut data)?;
    if u64::try_from(data.len()).map_err(|_| Error::Corrupt)? > limit {
        return Err(Error::Incomplete);
    }
    Ok(data)
}

fn is_app(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension == "app")
}

fn bundle_of(actions: &Path) -> Option<PathBuf> {
    let mut bundle = actions.parent()?.to_path_buf();
    while !bundle
        .extension()
        .is_some_and(|extension| extension == "app" || extension == "appex")
    {
        if !bundle.pop() || bundle == Path::new("/") {
            return None;
        }
    }
    Some(bundle)
}

fn hidden(action: &Value) -> bool {
    action["isDiscoverable"] == false
        || action["visibilityMetadata"]["isDiscoverable"] == false
        || action["visibilityMetadata"]["assistantOnly"] == true
}

struct Walk {
    items: Vec<Value>,
    seen: BTreeSet<String>,
    visited: usize,
    bytes: u64,
    result_bytes: usize,
    complete: bool,
    started: Instant,
}

impl Walk {
    fn exhausted(&self) -> bool {
        self.visited > MAX_VISITED
            || self.started.elapsed() > DEADLINE
            || self.items.len() >= MAX_ITEMS
    }

    fn bundle_identifier(&mut self, actions: &Path) -> Option<String> {
        let info = bundle_of(actions)?.join("Contents/Info.plist");
        let limit = MAX_INFO_FILE.min(MAX_TOTAL_BYTES - self.bytes);
        let data = read_limited(&info, limit).ok()?;
        let document = ::plist::Value::from_reader(std::io::Cursor::new(&data)).ok()?;
        let identifier = document
            .as_dictionary()?
            .get("CFBundleIdentifier")?
            .as_string()?
            .to_owned();
        if identifier.len() > 1024 {
            return None;
        }
        self.bytes += u64::try_from(data.len()).ok()?;
        Some(identifier)
    }

    fn collect(&mut self, actions_file: &Path, size: u64) {
        if size == 0 || size > MAX_ACTIONS_FILE || self.bytes + size > MAX_TOTAL_BYTES {
            self.complete = false;
            return;
        }
        self.bytes += size;
        let document: Option<Value> = read_limited(actions_file, MAX_ACTIONS_FILE.min(size))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok());
        let Some(actions) = document.as_ref().and_then(|d| d["actions"].as_object()) else {
            self.complete = false;
            return;
        };
        let Some(app) = self.bundle_identifier(actions_file) else {
            self.complete = false;
            return;
        };
        for (key, action) in actions {
            if self.started.elapsed() > DEADLINE || self.items.len() >= MAX_ITEMS {
                self.complete = false;
                break;
            }
            if !action.is_object() {
                self.complete = false;
                continue;
            }
            if hidden(action) {
                continue;
            }
            let id = action["identifier"].as_str().unwrap_or(key);
            let title = action["title"]["key"].as_str().unwrap_or(id);
            if id.is_empty() || id.len() > 1024 || title.len() > 4096 {
                self.complete = false;
                continue;
            }
            if !self.seen.insert(format!("{app}/{id}")) {
                continue;
            }
            let cost = (app.len() + id.len() + title.len()) * 6 + 128;
            if self.result_bytes + cost >= MAX_RESULT_BYTES {
                self.complete = false;
                break;
            }
            self.result_bytes += cost;
            self.items
                .push(json!({"app":app,"identifier":id,"title":title}));
        }
    }

    fn scan(&mut self, root: &Path) {
        let mut stack = vec![(root.to_path_buf(), 0_usize, false)];
        while let Some((directory, depth, inside_app)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                self.complete = false;
                continue;
            };
            let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            let mut children = Vec::new();
            for entry in entries {
                if self.exhausted() {
                    self.complete = false;
                    return;
                }
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.') {
                    continue;
                }
                self.visited += 1;
                if self.visited > MAX_VISITED {
                    self.complete = false;
                    return;
                }
                if depth + 1 > MAX_DEPTH {
                    self.complete = false;
                    continue;
                }
                let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                    continue;
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    children.push((entry.path(), depth + 1, inside_app || is_app(&name)));
                } else if name == ACTIONS_FILE
                    && inside_app
                    && directory
                        .file_name()
                        .is_some_and(|parent| parent == METADATA_DIRECTORY)
                {
                    self.collect(&entry.path(), metadata.len());
                }
            }
            stack.extend(children.into_iter().rev());
        }
    }
}

#[must_use]
pub fn discover(roots: &[PathBuf]) -> Value {
    let mut walk = Walk {
        items: Vec::new(),
        seen: BTreeSet::new(),
        visited: 0,
        bytes: 0,
        result_bytes: 0,
        complete: true,
        started: Instant::now(),
    };
    for root in roots {
        if !root.exists() {
            continue;
        }
        walk.scan(root);
        if walk.exhausted() {
            break;
        }
    }
    json!({"items":walk.items,"complete":walk.complete,"schema_guaranteed":false})
}
