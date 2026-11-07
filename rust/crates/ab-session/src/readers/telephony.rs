use super::{
    CaptureReader, Debug, Error, MAX_ROWS, Path, PathBuf, Readiness, ReadinessContext, ScanContext,
    Signature, Value, capture_value, confine_staging, context_for, decode, encode, file_signature,
    json, reader_error,
};
use ab_native::{capture, telephony};

#[derive(Debug)]
pub struct TelephonyReader {
    name: &'static str,
    path: PathBuf,
    preferred: Option<PathBuf>,
    state: capture::State,
    staging: Option<PathBuf>,
    assets: Option<telephony::VoicemailReader>,
    pending: Option<(capture::Pending, Signature)>,
    committed: Option<Signature>,
    failed: Option<Signature>,
    warm: bool,
}

impl TelephonyReader {
    #[must_use]
    pub fn calls(root: &Path) -> Self {
        Self::new("phone", root.join("CallHistory.storedata"))
    }

    pub fn voicemail(root: &Path, staging: &Path) -> Result<Self, Error> {
        confine_staging(root, staging)?;
        let mut reader = Self::new("voicemail", root.join("voicemail.db"));
        reader.preferred = Some(root.join("FaceTimeMessageStore-local.sqlitedb"));
        reader.staging = Some(staging.to_owned());
        Ok(reader)
    }

    fn assets(&mut self) -> Result<&mut telephony::VoicemailReader, Error> {
        if self.assets.is_none() {
            let staging = self.staging.as_ref().ok_or(Error::Invalid)?;
            self.assets = Some(telephony::VoicemailReader::open(staging).map_err(reader_error)?);
        }
        self.assets.as_mut().ok_or(Error::Conflict)
    }

    fn signature(&self) -> Signature {
        let mut signature = file_signature(&self.path);
        if let Some(reader) = &self.assets {
            for path in reader.tracked_audio() {
                signature.extend(file_signature(path));
            }
        }
        signature
    }

    fn new(name: &'static str, path: PathBuf) -> Self {
        Self {
            name,
            path,
            preferred: None,
            state: capture::State::default(),
            staging: None,
            assets: None,
            pending: None,
            committed: None,
            failed: None,
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
        if self.name == "voicemail" {
            return None;
        }
        self.committed.as_ref().map(encode)
    }

    fn resume(&mut self, token: &str) -> bool {
        if self.name == "voicemail" {
            self.committed = None;
            return false;
        }
        self.committed = decode(token);
        self.committed.is_some()
    }

    fn owns(&self, handle: &str) -> bool {
        self.name == "voicemail" && handle.starts_with("voicemail-blob:")
    }

    fn handle_prefix(&self) -> Option<&'static str> {
        (self.name == "voicemail").then_some("voicemail-blob:")
    }

    fn blob(&mut self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        self.assets()?
            .read_blob(handle, offset, length)
            .map_err(reader_error)
    }

    fn readiness(&mut self, _context: &ReadinessContext<'_>) -> Option<Readiness> {
        let unknown = json!({"fields": "unknown"});
        let store = self
            .preferred
            .as_ref()
            .filter(|path| path.exists())
            .unwrap_or(&self.path);
        if self.failed.is_some() {
            return Some(Readiness::not("degraded", unknown, "capture_failed"));
        }
        Some(if self.name == "phone" {
            if telephony::calls_readable(store) {
                Readiness::ready(json!({"fields": "complete"}), &[])
            } else {
                Readiness::not(
                    "unavailable",
                    unknown,
                    "callhistory_access_or_schema_unavailable",
                )
            }
        } else if !store.exists() {
            Readiness::not("unavailable", unknown, "store not present")
        } else if telephony::voicemail_readable(store) {
            Readiness::ready(telephony::voicemail_coverage(), &[])
        } else {
            Readiness::not("unavailable", unknown, "store_access_or_schema_unavailable")
        })
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
        let store = file_signature(&self.path);
        let current = self.signature();
        if store.is_empty() || self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let read = context_for(context)?;
        if self.failed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let scanned = if self.name == "voicemail" {
            let path = self.path.clone();
            let warm = self.warm;
            self.assets()?.scan(&path, &read, warm, MAX_ROWS)
        } else {
            telephony::calls(&self.path, &self.state, &read, self.warm, MAX_ROWS)
        };
        let pending = match scanned {
            Ok(pending) => pending,
            Err(error) if error.is_busy() => return Ok(None),
            Err(error) => {
                eprintln!("ab-session: {} scan: {error}", self.name);
                self.failed = Some(current);
                return Ok(None);
            }
        };
        self.failed = None;
        if file_signature(&self.path) != store {
            return Ok(None);
        }
        let current = self.signature();
        let value = capture_value(&pending)?;
        self.pending = Some((pending, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (pending, current) = self.pending.take().ok_or(Error::Conflict)?;
        if self.name == "voicemail" {
            self.assets()?.commit(pending).map_err(reader_error)?;
        } else {
            self.state.commit(pending).map_err(reader_error)?;
        }
        self.committed = Some(current);
        self.warm = true;
        Ok(())
    }
}
