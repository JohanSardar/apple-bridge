use crate::Error;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
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
    device: u64,
    inode: u64,
}

impl Running {
    pub(crate) fn current() -> Result<Self, Error> {
        let executable = std::env::current_exe()?;
        let metadata = executable.metadata()?;
        Ok(Self {
            version: env!("CARGO_PKG_VERSION").into(),
            executable,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    pub(crate) fn matches(&self, executable: &Path) -> bool {
        executable
            .metadata()
            .is_ok_and(|metadata| self.device == metadata.dev() && self.inode == metadata.ino())
    }

    pub(crate) fn save(&self, home: &Path) -> Result<(), Error> {
        ab_trust::atomic_write(
            &home.join("run/gateway-build.json"),
            &serde_json::to_vec(self)?,
        )
        .map_err(|_| Error::Invalid)
    }

    fn removed(&self) -> bool {
        self.executable
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
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let length = file.metadata()?.len();
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

pub(crate) async fn run(
    home: PathBuf,
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
                if let Err(error) = bound_log(&home.join("gateway.log")) {
                    eprintln!("Could not bound gateway.log: {error}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_is_not_removal_and_has_a_different_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let executable = home.path().join("bridge");
        std::fs::write(&executable, b"old")?;
        let metadata = executable.metadata()?;
        let running = Running {
            version: "old".into(),
            executable: executable.clone(),
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        assert!(running.matches(&executable));
        std::fs::rename(&executable, home.path().join("old"))?;
        std::fs::write(&executable, b"new")?;
        assert!(!running.matches(&executable));
        assert!(!running.removed());
        std::fs::remove_file(executable)?;
        assert!(running.removed());
        Ok(())
    }

    #[test]
    fn the_log_is_bounded_without_replacing_an_open_writers_inode()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        let mut writer = OpenOptions::new().create(true).append(true).open(&path)?;
        writer.write_all(&vec![b'x'; usize::try_from(LOG_LIMIT)?])?;
        writer.write_all(b"recent\n")?;
        let inode = writer.metadata()?.ino();
        bound_log(&path)?;
        assert_eq!(path.metadata()?.ino(), inode);
        assert_eq!(path.metadata()?.len(), LOG_KEEP);
        writer.write_all(b"still logging\n")?;
        let bytes = std::fs::read(path)?;
        assert!(bytes.ends_with(b"recent\nstill logging\n"));
        Ok(())
    }
}
