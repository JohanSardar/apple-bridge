use super::{
    CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Signature, SystemTime, Value,
    capture_value, confine_staging, decode, encode, reader_error, signature,
};
use ab_native::{capture, notes_reader};

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
