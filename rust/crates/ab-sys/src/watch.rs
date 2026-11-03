use std::{
    collections::BTreeMap,
    ffi::CString,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Path, PathBuf},
    time::Duration,
};

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
pub struct Watcher {
    queue: OwnedFd,
    files: BTreeMap<PathBuf, OwnedFd>,
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
            files: BTreeMap::new(),
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
        let change = libc::kevent {
            ident: usize::try_from(fd).map_err(|_| io::ErrorKind::InvalidInput)?,
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
        self.files.insert(path.to_path_buf(), owned);
        Ok(())
    }

    pub fn watch_tree(&mut self, root: &Path) -> io::Result<()> {
        let mut found = Vec::new();
        paths(root, false, &mut found);
        self.files.retain(|path, _| found.contains(path));
        for path in found {
            if !self.files.contains_key(&path) {
                self.register(&path)?;
            }
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
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
                Ok(false)
            } else {
                Err(error)
            };
        }
        Ok(count > 0)
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct Watcher {
    fd: OwnedFd,
    watched: BTreeMap<PathBuf, i32>,
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
            watched: BTreeMap::new(),
        })
    }

    #[allow(unsafe_code)]
    pub fn watch_tree(&mut self, root: &Path) -> io::Result<()> {
        let mut found = Vec::new();
        let single = std::fs::symlink_metadata(root).is_ok_and(|m| m.is_file());
        if single {
            found.push(root.to_path_buf());
        } else {
            paths(root, true, &mut found);
        }
        self.watched.retain(|path, _| found.contains(path));
        for path in found {
            if self.watched.contains_key(&path) {
                continue;
            }
            let c = cstring(&path)?;
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
                return Err(io::Error::last_os_error());
            }
            self.watched.insert(path, wd);
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
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
                Ok(false)
            } else {
                Err(error)
            };
        }
        if ready == 0 {
            return Ok(false);
        }
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
            if read <= 0 {
                break;
            }
        }
        Ok(true)
    }
}
