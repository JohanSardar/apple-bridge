use crate::Error;
use std::os::unix::process::CommandExt;
use std::{
    fs,
    io::Read,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const PACK_VERSION: &str = "1";
pub const MAX_BYTES: usize = 16 << 20;
const PACK_NAMES: [&str; 4] = ["AB Echo", "AB Clock", "AB Focus", "AB Pack v1"];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Entry {
    pub name: String,
    pub identifier: String,
}

#[derive(Debug, Clone, Default)]
pub struct Input {
    pub shortcut: String,
    pub data: Option<Vec<u8>>,
    pub timeout: Duration,
    pub output_type: String,
    pub output_limit: usize,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub data: Vec<u8>,
    pub exit_status: i32,
    pub timed_out: bool,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            exit_status: -1,
            timed_out: false,
        }
    }
}

pub trait Runner {
    fn run(&self, input: &Input) -> Result<Output, Error>;
}

#[derive(Debug, Clone, Default)]
pub struct Cli {
    pub executable: Option<PathBuf>,
}

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

fn uuid_suffix(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let total = line.len();
    if total < 39 || line[total - 1] != b')' {
        return None;
    }
    let head = &line[..total - 39];
    let tail = &line[total - 39..];
    if &tail[..2] != b" (" {
        return None;
    }
    let id = &tail[2..38];
    let valid = id.iter().enumerate().all(|(i, byte)| {
        if matches!(i, 8 | 13 | 18 | 23) {
            *byte == b'-'
        } else {
            is_hex(*byte)
        }
    });
    valid.then_some((head, id))
}

pub fn parse_inventory(raw: &[u8]) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let body = raw.strip_suffix(b"\n").unwrap_or(raw);
    for line in body.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Some((name, id)) = uuid_suffix(line).filter(|(name, _)| !name.is_empty()) else {
            return Err(Error::InvalidInventory);
        };
        let id = String::from_utf8_lossy(id).into_owned();
        if !seen.insert(id.to_lowercase()) {
            return Err(Error::InvalidInventory);
        }
        entries.push(Entry {
            name: String::from_utf8_lossy(name).into_owned(),
            identifier: id,
        });
    }
    Ok(entries)
}

fn drain(
    mut stream: impl Read + Send + 'static,
    keep: bool,
    exceeded: Arc<AtomicBool>,
    pid: u32,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut data = Vec::new();
        let mut total = 0usize;
        let mut buffer = vec![0u8; 65536];
        while let Ok(count) = stream.read(&mut buffer) {
            if count == 0 {
                break;
            }
            total += count;
            if total > MAX_BYTES {
                exceeded.store(true, Ordering::SeqCst);
                ab_sys::kill_process_group(pid);
                break;
            }
            if keep {
                data.extend_from_slice(&buffer[..count]);
            }
        }
        data
    })
}

fn temporary_directory() -> Result<PathBuf, Error> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "ab-shortcut-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::message("temporary directory unavailable"))
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Cli {
    fn executable(&self) -> &Path {
        self.executable
            .as_deref()
            .unwrap_or_else(|| Path::new("/usr/bin/shortcuts"))
    }

    fn spawn(&self, args: &[String]) -> Result<std::process::Child, Error> {
        Command::new(self.executable())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|_| Error::ProcessUnavailable)
    }

    pub fn inventory(&self, folders: bool, folder: &str) -> Result<Vec<Entry>, Error> {
        let mut args = vec!["list".to_owned(), "--show-identifiers".to_owned()];
        if folders {
            args.push("--folders".into());
        }
        if !folder.is_empty() {
            args.push("--folder-name".into());
            args.push(folder.to_owned());
        }
        let unavailable = || Error::InventoryUnavailable;
        let mut child = self.spawn(&args).map_err(|_| unavailable())?;
        let pid = child.id();
        let exceeded = Arc::new(AtomicBool::new(false));
        let stdout = child.stdout.take().ok_or_else(unavailable)?;
        let reader = drain(stdout, true, exceeded.clone(), pid);
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline || exceeded.load(Ordering::SeqCst) {
                ab_sys::kill_process_group(pid);
                let _ = child.wait();
                break None;
            }
            thread::sleep(Duration::from_millis(2));
        };
        ab_sys::kill_process_group(pid);
        let data = reader.join().unwrap_or_default();
        match status {
            Some(status) if status.success() && !exceeded.load(Ordering::SeqCst) => {
                parse_inventory(&data)
            }
            _ => Err(unavailable()),
        }
    }

    #[must_use]
    pub fn run_detailed(&self, input: &Input) -> (Output, Option<Error>) {
        let mut result = Output::default();
        match self.run_inner(input, &mut result) {
            Ok(()) => (result, None),
            Err(error) => (result, Some(error)),
        }
    }

    fn run_inner(&self, input: &Input, result: &mut Output) -> Result<(), Error> {
        let timeout = input.timeout;
        if input.shortcut.trim().is_empty()
            || input.shortcut.starts_with('-')
            || input.data.as_ref().is_some_and(|d| d.len() > MAX_BYTES)
            || timeout > Duration::from_secs(300)
        {
            return Err(Error::InvalidInput);
        }
        let timeout = if timeout.is_zero() {
            Duration::from_secs(30)
        } else {
            timeout
        };
        let limit = if input.output_limit == 0 {
            MAX_BYTES
        } else {
            input.output_limit.min(MAX_BYTES)
        };
        let deadline = Instant::now() + timeout;
        let scratch = Scratch(temporary_directory()?);
        let output = scratch.0.join("output");
        let mut args = vec![
            "run".to_owned(),
            input.shortcut.clone(),
            "--output-path".into(),
            output.to_string_lossy().into_owned(),
        ];
        if let Some(data) = &input.data {
            let path = scratch.0.join("input");
            write_private(&path, data)?;
            args.push("--input-path".into());
            args.push(path.to_string_lossy().into_owned());
        }
        if !input.output_type.is_empty() {
            args.push("--output-type".into());
            args.push(input.output_type.clone());
        }
        let mut child = self.spawn(&args)?;
        let pid = child.id();
        let exceeded = Arc::new(AtomicBool::new(false));
        let stdout = child.stdout.take().ok_or(Error::ProcessUnavailable)?;
        let reader = drain(stdout, false, exceeded.clone(), pid);
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if let Ok(info) = fs::symlink_metadata(&output)
                && (!info.is_file() || info.len() > limit as u64)
            {
                exceeded.store(true, Ordering::SeqCst);
            }
            if exceeded.load(Ordering::SeqCst) {
                ab_sys::kill_process_group(pid);
                break child.wait()?;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                ab_sys::kill_process_group(pid);
                break child.wait()?;
            }
            thread::sleep(Duration::from_millis(5));
        };
        ab_sys::kill_process_group(pid);
        let _ = reader.join();
        if exceeded.load(Ordering::SeqCst) {
            return Err(Error::OutputExceeded);
        }
        result.timed_out = timed_out;
        result.exit_status = status.code().unwrap_or(-1);
        if timed_out {
            return Err(Error::DeadlineExceeded);
        }
        if !status.success() {
            return Ok(());
        }
        let info = match fs::symlink_metadata(&output) {
            Ok(info) => info,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if !info.is_file() || info.len() > limit as u64 {
            return Err(Error::InvalidOutput);
        }
        let mut data = Vec::new();
        fs::File::open(&output)?
            .take(limit as u64 + 1)
            .read_to_end(&mut data)?;
        if data.len() > limit {
            return Err(Error::OutputExceeded);
        }
        result.data = data;
        Ok(())
    }
}

