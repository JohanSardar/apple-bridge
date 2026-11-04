use super::{
    CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Signature, Value, capture_value,
    decode, encode, reader_error, signature,
};
use ab_native::usage;

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
