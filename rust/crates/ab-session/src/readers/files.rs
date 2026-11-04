use super::{
    Arc, BTreeMap, CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Value, capture_value,
    context_for, native_facts, reader_error,
};
use ab_native::files;
use serde_json::json;

const UBIQUITY: &str = "files.ubiquity";
const ROOTS_FILE: &str = "Library/Application Support/AppleBridge/files-roots.json";

#[derive(Debug)]
pub struct FilesReader {
    roots: Vec<PathBuf>,
    client: Arc<native_facts::Client>,
    reader: files::Reader,
    pending: Option<files::Pending>,
}

fn configured(home: &Path) -> Result<Vec<PathBuf>, Error> {
    let bytes = match std::fs::read(home.join(ROOTS_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let names: Vec<String> = serde_json::from_slice(&bytes)?;
    if names.iter().any(|name| !name.starts_with('/')) {
        return Err(Error::Invalid);
    }
    Ok(names.into_iter().map(PathBuf::from).collect())
}

impl FilesReader {
    pub fn new(
        root: PathBuf,
        extra: Vec<PathBuf>,
        home: Option<&Path>,
        client: Arc<native_facts::Client>,
    ) -> Result<Self, Error> {
        let mut roots: Vec<PathBuf> = Vec::new();
        let listed = match home {
            Some(home) => configured(home)?,
            None => Vec::new(),
        };
        for candidate in std::iter::once(root).chain(extra).chain(listed) {
            let candidate = candidate.canonicalize().unwrap_or(candidate);
            if !roots.contains(&candidate) {
                roots.push(candidate);
            }
        }
        if roots.len() > files::MAX_ROOTS {
            return Err(Error::Invalid);
        }
        Ok(Self {
            roots,
            client,
            reader: files::Reader::default(),
            pending: None,
        })
    }

    fn statuses(&self, fence: u64, walk: &files::Walk) -> Result<BTreeMap<String, String>, Error> {
        let mut statuses = BTreeMap::new();
        let keys: Vec<&str> = walk
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        for batch in keys.chunks(native_facts::MAX_REVALIDATE) {
            let set = self
                .client
                .fetch_with(fence, UBIQUITY, &json!({"paths":batch}))?;
            if !set.complete || set.items.len() != batch.len() {
                return Err(Error::Invalid);
            }
            for (asked, item) in batch.iter().zip(&set.items) {
                let status = item["status"].as_str().filter(|text| !text.is_empty());
                if item["path"] != json!(asked) || status.is_none() {
                    return Err(Error::Invalid);
                }
                if let Some(status) = status {
                    statuses.insert((*asked).to_owned(), status.to_owned());
                }
            }
        }
        Ok(statuses)
    }
}

impl CaptureReader for FilesReader {
    fn source(&self) -> &'static str {
        files::SOURCE
    }

    fn root(&self) -> &Path {
        self.roots.first().map_or(Path::new("/"), PathBuf::as_path)
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    fn token(&self) -> Option<String> {
        serde_json::to_string(&self.reader).ok()
    }

    fn resume(&mut self, token: &str) -> bool {
        match serde_json::from_str::<files::Reader>(token) {
            Ok(reader) => {
                self.reader = reader;
                true
            }
            Err(_) => false,
        }
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let read = context_for(context)?;
        for root in &self.roots {
            let Ok(walk) = files::walk(root, &|path, name| {
                ab_sys::xattr::get(path, name).ok().flatten()
            }) else {
                continue;
            };
            let statuses = self.statuses(context.fence, &walk)?;
            let pending = self
                .reader
                .prepare(&walk, &statuses, &read)
                .map_err(reader_error)?;
            if pending.batch.batch.observations.is_empty() {
                continue;
            }
            let value = capture_value(&pending.batch)?;
            self.pending = Some(pending);
            return Ok(Some(value));
        }
        Ok(None)
    }

    fn commit(&mut self) -> Result<(), Error> {
        let pending = self.pending.take().ok_or(Error::Conflict)?;
        self.reader.commit(pending).map_err(reader_error)
    }
}
