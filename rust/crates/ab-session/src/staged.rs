use crate::{Error, readers::BlobServer};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Debug)]
pub struct StagedBlobs {
    prefix: &'static str,
    directory: PathBuf,
    known: Mutex<BTreeMap<String, PathBuf>>,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[must_use]
pub fn digest_of(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

impl StagedBlobs {
    #[must_use]
    pub fn new(prefix: &'static str, directory: PathBuf) -> Self {
        Self {
            prefix,
            directory,
            known: Mutex::new(BTreeMap::new()),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn handle(&self, digest: &str) -> String {
        format!("{}{digest}", self.prefix)
    }

    pub fn remember(&self, digest: &str, file: PathBuf) -> Result<(), Error> {
        if !is_digest(digest) {
            return Err(Error::Invalid);
        }
        self.known
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .insert(digest.to_owned(), file);
        Ok(())
    }

    pub fn path_of(&self, digest: &str) -> Result<Option<PathBuf>, Error> {
        if !is_digest(digest) {
            return Ok(None);
        }
        if let Some(file) = self
            .known
            .lock()
            .map_err(|_| Error::WorkerPanicked)?
            .get(digest)
        {
            return Ok(Some(file.clone()));
        }
        let file = self.directory.join(digest);
        Ok(file.is_file().then_some(file))
    }

    pub fn stage(&self, bytes: &[u8]) -> Result<(String, PathBuf), Error> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.directory)?;
        let digest = digest_of(bytes);
        let file = self.directory.join(&digest);
        let temporary = self
            .directory
            .join(format!(".{digest}.{}", std::process::id()));
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(ab_sys::O_NOFOLLOW)
            .open(&temporary)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        fs::rename(&temporary, &file)?;
        self.remember(&digest, file.clone())?;
        Ok((digest, file))
    }
}

impl BlobServer for StagedBlobs {
    fn owns(&self, handle: &str) -> bool {
        handle.starts_with(self.prefix)
    }

    fn read(&self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        let digest = handle.strip_prefix(self.prefix).ok_or(Error::Invalid)?;
        let path = self.path_of(digest)?.ok_or(Error::Conflict)?;
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        if offset > size {
            return Err(Error::Invalid);
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length];
        let mut filled = 0;
        while filled < length {
            let count = file.read(&mut bytes[filled..])?;
            if count == 0 {
                break;
            }
            filled += count;
        }
        bytes.truncate(filled);
        let end = offset + u64::try_from(filled).map_err(|_| Error::Invalid)?;
        Ok(json!({
            "t": "BlobChunk",
            "handle": handle,
            "offset": offset,
            "bytes": STANDARD.encode(&bytes),
            "digest": digest,
            "eof": end >= size,
        }))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ab-staged-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn staged_bytes_read_back_in_chunks() {
        let blobs = StagedBlobs::new("x:", directory());
        let (digest, _) = blobs.stage(b"abcdef").unwrap();
        let handle = blobs.handle(&digest);
        assert!(blobs.owns(&handle));
        let first = blobs.read(&handle, 0, 4).unwrap();
        assert_eq!(first["bytes"], STANDARD.encode("abcd"));
        assert_eq!(first["eof"], false);
        let last = blobs.read(&handle, 4, 4).unwrap();
        assert_eq!(last["bytes"], STANDARD.encode("ef"));
        assert_eq!(last["eof"], true);
        assert!(blobs.read(&handle, 7, 1).is_err());
    }

    #[test]
    fn a_handle_cannot_name_a_path() {
        let blobs = StagedBlobs::new("x:", directory());
        blobs.stage(b"abc").unwrap();
        for handle in ["x:../../etc/passwd", "x:", "x:abc", "y:abc"] {
            assert!(blobs.read(handle, 0, 1).is_err(), "{handle}");
        }
    }
}
