use super::{
    Arc, BTreeMap, CaptureReader, Debug, Duration, Error, Path, PathBuf, ScanContext, Value,
    context_for, native_facts, reader_error,
};
use ab_native::capture;

const OPERATIONS: &[&str] = &["pdf.artifacts"];
const SOURCE: &str = "pdf";
const COLLECTION: &str = "pdf.artifact";
const GENERATION: &str = "native/1";

#[derive(Debug)]
pub struct PdfReader {
    root: PathBuf,
    pull: native_facts::Pull,
    state: capture::State,
    pending: Option<capture::Pending>,
}

impl PdfReader {
    #[must_use]
    pub fn new(root: PathBuf, client: Arc<native_facts::Client>, interval: Duration) -> Self {
        Self {
            root,
            pull: native_facts::Pull::new(client, SOURCE, OPERATIONS, interval),
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
        if !self.pull.due() {
            return Ok(None);
        }
        let Some(collected) = self.pull.collect(context.fence)? else {
            return Ok(None);
        };
        let read = context_for(context)?;
        let records = collected
            .rows
            .iter()
            .map(|row| {
                let key = row["native_key"].as_str().ok_or(Error::Invalid)?;
                capture::upsert(COLLECTION, key, None, row["data"].clone(), &read)
                    .map_err(reader_error)
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
            return Ok(None);
        }
        pending.batch.blob_handles = collected
            .handles
            .into_iter()
            .filter_map(|(digest, handle)| Some((digest, handle.as_str()?.to_owned())))
            .collect::<BTreeMap<_, _>>();
        let value = super::capture_value(&pending)?;
        self.pending = Some(pending);
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.state.commit(pending).map_err(reader_error)
    }
}
