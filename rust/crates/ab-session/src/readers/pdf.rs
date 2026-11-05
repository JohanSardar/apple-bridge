use super::{
    BTreeMap, CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Value, context_for,
    reader_error,
};
use ab_native::capture;

const ADD: &str = "pdf.add_artifact";
const SOURCE: &str = "pdf";
const COLLECTION: &str = "pdf.artifact";
const GENERATION: &str = "native/1";

#[derive(Debug)]
pub struct PdfReader {
    root: PathBuf,
    artifacts: BTreeMap<String, Value>,
    handles: BTreeMap<String, String>,
    changed: bool,
    state: capture::State,
    pending: Option<capture::Pending>,
}

impl PdfReader {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            artifacts: BTreeMap::new(),
            handles: BTreeMap::new(),
            changed: false,
            state: capture::State::default(),
            pending: None,
        }
    }
}

impl CaptureReader for PdfReader {
    fn source(&self) -> &'static str {
        SOURCE
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        Vec::new()
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
        if !self.changed {
            return Ok(None);
        }
        let read = context_for(context)?;
        let records = self
            .artifacts
            .iter()
            .map(|(key, data)| {
                capture::upsert(COLLECTION, key, None, data.clone(), &read).map_err(reader_error)
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut pending = self
            .state
            .prepare_incomplete(capture::ScanInput {
                source: SOURCE,
                generation: GENERATION,
                records,
                scope: vec![COLLECTION.into()],
                coverage: Value::Null,
                context: &read,
                live: true,
            })
            .map_err(reader_error)?;
        if pending.batch.observations.is_empty() {
            self.changed = false;
            return Ok(None);
        }
        pending.batch.blob_handles = self.handles.clone();
        let value = super::capture_value(&pending)?;
        self.pending = Some(pending);
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.changed = false;
        self.state.commit(pending).map_err(reader_error)
    }

    fn answer(&mut self, operation: &str, args: &Value) -> Option<Result<Value, String>> {
        (operation == ADD).then(|| {
            let key = args["native_key"].as_str().filter(|key| !key.is_empty());
            let digest = args["digest"].as_str().filter(|digest| !digest.is_empty());
            let handle = args["handle"].as_str().filter(|handle| !handle.is_empty());
            let (Some(key), Some(digest), Some(handle)) = (key, digest, handle) else {
                return Err("invalid".to_owned());
            };
            self.artifacts.insert(key.to_owned(), args["data"].clone());
            self.handles.insert(digest.to_owned(), handle.to_owned());
            self.changed = true;
            Ok(Value::Null)
        })
    }
}
