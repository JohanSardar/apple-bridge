use super::{
    Arc, CaptureReader, Debug, Duration, Error, Path, PathBuf, ScanContext, Value, capture_value,
    context_for, native_facts, reader_error,
};
use ab_native::{capture, mac};

const OPERATIONS: &[&str] = &["mac.state"];

#[derive(Debug)]
pub struct MacReader {
    root: PathBuf,
    pull: native_facts::Pull,
    state: capture::State,
    pending: Option<capture::Pending>,
    warm: bool,
}

impl MacReader {
    #[must_use]
    pub fn new(root: PathBuf, client: Arc<native_facts::Client>, interval: Duration) -> Self {
        Self {
            root,
            pull: native_facts::Pull::new(client, mac::SOURCE, OPERATIONS, interval),
            state: capture::State::default(),
            pending: None,
            warm: false,
        }
    }
}

impl CaptureReader for MacReader {
    fn source(&self) -> &'static str {
        mac::SOURCE
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        serde_json::to_string(&self.state).ok()
    }

    fn resume(&mut self, token: &str) -> bool {
        match serde_json::from_str::<capture::State>(token) {
            Ok(state) => {
                self.state = state;
                true
            }
            Err(_) => false,
        }
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
        if collected.generation.is_empty() {
            return Ok(None);
        }
        let read = context_for(context)?;
        let pending =
            mac::prepare(&self.state, &collected.rows, &read, self.warm).map_err(reader_error)?;
        if pending.batch.observations.is_empty() {
            return Ok(None);
        }
        let value = capture_value(&pending)?;
        self.pending = Some(pending);
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.state.commit(pending).map_err(reader_error)?;
        self.warm = true;
        Ok(())
    }

    fn ingest(&mut self, capture: &Value) -> Result<bool, Error> {
        if capture["source"] == mac::SOURCE {
            self.pull.hint();
            return Ok(true);
        }
        Ok(false)
    }
}
