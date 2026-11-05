use super::{
    Arc, CaptureReader, Debug, Duration, Error, Path, PathBuf, ScanContext, Signature, Value,
    capture_value, context_for, file_signature, native_facts, reader_error,
};
use ab_native::{capture, focus};
use std::time::Instant;

pub const OPERATION: &str = "focus.status";
const ASSERTIONS: &str = "Library/DoNotDisturb/DB/Assertions.json";

#[derive(Debug)]
pub struct FocusReader {
    root: PathBuf,
    assertions: PathBuf,
    client: Arc<native_facts::Client>,
    interval: Duration,
    last: Option<Instant>,
    seen: Option<Signature>,
    state: capture::State,
    pending: Option<capture::Pending>,
    warm: bool,
}

impl FocusReader {
    #[must_use]
    pub fn new(
        root: PathBuf,
        home: &Path,
        client: Arc<native_facts::Client>,
        interval: Duration,
    ) -> Self {
        Self {
            root,
            assertions: home.join(ASSERTIONS),
            client,
            interval,
            last: None,
            seen: None,
            state: capture::State::default(),
            pending: None,
            warm: false,
        }
    }

    fn due(&mut self) -> bool {
        let signature = file_signature(&self.assertions);
        let changed = self.seen.as_ref() != Some(&signature);
        self.seen = Some(signature);
        changed || self.last.is_none_or(|at| at.elapsed() >= self.interval)
    }
}

impl CaptureReader for FocusReader {
    fn source(&self) -> &'static str {
        focus::SOURCE
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        self.assertions
            .parent()
            .map_or_else(Vec::new, |directory| vec![directory.to_path_buf()])
    }

    fn token(&self) -> Option<String> {
        serde_json::to_string(&self.state).ok()
    }

    fn resume(&mut self, token: &str) -> bool {
        match serde_json::from_str::<capture::State>(token) {
            Ok(state) => {
                self.state = state;
                self.warm = true;
                true
            }
            Err(_) => false,
        }
    }

    fn answer(&mut self, operation: &str, _args: &Value) -> Option<Result<Value, String>> {
        (operation == "focus.assertions_path")
            .then(|| Ok(serde_json::json!({"path": self.assertions.to_string_lossy()})))
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        if !self.due() {
            return Ok(None);
        }
        self.last = Some(Instant::now());
        let set = self.client.fetch(context.fence, OPERATION)?;
        let [fact] = set.items.as_slice() else {
            return Err(Error::Invalid);
        };
        let assertions = std::fs::read(&self.assertions).ok();
        let data = focus::status(fact, assertions.as_deref());
        let read = context_for(context)?;
        let pending = focus::prepare(&self.state, data, &read, self.warm).map_err(reader_error)?;
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
}
