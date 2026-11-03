use crate::Error;
use ab_native::{capture, notes_reader, safari, usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

fn reader_error(error: impl std::fmt::Display) -> Error {
    Error::Reader(error.to_string())
}

#[derive(Clone, Debug)]
pub struct ScanContext {
    pub checkpoint: String,
    pub scan_id: String,
    pub now: f64,
}

impl ScanContext {
    fn observed_at(&self) -> Result<String, Error> {
        capture::timestamp(self.now).map_err(reader_error)
    }
}

pub trait CaptureReader: Send + Debug {
    fn source(&self) -> &'static str;
    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error>;
    fn commit(&mut self) -> Result<(), Error>;
}

fn capture_value(batch: &impl Serialize) -> Result<Value, Error> {
    let mut value = serde_json::to_value(batch)?;
    value
        .as_object_mut()
        .ok_or(Error::Invalid)?
        .remove("coverage");
    Ok(value)
}

#[derive(Debug)]
pub struct SafariReader {
    root: PathBuf,
    generation: String,
    reader: safari::Reader,
    pending: Option<safari::Pending>,
}

impl SafariReader {
    #[must_use]
    pub fn new(root: PathBuf, generation: String) -> Self {
        Self {
            root,
            generation,
            reader: safari::Reader::default(),
            pending: None,
        }
    }
}

impl CaptureReader for SafariReader {
    fn source(&self) -> &'static str {
        "safari"
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        for kind in [
            safari::Kind::History,
            safari::Kind::Bookmarks,
            safari::Kind::Cloud,
        ] {
            let read = safari::ReadContext {
                capture: capture::Context {
                    observed_at: context.observed_at()?,
                    checkpoint: context.checkpoint.clone(),
                    scan_id: context.scan_id.clone(),
                },
                generation: self.generation.clone(),
                now: context.now,
                days: 90.0,
                max_rows: 1_000_000,
                automation_permitted: true,
            };
            let Ok(Some(pending)) = self.reader.poll(&self.root, kind, &read) else {
                continue;
            };
            let value = capture_value(&pending.batch)?;
            self.pending = Some(pending);
            return Ok(Some(value));
        }
        Ok(None)
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.reader.commit(pending).map_err(reader_error)
    }
}

type Signature = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;

fn signature(root: &Path) -> Result<Signature, Error> {
    fn walk(directory: &Path, out: &mut Signature) -> Result<(), Error> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                walk(&entry.path(), out)?;
            } else {
                out.insert(entry.path(), (metadata.len(), metadata.modified().ok()));
            }
        }
        Ok(())
    }
    let mut out = Signature::new();
    walk(root, &mut out)?;
    Ok(out)
}

#[derive(Debug)]
pub struct UsageReader {
    root: PathBuf,
    generation: String,
    zone: String,
    local_name: String,
    reader: usage::Reader,
    pending: Option<(usage::Scan, Signature)>,
    committed: Option<Signature>,
}

impl UsageReader {
    #[must_use]
    pub fn new(root: PathBuf, generation: String, zone: String, local_name: String) -> Self {
        Self {
            root,
            generation,
            zone,
            local_name,
            reader: usage::Reader::default(),
            pending: None,
            committed: None,
        }
    }
}

