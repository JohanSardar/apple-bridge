use crate::{Error, readers::BlobServer};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

pub const CAPTURE_RETENTION: Duration = Duration::from_secs(300);
const TRANSFER_IDLE: Duration = Duration::from_secs(90);

#[derive(Debug, Default)]
struct CaptureFiles {
    directory: Option<File>,
    transfers: BTreeMap<String, (Instant, u64)>,
    completed: BTreeMap<String, Instant>,
}

#[derive(Debug)]
pub struct StagedBlobs {
    prefix: &'static str,
    directory: PathBuf,
    known: Mutex<BTreeMap<String, PathBuf>>,
    capture: Option<Mutex<CaptureFiles>>,
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
            capture: (prefix == crate::production::CAPTURE_BLOB).then(Mutex::default),
        }
    }

    pub fn capture(directory: PathBuf) -> Result<Arc<Self>, Error> {
        let blobs = Arc::new(Self::new(crate::production::CAPTURE_BLOB, directory));
        blobs.cleanup()?;
        let weak = Arc::downgrade(&blobs);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let Some(blobs) = weak.upgrade() else { break };
                if let Err(error) = blobs.cleanup() {
                    eprintln!("ab-session: capture staging cleanup: {error}");
                }
            }
        });
        Ok(blobs)
    }

    fn capture_directory(&self, state: &mut CaptureFiles) -> Result<(), Error> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.directory)?;
        let metadata = fs::symlink_metadata(&self.directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::Invalid);
        }
        if state.directory.is_none() {
            state.directory = Some(
                File::options()
                    .read(true)
                    .custom_flags(ab_sys::O_NOFOLLOW | ab_sys::O_DIRECTORY)
                    .open(&self.directory)?,
            );
        }
        let pinned = state.directory.as_ref().ok_or(Error::Invalid)?.metadata()?;
        if pinned.dev() != metadata.dev() || pinned.ino() != metadata.ino() {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    fn remove_capture(&self, state: &CaptureFiles, path: &Path) -> Result<(), Error> {
        if path.parent() != Some(self.directory.as_path()) {
            return Err(Error::Invalid);
        }
        ab_sys::remove_file_in(
            state.directory.as_ref().ok_or(Error::Invalid)?,
            path.file_name().ok_or(Error::Invalid)?,
        )?;
        Ok(())
    }

    fn expired(metadata: &fs::Metadata) -> Result<bool, Error> {
        Ok(SystemTime::now()
            .duration_since(metadata.modified()?)
            .unwrap_or_default()
            >= CAPTURE_RETENTION)
    }

    pub fn cleanup(&self) -> Result<(), Error> {
        let Some(capture) = &self.capture else {
            return Ok(());
        };
        let mut state = capture.lock().map_err(|_| Error::WorkerPanicked)?;
        self.capture_directory(&mut state)?;
        state
            .transfers
            .retain(|_, (at, _)| at.elapsed() < TRANSFER_IDLE);
        let mut known = self.known.lock().map_err(|_| Error::WorkerPanicked)?;
        let completed: Vec<String> = state
            .completed
            .iter()
            .filter(|(digest, at)| {
                at.elapsed() >= TRANSFER_IDLE && !state.transfers.contains_key(*digest)
            })
            .map(|(digest, _)| digest.clone())
            .collect();
        for digest in completed {
            let path = self.directory.join(&digest);
            match self.remove_capture(&state, &path) {
                Ok(()) => {}
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            known.remove(&digest);
            state.completed.remove(&digest);
        }
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_file()
                && Self::expired(&metadata)?
                && !entry.file_name().to_str().is_some_and(|digest| {
                    state.transfers.contains_key(digest)
                        || state
                            .completed
                            .get(digest)
                            .is_some_and(|at| at.elapsed() < TRANSFER_IDLE)
                })
                && !known
                    .iter()
                    .any(|(digest, file)| file == &path && state.transfers.contains_key(digest))
            {
                self.remove_capture(&state, &path)?;
                known.retain(|_, file| file != &path);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn handle(&self, digest: &str) -> String {
        format!("{}{digest}", self.prefix)
    }

    pub fn remember(&self, digest: &str, mut file: PathBuf) -> Result<(), Error> {
        if !is_digest(digest) {
            return Err(Error::Invalid);
        }
        let mut capture = self
            .capture
            .as_ref()
            .map(|capture| capture.lock().map_err(|_| Error::WorkerPanicked))
            .transpose()?;
        if let Some(state) = capture.as_mut() {
            self.capture_directory(state)?;
            if file.parent() != Some(self.directory.as_path())
                || !fs::symlink_metadata(&file)?.is_file()
            {
                return Err(Error::Invalid);
            }
            let retained = self.directory.join(digest);
            if file != retained {
                if retained.exists() && state.transfers.contains_key(digest) {
                    self.remove_capture(state, &file)?;
                } else {
                    fs::rename(&file, &retained)?;
                }
                file = retained;
            }
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
        if !is_digest(digest) || length == 0 || length > 131_072 {
            return Err(Error::Invalid);
        }
        let mut capture = self
            .capture
            .as_ref()
            .map(|capture| capture.lock().map_err(|_| Error::WorkerPanicked))
            .transpose()?;
        if let Some(state) = capture.as_mut() {
            self.capture_directory(state)?;
        }
        let path = self.path_of(digest)?.ok_or(Error::Conflict)?;
        let mut file = File::options()
            .read(true)
            .custom_flags(ab_sys::O_NOFOLLOW)
            .open(&path)?;
        let metadata = file.metadata()?;
        let size = metadata.len();
        if !metadata.is_file() || offset > size {
            return Err(Error::Invalid);
        }
        if let Some(state) = capture.as_mut() {
            let active = state
                .transfers
                .get(digest)
                .is_some_and(|(at, next)| at.elapsed() < TRANSFER_IDLE && *next == offset);
            let completed = state
                .completed
                .get(digest)
                .is_some_and(|at| at.elapsed() < TRANSFER_IDLE);
            if !active && !completed && Self::expired(&metadata)? {
                self.remove_capture(state, &path)?;
                self.known
                    .lock()
                    .map_err(|_| Error::WorkerPanicked)?
                    .remove(digest);
                state.transfers.remove(digest);
                return Err(Error::Conflict);
            }
            if !active && !completed && offset != 0 {
                return Err(Error::Conflict);
            }
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
        if let Some(state) = capture.as_mut() {
            if end >= size {
                state
                    .completed
                    .entry(digest.to_owned())
                    .or_insert_with(Instant::now);
                state.transfers.remove(digest);
            } else {
                state
                    .transfers
                    .insert(digest.to_owned(), (Instant::now(), end));
            }
        }
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

    fn expire_recovery(blobs: &StagedBlobs, digest: &str) {
        blobs
            .capture
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .completed
            .insert(
                digest.to_owned(),
                Instant::now().checked_sub(TRANSFER_IDLE).unwrap(),
            );
        blobs.cleanup().unwrap();
    }

    #[test]
    fn retrieved_capture_original_is_removed_only_after_the_final_chunk() {
        let root = directory();
        let blobs = StagedBlobs::new(crate::production::CAPTURE_BLOB, root.clone());
        let (digest, path) = blobs.stage(b"abcdef").unwrap();
        let handle = blobs.handle(&digest);
        assert_eq!(blobs.read(&handle, 0, 3).unwrap()["eof"], false);
        assert!(path.exists(), "an in-progress transfer keeps its original");
        assert_eq!(
            blobs.read(&handle, 3, 3).unwrap()["bytes"],
            STANDARD.encode(b"def")
        );
        assert!(
            path.exists(),
            "a lost final reply can retry during recovery"
        );
        assert_eq!(
            blobs.read(&handle, 3, 3).unwrap()["bytes"],
            STANDARD.encode(b"def")
        );
        expire_recovery(&blobs, &digest);
        assert!(
            !path.exists(),
            "a retrieved capture must not remain on disk"
        );
        assert!(blobs.path_of(&digest).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_expired_capture_is_deleted_instead_of_being_served() {
        let root = directory();
        let blobs = StagedBlobs::new(crate::production::CAPTURE_BLOB, root.clone());
        let (digest, path) = blobs.stage(b"expired capture").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        assert!(blobs.read(&blobs.handle(&digest), 0, 32).is_err());
        assert!(!path.exists(), "an expired capture must not remain on disk");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_preserves_an_active_transfer_then_removes_its_completed_original() {
        let root = directory();
        let blobs = StagedBlobs::new(crate::production::CAPTURE_BLOB, root.clone());
        let (digest, path) = blobs.stage(b"abcdef").unwrap();
        let handle = blobs.handle(&digest);
        blobs.read(&handle, 0, 3).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        blobs.cleanup().unwrap();
        assert!(path.exists());
        assert_eq!(
            blobs.read(&handle, 3, 3).unwrap()["bytes"],
            STANDARD.encode(b"def")
        );
        expire_recovery(&blobs, &digest);
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_cleans_stale_orphans_and_keeps_recent_recovery_files_inside_the_folder() {
        let root = directory();
        fs::create_dir_all(&root).unwrap();
        let stale = root.join("old-original.png");
        fs::write(&stale, b"expired").unwrap();
        File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        let outside = root.with_extension("outside");
        fs::write(&outside, b"do not delete").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let digest = digest_of(b"recoverable");
        let recent = root.join(&digest);
        fs::write(&recent, b"recoverable").unwrap();
        let blobs = StagedBlobs::capture(root.clone()).unwrap();
        assert!(!stale.exists());
        assert!(outside.exists());
        assert!(
            blobs
                .remember(&digest_of(b"outside"), outside.clone())
                .is_err()
        );
        assert!(
            blobs
                .read(&blobs.handle(&digest_of(b"link")), 0, 1)
                .is_err()
        );
        assert_eq!(
            blobs.read(&blobs.handle(&digest), 0, 4).unwrap()["bytes"],
            STANDARD.encode(b"reco")
        );
        File::options()
            .write(true)
            .open(&recent)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        blobs.cleanup().unwrap();
        assert!(
            recent.exists(),
            "a resumed recovery transfer survives expiry"
        );
        assert_eq!(
            blobs.read(&blobs.handle(&digest), 4, 64).unwrap()["bytes"],
            STANDARD.encode(b"verable")
        );
        expire_recovery(&blobs, &digest);
        assert!(!recent.exists());
        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn capture_cleanup_refuses_a_symlinked_capture_folder() {
        let root = directory();
        let outside = root.with_extension("outside-dir");
        fs::create_dir_all(&outside).unwrap();
        let file = outside.join("old");
        fs::write(&file, b"keep").unwrap();
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        std::os::unix::fs::symlink(&outside, &root).unwrap();
        assert!(StagedBlobs::capture(root.clone()).is_err());
        assert!(file.exists());
        fs::remove_file(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn an_idle_abandoned_transfer_expires_and_directory_replacement_never_deletes_outside() {
        let root = directory();
        let blobs = StagedBlobs::new(crate::production::CAPTURE_BLOB, root.clone());
        let (digest, path) = blobs.stage(b"abcdef").unwrap();
        blobs.read(&blobs.handle(&digest), 0, 2).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        blobs
            .capture
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .transfers
            .insert(
                digest,
                (Instant::now().checked_sub(TRANSFER_IDLE).unwrap(), 2),
            );
        blobs.cleanup().unwrap();
        assert!(!path.exists());
        let original = root.with_extension("original");
        fs::rename(&root, &original).unwrap();
        let outside = root.with_extension("outside-dir");
        fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("old");
        fs::write(&victim, b"keep").unwrap();
        File::options()
            .write(true)
            .open(&victim)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH)
            .unwrap();
        std::os::unix::fs::symlink(&outside, &root).unwrap();
        assert!(blobs.cleanup().is_err());
        assert!(victim.exists());
        fs::remove_file(root).unwrap();
        fs::remove_dir_all(original).unwrap();
        fs::remove_dir_all(outside).unwrap();
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
