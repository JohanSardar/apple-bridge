use super::{
    Arc, CaptureReader, Debug, Duration, Error, Path, PathBuf, Readiness, ReadinessContext,
    ScanContext, Value, capture_value, context_for, json, native_facts, reader_error,
};
use ab_native::{capture, mac};

const OPERATIONS: &[&str] = &["mac.state"];
const PASTEBOARD: &str = "mac.pasteboard";

#[derive(Debug)]
pub struct MacReader {
    root: PathBuf,
    client: Arc<native_facts::Client>,
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
            pull: native_facts::Pull::new(Arc::clone(&client), mac::SOURCE, OPERATIONS, interval),
            client,
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

    fn readiness(&mut self, context: &ReadinessContext<'_>) -> Option<Readiness> {
        let pasteboard = self.client.fetch(context.fence, PASTEBOARD).ok();
        let allowed = pasteboard
            .as_ref()
            .and_then(|set| set.items.first())
            .is_some_and(|item| item["always_allow"] == true);
        let mut missing = vec![
            "Night Shift has no supported public API",
            "SSID requires Location authorization",
        ];
        if !allowed {
            missing.push("Clipboard requires Always Allow in Paste from Other Apps");
        }
        Some(Readiness::ready(
            json!({"fields": "partial", "missing": missing}),
            &crate::session::MAC_CAPABILITIES,
        ))
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
}
