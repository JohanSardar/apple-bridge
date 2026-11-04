use super::{
    BTreeMap, CaptureReader, Debug, Duration, Error, Path, PathBuf, ScanContext, Signature,
    SystemTime, Value, capture_value, confine_staging, decode, encode, reader_error, signature,
};
use crate::media_facts::MediaFacts;
use ab_native::{capture, notes_reader};
use std::time::Instant;

const HASHTAG: &str = "com.apple.notes.inlinetextattachment.hashtag";
const FACT_RETRY: Duration = Duration::from_secs(60);

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
    staging: PathBuf,
    facts: notes_reader::Facts,
    media: Option<MediaFacts>,
    failed_facts: BTreeMap<String, Instant>,
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
            staging: staging.to_path_buf(),
            facts: notes_reader::Facts {
                mime: BTreeMap::from([(HASHTAG.to_owned(), None)]),
                ..notes_reader::Facts::default()
            },
            media: None,
            failed_facts: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn holding(mut self, hold: Option<PathBuf>) -> Self {
        self.hold = hold;
        self
    }

    #[must_use]
    pub fn with_media(mut self, media: Option<MediaFacts>) -> Self {
        self.media = media;
        self
    }

    fn resolve(&mut self, fact: &str) -> bool {
        if self
            .failed_facts
            .get(fact)
            .is_some_and(|at| at.elapsed() < FACT_RETRY)
        {
            return false;
        }
        let resolved = match &self.media {
            Some(media) => media.resolve(fact, &self.staging, &mut self.facts),
            None => Err(Error::Conflict),
        };
        match resolved {
            Ok(()) => {
                self.failed_facts.remove(fact);
                true
            }
            Err(error) => {
                eprintln!("ab-session: notes fact {fact}: {error}");
                self.failed_facts.insert(fact.to_owned(), Instant::now());
                false
            }
        }
    }

    fn held(&self) -> bool {
        self.hold.as_ref().is_some_and(|hold| hold.exists())
    }

    fn notes_context(&self, context: &ScanContext) -> Result<notes_reader::Context, Error> {
        Ok(notes_reader::Context {
            capture: capture::Context {
                observed_at: context.observed_at()?,
                checkpoint: context.checkpoint.clone(),
                scan_id: context.scan_id.clone(),
            },
            coverage: serde_json::json!({}),
            facts: self.facts.clone(),
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
        let mut notes = self.notes_context(context)?;
        let mut budget = self.reader.queued();
        while !self.held() && self.reader.has_hydration() {
            match self.reader.hydrate_next(&notes) {
                Ok(Some(hydration)) => {
                    let value = capture_value(&hydration.batch)?;
                    self.pending = Some((NotesPending::Hydration(Box::new(hydration)), None));
                    return Ok(Some(value));
                }
                Ok(None) => {}
                Err(capture::Error::FactsRequired(fact)) => {
                    if self.resolve(&fact) {
                        notes = self.notes_context(context)?;
                    } else if budget == 0 {
                        break;
                    } else {
                        budget -= 1;
                    }
                }
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

    fn refresh(&mut self) -> bool {
        true
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
