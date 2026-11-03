use crate::Error;
use ab_native::{capture, safari, telephony, usage, voice_memo};
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

const MAX_ROWS: usize = 1_000_000;

fn file_signature(path: &Path) -> Signature {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let mut out = Signature::new();
    for file in [path.to_path_buf(), PathBuf::from(wal)] {
        if let Ok(metadata) = fs::metadata(&file) {
            out.insert(file, (metadata.len(), metadata.modified().ok()));
        }
    }
    out
}

fn context_for(context: &ScanContext) -> Result<capture::Context, Error> {
    Ok(capture::Context {
        observed_at: context.observed_at()?,
        checkpoint: context.checkpoint.clone(),
        scan_id: context.scan_id.clone(),
    })
}

type TelephonyScan =
    fn(&Path, &capture::State, &capture::Context, bool, usize) -> capture::Result<capture::Pending>;

#[derive(Debug)]
pub struct TelephonyReader {
    name: &'static str,
    path: PathBuf,
    scan: TelephonyScan,
    state: capture::State,
    pending: Option<(capture::Pending, Signature)>,
    committed: Option<Signature>,
}

impl TelephonyReader {
    #[must_use]
    pub fn calls(root: &Path) -> Self {
        Self::new(
            "phone",
            root.join("CallHistory.storedata"),
            telephony::calls,
        )
    }

    #[must_use]
    pub fn voicemail(root: &Path) -> Self {
        Self::new("voicemail", root.join("voicemail.db"), telephony::voicemail)
    }

    fn new(name: &'static str, path: PathBuf, scan: TelephonyScan) -> Self {
        Self {
            name,
            path,
            scan,
            state: capture::State::default(),
            pending: None,
            committed: None,
        }
    }
}

impl CaptureReader for TelephonyReader {
    fn source(&self) -> &'static str {
        self.name
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let current = file_signature(&self.path);
        if current.is_empty() || self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let read = context_for(context)?;
        let Ok(pending) = (self.scan)(
            &self.path,
            &self.state,
            &read,
            self.committed.is_some(),
            MAX_ROWS,
        ) else {
            return Ok(None);
        };
        let value = capture_value(&pending)?;
        self.pending = Some((pending, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (pending, current) = self.pending.take().ok_or(Error::Conflict)?;
        self.state.commit(pending).map_err(reader_error)?;
        self.committed = Some(current);
        Ok(())
    }
}

#[derive(Debug)]
pub struct VoiceMemosReader {
    path: PathBuf,
    staging: PathBuf,
    reader: Option<voice_memo::Reader>,
    pending: Option<(capture::Pending, Signature)>,
    committed: Option<Signature>,
}

impl VoiceMemosReader {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("CloudRecordings.db"),
            staging: root.join(".ab-session-assets"),
            reader: None,
            pending: None,
            committed: None,
        }
    }
}

impl CaptureReader for VoiceMemosReader {
    fn source(&self) -> &'static str {
        "voice_memos"
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let current = file_signature(&self.path);
        if current.is_empty() || self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        if self.reader.is_none() {
            self.reader = Some(voice_memo::Reader::open(&self.staging).map_err(reader_error)?);
        }
        let read = context_for(context)?;
        let live = self.committed.is_some();
        let Some(reader) = self.reader.as_mut() else {
            return Ok(None);
        };
        let Ok(pending) = reader.scan(&self.path, &read, live, MAX_ROWS) else {
            return Ok(None);
        };
        if !pending.batch.blob_handles.is_empty() {
            return Ok(None);
        }
        let value = capture_value(&pending)?;
        self.pending = Some((pending, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (pending, current) = self.pending.take().ok_or(Error::Conflict)?;
        self.reader
            .as_mut()
            .ok_or(Error::Conflict)?
            .commit(pending)
            .map_err(reader_error)?;
        self.committed = Some(current);
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
            "phone" => Box::new(TelephonyReader::calls(&config.root)),
            "voicemail" => Box::new(TelephonyReader::voicemail(&config.root)),
            "voice_memos" => Box::new(VoiceMemosReader::new(&config.root)),
            _ => return Err(Error::Invalid),
        };
        readers.insert(source, reader);
    }
    Ok(readers)
}
