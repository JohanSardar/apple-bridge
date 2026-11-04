use super::{
    CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Signature, Value, capture_value,
    reader_error, signature,
};
use ab_native::{capture, mail};

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
