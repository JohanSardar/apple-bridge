use crate::Error;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const LOG_LIMIT: u64 = 8 * 1024 * 1024;
const LOG_KEEP: u64 = LOG_LIMIT / 2;
const POLL: Duration = Duration::from_secs(1);
const REMOVAL_GRACE: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Running {
    pub version: String,
    executable: PathBuf,
    #[serde(default)]
    build: String,
    #[serde(skip)]
    file: Option<File>,
}

fn build_identity(mut file: &File) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest)?;
    Ok(format!("{:x}", digest.finalize()))
}

impl Running {
    pub(crate) fn current() -> Result<Self, Error> {
        Self::open(&std::env::current_exe()?)
    }

    fn open(executable: &Path) -> Result<Self, Error> {
        let file = File::open(executable)?;
        Ok(Self {
            version: env!("CARGO_PKG_VERSION").into(),
            executable: executable.to_owned(),
            build: build_identity(&file)?,
            file: Some(file),
        })
    }

    pub(crate) fn matches(&self, executable: &Path) -> bool {
        File::open(executable)
            .and_then(|file| build_identity(&file))
            .is_ok_and(|build| self.build == build)
    }

    pub(crate) fn save(&self, home: &Path) -> Result<(), Error> {
        ab_trust::atomic_write(
            &home.join("run/gateway-build.json"),
            &serde_json::to_vec(self)?,
        )
        .map_err(|_| Error::Invalid)
    }

    fn removed(&self) -> bool {
        self.file
            .as_ref()
            .is_some_and(|file| file.metadata().is_ok_and(|metadata| metadata.nlink() == 0))
            && self
                .executable
                .metadata()
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    }
}

pub(crate) fn older(home: &Path) -> Option<String> {
    let bytes = std::fs::read(home.join("run/gateway-build.json")).ok()?;
    let running: Running = serde_json::from_slice(&bytes).ok()?;
    let executable = std::env::current_exe().ok()?;
    (!running.matches(&executable)).then_some(running.version)
}

fn bound_log(path: &Path) -> Result<(), Error> {
    let mut file = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::Invalid);
    }
    let length = metadata.len();
    if length <= LOG_LIMIT {
        return Ok(());
    }
    file.seek(SeekFrom::Start(length - LOG_KEEP))?;
    let mut recent = Vec::new();
    (&mut file).take(LOG_KEEP).read_to_end(&mut recent)?;
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&recent)?;
    Ok(())
}

pub(crate) fn open_log(path: &Path) -> Result<File, Error> {
    bound_log(path)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::Invalid);
    }
    Ok(file)
}

