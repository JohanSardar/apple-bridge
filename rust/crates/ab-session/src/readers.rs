use crate::Error;
use ab_native::{
    agenda, capture, contacts, mail, notes_reader, safari, telephony, usage, voice_memo,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
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
    fn root(&self) -> &Path;
    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error>;
    fn commit(&mut self) -> Result<(), Error>;
    fn token(&self) -> Option<String>;
    fn resume(&mut self, token: &str) -> bool;
    fn owns(&self, _handle: &str) -> bool {
        false
    }
    fn blob(&mut self, _handle: &str, _offset: u64, _length: usize) -> Result<Value, Error> {
        Err(Error::Invalid)
    }
    fn ingest(&mut self, _capture: &Value) -> Result<bool, Error> {
        Ok(false)
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

#[derive(Debug)]
pub struct SafariReader {
    root: PathBuf,
    generation: String,
    reader: safari::Reader,
    pending: Option<safari::Pending>,
    settled: Option<Signature>,
}

impl SafariReader {
    #[must_use]
    pub fn new(root: PathBuf, generation: String) -> Self {
        Self {
            root,
            generation,
            reader: safari::Reader::default(),
            pending: None,
            settled: None,
        }
    }
}

impl CaptureReader for SafariReader {
    fn source(&self) -> &'static str {
        "safari"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        self.settled.as_ref().map(encode)
    }

    fn resume(&mut self, token: &str) -> bool {
        self.settled = decode(token);
        self.settled.is_some()
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let current = signature(&self.root)?;
        if self.settled.as_ref() == Some(&current) {
            return Ok(None);
        }
        let mut failed = false;
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
            let pending = match self.reader.poll(&self.root, kind, &read) {
                Ok(Some(pending)) => pending,
                Ok(None) => continue,
                Err(_) => {
                    failed = true;
                    continue;
                }
            };
            let value = capture_value(&pending.batch)?;
            self.settled = None;
            self.pending = Some(pending);
            return Ok(Some(value));
        }
        if !failed {
            self.settled = Some(current);
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

#[derive(Debug)]
pub struct UsageReader {
    root: PathBuf,
    generation: String,
    zone: String,
    local_name: String,
    reader: usage::Reader,
    pending: Option<(usage::Scan, Signature)>,
    committed: Option<Signature>,
    warm: bool,
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
            warm: false,
        }
    }
}

impl CaptureReader for UsageReader {
    fn source(&self) -> &'static str {
        "usage"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        self.committed.as_ref().map(encode)
    }

    fn resume(&mut self, token: &str) -> bool {
        self.committed = decode(token);
        self.committed.is_some()
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
        let Ok(scan) = self.reader.scan(&self.root, &read, self.warm) else {
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
        self.warm = true;
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
    warm: bool,
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
            warm: false,
        }
    }
}

impl CaptureReader for TelephonyReader {
    fn source(&self) -> &'static str {
        self.name
    }

    fn root(&self) -> &Path {
        self.path.parent().unwrap_or(&self.path)
    }

    fn token(&self) -> Option<String> {
        self.committed.as_ref().map(encode)
    }

    fn resume(&mut self, token: &str) -> bool {
        self.committed = decode(token);
        self.committed.is_some()
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
        let Ok(pending) = (self.scan)(&self.path, &self.state, &read, self.warm, MAX_ROWS) else {
            return Ok(None);
        };
        if file_signature(&self.path) != current {
            return Ok(None);
        }
        let value = capture_value(&pending)?;
        self.pending = Some((pending, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (pending, current) = self.pending.take().ok_or(Error::Conflict)?;
        self.state.commit(pending).map_err(reader_error)?;
        self.committed = Some(current);
        self.warm = true;
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
    warm: bool,
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
            warm: false,
        }
    }

    fn assets(&mut self) -> Result<&mut voice_memo::Reader, Error> {
        if self.reader.is_none() {
            self.reader = Some(voice_memo::Reader::open(&self.staging).map_err(reader_error)?);
        }
        self.reader.as_mut().ok_or(Error::Conflict)
    }
}