impl CaptureReader for UsageReader {
    fn source(&self) -> &'static str {
        "usage"
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let current = signature(&self.root)?;
        if self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let read = usage::Context {
            generation: self.generation.clone(),
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
            observed_at: context.observed_at()?,
            zone: self.zone.clone(),
            local_name: self.local_name.clone(),
            blocked: Vec::new(),
            power: Vec::new(),
            history: None,
        };
        let Ok(scan) = self
            .reader
            .scan(&self.root, &read, self.committed.is_some())
        else {
            return Ok(None);
        };
        let value = capture_value(&scan)?;
        self.pending = Some((scan, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (scan, current) = self.pending.take().ok_or(Error::Conflict)?;
        self.reader.commit(scan).map_err(reader_error)?;
        self.committed = Some(current);
        Ok(())
    }
}

#[derive(Debug)]
enum NotesPending {
    Scan(Box<notes_reader::Pending>),
    Hydration(Box<notes_reader::Hydration>),
}

#[derive(Debug)]
pub struct NotesReader {
    root: PathBuf,
    reader: notes_reader::Reader,
    pending: Option<(NotesPending, Option<Signature>)>,
    committed: Option<Signature>,
}

impl NotesReader {
    pub fn new(root: PathBuf) -> Result<Self, Error> {
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(Error::Invalid)?;
        let staging = root.with_file_name(format!("{name}-staging"));
        let reader = notes_reader::Reader::open(&root.join("NoteStore.sqlite"), &staging)
            .map_err(reader_error)?;
        Ok(Self {
            root,
            reader,
            pending: None,
            committed: None,
        })
    }

    fn notes_context(context: &ScanContext) -> Result<notes_reader::Context, Error> {
        Ok(notes_reader::Context {
            capture: capture::Context {
                observed_at: context.observed_at()?,
                checkpoint: context.checkpoint.clone(),
                scan_id: context.scan_id.clone(),
            },
            coverage: serde_json::json!({}),
            facts: notes_reader::Facts::default(),
            asset_paths: Vec::new(),
        })
    }
}

impl CaptureReader for NotesReader {
    fn source(&self) -> &'static str {
        "notes"
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let notes = Self::notes_context(context)?;
        while self.reader.has_hydration() {
            match self.reader.hydrate_next(&notes) {
                Ok(Some(hydration)) => {
                    let value = capture_value(&hydration.batch)?;
                    self.pending = Some((NotesPending::Hydration(Box::new(hydration)), None));
                    return Ok(Some(value));
                }
                Ok(None) => {}
                Err(error) => return Err(reader_error(error)),
            }
        }
        self.reader
            .finish_hydration(SystemTime::now())
            .map_err(reader_error)?;
        let current = signature(&self.root)?;
        if self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let scan = self.reader.reconcile(&notes).map_err(reader_error)?;
        let value = capture_value(&scan.batch)?;
        self.pending = Some((NotesPending::Scan(Box::new(scan)), Some(current)));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (pending, current) = self.pending.take().ok_or(Error::Conflict)?;
        match pending {
            NotesPending::Scan(scan) => {
                self.reader.commit(*scan).map_err(reader_error)?;
                self.committed = current;
            }
            NotesPending::Hydration(hydration) => {
                self.reader
                    .commit_hydration(*hydration)
                    .map_err(reader_error)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceConfig {
    root: PathBuf,
    #[serde(default = "default_generation")]
    generation: String,
    #[serde(default = "default_zone")]
    zone: String,
    #[serde(default = "default_local_name")]
    local_name: String,
}

fn default_generation() -> String {
    "rust-native/1".into()
}

fn default_zone() -> String {
    "UTC".into()
}

fn default_local_name() -> String {
    "local".into()
}

pub fn from_config(config: &str) -> Result<BTreeMap<String, Box<dyn CaptureReader>>, Error> {
    let sources: BTreeMap<String, SourceConfig> = serde_json::from_str(config)?;
    let mut readers: BTreeMap<String, Box<dyn CaptureReader>> = BTreeMap::new();
    for (source, config) in sources {
        let reader: Box<dyn CaptureReader> = match source.as_str() {
            "safari" => Box::new(SafariReader::new(config.root, config.generation)),
            "usage" => Box::new(UsageReader::new(
                config.root,
                config.generation,
                config.zone,
                config.local_name,
            )),
            "notes" => Box::new(NotesReader::new(config.root)?),
            _ => return Err(Error::Invalid),
        };
        readers.insert(source, reader);
    }
    Ok(readers)
}
