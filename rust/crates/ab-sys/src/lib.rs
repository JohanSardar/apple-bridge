#![deny(unsafe_code)]

pub mod keychain;
pub mod watch;
pub mod xattr;

use std::{
    io,
    os::{fd::FromRawFd, unix::net::UnixStream},
};

#[allow(unsafe_code)]
#[must_use]
pub fn user_id() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[allow(unsafe_code)]
pub fn raise_open_files(wanted: u64) {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit and setrlimit read and write one live rlimit.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) != 0 || limit.rlim_cur >= wanted {
            return;
        }
        limit.rlim_cur = wanted.min(limit.rlim_max);
        libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit);
    }
}

#[allow(unsafe_code)]
pub fn inherited_unix_stream(fd: i32) -> io::Result<UnixStream> {
    if fd < 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid inherited fd",
        ));
    }
    // SAFETY: fcntl inspects the integer descriptor without dereferencing memory.
    let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicated < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl returned a fresh descriptor; this is its only owning wrapper.
    let stream = unsafe { UnixStream::from_raw_fd(duplicated) };
    stream.peer_addr()?;
    // SAFETY: the inherited original is no longer needed; only its duplicate is exposed.
    unsafe { libc::close(fd) };
    Ok(stream)
}

#[allow(unsafe_code)]
pub fn redirect_stdout(file: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: dup2 duplicates a live borrowed descriptor onto descriptor 1 without dereferencing memory.
    if unsafe { libc::dup2(file.as_raw_fd(), 1) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
pub fn redirect_stderr(file: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: dup2 duplicates a live borrowed descriptor onto descriptor 2 without dereferencing memory.
    if unsafe { libc::dup2(file.as_raw_fd(), 2) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[allow(unsafe_code)]
pub fn lock_guard(file: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: flock uses a live borrowed descriptor and does not take ownership.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn open_guard(path: &std::path::Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
        || metadata.uid() != effective_uid()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid guard file",
        ));
    }
    lock_guard(&file)?;
    Ok(file)
}

#[allow(unsafe_code)]
fn effective_uid() -> u32 {
    // SAFETY: geteuid has no arguments or memory effects.
    unsafe { libc::geteuid() }
}

#[allow(unsafe_code)]
pub fn kill_process_group(pid: u32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    if pid <= 1 {
        return;
    }
    // SAFETY: kill takes integer arguments only and has no memory effects.
    unsafe { libc::kill(-pid, libc::SIGKILL) };
}

#[allow(unsafe_code)]
pub fn reset_signals() {
    // SAFETY: signal, sigemptyset and pthread_sigmask receive valid constants and a local set.
    unsafe {
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
            libc::signal(signal, libc::SIG_DFL);
        }
        let mut empty: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&raw mut empty);
        libc::pthread_sigmask(libc::SIG_SETMASK, &raw const empty, std::ptr::null_mut());
    }
}

pub const O_NOFOLLOW: i32 = libc::O_NOFOLLOW;
pub const O_NONBLOCK: i32 = libc::O_NONBLOCK;

#[allow(unsafe_code)]
pub fn rename_exclusive(from: &std::path::Path, to: &std::path::Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from =
        CString::new(from.as_os_str().as_bytes()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let to = CString::new(to.as_os_str().as_bytes()).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: both paths are valid NUL-terminated strings for the duration of the call.
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    // SAFETY: both paths are valid NUL-terminated strings for the duration of the call.
    #[cfg(not(target_os = "macos"))]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
