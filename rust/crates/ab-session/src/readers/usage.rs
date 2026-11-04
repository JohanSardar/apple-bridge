use super::{
    CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Signature, Value, capture_value,
    decode, encode, file_signature, reader_error, signature,
};
use ab_native::usage;

#[derive(Debug)]
pub struct UsageReader {
    root: PathBuf,
    generation: String,
    zone: String,
    local_name: String,
    history: Option<PathBuf>,
    hardware_id: Option<String>,
    reader: usage::Reader,
    pending: Option<(usage::Scan, Signature)>,
    committed: Option<Signature>,
    warm: bool,
}

fn knowledge_database(root: &Path) -> Option<PathBuf> {
    if root.file_name()? != "Biome" {
        return None;
    }
    let database = root
        .parent()?
        .join("Application Support/Knowledge/knowledgeC.db");
    database.is_file().then_some(database)
}

fn hardware_id() -> String {
    std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| {
            let line = text
                .lines()
                .find(|line| line.contains("\"IOPlatformUUID\""))?;
            let (_, value) = line.split_once('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
        .unwrap_or_default()
}

impl UsageReader {
    #[must_use]
    pub fn new(root: PathBuf, generation: String, zone: String, local_name: String) -> Self {
        let history = knowledge_database(&root);
        Self {
            root,
            generation,
            zone,
            local_name,
            history,
            hardware_id: None,
            reader: usage::Reader::default(),
            pending: None,
            committed: None,
            warm: false,
        }
    }
}

impl CaptureReader for UsageReader {
    fn source(&self) -> &'static str {
        "usage"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn token(&self) -> Option<String> {
        self.committed.as_ref().map(encode)
    }

    fn resume(&mut self, token: &str) -> bool {
        self.committed = decode(token);
        self.committed.is_some()
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let mut current = signature(&self.root)?;
        if let Some(history) = &self.history {
            current.extend(file_signature(history));
        }
        if self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let history = match &self.history {
            Some(database) => {
                let id = self.hardware_id.get_or_insert_with(hardware_id).clone();
                Some(usage::HistoryContext {
                    database: database.to_str().ok_or(Error::Invalid)?.to_owned(),
                    hardware_id: id,
                    now: context.now,
                })
            }
            None => None,
        };
        let read = usage::Context {
            generation: self.generation.clone(),
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
            observed_at: context.observed_at()?,
            zone: self.zone.clone(),
            local_name: self.local_name.clone(),
            blocked: Vec::new(),
            power: Vec::new(),
            history,
        };
        let Ok(scan) = self.reader.scan(&self.root, &read, self.warm) else {
            return Ok(None);
        };
        let value = capture_value(&scan)?;
        self.pending = Some((scan, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (scan, current) = self.pending.take().ok_or(Error::Conflict)?;
        self.reader.commit(scan).map_err(reader_error)?;
        self.committed = Some(current);
        self.warm = true;
        Ok(())
    }
}
