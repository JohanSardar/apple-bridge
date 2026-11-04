use std::{
    collections::{HashMap, HashSet},
    ffi::CString,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub structural: bool,
}

fn paths(root: &Path, directories_only: bool, out: &mut Vec<PathBuf>) {
    let Ok(metadata) = std::fs::symlink_metadata(root) else {
        return;
    };
    if metadata.is_dir() {
        out.push(root.to_path_buf());
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                paths(&entry.path(), directories_only, out);
            }
        }
    } else if metadata.is_file() && !directories_only {
        out.push(root.to_path_buf());
    }
}

#[cfg(target_os = "linux")]
fn millis(timeout: Duration) -> i32 {
    i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX)
}

fn cstring(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct Entry {
    dev: u64,
    ino: u64,
    directory: bool,
    fd: OwnedFd,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct Watcher {
    queue: OwnedFd,
    files: HashMap<PathBuf, Entry>,
    idents: HashMap<usize, PathBuf>,
}

#[cfg(target_os = "macos")]
impl Watcher {
    #[allow(unsafe_code)]
    pub fn new() -> io::Result<Self> {
        // SAFETY: kqueue takes no arguments and returns a fresh descriptor or -1.
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: kqueue returned this descriptor and nothing else owns it.
        let queue = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            queue,
            files: HashMap::new(),
            idents: HashMap::new(),
        })
    }

    #[allow(unsafe_code)]
    fn register(&mut self, path: &Path) -> io::Result<()> {
        let c = cstring(path)?;
        // SAFETY: c is a live NUL-terminated string for the duration of the call.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: open returned this descriptor and nothing else owns it.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        let identity = std::fs::File::from(owned.try_clone()?).metadata()?;
        let ident = usize::try_from(fd).map_err(|_| io::ErrorKind::InvalidInput)?;
        let change = libc::kevent {
            ident,
            filter: libc::EVFILT_VNODE,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: libc::NOTE_WRITE
                | libc::NOTE_DELETE
                | libc::NOTE_RENAME
                | libc::NOTE_EXTEND
                | libc::NOTE_ATTRIB
                | libc::NOTE_LINK
                | libc::NOTE_REVOKE,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: change points to one live kevent and no output list is requested.
        let result = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                &raw const change,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        self.idents.insert(ident, path.to_path_buf());
        self.files.insert(
            path.to_path_buf(),
            Entry {
                dev: identity.dev(),
                ino: identity.ino(),
                directory: identity.is_dir(),
                fd: owned,
            },
        );
        Ok(())
    }

    pub fn watch_tree(&mut self, root: &Path) -> io::Result<bool> {
        let mut found = Vec::new();
        paths(root, false, &mut found);
        let present: HashSet<&Path> = found.iter().map(PathBuf::as_path).collect();
        let mut dropped = Vec::new();
        self.files.retain(|path, entry| {
            let keep = present.contains(path.as_path())
                && std::fs::symlink_metadata(path)
                    .is_ok_and(|current| current.dev() == entry.dev && current.ino() == entry.ino);
            if !keep {
                dropped.push(entry.fd.as_raw_fd());
            }
            keep
        });
        for fd in dropped {
            if let Ok(ident) = usize::try_from(fd) {
                self.idents.remove(&ident);
            }
        }
        let mut grew = false;
        let mut failure = None;
        for path in &found {
            if self.files.contains_key(path) {
                continue;
            }
            match self.register(path) {
                Ok(()) => grew = true,
                Err(error) => failure = failure.or(Some(error)),
            }
        }
        failure.map_or(Ok(grew), Err)
    }

    #[allow(unsafe_code)]
    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
        Ok(!self.poll(timeout)?.is_empty())
    }

    #[allow(unsafe_code)]
    pub fn poll(&mut self, timeout: Duration) -> io::Result<Vec<Change>> {
        let limit = libc::timespec {
            tv_sec: libc::time_t::try_from(timeout.as_secs()).unwrap_or(libc::time_t::MAX),
            tv_nsec: libc::c_long::from(timeout.subsec_nanos()),
        };
        let mut events = [libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        }; 16];
        // SAFETY: events is a live buffer of the advertised length and limit outlives the call.
        let count = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                std::ptr::null(),
                0,
                events.as_mut_ptr(),
                16,
                &raw const limit,
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(Vec::new())
            } else {
                Err(error)
            };
        }
        let count = usize::try_from(count).unwrap_or(0);
        let mut changes = Vec::with_capacity(count);
        for event in &events[..count] {
            let (ident, flags) = (event.ident, event.fflags);
            let Some(path) = self.idents.get(&ident) else {
                continue;
            };
            let directory = self.files.get(path).is_some_and(|entry| entry.directory);
            let replaced = flags
                & (libc::NOTE_DELETE | libc::NOTE_RENAME | libc::NOTE_REVOKE | libc::NOTE_LINK)
                != 0;
            let entries = directory && flags & libc::NOTE_WRITE != 0;
            changes.push(Change {
                path: path.clone(),
                structural: replaced || entries,
            });
        }
        Ok(changes)
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct Watcher {
    fd: OwnedFd,
    watched: HashMap<PathBuf, i32>,
    descriptors: HashMap<i32, PathBuf>,
}

