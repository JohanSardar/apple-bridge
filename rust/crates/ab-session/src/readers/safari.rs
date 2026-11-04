use super::{
    BTreeSet, CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Signature, Value,
    capture_value, decode, encode, json, reader_error, signature,
};
use ab_native::{capture, safari};

#[derive(Debug)]
pub struct SafariReader {
    root: PathBuf,
    generation: String,
    reader: safari::Reader,
    pending: Option<safari::Pending>,
    settled: Option<Signature>,
    tabs: Option<Vec<safari::Tab>>,
}

fn tab_facts(capture: &Value) -> Result<Option<Vec<safari::Tab>>, Error> {
    if capture["scope"] != json!(["safari.tab"]) {
        return Ok(None);
    }
    if capture["completion_evidence"].as_str() != Some("complete") {
        return Err(Error::Invalid);
    }
    let mut tabs = Vec::new();
    let mut seen = BTreeSet::new();
    for row in capture["observations"].as_array().ok_or(Error::Invalid)? {
        if row["collection"].as_str() != Some("safari.tab") {
            return Err(Error::Invalid);
        }
        match row["kind"].as_str() {
            Some("absent") => continue,
            Some("upsert") => {}
            _ => return Err(Error::Invalid),
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
    Ok(Some(tabs))
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
            tabs: None,
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

    fn ingest(&mut self, capture: &Value) -> Result<bool, Error> {
        if let Some(tabs) = tab_facts(capture)? {
            self.tabs = Some(tabs);
        }
        Ok(false)
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        if let Some(tabs) = self.tabs.take() {
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
