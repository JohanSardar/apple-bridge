use crate::{Error, fingerprint};
use rand::RngCore;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
pub trait CredentialStore: std::fmt::Debug + Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), Error>;
    fn get(&self, key: &str) -> Result<Vec<u8>, Error>;
    fn delete(&self, key: &str) -> Result<(), Error>;
}
#[derive(Debug)]
pub struct FileStore {
    dir: PathBuf,
}
impl FileStore {
    pub fn new(home: &Path) -> Result<Self, Error> {
        let dir = home.join("secrets");
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(Self { dir })
    }
}
impl CredentialStore for FileStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), Error> {
        atomic_write(&self.dir.join(fingerprint(key.as_bytes())), bytes)
    }
    fn get(&self, key: &str) -> Result<Vec<u8>, Error> {
        let path = self.dir.join(fingerprint(key.as_bytes()));
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.permissions().mode() & 0o777 != 0o600 {
            return Err(Error::Unauthorized);
        }
        Ok(fs::read(path)?)
    }
    fn delete(&self, key: &str) -> Result<(), Error> {
        match fs::remove_file(self.dir.join(fingerprint(key.as_bytes()))) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
#[derive(Debug)]
pub struct KeychainStore;
impl CredentialStore for KeychainStore {
    fn put(&self, _: &str, _: &[u8]) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
    fn get(&self, _: &str) -> Result<Vec<u8>, Error> {
        Err(Error::Unavailable)
    }
    fn delete(&self, _: &str) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path.parent().ok_or(Error::Invalid)?;
    let mut nonce = [0; 16];
    rand::thread_rng().fill_bytes(&mut nonce);
    let temp = parent.join(format!(".write-{}", fingerprint(&nonce)));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if temp.exists() {
        fs::remove_file(temp)?;
    }
    result
}
