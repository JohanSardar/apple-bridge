use crate::capture::{Error, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    prefix: &'static str,
    files: BTreeMap<String, PathBuf>,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Reference {
    pub digest: String,
    pub size: u64,
    pub state: &'static str,
}
#[derive(Debug)]
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn dataless(metadata: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        metadata.st_flags() & 0x4000_0000 != 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = metadata;
        false
    }
}
pub fn open_regular(path: &Path) -> Result<File> {
    if dataless(&std::fs::symlink_metadata(path)?) {
        return Err(Error::Incomplete);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || dataless(&metadata) {
        return Err(Error::Incomplete);
    }
    Ok(file)
}
fn stable(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}
impl Store {
    pub fn open(root: &Path, prefix: &'static str) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        if !std::fs::symlink_metadata(root)?.is_dir() {
            return Err(Error::Corrupt);
        }
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
        let root = root.canonicalize()?;
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() == 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                && entry.metadata()?.is_file()
            {
                files.insert(name, entry.path());
            }
        }
        Ok(Self {
            root,
            prefix,
            files,
        })
    }
    fn temporary(&self) -> Result<(Temporary, File)> {
        let token = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Corrupt)?
            .as_nanos();
        let path = self.root.join(format!(
            "stage-{}-{token}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        Ok((Temporary(path), file))
    }
    fn finish(
        &mut self,
        temporary: &Temporary,
        output: &File,
        hash: Sha256,
        size: u64,
    ) -> Result<Reference> {
        output.sync_all()?;
        let digest = format!("{:x}", hash.finalize());
        let destination = self.root.join(&digest);
        match std::fs::hard_link(&temporary.0, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut file = open_regular(&destination)?;
                let mut hash = Sha256::new();
                let mut buffer = vec![0; 131_072];
                let mut actual = 0;
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    actual += u64::try_from(count).map_err(|_| Error::Corrupt)?;
                    hash.update(&buffer[..count]);
                }
                if actual != size || format!("{:x}", hash.finalize()) != digest {
                    return Err(Error::Conflict);
                }
            }
            Err(error) => return Err(error.into()),
        }
        File::open(&self.root)?.sync_all()?;
        self.files.insert(digest.clone(), destination);
        Ok(Reference {
            digest,
            size,
            state: "present",
        })
    }
    pub fn stage(&mut self, source: &Path, allowed_root: &Path) -> Result<Reference> {
        let resolved = source.canonicalize()?;
        let root = allowed_root.canonicalize()?;
        if !resolved.starts_with(&root) || resolved == root {
            return Err(Error::Corrupt);
        }
        let mut input = open_regular(&resolved)?;
        let before = input.metadata()?;
        if resolved.canonicalize()? != resolved || !stable(&before, &std::fs::metadata(&resolved)?)
        {
            return Err(Error::Conflict);
        }
        let (temporary, mut output) = self.temporary()?;
        let mut hash = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = vec![0; 131_072];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(u64::try_from(count).map_err(|_| Error::Corrupt)?)
                .ok_or(Error::Corrupt)?;
            if size > before.len() {
                return Err(Error::Conflict);
            }
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        if size != before.len() || !stable(&before, &input.metadata()?) {
            return Err(Error::Conflict);
        }
        self.finish(&temporary, &output, hash, size)
    }
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<Reference> {
        let (temporary, mut output) = self.temporary()?;
        output.write_all(bytes)?;
        let mut hash = Sha256::new();
        hash.update(bytes);
        self.finish(
            &temporary,
            &output,
            hash,
            u64::try_from(bytes.len()).map_err(|_| Error::Corrupt)?,
        )
    }
    pub fn file(&self, digest: &str) -> Option<&Path> {
        self.files.get(digest).map(PathBuf::as_path)
    }
    pub fn handle(&self, digest: &str) -> Option<String> {
        self.files
            .contains_key(digest)
            .then(|| format!("{}{digest}", self.prefix))
    }
    pub fn prune(&mut self, active: &BTreeSet<String>, now: SystemTime) -> Result<()> {
        let Some(cutoff) = now.checked_sub(std::time::Duration::from_secs(3600)) else {
            return Ok(());
        };
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if active.contains(&name) {
                continue;
            }
            let info = std::fs::symlink_metadata(entry.path())?;
            if info.is_file() && info.modified().is_ok_and(|time| time < cutoff) {
                std::fs::remove_file(entry.path())?;
                self.files.remove(&name);
            }
        }
        Ok(())
    }
    pub fn read(&self, handle: &str, offset: u64, length: usize) -> Result<Value> {
        if length == 0 || length > 131_072 {
            return Err(Error::Corrupt);
        }
        let digest = handle.strip_prefix(self.prefix).ok_or(Error::Corrupt)?;
        let path = self.file(digest).ok_or(Error::Corrupt)?;
        let mut input = open_regular(path)?;
        let size = input.metadata()?.len();
        if offset > size {
            return Err(Error::Corrupt);
        }
        input.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        input
            .take(u64::try_from(length).map_err(|_| Error::Corrupt)?)
            .read_to_end(&mut bytes)?;
        let eof = offset + u64::try_from(bytes.len()).map_err(|_| Error::Corrupt)? == size;
        Ok(
            json!({"t":"BlobChunk","handle":handle,"offset":offset,"bytes":STANDARD.encode(bytes),"digest":digest,"eof":eof}),
        )
    }
}
