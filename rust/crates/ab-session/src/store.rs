use crate::Error;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Record {
    pub generation: String,
    pub checkpoint: String,
    pub token: String,
}

#[derive(Debug)]
pub struct Store {
    directory: PathBuf,
}

fn valid(source: &str) -> bool {
    !source.is_empty()
        && source
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Store {
    pub fn open(directory: &Path) -> Result<Self, Error> {
        fs::create_dir_all(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn load(&self, source: &str) -> Option<Record> {
        if !valid(source) {
            return None;
        }
        let bytes = fs::read(self.directory.join(format!("{source}.json"))).ok()?;
        let record: Record = serde_json::from_slice(&bytes).ok()?;
        (!record.generation.is_empty() && !record.checkpoint.is_empty() && !record.token.is_empty())
            .then_some(record)
    }

    pub fn save(&self, source: &str, record: &Record) -> Result<(), Error> {
        if !valid(source) {
            return Err(Error::Invalid);
        }
        let temporary = self.directory.join(format!(".{source}.json.tmp"));
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(record)?)?;
        file.sync_all()?;
        fs::rename(&temporary, self.directory.join(format!("{source}.json")))?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}
