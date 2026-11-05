use super::{
    Arc, BTreeMap, CaptureReader, Debug, Duration, Error, Path, PathBuf, Readiness,
    ReadinessContext, ScanContext, Signature, SystemTime, Value, capture_value, confine_staging,
    decode, encode, json, native_facts, reader_error, signature,
};
use crate::media_facts::MediaFacts;
use ab_native::{capture, notes_reader};
use std::time::Instant;

const HASHTAG: &str = "com.apple.notes.inlinetextattachment.hashtag";
const FACT_RETRY: Duration = Duration::from_secs(60);

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SavedState {
    signature: String,
    generation: String,
    identities: Vec<(String, String)>,
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
    staging: PathBuf,
    facts: notes_reader::Facts,
    media: Option<MediaFacts>,
    failed_facts: BTreeMap<String, Instant>,
    scanned_last: bool,
    client: Option<Arc<native_facts::Client>>,
    home: Option<PathBuf>,
}

const OPERATIONS: [&str; 6] = [
    "notes.create",
    "notes.update",
    "notes.delete",
    "notes.create_folder",
    "notes.move",
    "notes.rename_folder",
];
const DELETION: [&str; 2] = ["notes.delete", "notes.delete_folder"];
const EXTRAS: [&str; 2] = ["notes.move", "notes.rename_folder"];
const MISSING: &str = "unknown protobuf fields and unavailable attachment bytes are explicit; rich HTML text edits require preserved structure; checklist edits remain unsupported; folder deletion requires per-build proof";
const PROOFS: &str = "notes-deletion-proofs.json";
const TRUST: &str = "accessibility.trusted";
const MAX_COMMAND_FACTS: u32 = 64;

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
            scanned_last: false,
            client: None,
            home: None,
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

impl NotesReader {
    #[must_use]
    pub fn with_status(
        mut self,
        client: Option<Arc<native_facts::Client>>,
        home: Option<PathBuf>,
    ) -> Self {
        self.client = client;
        self.home = home;
        self
    }

    fn with_facts<T>(
        &mut self,
        read: impl Fn(&notes_reader::Facts) -> capture::Result<T>,
    ) -> capture::Result<T> {
        let mut budget = MAX_COMMAND_FACTS;
        loop {
            match read(&self.facts) {
                Err(capture::Error::FactsRequired(fact)) if budget > 0 && self.resolve(&fact) => {
                    budget -= 1;
                }
                other => return other,
            }
        }
    }

    fn proven_deletion(&self, operation: &str, os_build: &str, generation: Option<&str>) -> bool {
        if operation == "notes.delete" {
            return true;
        }
        let Some(ledger) = self
            .home
            .as_ref()
            .and_then(|home| std::fs::read(home.join(PROOFS)).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            return false;
        };
        if ledger["version"] != 1
            || ledger["os_build"] != os_build
            || generation.is_none_or(|generation| ledger["generation"] != generation)
        {
            return false;
        }
        let proof = &ledger["operations"][operation];
        proof["before_active"] == true
            && proof["command_completed"] == true
            && proof["notestore_verified"] == true
            && ["gone", "marked", "trashed"]
                .contains(&proof["after_state"].as_str().unwrap_or_default())
    }
}

