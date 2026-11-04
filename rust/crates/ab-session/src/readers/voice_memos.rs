use super::{
    CaptureReader, Debug, Error, MAX_ROWS, Path, PathBuf, Readiness, ReadinessContext, ScanContext,
    Signature, Value, capture_value, confine_staging, context_for, decode, encode, file_signature,
    json, reader_error,
};
use ab_native::{capture, voice_memo};

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

    fn readiness(&mut self, _context: &ReadinessContext<'_>) -> Option<Readiness> {
        let unknown = json!({"fields": "unknown"});
        Some(if !self.path.exists() {
            Readiness::not("unavailable", unknown, "store not present")
        } else if voice_memo::readable(&self.path) {
            Readiness::ready(
                json!({"fields": "metadata_and_local_assets", "transcript": "embedded_if_present", "audio": "local_if_present"}),
                &[],
            )
        } else {
            Readiness::not("unavailable", unknown, "store_access_or_schema_unavailable")
        })
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
