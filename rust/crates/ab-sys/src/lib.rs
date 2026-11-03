#![deny(unsafe_code)]

use std::{
    io,
    os::{fd::FromRawFd, unix::net::UnixStream},
};

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