impl CaptureReader for NotesReader {
    fn source(&self) -> &'static str {
        "notes"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        [
            "NoteStore.sqlite",
            "NoteStore.sqlite-wal",
            "NoteStore.sqlite-shm",
        ]
        .into_iter()
        .map(|name| self.root.join(name))
        .collect()
    }

    fn token(&self) -> Option<String> {
        let signature = encode(self.committed.as_ref()?);
        serde_json::to_string(&SavedState {
            signature,
            generation: self.reader.generation().to_owned(),
            identities: self.reader.identities(),
        })
        .ok()
    }

    fn resume(&mut self, token: &str) -> bool {
        let Ok(saved) = serde_json::from_str::<SavedState>(token) else {
            self.committed = None;
            return decode(token).is_some();
        };
        self.committed = decode(&saved.signature);
        if !saved.generation.is_empty() {
            let _ = self
                .reader
                .resume_identities(&saved.generation, &saved.identities);
        }
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
        let current = signature(&self.root)?;
        let changed = self.committed.as_ref() != Some(&current);
        if changed && !self.scanned_last {
            let scan = self.reader.reconcile(&notes).map_err(reader_error)?;
            let value = capture_value(&scan.batch)?;
            self.pending = Some((NotesPending::Scan(Box::new(scan)), Some(current)));
            self.scanned_last = true;
            return Ok(Some(value));
        }
        self.scanned_last = false;
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
        if !changed {
            return Ok(None);
        }
        let scan = self.reader.reconcile(&notes).map_err(reader_error)?;
        let value = capture_value(&scan.batch)?;
        self.pending = Some((NotesPending::Scan(Box::new(scan)), Some(current)));
        self.scanned_last = true;
        Ok(Some(value))
    }

    fn refresh(&mut self) -> bool {
        true
    }

    fn answer(&mut self, operation: &str, args: &Value) -> Option<Result<Value, String>> {
        let database = self.root.join("NoteStore.sqlite");
        let key = args["native_key"].as_str().unwrap_or_default();
        let collection = args["collection"].as_str().unwrap_or("notes.note");
        let answer = match operation {
            "notes.read" => self
                .with_facts(|facts| notes_reader::read_record(&database, collection, key, facts))
                .map(|found| found.unwrap_or_else(|| json!({"exists": false}))),
            "notes.observe" => {
                let script = args["script_id"].as_str().unwrap_or_default();
                self.with_facts(|facts| notes_reader::read_script_id(&database, script, facts))
                    .map(|found| {
                        let mut seen = found.map_or_else(
                            || json!({"observed": false}),
                            |found| {
                                json!({
                                    "observed": true,
                                    "native_key": found["native_key"],
                                    "data": found["data"],
                                })
                            },
                        );
                        if !key.is_empty() {
                            seen["deletion_state"] = json!(
                                notes_reader::deletion_state(&database, key, "notes.note")
                                    .unwrap_or_default()
                            );
                        }
                        seen
                    })
            }
            "notes.trash_folder" => self
                .with_facts(|facts| notes_reader::trash_folder(&database, key, facts))
                .map(|alias| json!({"scoped_alias": alias})),
            _ => return None,
        };
        Some(answer.map_err(|error| {
            eprintln!("ab-session: {operation}: {error}");
            match error {
                capture::Error::Conflict => "conflict".to_owned(),
                _ => "unavailable".to_owned(),
            }
        }))
    }

    fn readiness(&mut self, context: &ReadinessContext<'_>) -> Option<Readiness> {
        let database = self.root.join("NoteStore.sqlite");
        let generation = notes_reader::store_generation(&database);
        let trusted = self.client.as_ref().is_some_and(|client| {
            client.fetch(context.fence, TRUST).is_ok_and(|set| {
                set.items
                    .first()
                    .is_some_and(|item| item["trusted"] == true)
            })
        });
        let extra = |operation: &str| operation != "notes.rename_folder" || trusted;
        let operations: Vec<Value> = DELETION
            .iter()
            .chain(EXTRAS.iter())
            .map(|operation| {
                let proven =
                    self.proven_deletion(operation, context.os_build, generation.as_deref());
                let is_extra = EXTRAS.contains(operation) && extra(operation);
                let reason = if *operation == "notes.delete" {
                    "public_script_notestore_verified"
                } else if proven {
                    "os_build_notestore_deletion_verified"
                } else if is_extra {
                    "public_script_move_rename"
                } else {
                    "capability_unproven"
                };
                json!({"operation": operation, "proven": proven || is_extra, "reason": reason})
            })
            .collect();
        let coverage = json!({"fields": "partial", "operations": operations, "missing": [MISSING]});
        let readable = generation.is_some();
        Some(Readiness {
            state: if readable {
                "ready"
            } else {
                "permission_missing"
            },
            coverage,
            capabilities: if readable {
                OPERATIONS
                    .iter()
                    .filter(|operation| !EXTRAS.contains(operation) || extra(operation))
                    .map(|operation| (*operation).to_owned())
                    .collect()
            } else {
                Vec::new()
            },
            reason: if readable {
                String::new()
            } else {
                "notes_database_access_required".to_owned()
            },
        })
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