impl CaptureReader for VoiceMemosReader {
    fn source(&self) -> &'static str {
        "voice_memos"
    }

    fn root(&self) -> &Path {
        self.path.parent().unwrap_or(&self.path)
    }

    fn token(&self) -> Option<String> {
        self.committed.as_ref().map(encode)
    }

    fn resume(&mut self, token: &str) -> bool {
        self.committed = decode(token);
        self.committed.is_some()
    }

    fn owns(&self, handle: &str) -> bool {
        handle.starts_with("voice-memo-blob:")
    }

    fn blob(&mut self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        self.assets()?
            .read_blob(handle, offset, length)
            .map_err(reader_error)
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
        let live = self.warm;
        let path = self.path.clone();
        let Ok(pending) = self.assets()?.scan(&path, &read, live, MAX_ROWS) else {
            return Ok(None);
        };
        if file_signature(&self.path) != current {
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
        self.warm = true;
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
    hold: Option<PathBuf>,
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
            hold: None,
        })
    }

    #[must_use]
    pub fn holding(mut self, hold: Option<PathBuf>) -> Self {
        self.hold = hold;
        self
    }

    fn held(&self) -> bool {
        self.hold.as_ref().is_some_and(|hold| hold.exists())
    }

    fn notes_context(context: &ScanContext) -> Result<notes_reader::Context, Error> {
        Ok(notes_reader::Context {
            capture: capture::Context {
                observed_at: context.observed_at()?,
                checkpoint: context.checkpoint.clone(),
                scan_id: context.scan_id.clone(),
            },
            coverage: serde_json::json!({}),
            facts: notes_reader::Facts {
                mime: std::collections::BTreeMap::from([(
                    "com.apple.notes.inlinetextattachment.hashtag".to_owned(),
                    None,
                )]),
                ..notes_reader::Facts::default()
            },
            asset_paths: Vec::new(),
        })
    }
}

impl CaptureReader for NotesReader {
    fn source(&self) -> &'static str {
        "notes"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        self.committed.as_ref().map(encode)
    }

    fn resume(&mut self, token: &str) -> bool {
        self.committed = decode(token);
        self.committed.is_some()
    }

    fn owns(&self, handle: &str) -> bool {
        handle.starts_with("notes-blob:")
    }

    fn blob(&mut self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        self.reader
            .read_blob(handle, offset, length)
            .map_err(reader_error)
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let notes = Self::notes_context(context)?;
        while !self.held() && self.reader.has_hydration() {
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
        if !self.held() {
            self.reader
                .finish_hydration(SystemTime::now())
                .map_err(reader_error)?;
        }
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

const MAIL_DATABASE: &str = "Envelope Index";
const MAIL_MAX_ROWS: usize = 1_000_000;

#[derive(Debug)]
pub struct MailReader {
    root: PathBuf,
    reader: mail::Reader,
    pending: Option<(mail::Pending, Signature)>,
    committed: Option<Signature>,
}

impl MailReader {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            reader: mail::Reader::default(),
            pending: None,
            committed: None,
        }
    }
}

