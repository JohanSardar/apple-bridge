use crate::{Error, native_facts};

mod agenda;
mod background;
mod config;
mod files;
mod focus;
mod mac;
mod mail;
mod messages;
mod notes;
mod pdf;
mod safari;
mod shortcuts;
mod telephony;
mod usage;
mod voice_memos;
use ab_native::capture;
pub use agenda::AgendaReader;
pub use config::{confine_staging, from_config, from_config_linked, wants_inherited_facts};
pub use files::FilesReader;
pub use focus::FocusReader;
pub use mac::MacReader;
pub use mail::MailReader;
pub use messages::MessagesReader;
pub use notes::NotesReader;
pub use pdf::PdfReader;
pub use safari::SafariReader;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub use shortcuts::ShortcutsReader;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub use telephony::TelephonyReader;
pub use usage::UsageReader;
pub use voice_memos::VoiceMemosReader;

fn reader_error(error: impl std::fmt::Display) -> Error {
    Error::Reader(error.to_string())
}

#[derive(Clone, Debug)]
pub struct ScanContext {
    pub checkpoint: String,
    pub scan_id: String,
    pub now: f64,
    pub fence: u64,
}

impl ScanContext {
    fn observed_at(&self) -> Result<String, Error> {
        capture::timestamp(self.now).map_err(reader_error)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Readiness {
    pub state: &'static str,
    pub coverage: Value,
    pub capabilities: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct ReadinessContext<'a> {
    pub fence: u64,
    pub os_build: &'a str,
}

impl Readiness {
    #[must_use]
    pub fn ready(coverage: Value, capabilities: &[&str]) -> Self {
        Self {
            state: "ready",
            coverage,
            capabilities: capabilities.iter().map(|text| (*text).to_owned()).collect(),
            reason: String::new(),
        }
    }

    #[must_use]
    pub fn not(state: &'static str, coverage: Value, reason: &str) -> Self {
        Self {
            state,
            coverage,
            capabilities: Vec::new(),
            reason: reason.to_owned(),
        }
    }
}

pub trait BlobServer: Send + Sync + Debug {
    fn owns(&self, handle: &str) -> bool;
    fn read(&self, handle: &str, offset: u64, length: usize) -> Result<Value, Error>;
}

pub trait CaptureReader: Send + Debug {
    fn source(&self) -> &'static str;
    fn root(&self) -> &Path;
    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.root().to_path_buf()]
    }
    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error>;
    fn commit(&mut self) -> Result<(), Error>;
    fn token(&self) -> Option<String>;
    fn resume(&mut self, token: &str) -> bool;
    fn owns(&self, _handle: &str) -> bool {
        false
    }
    fn handle_prefix(&self) -> Option<&'static str> {
        None
    }
    fn blobs(&self) -> Option<std::sync::Arc<dyn BlobServer>> {
        None
    }
    fn more(&self) -> bool {
        false
    }
    fn adopts_generation(&self) -> bool {
        true
    }
    fn blob(&mut self, _handle: &str, _offset: u64, _length: usize) -> Result<Value, Error> {
        Err(Error::Invalid)
    }
    fn refresh(&mut self) -> bool {
        false
    }
    fn readiness(&mut self, _context: &ReadinessContext<'_>) -> Option<Readiness> {
        None
    }
    fn answer(&mut self, _operation: &str, _args: &Value) -> Option<Result<Value, String>> {
        None
    }
    fn lookups(&self) -> Option<Arc<crate::calendar_lookup::Lookups>> {
        None
    }
}

fn capture_value(batch: &impl Serialize) -> Result<Value, Error> {
    let mut value = serde_json::to_value(batch)?;
    value
        .as_object_mut()
        .ok_or(Error::Invalid)?
        .remove("coverage");
    Ok(value)
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

fn encode(signature: &Signature) -> String {
    let rows: Vec<(String, u64, u128)> = signature
        .iter()
        .map(|(path, (size, modified))| {
            (
                path.to_string_lossy().into_owned(),
                *size,
                modified
                    .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos()),
            )
        })
        .collect();
    serde_json::to_string(&rows).unwrap_or_default()
}

fn decode(token: &str) -> Option<Signature> {
    let rows: Vec<(String, u64, u128)> = serde_json::from_str(token).ok()?;
    let mut out = Signature::new();
    for (path, size, nanos) in rows {
        let modified = if nanos == 0 {
            None
        } else {
            Some(UNIX_EPOCH + Duration::from_nanos(u64::try_from(nanos).ok()?))
        };
        out.insert(PathBuf::from(path), (size, modified));
    }
    Some(out)
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
