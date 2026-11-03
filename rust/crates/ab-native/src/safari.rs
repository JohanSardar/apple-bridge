use crate::{
    body_digest,
    capture::{self, Context, Error, Observation, Result, Snapshot},
    plist::{self, Value},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

const HISTORY_GENERATION: &str = "SELECT 'safari-history-1'";
const CLOUD_GENERATION: &str = "SELECT 'safari-cloud-1'";

#[derive(Debug)]
pub struct Scan {
    pub generation: String,
    pub observations: Vec<Observation>,
    pub expires_at: Option<f64>,
}
fn observation(
    collection: &str,
    key: &str,
    data: serde_json::Value,
    context: &Context,
) -> Result<Observation> {
    capture::upsert(
        collection,
        key,
        Some(format!("safari/{collection}/{key}")),
        data,
        context,
    )
}

pub fn history(
    directory: &Path,
    now: f64,
    days: f64,
    max_rows: usize,
    context: &Context,
) -> Result<Scan> {
    if !now.is_finite() || !days.is_finite() {
        return Err(Error::Corrupt);
    }
    let reader = Snapshot::open(&directory.join("History.db"), HISTORY_GENERATION)?;
    reader.require(&[
        ("history_items", &["id", "url", "visit_count"]),
        (
            "history_visits",
            &["id", "history_item", "visit_time", "title"],
        ),
    ])?;
    let rows=reader.query("SELECT CAST(i.id AS TEXT) AS key, i.url, COUNT(v.id) AS visit_count, MAX(v.visit_time) AS last_visit, (SELECT title FROM history_visits WHERE history_item=i.id ORDER BY visit_time DESC,id DESC LIMIT 1) AS title FROM history_items i JOIN history_visits v ON v.history_item=i.id WHERE v.visit_time >= ? GROUP BY i.id ORDER BY i.id",&[(now-978_307_200.0-days*86400.0).to_string()],max_rows,Duration::from_secs(15))?;
    let mut observations = Vec::new();
    let mut keys = BTreeSet::new();
    for row in rows {
        let url = capture::text(&row, "url");
        let key = body_digest(url.as_bytes());
        if !keys.insert(key.clone()) || capture::text(&row, "key").is_empty() {
            return Err(Error::Corrupt);
        }
        observations.push(observation("safari.history",&key,json!({"url":url,"title":capture::text(&row,"title"),"last_visit":capture::timestamp(capture::number(&row,"last_visit")+978_307_200.0)?,"visit_count":capture::number(&row,"visit_count")}),context)?);
    }
    let expiry = reader.query(
        "SELECT MIN(visit_time) AS oldest FROM history_visits WHERE visit_time >= ?",
        &[(now - 978_307_200.0 - days * 86400.0).to_string()],
        2,
        Duration::from_secs(15),
    )?;
    let expires_at = expiry
        .first()
        .and_then(|r| r.get("oldest"))
        .and_then(|v| match v {
            capture::Cell::Integer(_) | capture::Cell::Real(_) => {
                Some(v.number() + 978_307_200.0 + days * 86400.0 + 0.001)
            }
            _ => None,
        });
    reader.finish(HISTORY_GENERATION)?;
    Ok(Scan {
        generation: reader.generation,
        observations,
        expires_at,
    })
}
pub fn cloud_tabs(directory: &Path, max_rows: usize, context: &Context) -> Result<Scan> {
    let reader = Snapshot::open(&directory.join("CloudTabs.db"), CLOUD_GENERATION)?;
    reader.require(&[
        ("cloud_tabs", &["tab_uuid", "device_uuid", "title", "url"]),
        ("cloud_tab_devices", &["device_uuid", "device_name"]),
    ])?;
    let rows=reader.query("SELECT t.tab_uuid AS key,t.title,t.url,COALESCE(d.device_name,t.device_uuid) AS device FROM cloud_tabs t LEFT JOIN cloud_tab_devices d ON t.device_uuid=d.device_uuid ORDER BY t.tab_uuid",&[],max_rows,Duration::from_secs(5))?;
    let mut keys = BTreeSet::new();
    let mut observations = Vec::new();
    for row in rows {
        let key = capture::text(&row, "key");
        if key.is_empty() || !keys.insert(key.to_owned()) {
            return Err(Error::Corrupt);
        }
        observations.push(observation("safari.cloud_tab",key,json!({"device":capture::text(&row,"device"),"title":capture::text(&row,"title"),"url":capture::text(&row,"url")}),context)?);
    }
    reader.finish(CLOUD_GENERATION)?;
    Ok(Scan {
        generation: reader.generation,
        observations,
        expires_at: None,
    })
}

struct Bookmarks<'a> {
    context: &'a Context,
    keys: BTreeSet<String>,
    observations: Vec<Observation>,
}
impl Bookmarks<'_> {
    fn walk(
        &mut self,
        node: &BTreeMap<String, Value>,
        folders: &[String],
        reading: bool,
        depth: usize,
    ) -> Result<()> {
        if depth >= 100 {
            return Err(Error::Corrupt);
        }
        let text = |key: &str| node.get(key).and_then(Value::string).unwrap_or_default();
        let kind = text("WebBookmarkType");
        let title = text("Title");
        if kind == "WebBookmarkTypeList" || node.contains_key("Children") {
            let mut folders = folders.to_vec();
            if !title.is_empty() {
                folders.push(title.into());
            }
            let children = match node.get("Children") {
                None => &[][..],
                Some(Value::Array(v)) => v.as_slice(),
                _ => return Err(Error::Corrupt),
            };
            for child in children {
                self.walk(
                    child.dictionary().ok_or(Error::Corrupt)?,
                    &folders,
                    reading || title == "com.apple.ReadingList",
                    depth + 1,
                )?;
            }
        } else if kind == "WebBookmarkTypeLeaf" {
            let url = node
                .get("URLString")
                .and_then(Value::string)
                .ok_or(Error::Corrupt)?;
            let key = text("WebBookmarkUUID");
            if key.is_empty() || !self.keys.insert(key.into()) {
                return Err(Error::Corrupt);
            }
            let title = node
                .get("URIDictionary")
                .and_then(Value::dictionary)
                .and_then(|m| m.get("title"))
                .and_then(Value::string)
                .unwrap_or(title);
            let mut data = json!({"url":url,"title":title});
            if reading {
                let meta = node.get("ReadingList").and_then(Value::dictionary);
                data["added"] = match meta.and_then(|m| m.get("DateAdded")) {
                    Some(Value::Date(t)) => json!(capture::timestamp(t + 978_307_200.0)?),
                    _ => serde_json::Value::Null,
                };
                data["read"] = json!(meta.is_some_and(|m| m.contains_key("DateLastViewed")));
            } else {
                data["folder_path"] = json!(folders.join("/"));
            }
            self.observations.push(observation(
                if reading {
                    "safari.reading_list"
                } else {
                    "safari.bookmark"
                },
                key,
                data,
                self.context,
            )?);
        } else if kind != "WebBookmarkTypeProxy" {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}
pub fn bookmarks(directory: &Path, context: &Context) -> Result<Scan> {
    let path = directory.join("Bookmarks.plist");
    let bytes = std::fs::read(&path)?;
    let root = plist::decode(&bytes)?;
    let root = root.dictionary().ok_or(Error::Corrupt)?;
    if !matches!(root.get("Children"),Some(Value::Array(values)) if values.iter().all(|v|v.dictionary().is_some()))
    {
        return Err(Error::Corrupt);
    }
    let mut reader = Bookmarks {
        context,
        keys: BTreeSet::new(),
        observations: Vec::new(),
    };
    reader.walk(root, &[], false, 0)?;
    if std::fs::read(path)? != bytes {
        return Err(Error::Conflict);
    }
    Ok(Scan {
        generation: "bookmarks-1".into(),
        observations: reader.observations,
        expires_at: None,
    })
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    History,
    Bookmarks,
    Cloud,
}
impl Kind {
    pub fn scope(self) -> Vec<String> {
        match self {
            Self::History => vec!["safari.history".into()],
            Self::Bookmarks => vec!["safari.bookmark".into(), "safari.reading_list".into()],
            Self::Cloud => vec!["safari.cloud_tab".into()],
        }
    }
    fn filename(self) -> &'static str {
        match self {
            Self::History => "History.db",
            Self::Bookmarks => "Bookmarks.plist",
            Self::Cloud => "CloudTabs.db",
        }
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReadContext {
    #[serde(flatten)]
    pub capture: Context,
    pub generation: String,
    pub now: f64,
    #[serde(default = "default_days")]
    pub days: f64,
    #[serde(default = "default_rows")]
    pub max_rows: usize,
    #[serde(default)]
    pub automation_permitted: bool,
}
fn default_days() -> f64 {
    90.0
}
fn default_rows() -> usize {
    1_000_000
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Key {
    pub collection: String,
    #[serde(rename = "nativeKey")]
    pub native_key: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CachedScan {
    pub generation: String,
    pub keys: Vec<Key>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cache {
    pub generation: String,
    pub scans: BTreeMap<Kind, CachedScan>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    key: Key,
    data: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    mode: u32,
    uid: u32,
    gid: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Inputs {
    main: Option<FileStamp>,
    wal: Option<FileStamp>,
}
fn inputs(root: &Path, kind: Kind) -> Inputs {
    use std::os::unix::fs::MetadataExt;
    let path = root.join(kind.filename());
    let path = path.canonicalize().unwrap_or(path);
    let stamp = |path: &Path, wal: bool| {
        std::fs::metadata(path)
            .ok()
            .filter(|m| !wal || m.len() > 32)
            .map(|m| FileStamp {
                device: m.dev(),
                inode: m.ino(),
                size: m.len(),
                modified: m.modified().ok(),
                created: m.created().ok(),
                mode: m.mode(),
                uid: m.uid(),
                gid: m.gid(),
            })
    };
    Inputs {
        main: stamp(&path, false),
        wal: if kind == Kind::Bookmarks {
            None
        } else {
            stamp(
                &std::path::PathBuf::from(format!("{}-wal", path.display())),
                true,
            )
        },
    }
}
#[derive(Debug, Clone, Default)]
struct KindState {
    generation: String,
    records: Vec<Stored>,
    material: Option<(String, Vec<Stored>)>,
    inputs: Option<Inputs>,
    expires_at: Option<f64>,
    revision: u64,
}
#[derive(Debug, Default)]
pub struct Reader {
    states: BTreeMap<Kind, KindState>,
}
#[derive(Debug, serde::Serialize)]
pub struct Pending {
    #[serde(flatten)]
    pub batch: capture::Batch,
    pub reconcile_again: bool,
    #[serde(skip)]
    kind: Kind,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    next: KindState,
}
impl Reader {
    pub fn resume(cache: Cache) -> Result<Self> {
        if cache.generation.is_empty() {
            return Err(Error::Corrupt);
        }
        let mut states = BTreeMap::new();
        for (kind, scan) in cache.scans {
            if scan.generation.is_empty() {
                return Err(Error::Corrupt);
            }
            let mut seen = BTreeSet::new();
            let scope = kind.scope();
            let mut records = Vec::new();
            for key in scan.keys {
                if key.native_key.is_empty()
                    || !scope.contains(&key.collection)
                    || !seen.insert((key.collection.clone(), key.native_key.clone()))
                {
                    return Err(Error::Corrupt);
                }
                records.push(Stored {
                    key,
                    data: "null".into(),
                });
            }
            states.insert(
                kind,
                KindState {
                    generation: scan.generation,
                    records,
                    ..KindState::default()
                },
            );
        }
        Ok(Self { states })
    }
    pub fn cache(&self, generation: &str) -> Cache {
        Cache {
            generation: generation.into(),
            scans: self
                .states
                .iter()
                .map(|(kind, state)| {
                    (
                        *kind,
                        CachedScan {
                            generation: state.generation.clone(),
                            keys: state.records.iter().map(|s| s.key.clone()).collect(),
                        },
                    )
                })
                .collect(),
        }
    }
    pub fn poll(
        &mut self,
        root: &Path,
        kind: Kind,
        context: &ReadContext,
    ) -> Result<Option<Pending>> {
        let input = inputs(root, kind);
        let prior = self.states.get(&kind).cloned().unwrap_or_default();
        let expired =
            kind == Kind::History && prior.expires_at.is_some_and(|expiry| context.now >= expiry);
        if prior.inputs.as_ref() == Some(&input) && !expired {
            return Ok(None);
        }
        let scan = match kind {
            Kind::History => history(
                root,
                context.now,
                context.days,
                context.max_rows,
                &context.capture,
            )?,
            Kind::Bookmarks => bookmarks(root, &context.capture)?,
            Kind::Cloud => cloud_tabs(root, context.max_rows.min(100_000), &context.capture)?,
        };
        self.prepare(kind, scan, input, &prior, context)
    }
    fn prepare(
        &mut self,
        kind: Kind,
        scan: Scan,
        input: Inputs,
        prior: &KindState,
        context: &ReadContext,
    ) -> Result<Option<Pending>> {
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for row in &scan.observations {
            let key = Key {
                collection: row.collection.clone(),
                native_key: row.native_key.clone(),
            };
            if !seen.insert((key.collection.clone(), key.native_key.clone())) {
                return Err(Error::Corrupt);
            }
            rows.push(Stored {
                key,
                data: row.data.as_ref().ok_or(Error::Corrupt)?.get().into(),
            });
        }
        let material = (scan.generation.clone(), rows.clone());
        if prior.material.as_ref() == Some(&material) {
            if let Some(state) = self.states.get_mut(&kind) {
                state.inputs = Some(input);
                state.expires_at = scan.expires_at;
            }
            return Ok(None);
        }
        let old = prior
            .records
            .iter()
            .map(|r| ((r.key.collection.as_str(), r.key.native_key.as_str()), r))
            .collect::<BTreeMap<_, _>>();
        let mut observations = Vec::new();
        for row in scan.observations {
            if old
                .get(&(row.collection.as_str(), row.native_key.as_str()))
                .is_none_or(|old| {
                    !capture::data_equal(&old.data, row.data.as_ref().map_or("", |v| v.get()))
                })
            {
                observations.push(row);
            }
        }
        let same = prior.generation == scan.generation;
        let missing = prior
            .records
            .iter()
            .filter(|old| !seen.contains(&(old.key.collection.clone(), old.key.native_key.clone())))
            .cloned()
            .collect::<Vec<_>>();
        if same {
            for old in &missing {
                observations.push(capture::absent(
                    &old.key.collection,
                    &old.key.native_key,
                    None,
                    &context.capture,
                ));
            }
        }
        let carry = !prior.generation.is_empty() && !same && !missing.is_empty();
        if carry {
            rows.extend(missing);
        }
        let next = KindState {
            generation: scan.generation,
            records: rows,
            material: if carry { None } else { Some(material) },
            inputs: if carry { None } else { Some(input) },
            expires_at: scan.expires_at,
            revision: prior.revision.checked_add(1).ok_or(Error::Conflict)?,
        };
        let mut missing_coverage = vec!["CloudTabs schema availability varies"];
        if !context.automation_permitted {
            missing_coverage.push("Automation dev.applebridge.helper → com.apple.Safari");
        }
        let batch = capture::Batch {
            source: "safari".into(),
            generation: context.generation.clone(),
            checkpoint: context.capture.checkpoint.clone(),
            scan_id: context.capture.scan_id.clone(),
            scan_started_at: context.capture.observed_at.clone(),
            scope: kind.scope(),
            completion_evidence: "complete",
            coverage: json!({"fields":"partial","missing":missing_coverage}),
            observations,
            blob_handles: BTreeMap::new(),
            live: prior.material.is_some() || prior.revision > 0,
        };
        Ok(Some(Pending {
            batch,
            reconcile_again: carry,
            kind,
            revision: prior.revision,
            next,
        }))
    }
    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        if self.states.get(&pending.kind).map_or(0, |s| s.revision) != pending.revision {
            return Err(Error::Conflict);
        }
        self.states.insert(pending.kind, pending.next);
        Ok(())
    }
}
