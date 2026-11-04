use super::{
    CaptureReader, Debug, Error, MAX_ROWS, Path, PathBuf, ScanContext, Signature, Value,
    capture_value, context_for, decode, encode, file_signature, reader_error,
};
use ab_native::{capture, telephony};

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