impl CaptureReader for MailReader {
    fn source(&self) -> &'static str {
        "mail"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        None
    }

    fn resume(&mut self, _token: &str) -> bool {
        false
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let database = self.root.join(MAIL_DATABASE);
        if !database.is_file() {
            return Ok(None);
        }
        let current = signature(&self.root)?;
        if self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let read = capture::Context {
            observed_at: context.observed_at()?,
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
        };
        let pending = match self.reader.poll(&database, &read, MAIL_MAX_ROWS) {
            Ok(Some(pending)) => pending,
            Ok(None) => {
                self.committed = Some(current);
                return Ok(None);
            }
            Err(_) => return Ok(None),
        };
        let value = capture_value(&pending)?;
        self.pending = Some((pending, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (pending, current) = self.pending.take().ok_or(Error::Conflict)?;
        self.reader.commit(pending).map_err(reader_error)?;
        self.committed = Some(current);
        Ok(())
    }
}

#[derive(Debug)]
enum AgendaState {
    Calendar(agenda::Reader),
    Contacts(contacts::Reader),
}

impl AgendaState {
    fn scope(&self) -> Vec<String> {
        match self {
            Self::Calendar(_) => vec!["calendar.calendar".into(), "calendar.event".into()],
            Self::Contacts(_) => vec![
                "contacts.contact".into(),
                "contacts.group".into(),
                "contacts.container".into(),
            ],
        }
    }

    fn knows(&self, identity: &str) -> bool {
        match self {
            Self::Calendar(reader) => reader.knows(identity),
            Self::Contacts(reader) => reader.knows(identity),
        }
    }

    fn generation(&self) -> &str {
        match self {
            Self::Calendar(reader) => reader.generation(),
            Self::Contacts(reader) => reader.generation(),
        }
    }

    fn prepare(
        &self,
        snapshot: agenda::Snapshot,
        context: &capture::Context,
    ) -> capture::Result<agenda::Pending> {
        match self {
            Self::Calendar(reader) => reader.prepare(snapshot, context),
            Self::Contacts(reader) => reader.prepare(snapshot, context),
        }
    }

    fn commit(&mut self, pending: agenda::Pending) -> capture::Result<()> {
        match self {
            Self::Calendar(reader) => reader.commit(pending),
            Self::Contacts(reader) => reader.commit(pending),
        }
    }
}

#[derive(Debug)]
struct Facts {
    generation: String,
    rows: Vec<Value>,
    absences: BTreeMap<String, agenda::Revalidation>,
    live: bool,
    blob_handles: Option<Value>,
}

#[derive(Debug)]
pub struct AgendaReader {
    source: &'static str,
    root: PathBuf,
    state: AgendaState,
    facts: Option<Facts>,
    partial: bool,
    pending: Option<agenda::Pending>,
}

fn identity(row: &Value) -> String {
    format!(
        "{}/{}",
        row["collection"].as_str().unwrap_or_default(),
        row["native_key"].as_str().unwrap_or_default()
    )
}

impl Facts {
    fn merge(mut self, later: Self) -> Self {
        let mut rows: BTreeMap<String, Value> = self
            .rows
            .drain(..)
            .map(|row| (identity(&row), row))
            .collect();
        for row in later.rows {
            let key = identity(&row);
            self.absences.remove(&key);
            rows.insert(key, row);
        }
        for (key, disposition) in later.absences {
            rows.remove(&key);
            self.absences.insert(key, disposition);
        }
        self.rows = rows.into_values().collect();
        self.live |= later.live;
        if later.blob_handles.is_some() {
            self.blob_handles = later.blob_handles;
        }
        self
    }
}

impl AgendaReader {
    #[must_use]
    pub fn calendar(root: PathBuf) -> Self {
        Self {
            source: "calendar",
            root,
            state: AgendaState::Calendar(agenda::Reader::default()),
            facts: None,
            partial: false,
            pending: None,
        }
    }

    #[must_use]
    pub fn contacts(root: PathBuf) -> Self {
        Self {
            source: "contacts",
            root,
            state: AgendaState::Contacts(contacts::Reader::default()),
            facts: None,
            partial: false,
            pending: None,
        }
    }

    fn parse(&self, capture: &Value) -> Result<Facts, Error> {
        let generation = capture["generation"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or(Error::Invalid)?
            .to_owned();
        let observations = capture["observations"].as_array().ok_or(Error::Invalid)?;
        let scope = self.state.scope();
        let mut rows = Vec::new();
        let mut absences = BTreeMap::new();
        for observation in observations {
            let collection = observation["collection"].as_str().ok_or(Error::Invalid)?;
            let key = observation["native_key"].as_str().ok_or(Error::Invalid)?;
            if !scope.iter().any(|value| value == collection) {
                return Err(Error::Invalid);
            }
            match observation["kind"].as_str() {
                Some("upsert") => rows.push(json!({
                    "kind": "upsert",
                    "collection": collection,
                    "native_key": key,
                    "scoped_alias": observation["scoped_alias"].as_str().unwrap_or_default(),
                    "data": observation["data"],
                })),
                Some(kind @ ("absent" | "evict")) => {
                    let disposition = if kind == "evict" && self.source == "calendar" {
                        agenda::Revalidation::Evict
                    } else {
                        agenda::Revalidation::Absent
                    };
                    absences.insert(format!("{collection}/{key}"), disposition);
                }
                _ => return Err(Error::Invalid),
            }
        }
        Ok(Facts {
            generation,
            rows,
            absences,
            live: capture["live"].as_bool().unwrap_or(false),
            blob_handles: capture
                .get("blob_handles")
                .filter(|value| value.as_object().is_some_and(|map| !map.is_empty()))
                .cloned(),
        })
    }
}

impl CaptureReader for AgendaReader {
    fn source(&self) -> &'static str {
        self.source
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        None
    }

    fn resume(&mut self, _token: &str) -> bool {
        false
    }

    fn ingest(&mut self, capture: &Value) -> Result<bool, Error> {
        let parsed = self.parse(capture)?;
        self.partial = capture["scan_id"]
            .as_str()
            .is_some_and(|scan| !scan.is_empty())
            && capture["completion_evidence"].as_str() != Some("complete");
        self.facts = Some(match self.facts.take() {
            Some(earlier) if earlier.generation == parsed.generation => earlier.merge(parsed),
            _ => parsed,
        });
        Ok(!self.partial)
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        if self.partial {
            return Ok(None);
        }
        let Some(facts) = self.facts.take() else {
            return Ok(None);
        };
        let present: BTreeSet<String> = facts.rows.iter().map(identity).collect();
        let revalidated = facts
            .absences
            .into_iter()
            .filter(|(identity, _)| self.state.knows(identity) && !present.contains(identity))
            .collect();
        let read = capture::Context {
            observed_at: context.observed_at()?,
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
        };
        let snapshot = agenda::Snapshot {
            generation: facts.generation.clone(),
            scope: self.state.scope(),
            complete: true,
            records: facts.rows,
            revalidated,
            force: false,
        };
        let pending = self.state.prepare(snapshot, &read).map_err(reader_error)?;
        if pending.observations.is_empty() && self.state.generation() == facts.generation {
            return Ok(None);
        }
        let mut value = serde_json::to_value(&pending)?;
        let object = value.as_object_mut().ok_or(Error::Invalid)?;
        if facts.live {
            object.insert("live".into(), Value::Bool(true));
        }
        if let Some(handles) = facts.blob_handles {
            object.insert("blob_handles".into(), handles);
        }
        self.pending = Some(pending);
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.state.commit(pending).map_err(reader_error)
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
    #[serde(default)]
    hydration_hold: Option<PathBuf>,
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
            "mail" => Box::new(MailReader::new(config.root)),
            "usage" => Box::new(UsageReader::new(
                config.root,
                config.generation,
                config.zone,
                config.local_name,
            )),
            "phone" => Box::new(TelephonyReader::calls(&config.root)),
            "voicemail" => Box::new(TelephonyReader::voicemail(&config.root)),
            "voice_memos" => Box::new(VoiceMemosReader::new(&config.root)),
            "notes" => Box::new(NotesReader::new(config.root)?.holding(config.hydration_hold)),
            "calendar" => Box::new(AgendaReader::calendar(config.root)),
            "contacts" => Box::new(AgendaReader::contacts(config.root)),
            _ => return Err(Error::Invalid),
        };
        readers.insert(source, reader);
    }
    Ok(readers)
}