fn write_private(path: &Path, data: &[u8]) -> Result<(), Error> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(data)?;
    Ok(())
}

impl Runner for Cli {
    fn run(&self, input: &Input) -> Result<Output, Error> {
        match self.run_detailed(input) {
            (output, None) => Ok(output),
            (_, Some(error)) => Err(error),
        }
    }
}

pub fn open_pack(directory: &Path) -> Result<(), Error> {
    open_pack_with(Path::new("/usr/bin/open"), directory)
}

pub fn open_pack_with(program: &Path, directory: &Path) -> Result<(), Error> {
    let mut paths = Vec::new();
    for name in PACK_NAMES {
        let path = std::path::absolute(directory.join(format!("{name}.shortcut")))?;
        let mut header = [0u8; 4];
        let count = fs::File::open(&path)?.read(&mut header)?;
        if count != 4 || &header != b"AEA1" {
            return Err(Error::Message(format!(
                "pack requires signed {name}.shortcut"
            )));
        }
        paths.push(path);
    }
    let status = Command::new(program).args(&paths).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::message("open failed"))
    }
}

#[cfg(test)]
mod install_pack_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    type Check = Result<(), Box<dyn std::error::Error>>;

    fn fixture(
        signed: bool,
    ) -> Result<(tempfile::TempDir, PathBuf, PathBuf), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let pack = dir.path().join("pack");
        fs::create_dir(&pack)?;
        for name in PACK_NAMES {
            fs::write(
                pack.join(format!("{name}.shortcut")),
                if signed {
                    &b"AEA1payload"[..]
                } else {
                    &b"unsigned"[..]
                },
            )?;
        }
        let log = dir.path().join("argv");
        let program = dir.path().join("open");
        fs::write(
            &program,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", log.display()),
        )?;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755))?;
        Ok((dir, pack, program))
    }

    #[test]
    fn install_pack_opens_every_signed_file() -> Check {
        let (dir, pack, program) = fixture(true)?;
        open_pack_with(&program, &pack)?;
        let opened = fs::read_to_string(dir.path().join("argv"))?;
        let lines: Vec<&str> = opened.lines().collect();
        assert_eq!(lines.len(), PACK_NAMES.len());
        for (line, name) in lines.iter().zip(PACK_NAMES) {
            assert!(line.starts_with('/'));
            assert!(line.ends_with(&format!("{name}.shortcut")));
        }
        Ok(())
    }

    #[test]
    fn install_pack_rejects_unsigned_and_missing_files() -> Check {
        let (dir, pack, program) = fixture(false)?;
        assert!(open_pack_with(&program, &pack).is_err());
        assert!(!dir.path().join("argv").exists());
        let (_dir, pack, program) = fixture(true)?;
        fs::remove_file(pack.join(format!("{}.shortcut", PACK_NAMES[3])))?;
        assert!(open_pack_with(&program, &pack).is_err());
        Ok(())
    }
}
