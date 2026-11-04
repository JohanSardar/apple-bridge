use crate::{Error, native_facts};

mod messages;
use ab_native::{
    agenda, capture, contacts, mail, notes_reader, safari, telephony, usage, voice_memo,
};
pub use messages::MessagesReader;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
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
    pub fence: u64,
}

impl ScanContext {
    fn observed_at(&self) -> Result<String, Error> {
        capture::timestamp(self.now).map_err(reader_error)
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
    tabs: Option<Vec<safari::Tab>>,
}

fn tab_facts(capture: &Value) -> Result<Option<Vec<safari::Tab>>, Error> {
    if capture["scope"] != json!(["safari.tab"]) {
        return Ok(None);
    }
    if capture["completion_evidence"].as_str() != Some("complete") {
        return Err(Error::Invalid);
    }
    let mut tabs = Vec::new();
    let mut seen = BTreeSet::new();
    for row in capture["observations"].as_array().ok_or(Error::Invalid)? {
        if row["collection"].as_str() != Some("safari.tab") {
            return Err(Error::Invalid);
        }
        match row["kind"].as_str() {
            Some("absent") => continue,
            Some("upsert") => {}
            _ => return Err(Error::Invalid),
        }
        let data = &row["data"];
        let tab = safari::Tab {
            window: data["window"].as_i64().ok_or(Error::Invalid)?,
            index: data["index"].as_i64().ok_or(Error::Invalid)?,
            title: data["title"].as_str().ok_or(Error::Invalid)?.to_owned(),
            url: data["url"].as_str().ok_or(Error::Invalid)?.to_owned(),
            active: data["active"].as_bool().ok_or(Error::Invalid)?,
        };
        if row["native_key"].as_str() != Some(&format!("{}:{}", tab.window, tab.index))
            || !seen.insert((tab.window, tab.index))
        {
            return Err(Error::Invalid);
        }
        tabs.push(tab);
    }
    Ok(Some(tabs))
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
            tabs: None,
        }
    }

    fn read_context(&self, context: &ScanContext) -> Result<safari::ReadContext, Error> {
        Ok(safari::ReadContext {
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
        })
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

    fn ingest(&mut self, capture: &Value) -> Result<bool, Error> {
        if let Some(tabs) = tab_facts(capture)? {
            self.tabs = Some(tabs);
        }
        Ok(false)
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        if let Some(tabs) = self.tabs.take() {
            let read = self.read_context(context)?;
            if let Some(pending) = self.reader.tabs(&tabs, &read).map_err(reader_error)? {
                let value = capture_value(&pending.batch)?;
                self.pending = Some(pending);
                return Ok(Some(value));
            }
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
            let read = self.read_context(context)?;
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
    preferred: Option<PathBuf>,
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
        let mut reader = Self::new("voicemail", root.join("voicemail.db"), telephony::voicemail);
        reader.preferred = Some(root.join("FaceTimeMessageStore-local.sqlitedb"));
        reader
    }

    fn new(name: &'static str, path: PathBuf, scan: TelephonyScan) -> Self {
        Self {
            name,
            path,
            preferred: None,
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
        if let Some(preferred) = &self.preferred
            && preferred.exists()
        {
            self.path = preferred.clone();
            self.preferred = None;
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
    pub fn new(root: &Path, staging: &Path) -> Result<Self, Error> {
        confine_staging(root, staging)?;
        Ok(Self {
            path: root.join("CloudRecordings.db"),
            staging: staging.to_path_buf(),
            reader: None,
            pending: None,
            committed: None,
            warm: false,
        })
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

    fn handle_prefix(&self) -> Option<&'static str> {
        Some("voice-memo-blob:")
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
    pub fn new(root: PathBuf, staging: &Path) -> Result<Self, Error> {
        confine_staging(&root, staging)?;
        let reader = notes_reader::Reader::open(&root.join("NoteStore.sqlite"), staging)
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

    fn handle_prefix(&self) -> Option<&'static str> {
        Some("notes-blob:")
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
    Reminders(agenda::Reader),
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
            Self::Reminders(_) => vec!["reminders.list".into(), "reminders.reminder".into()],
        }
    }

    fn knows(&self, identity: &str) -> bool {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.knows(identity),
            Self::Contacts(reader) => reader.knows(identity),
        }
    }

    fn generation(&self) -> &str {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.generation(),
            Self::Contacts(reader) => reader.generation(),
        }
    }

    fn prepare(
        &self,
        snapshot: agenda::Snapshot,
        context: &capture::Context,
    ) -> capture::Result<agenda::Pending> {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.prepare(snapshot, context),
            Self::Contacts(reader) => reader.prepare(snapshot, context),
        }
    }

    fn commit(&mut self, pending: agenda::Pending) -> capture::Result<()> {
        match self {
            Self::Calendar(reader) | Self::Reminders(reader) => reader.commit(pending),
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
    pull: Option<native_facts::Pull>,
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
            pull: None,
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
            pull: None,
        }
    }

    #[must_use]
    pub fn reminders(root: PathBuf) -> Self {
        Self {
            source: "reminders",
            root,
            state: AgendaState::Reminders(agenda::Reader::reminders()),
            facts: None,
            partial: false,
            pending: None,
            pull: None,
        }
    }

    #[must_use]
    pub fn paged(mut self, client: Arc<native_facts::Client>, interval: Duration) -> Self {
        let operations: &'static [&'static str] = match self.state {
            AgendaState::Calendar(_) => &["agenda.calendars", "agenda.events"],
            AgendaState::Contacts(_) => &[
                "contacts.containers",
                "contacts.groups",
                "contacts.contacts",
            ],
            AgendaState::Reminders(_) => &["agenda.reminder_lists", "agenda.reminders"],
        };
        self.pull = Some(native_facts::Pull::new(client, operations, interval));
        self
    }

    fn page(&mut self, context: &ScanContext) -> Result<(), Error> {
        let Some(pull) = self.pull.as_mut() else {
            return Ok(());
        };
        if !pull.due() {
            return Ok(());
        }
        let live = pull.live();
        let Some(collected) = pull.collect(context.fence)? else {
            return Ok(());
        };
        let capture = json!({"generation": collected.generation, "observations": collected.rows, "blob_handles": collected.handles, "live": live});
        self.facts = Some(self.parse(&capture)?);
        self.partial = false;
        Ok(())
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
        if let Some(pull) = self.pull.as_mut() {
            let partial = capture["scan_id"]
                .as_str()
                .is_some_and(|scan| !scan.is_empty())
                && capture["completion_evidence"].as_str() != Some("complete");
            if partial {
                return Ok(false);
            }
            pull.hint();
            return Ok(true);
        }
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
        self.page(context)?;
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
    #[serde(default)]
    staging: Option<PathBuf>,
    #[serde(default)]
    home: Option<PathBuf>,
    #[serde(default)]
    facts: Option<FactsLink>,
    #[serde(default = "default_facts_poll")]
    facts_poll_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FactsLink {
    Native(NativeMarker),
    Socket { socket: PathBuf },
}

#[derive(Debug, Deserialize)]
enum NativeMarker {
    #[serde(rename = "native")]
    Native,
}

fn default_facts_poll() -> u64 {
    15_000
}

fn lexical(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute() {
        return Err(Error::Invalid);
    }
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                if !out.pop() {
                    return Err(Error::Invalid);
                }
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

fn resolved(path: &Path) -> Result<PathBuf, Error> {
    let path = lexical(path)?;
    let mut tail = Vec::new();
    let mut head = path.as_path();
    loop {
        if let Ok(mut base) = head.canonicalize() {
            for part in tail.iter().rev() {
                base.push(part);
            }
            return Ok(base);
        }
        let name = head.file_name().ok_or(Error::Invalid)?;
        tail.push(name.to_owned());
        head = head.parent().ok_or(Error::Invalid)?;
    }
}

pub fn confine_staging(root: &Path, staging: &Path) -> Result<(), Error> {
    let root = resolved(root)?;
    let staging = resolved(staging)?;
    if staging.starts_with(&root) || root.starts_with(&staging) {
        return Err(Error::Invalid);
    }
    Ok(())
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
    from_config_linked(config, None)
}

#[must_use]
pub fn wants_inherited_facts(config: &str) -> bool {
    serde_json::from_str::<BTreeMap<String, SourceConfig>>(config).is_ok_and(|sources| {
        sources
            .values()
            .any(|source| matches!(source.facts, Some(FactsLink::Native(_))))
    })
}

fn linked_client(
    facts: Option<&FactsLink>,
    inherited: Option<&Arc<native_facts::Client>>,
) -> Result<Option<Arc<native_facts::Client>>, Error> {
    match facts {
        None => Ok(None),
        Some(FactsLink::Native(_)) => Ok(Some(Arc::clone(inherited.ok_or(Error::Invalid)?))),
        Some(FactsLink::Socket { socket }) => Ok(Some(Arc::new(native_facts::Client::socket(
            socket.clone(),
        )?))),
    }
}

pub fn from_config_linked(
    config: &str,
    inherited: Option<&Arc<native_facts::Client>>,
) -> Result<BTreeMap<String, Box<dyn CaptureReader>>, Error> {
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
            "voice_memos" => {
                let staging = config.staging.clone().ok_or(Error::Invalid)?;
                Box::new(VoiceMemosReader::new(&config.root, &staging)?)
            }
            "notes" => {
                let staging = config.staging.clone().ok_or(Error::Invalid)?;
                Box::new(NotesReader::new(config.root, &staging)?.holding(config.hydration_hold))
            }
            "messages" => {
                let home = match config.home.clone() {
                    Some(home) => home,
                    None if config.root.ends_with("Library/Messages") => config
                        .root
                        .parent()
                        .and_then(Path::parent)
                        .ok_or(Error::Invalid)?
                        .to_path_buf(),
                    None => return Err(Error::Invalid),
                };
                if !home.is_absolute() {
                    return Err(Error::Invalid);
                }
                Box::new(MessagesReader::new(config.root, home))
            }
            "calendar" | "contacts" | "reminders" => {
                let reader = match source.as_str() {
                    "calendar" => AgendaReader::calendar(config.root),
                    "contacts" => AgendaReader::contacts(config.root),
                    _ => AgendaReader::reminders(config.root),
                };
                match linked_client(config.facts.as_ref(), inherited)? {
                    Some(client) => Box::new(
                        reader.paged(client, Duration::from_millis(config.facts_poll_ms.max(1))),
                    ),
                    None => Box::new(reader),
                }
            }
            _ => return Err(Error::Invalid),
        };
        readers.insert(source, reader);
    }
    Ok(readers)
}