pub(crate) async fn run(
    running: Running,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Error> {
    let mut tick = tokio::time::interval(POLL);
    let mut missing_since = None;
    loop {
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tick.tick() => {
                if running.removed() {
                    let since = missing_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= REMOVAL_GRACE {
                        eprintln!("Apple Bridge was removed; stopping the gateway.");
                        return Ok(());
                    }
                } else { missing_since = None; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_trimming_refuses_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let target = home.path().join("unrelated");
        let path = home.path().join("gateway.log");
        let bytes = vec![b'x'; usize::try_from(LOG_LIMIT + 1)?];
        std::fs::write(&target, &bytes)?;
        std::os::unix::fs::symlink(&target, &path)?;
        assert!(bound_log(&path).is_err());
        assert!(open_log(&path).is_err());
        assert_eq!(std::fs::read(target)?, bytes);
        Ok(())
    }

    #[test]
    fn log_trimming_refuses_non_regular_descriptors() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()?
                .success()
        );
        assert!(bound_log(&path).is_err());
        assert!(open_log(&path).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn lifetime_polling_preserves_append_writes() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        let mut writer = open_log(&path)?;
        let mut expected = vec![b'x'; usize::try_from(LOG_LIMIT + 1)?];
        writer.write_all(&expected)?;
        let running = Running::current()?;
        let (stop, receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(run(running, receiver));
        for _ in 0..3 {
            writer.write_all(b"appended while polling\n")?;
            expected.extend_from_slice(b"appended while polling\n");
            tokio::time::sleep(POLL).await;
        }
        stop.send(true)?;
        task.await??;
        assert_eq!(std::fs::metadata(&path)?.len(), expected.len() as u64);
        assert_eq!(std::fs::read(path)?, expected);
        Ok(())
    }

    #[test]
    fn moving_an_install_is_not_removal() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let install = home.path().join("install");
        std::fs::create_dir(&install)?;
        let executable = install.join("bridge");
        std::fs::write(&executable, b"build")?;
        let running = Running::open(&executable)?;
        let moved = home.path().join("moved");
        std::fs::rename(install, &moved)?;
        assert!(!running.removed());
        std::fs::remove_file(moved.join("bridge"))?;
        assert!(running.removed());
        Ok(())
    }

    #[test]
    fn identical_builds_in_different_installs_match() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let executable = home.path().join("bridge");
        let copy = home.path().join("other-install");
        std::fs::write(&executable, b"same build")?;
        std::fs::copy(&executable, &copy)?;
        let running = Running::open(&executable)?;
        assert_ne!(executable.metadata()?.ino(), copy.metadata()?.ino());
        assert!(running.matches(&copy));
        let saved = serde_json::to_vec(&running)?;
        let restored: Running = serde_json::from_slice(&saved)?;
        assert!(restored.matches(&copy));
        std::fs::write(&copy, b"different build, same version")?;
        assert!(!restored.matches(&copy));
        Ok(())
    }

    #[test]
    fn older_uses_build_identity_across_installs() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        std::fs::create_dir(home.path().join("run"))?;
        let copy = home.path().join("bridge");
        std::fs::copy(std::env::current_exe()?, &copy)?;
        let mut running = Running::open(&copy)?;
        running.version = "fixture".into();
        running.save(home.path())?;
        assert_eq!(older(home.path()), None);
        running.build = "different build".into();
        running.save(home.path())?;
        assert_eq!(older(home.path()).as_deref(), Some("fixture"));
        let mut legacy = serde_json::to_value(&running)?;
        legacy
            .as_object_mut()
            .ok_or(Error::Invalid)?
            .remove("build");
        std::fs::write(
            home.path().join("run/gateway-build.json"),
            serde_json::to_vec(&legacy)?,
        )?;
        assert_eq!(older(home.path()).as_deref(), Some("fixture"));
        Ok(())
    }

    #[test]
    fn replacement_is_not_removal_and_has_a_different_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let executable = home.path().join("bridge");
        std::fs::write(&executable, b"old")?;
        let running = Running::open(&executable)?;
        assert!(running.matches(&executable));
        let replacement = home.path().join("update");
        std::fs::write(&replacement, b"new")?;
        std::fs::rename(replacement, &executable)?;
        assert_eq!(
            running
                .file
                .as_ref()
                .ok_or(Error::Invalid)?
                .metadata()?
                .nlink(),
            0
        );
        assert!(!running.matches(&executable));
        assert!(!running.removed());
        std::fs::remove_file(executable)?;
        assert!(running.removed());
        Ok(())
    }

    #[test]
    fn the_log_is_bounded_before_opening_the_append_writer()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        let mut previous = vec![b'x'; usize::try_from(LOG_LIMIT)?];
        previous.extend_from_slice(b"recent\n");
        std::fs::write(&path, &previous)?;
        let mut writer = open_log(&path)?;
        assert_eq!(writer.metadata()?.len(), LOG_KEEP);
        assert_eq!(
            std::fs::read(&path)?,
            previous[previous.len() - usize::try_from(LOG_KEEP)?..]
        );
        writer.write_all(b"still logging\n")?;
        let bytes = std::fs::read(path)?;
        assert!(bytes.ends_with(b"recent\nstill logging\n"));
        Ok(())
    }
}
