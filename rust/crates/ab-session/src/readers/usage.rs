use super::{
    CaptureReader, Debug, Error, Path, PathBuf, ScanContext, Signature, Value, capture_value,
    decode, encode, file_signature, reader_error, signature,
};
use ab_native::usage;

const PEERS: &str = "sync";
const STREAM: &str = "streams/restricted/App.InFocus";

fn consumed(root: &Path) -> Result<Signature, Error> {
    let mut out = file_signature(&root.join(PEERS).join("sync.db"));
    let stream = root.join(STREAM);
    if stream.is_dir() {
        out.extend(signature(&stream)?);
    }
    Ok(out)
}

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

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.root.join(PEERS), self.root.join(STREAM)]
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
        let mut current = consumed(&self.root)?;
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

#[cfg(test)]
mod tests {
    use super::{STREAM, consumed};
    use std::{fs, io, path::PathBuf};

    #[test]
    fn signature_follows_only_the_files_the_scan_reads() -> io::Result<()> {
        let root: PathBuf =
            std::env::temp_dir().join(format!("ab-usage-signature-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sync"))?;
        fs::create_dir_all(root.join(STREAM).join("local"))?;
        fs::create_dir_all(root.join("streams/other"))?;
        fs::write(root.join("sync/sync.db"), "peers")?;
        fs::write(root.join(STREAM).join("local/1"), "a")?;
        let before = consumed(&root).map_err(|error| io::Error::other(error.to_string()))?;
        fs::write(root.join("streams/other/9"), "unrelated churn")?;
        fs::write(root.join("sync/sync.db-shm"), "shared memory")?;
        let after = consumed(&root).map_err(|error| io::Error::other(error.to_string()))?;
        assert_eq!(before, after);
        fs::write(root.join(STREAM).join("local/1"), "a longer segment")?;
        let changed = consumed(&root).map_err(|error| io::Error::other(error.to_string()))?;
        assert_ne!(before, changed);
        fs::remove_dir_all(&root)
    }
}
