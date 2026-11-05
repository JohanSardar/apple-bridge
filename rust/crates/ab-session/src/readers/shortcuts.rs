use super::{
    CaptureReader, Debug, Duration, Error, Path, PathBuf, Readiness, ReadinessContext, ScanContext,
    Value, background::Background, capture_value, context_for, json, reader_error,
};
use ab_native::{capture, shortcuts};
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::Instant,
};

pub const EXECUTABLE: &str = "/usr/bin/shortcuts";
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_LISTING: u64 = 16 << 20;

fn listing(executable: &Path, extra: &[&str]) -> Result<Vec<shortcuts::Entry>, Error> {
    let mut child = Command::new(executable)
        .args(["list", "--show-identifiers"])
        .args(extra)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let output = child.stdout.take().ok_or(Error::Invalid)?;
    let reading = thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(MAX_LISTING + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + LIST_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Reader("shortcuts listing timed out".into()));
        }
        thread::sleep(Duration::from_millis(25));
    };
    let bytes = reading.join().map_err(|_| Error::WorkerPanicked)??;
    if !status.success() || bytes.len() as u64 > MAX_LISTING {
        return Err(Error::Reader("shortcuts listing failed".into()));
    }
    let text = String::from_utf8(bytes).map_err(|_| Error::Invalid)?;
    shortcuts::parse_listing(&text).map_err(reader_error)
}

fn inventory(executable: &Path) -> Result<shortcuts::Inventory, Error> {
    let mut inventory = shortcuts::Inventory {
        shortcuts: listing(executable, &[])?,
        folders: listing(executable, &["--folders"])?,
        ..shortcuts::Inventory::default()
    };
    for folder in inventory.folders.clone() {
        let members = listing(executable, &["--folder-name", &folder.name])?;
        inventory
            .add_members(&folder, members)
            .map_err(reader_error)?;
    }
    Ok(inventory)
}

#[derive(Debug)]
pub struct ShortcutsReader {
    root: PathBuf,
    executable: PathBuf,
    work: Background<shortcuts::Inventory>,
    pack: Option<&'static str>,
    failed: bool,
    state: capture::State,
    pending: Option<capture::Pending>,
    warm: bool,
}

impl ShortcutsReader {
    #[must_use]
    pub fn new(root: PathBuf, executable: PathBuf, interval: Duration) -> Self {
        Self {
            root,
            executable,
            work: Background::new(interval),
            pack: None,
            failed: false,
            state: capture::State::default(),
            pending: None,
            warm: false,
        }
    }
}

impl CaptureReader for ShortcutsReader {
    fn source(&self) -> &'static str {
        shortcuts::SOURCE
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        Vec::new()
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

    fn readiness(&mut self, _context: &ReadinessContext<'_>) -> Option<Readiness> {
        let missing = if self.failed {
            json!([
                "installed inventory unavailable",
                "installed pack version unverified"
            ])
        } else if self.pack.is_some() {
            json!([
                "accepts input unavailable from CLI",
                "file blob input/output not qualified"
            ])
        } else {
            json!([
                "accepts input unavailable from CLI",
                "installed pack version unverified",
                "file blob input/output not qualified"
            ])
        };
        let mut coverage = json!({"fields": "partial", "missing": missing});
        if let Some(pack) = self.pack {
            coverage["pack_version"] = json!(pack);
        }
        Some(Readiness {
            state: "ready",
            coverage,
            capabilities: vec!["shortcuts.run".into()],
            reason: String::new(),
        })
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let answer = self.work.take();
        let executable = self.executable.clone();
        self.work.start_if_due(move || inventory(&executable));
        let found = match answer {
            None => return Ok(None),
            Some(Err(error)) => {
                self.failed = true;
                return Err(error);
            }
            Some(Ok(found)) => found,
        };
        self.failed = false;
        self.pack = found.pack_version();
        let read = context_for(context)?;
        let pending =
            shortcuts::prepare(&self.state, &found, &read, self.warm).map_err(reader_error)?;
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
