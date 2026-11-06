use crate::{Error, HomeLock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::{
        fd::AsFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const LOG_LIMIT: u64 = 8 * 1024 * 1024;
const POLL: Duration = Duration::from_secs(1);

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Running {
    pub version: String,
    #[serde(default)]
    build: String,
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
        Ok(Self {
            version: env!("CARGO_PKG_VERSION").into(),
            build: build_identity(&File::open(executable)?)?,
        })
    }

    fn matches(&self, executable: &Path) -> bool {
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
}

pub(crate) fn older(home: &Path) -> Option<String> {
    let bytes = std::fs::read(home.join("run/gateway-build.json")).ok()?;
    let running: Running = serde_json::from_slice(&bytes).ok()?;
    let executable = std::env::current_exe().ok()?;
    (!running.matches(&executable)).then_some(running.version)
}

fn append_log(path: &Path) -> Result<File, Error> {
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

pub(crate) fn open_log(path: &Path) -> Result<File, Error> {
    let _ownership = HomeLock::acquire(path.parent().ok_or(Error::Invalid)?)?;
    append_log(path)
}

fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.is_file() && right.is_file() && left.dev() == right.dev() && left.ino() == right.ino()
}

#[derive(Debug)]
pub(crate) struct Log {
    path: PathBuf,
    file: File,
}

impl Log {
    pub(crate) fn inherited(home: &Path) -> Result<Option<Self>, Error> {
        let file = File::from(std::io::stderr().as_fd().try_clone_to_owned()?);
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Ok(None);
        }
        let stdout = File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
        let path = home.join("gateway.log");
        let on_disk = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        Ok(
            (same_file(&metadata, &on_disk) && same_file(&metadata, &stdout.metadata()?))
                .then_some(Self { path, file }),
        )
    }

    fn rotate(&mut self) -> Result<(), Error> {
        self.rotate_with(
            |file| {
                ab_sys::redirect_stdout(file)?;
                ab_sys::redirect_stderr(file)
            },
            |from, to| std::fs::rename(from, to),
        )
    }

    fn rotate_with(
        &mut self,
        mut redirect: impl FnMut(&File) -> std::io::Result<()>,
        publish: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<(), Error> {
        let metadata = self.file.metadata()?;
        if metadata.len() <= LOG_LIMIT {
            return Ok(());
        }
        if !same_file(&metadata, &std::fs::symlink_metadata(&self.path)?) {
            return Err(Error::Invalid);
        }
        redirect(&self.file)?;
        let temporary = self.path.with_extension("log.next");
        let replacement = append_log(&temporary)?;
        let prepared = replacement.metadata()?;
        if prepared.nlink() != 1 || same_file(&metadata, &prepared) {
            return Err(Error::Invalid);
        }
        let archive = self.path.with_extension("log.1");
        match std::fs::remove_file(&archive) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        std::fs::hard_link(&self.path, archive)?;
        if let Err(error) = redirect(&replacement).and_then(|()| publish(&temporary, &self.path)) {
            let _ = redirect(&self.file);
            return Err(error.into());
        }
        self.file = replacement;
        Ok(())
    }
}

pub(crate) async fn run(
    mut log: Log,
    mut stop: tokio::sync::watch::Receiver<bool>,
    _ownership: Arc<HomeLock>,
) -> Result<(), Error> {
    let mut tick = tokio::time::interval(POLL);
    loop {
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tick.tick() => {
                if let Err(error) = log.rotate() {
                    let _ = writeln!(std::io::stderr(), "Could not rotate gateway.log: {error}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn log_open_refuses_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let target = home.path().join("unrelated");
        let path = home.path().join("gateway.log");
        let bytes = vec![b'x'; usize::try_from(LOG_LIMIT + 1)?];
        std::fs::write(&target, &bytes)?;
        std::os::unix::fs::symlink(&target, &path)?;
        assert!(open_log(&path).is_err());
        assert_eq!(std::fs::read(target)?, bytes);
        Ok(())
    }

    #[test]
    fn log_open_refuses_non_regular_descriptors() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&path)
                .status()?
                .success()
        );
        assert!(open_log(&path).is_err());
        Ok(())
    }

    #[test]
    fn delayed_startup_cannot_trim_an_owned_homes_log() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        let ready = Arc::new(std::sync::Barrier::new(2));
        let delayed = {
            let path = path.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                ready.wait();
                open_log(&path)
            })
        };
        let _owner = HomeLock::acquire(home.path())?;
        let mut writer = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut expected = vec![b'x'; usize::try_from(LOG_LIMIT + 1)?];
        writer.write_all(&expected)?;
        ready.wait();
        let result = delayed.join().map_err(|_| Error::Invalid)?;
        assert!(matches!(result, Err(Error::Locked)));
        writer.write_all(b"owner still logging\n")?;
        expected.extend_from_slice(b"owner still logging\n");
        assert_eq!(std::fs::read(path)?, expected);
        Ok(())
    }

    #[test]
    fn startup_log_releases_ownership_and_never_trims() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        let bytes = vec![b'x'; usize::try_from(LOG_LIMIT + 1)?];
        std::fs::write(&path, &bytes)?;
        let log = open_log(&path)?;
        let _gateway_ownership = HomeLock::acquire(home.path())?;
        assert!(log.metadata()?.is_file());
        assert_eq!(std::fs::read(path)?, bytes);
        Ok(())
    }

    #[test]
    fn running_rotation_preserves_concurrent_stdout_and_stderr()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        let mut log = open_log(&path)?;
        let mut previous = vec![b'x'; usize::try_from(LOG_LIMIT + 1)?];
        previous.extend_from_slice(b"\nbefore rotation\n");
        log.write_all(&previous)?;
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "gateway_lifetime::tests::rotation_fixture",
                "--nocapture",
            ])
            .env("APPLE_BRIDGE_LOG_ROTATION_FIXTURE", home.path())
            .stdout(log.try_clone()?)
            .stderr(log)
            .status()?;
        assert!(output.success());
        let archive = std::fs::read(home.path().join("gateway.log.1"))?;
        let current = std::fs::read(&path)?;
        assert!(archive.starts_with(&previous));
        assert!(current.len() < usize::try_from(LOG_LIMIT)?);
        let all = [archive, current].concat();
        let text = String::from_utf8(all)?;
        for number in 0..100 {
            assert_eq!(
                text.matches(&format!("stdout fixture {number:03}\n"))
                    .count(),
                1
            );
            assert_eq!(
                text.matches(&format!("stderr fixture {number:03}\n"))
                    .count(),
                1
            );
        }
        assert!(text.contains("after rotation\n"));
        Ok(())
    }

    #[test]
    fn repeated_rotation_retains_only_one_archive() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("gateway.log");
        let log = open_log(&path)?;
        std::fs::write(&path, vec![b'x'; usize::try_from(LOG_LIMIT + 1)?])?;
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "gateway_lifetime::tests::rotation_fixture",
                "--nocapture",
            ])
            .env("APPLE_BRIDGE_LOG_ROTATION_FIXTURE", home.path())
            .env("APPLE_BRIDGE_LOG_ROTATION_TWICE", "1")
            .stdout(log.try_clone()?)
            .stderr(log)
            .status()?;
        assert!(output.success());
        let archive = std::fs::read(home.path().join("gateway.log.1"))?;
        assert!(
            archive
                .windows(b"second generation".len())
                .any(|bytes| bytes == b"second generation")
        );
        assert!(archive.len() < usize::try_from(LOG_LIMIT * 2)?);
        assert!(path.metadata()?.len() < LOG_LIMIT);
        assert!(!home.path().join("gateway.log.2").exists());
        Ok(())
    }

    #[tokio::test]
    async fn rotation_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let Some(home) = std::env::var_os("APPLE_BRIDGE_LOG_ROTATION_FIXTURE") else {
            return Ok(());
        };
        let home = PathBuf::from(home);
        let ownership = Arc::new(HomeLock::acquire(&home)?);
        let log = Log::inherited(&home)?.ok_or(Error::Invalid)?;
        let (stop, receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(run(log, receiver, ownership));
        let writer = std::thread::spawn(|| {
            for number in 0..100 {
                println!("stdout fixture {number:03}");
                eprintln!("stderr fixture {number:03}");
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        tokio::time::sleep(POLL * 2).await;
        writer.join().map_err(|_| Error::Invalid)?;
        if std::env::var_os("APPLE_BRIDGE_LOG_ROTATION_TWICE").is_some() {
            println!("second generation");
            std::io::stdout().write_all(&vec![b'y'; usize::try_from(LOG_LIMIT + 1)?])?;
            tokio::time::sleep(POLL * 2).await;
        }
        stop.send(true)?;
        task.await??;
        eprintln!("after rotation");
        Ok(())
    }

    #[test]
    fn replacement_open_failure_keeps_the_current_log_and_retries()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let log = open_log(&home.path().join("gateway.log"))?;
        std::fs::write(
            home.path().join("gateway.log"),
            vec![b'x'; usize::try_from(LOG_LIMIT + 1)?],
        )?;
        let result = std::process::Command::new("/bin/sh")
            .args(["-c", "ulimit -n 64; exec \"$@\"", "fixture"])
            .arg(std::env::current_exe()?)
            .args([
                "--exact",
                "gateway_lifetime::tests::open_failure_fixture",
                "--nocapture",
            ])
            .env("APPLE_BRIDGE_LOG_ROTATION_FIXTURE", home.path())
            .stdout(log.try_clone()?)
            .stderr(log)
            .status()?;
        assert!(result.success());
        assert!(
            std::fs::read_to_string(home.path().join("gateway.log"))?
                .contains("rotation recovered")
        );
        Ok(())
    }

    #[test]
    fn open_failure_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let Some(home) = std::env::var_os("APPLE_BRIDGE_LOG_ROTATION_FIXTURE") else {
            return Ok(());
        };
        let home = PathBuf::from(home);
        let _ownership = HomeLock::acquire(&home)?;
        let mut log = Log::inherited(&home)?.ok_or(Error::Invalid)?;
        let original = log.file.metadata()?;
        let mut exhausted = Vec::new();
        while let Ok(file) = File::open("/dev/null") {
            exhausted.push(file);
        }
        assert!(log.rotate().is_err());
        assert!(same_file(&original, &std::fs::metadata(&log.path)?));
        drop(exhausted);
        log.rotate()?;
        println!("rotation recovered");
        eprintln!("stderr recovered");
        let current = log.path.metadata()?;
        assert!(same_file(
            &current,
            &File::from(std::io::stdout().as_fd().try_clone_to_owned()?).metadata()?
        ));
        assert!(same_file(
            &current,
            &File::from(std::io::stderr().as_fd().try_clone_to_owned()?).metadata()?
        ));
        Ok(())
    }

    #[test]
    fn interrupted_rotation_recovers_tempfiles_and_both_descriptors()
    -> Result<(), Box<dyn std::error::Error>> {
        for mode in ["partial", "publish"] {
            let home = tempfile::tempdir()?;
            let log = open_log(&home.path().join("gateway.log"))?;
            std::fs::write(
                home.path().join("gateway.log"),
                vec![b'x'; usize::try_from(LOG_LIMIT + 1)?],
            )?;
            std::fs::write(home.path().join("gateway.log.next"), b"crash leftover\n")?;
            let result = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "gateway_lifetime::tests::interrupted_rotation_fixture",
                    "--nocapture",
                ])
                .env("APPLE_BRIDGE_LOG_ROTATION_FIXTURE", home.path())
                .env("APPLE_BRIDGE_LOG_ROTATION_FAILURE", mode)
                .stdout(log.try_clone()?)
                .stderr(log)
                .status()?;
            assert!(result.success(), "{mode}");
            let current = std::fs::read(home.path().join("gateway.log"))?;
            assert!(current.starts_with(b"crash leftover\n"));
            let all = [std::fs::read(home.path().join("gateway.log.1"))?, current].concat();
            let text = String::from_utf8(all)?;
            for line in [
                "before retry stdout",
                "before retry stderr",
                "after retry stdout",
                "after retry stderr",
            ] {
                assert_eq!(text.matches(line).count(), 1, "{mode}: {line}");
            }
            assert!(!home.path().join("gateway.log.next").exists());
        }
        Ok(())
    }

    #[test]
    fn interrupted_rotation_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let Some(home) = std::env::var_os("APPLE_BRIDGE_LOG_ROTATION_FIXTURE") else {
            return Ok(());
        };
        let home = PathBuf::from(home);
        let _ownership = HomeLock::acquire(&home)?;
        let mut log = Log::inherited(&home)?.ok_or(Error::Invalid)?;
        let original = log.file.metadata()?;
        let partial = std::env::var("APPLE_BRIDGE_LOG_ROTATION_FAILURE")? == "partial";
        let mut calls = 0;
        let result = log.rotate_with(
            |file| {
                calls += 1;
                if partial && calls == 2 {
                    ab_sys::redirect_stdout(file)?;
                    return Err(std::io::Error::other("injected stderr dup2 failure"));
                }
                if partial && calls == 3 {
                    return Err(std::io::Error::other("injected rollback failure"));
                }
                ab_sys::redirect_stdout(file)?;
                ab_sys::redirect_stderr(file)
            },
            |from, to| {
                if partial {
                    std::fs::rename(from, to)
                } else {
                    Err(std::io::Error::other("injected publish failure"))
                }
            },
        );
        assert!(result.is_err());
        assert!(same_file(&original, &log.path.metadata()?));
        println!("before retry stdout");
        eprintln!("before retry stderr");
        log.rotate()?;
        println!("after retry stdout");
        eprintln!("after retry stderr");
        let current = log.path.metadata()?;
        assert!(same_file(
            &current,
            &File::from(std::io::stdout().as_fd().try_clone_to_owned()?).metadata()?
        ));
        assert!(same_file(
            &current,
            &File::from(std::io::stderr().as_fd().try_clone_to_owned()?).metadata()?
        ));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rotation_error_reporting_on_a_full_disk_does_not_stop_the_task()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        std::fs::write(
            home.path().join("gateway.log"),
            vec![b'x'; usize::try_from(LOG_LIMIT + 1)?],
        )?;
        let result = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "gateway_lifetime::tests::full_disk_fixture",
                "--nocapture",
            ])
            .env("APPLE_BRIDGE_LOG_ROTATION_FIXTURE", home.path())
            .stdout(std::process::Stdio::null())
            .stderr(OpenOptions::new().write(true).open("/dev/full")?)
            .status()?;
        assert!(result.success());
        Ok(())
    }

    #[tokio::test]
    async fn full_disk_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let Some(home) = std::env::var_os("APPLE_BRIDGE_LOG_ROTATION_FIXTURE") else {
            return Ok(());
        };
        let home = PathBuf::from(home);
        let ownership = Arc::new(HomeLock::acquire(&home)?);
        let log = Log {
            path: home.join("missing"),
            file: File::open(home.join("gateway.log"))?,
        };
        let (stop, receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(run(log, receiver, ownership));
        tokio::time::sleep(POLL * 2).await;
        assert!(
            !task.is_finished(),
            "rotation error reporting stopped the supervisor"
        );
        stop.send(true)?;
        task.await??;
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
}