#[cfg(target_os = "linux")]
impl Watcher {
    #[allow(unsafe_code)]
    pub fn new() -> io::Result<Self> {
        // SAFETY: inotify_init1 takes only flags and returns a fresh descriptor or -1.
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            // SAFETY: inotify_init1 returned this descriptor and nothing else owns it.
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
            watched: HashMap::new(),
            descriptors: HashMap::new(),
        })
    }

    #[allow(unsafe_code)]
    pub fn watch_tree(&mut self, root: &Path) -> io::Result<bool> {
        let mut found = Vec::new();
        let single = std::fs::symlink_metadata(root).is_ok_and(|m| m.is_file());
        if single {
            found.push(root.to_path_buf());
        } else {
            paths(root, true, &mut found);
        }
        let present: HashSet<&Path> = found.iter().map(PathBuf::as_path).collect();
        let descriptors = &mut self.descriptors;
        self.watched.retain(|path, wd| {
            let keep = present.contains(path.as_path());
            if !keep {
                descriptors.remove(wd);
            }
            keep
        });
        let mut grew = false;
        let mut failure = None;
        for path in found {
            if self.watched.contains_key(&path) {
                continue;
            }
            let c = match cstring(&path) {
                Ok(c) => c,
                Err(error) => {
                    failure = failure.or(Some(error));
                    continue;
                }
            };
            // SAFETY: c is a live NUL-terminated string and fd is a live inotify descriptor.
            let wd = unsafe {
                libc::inotify_add_watch(
                    self.fd.as_raw_fd(),
                    c.as_ptr(),
                    libc::IN_CREATE
                        | libc::IN_MODIFY
                        | libc::IN_CLOSE_WRITE
                        | libc::IN_MOVED_TO
                        | libc::IN_MOVED_FROM
                        | libc::IN_DELETE
                        | libc::IN_DELETE_SELF
                        | libc::IN_MOVE_SELF
                        | libc::IN_ATTRIB,
                )
            };
            if wd < 0 {
                failure = failure.or(Some(io::Error::last_os_error()));
                continue;
            }
            self.descriptors.insert(wd, path.clone());
            self.watched.insert(path, wd);
            grew = true;
        }
        failure.map_or(Ok(grew), Err)
    }

    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
        Ok(!self.poll(timeout)?.is_empty())
    }

    #[allow(unsafe_code)]
    pub fn poll(&mut self, timeout: Duration) -> io::Result<Vec<Change>> {
        let mut pollfd = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd is one live entry and the count matches.
        let ready = unsafe { libc::poll(&raw mut pollfd, 1, millis(timeout)) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::Interrupted {
                Ok(Vec::new())
            } else {
                Err(error)
            };
        }
        if ready == 0 {
            return Ok(Vec::new());
        }
        let header = std::mem::size_of::<libc::inotify_event>();
        let structural = libc::IN_CREATE
            | libc::IN_MOVED_TO
            | libc::IN_MOVED_FROM
            | libc::IN_DELETE
            | libc::IN_DELETE_SELF
            | libc::IN_MOVE_SELF
            | libc::IN_IGNORED
            | libc::IN_Q_OVERFLOW;
        let mut changes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            // SAFETY: buffer is a live writable region of the advertised length.
            let read = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            let Ok(read) = usize::try_from(read) else {
                break;
            };
            if read == 0 {
                break;
            }
            let mut offset = 0;
            while offset + header <= read {
                // SAFETY: the kernel wrote a whole inotify_event header at this offset inside buffer.
                let event = unsafe {
                    std::ptr::read_unaligned(
                        buffer.as_ptr().add(offset).cast::<libc::inotify_event>(),
                    )
                };
                offset += header + usize::try_from(event.len).unwrap_or(0);
                let path = self.descriptors.get(&event.wd).cloned().unwrap_or_default();
                if event.mask & libc::IN_IGNORED != 0 {
                    self.descriptors.remove(&event.wd);
                    if self.watched.get(&path) == Some(&event.wd) {
                        self.watched.remove(&path);
                    }
                }
                changes.push(Change {
                    path,
                    structural: event.mask & structural != 0,
                });
            }
        }
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::Watcher;
    use std::{fs, io, io::Write, path::Path, time::Duration};

    fn settle(watcher: &mut Watcher) -> io::Result<()> {
        while watcher.wait(Duration::from_millis(50))? {}
        Ok(())
    }

    fn woke(watcher: &mut Watcher) -> io::Result<bool> {
        watcher.wait(Duration::from_secs(3))
    }

    fn scratch(name: &str) -> io::Result<std::path::PathBuf> {
        let dir = std::env::temp_dir().join(format!("ab-sys-watch-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn replace(dir: &Path, name: &str, body: &str) -> io::Result<()> {
        let staging = dir.join(format!("{name}.tmp"));
        fs::write(&staging, body)?;
        fs::rename(&staging, dir.join(name))?;
        Ok(())
    }

    #[test]
    fn wakes_on_new_file_and_in_place_write() -> io::Result<()> {
        let dir = scratch("create")?;
        fs::write(dir.join("a.db"), "one")?;
        let mut watcher = Watcher::new()?;
        watcher.watch_tree(&dir)?;
        settle(&mut watcher)?;
        fs::write(dir.join("b.db"), "two")?;
        assert!(woke(&mut watcher)?);
        watcher.watch_tree(&dir)?;
        settle(&mut watcher)?;
        let mut file = fs::OpenOptions::new().append(true).open(dir.join("a.db"))?;
        file.write_all(b"more")?;
        file.sync_all()?;
        assert!(woke(&mut watcher)?);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn follows_atomic_rename_replacement() -> io::Result<()> {
        let dir = scratch("rename")?;
        replace(&dir, "sync.db", "one")?;
        let mut watcher = Watcher::new()?;
        watcher.watch_tree(&dir)?;
        settle(&mut watcher)?;
        replace(&dir, "sync.db", "two")?;
        assert!(woke(&mut watcher)?);
        watcher.watch_tree(&dir)?;
        settle(&mut watcher)?;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(dir.join("sync.db"))?;
        file.write_all(b"three")?;
        file.sync_all()?;
        assert!(woke(&mut watcher)?);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn wakes_on_nested_directory_change() -> io::Result<()> {
        let dir = scratch("nested")?;
        fs::create_dir_all(dir.join("sub/deep"))?;
        let mut watcher = Watcher::new()?;
        watcher.watch_tree(&dir)?;
        settle(&mut watcher)?;
        fs::write(dir.join("sub/deep/segment"), "x")?;
        assert!(woke(&mut watcher)?);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn separates_structure_from_content_and_skips_settled_walks() -> io::Result<()> {
        let dir = scratch("structure")?;
        fs::write(dir.join("a.db"), "one")?;
        let mut watcher = Watcher::new()?;
        assert!(watcher.watch_tree(&dir)?);
        assert!(!watcher.watch_tree(&dir)?);
        settle(&mut watcher)?;
        let mut file = fs::OpenOptions::new().append(true).open(dir.join("a.db"))?;
        file.write_all(b"more")?;
        file.sync_all()?;
        let changes = watcher.poll(Duration::from_secs(3))?;
        assert_ne!(changes.len(), 0);
        assert!(changes.iter().all(|change| !change.structural));
        settle(&mut watcher)?;
        fs::create_dir(dir.join("sub"))?;
        let changes = watcher.poll(Duration::from_secs(3))?;
        assert!(changes.iter().any(|change| change.structural));
        assert!(watcher.watch_tree(&dir)?);
        assert!(!watcher.watch_tree(&dir)?);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn watches_a_single_file_root() -> io::Result<()> {
        let dir = scratch("single")?;
        let path = dir.join("one.db");
        fs::write(&path, "one")?;
        let mut watcher = Watcher::new()?;
        watcher.watch_tree(&path)?;
        settle(&mut watcher)?;
        let mut file = fs::OpenOptions::new().append(true).open(&path)?;
        file.write_all(b"two")?;
        file.sync_all()?;
        assert!(woke(&mut watcher)?);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
