use super::{
    Arc, BTreeSet, CaptureReader, Debug, Duration, Error, Path, PathBuf, ScanContext, Signature,
    Value, background::Background, capture_value, decode, encode, native_facts, reader_error,
    signature,
};
use ab_native::{capture, safari};

#[derive(Debug)]
pub struct SafariReader {
    root: PathBuf,
    generation: String,
    reader: safari::Reader,
    pending: Option<safari::Pending>,
    settled: Option<Signature>,
    live_tabs: Option<LiveTabs>,
}

const TABS: &str = "safari.tabs";
const TAB_PATIENCE: Duration = Duration::from_millis(500);

#[derive(Debug)]
struct LiveTabs {
    client: Arc<native_facts::Client>,
    work: Background<native_facts::FactSet>,
}

fn tab_rows(rows: &[Value]) -> Result<Vec<safari::Tab>, Error> {
    let mut tabs = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        if row["collection"].as_str() != Some("safari.tab") {
            return Err(Error::Invalid);
        }
        let data = &row["data"];
        let tab = safari::Tab {
            window: data["window"].as_i64().ok_or(Error::Invalid)?,
            index: data["index"].as_i64().ok_or(Error::Invalid)?,
            title: data["title"].as_str().ok_or(Error::Invalid)?.to_owned(),
            url: data["url"].as_str().ok_or(Error::Invalid)?.to_owned(),
            active: data["active"].as_bool().ok_or(Error::Invalid)?,
        };
        if row["native_key"].as_str() != Some(&format!("{}:{}", tab.window, tab.index))
            || !seen.insert((tab.window, tab.index))
        {
            return Err(Error::Invalid);
        }
        tabs.push(tab);
    }
    Ok(tabs)
}

impl SafariReader {
    #[must_use]
    pub fn new(root: PathBuf, generation: String) -> Self {
        Self {
            root,
            generation,
            reader: safari::Reader::default(),
            pending: None,
            settled: None,
            live_tabs: None,
        }
    }

    #[must_use]
    pub fn with_tabs(mut self, client: Option<Arc<native_facts::Client>>, every: Duration) -> Self {
        self.live_tabs = client.map(|client| LiveTabs {
            client,
            work: Background::new(every),
        });
        self
    }

    fn tabs_answer(&mut self, fence: u64) -> Option<Vec<safari::Tab>> {
        let live = self.live_tabs.as_mut()?;
        let client = Arc::clone(&live.client);
        live.work.start_if_due(move || client.fetch(fence, TABS));
        live.work.wait(TAB_PATIENCE);
        match live.work.take()? {
            Ok(set) if set.complete => tab_rows(&set.items).ok(),
            Ok(_) | Err(_) => None,
        }
    }

    fn read_context(&self, context: &ScanContext) -> Result<safari::ReadContext, Error> {
        Ok(safari::ReadContext {
            capture: capture::Context {
                observed_at: context.observed_at()?,
                checkpoint: context.checkpoint.clone(),
                scan_id: context.scan_id.clone(),
            },
            generation: self.generation.clone(),
            now: context.now,
            days: 90.0,
            max_rows: 1_000_000,
            automation_permitted: true,
        })
    }
}

impl CaptureReader for SafariReader {
    fn source(&self) -> &'static str {
        "safari"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        [
            "History.db",
            "History.db-wal",
            "CloudTabs.db",
            "CloudTabs.db-wal",
            "Bookmarks.plist",
        ]
        .into_iter()
        .map(|name| self.root.join(name))
        .collect()
    }

    fn token(&self) -> Option<String> {
        self.settled.as_ref().map(encode)
    }

    fn adopts_generation(&self) -> bool {
        false
    }

    fn resume(&mut self, token: &str) -> bool {
        self.settled = decode(token);
        self.settled.is_some()
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        if let Some(tabs) = self.tabs_answer(context.fence) {
            let read = self.read_context(context)?;
            if let Some(pending) = self.reader.tabs(&tabs, &read).map_err(reader_error)? {
                let value = capture_value(&pending.batch)?;
                self.pending = Some(pending);
                return Ok(Some(value));
            }
        }
        let current = signature(&self.root)?;
        if self.settled.as_ref() == Some(&current) {
            return Ok(None);
        }
        let mut failed = false;
        for kind in [
            safari::Kind::History,
            safari::Kind::Bookmarks,
            safari::Kind::Cloud,
        ] {
            let read = self.read_context(context)?;
            let pending = match self.reader.poll(&self.root, kind, &read) {
                Ok(Some(pending)) => pending,
                Ok(None) => continue,
                Err(_) => {
                    failed = true;
                    continue;
                }
            };
            let value = capture_value(&pending.batch)?;
            self.settled = None;
            self.pending = Some(pending);
            return Ok(Some(value));
        }
        if !failed {
            self.settled = Some(current);
        }
        Ok(None)
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.reader.commit(pending).map_err(reader_error)
    }
}
