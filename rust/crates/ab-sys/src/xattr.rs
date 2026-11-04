use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::Path};

#[allow(unsafe_code)]
fn call(path: &CString, name: &CString, buffer: *mut libc::c_void, size: usize) -> isize {
    #[cfg(target_os = "macos")]
    // SAFETY: NUL-terminated names live for the call; the buffer is null with size 0 or writable for size bytes.
    unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr(),
            buffer,
            size,
            0,
            libc::XATTR_NOFOLLOW,
        )
    }
    #[cfg(not(target_os = "macos"))]
    // SAFETY: NUL-terminated names live for the call; the buffer is null with size 0 or writable for size bytes.
    unsafe {
        libc::lgetxattr(path.as_ptr(), name.as_ptr(), buffer, size)
    }
}

#[cfg(target_os = "macos")]
const NO_ATTRIBUTE: i32 = libc::ENOATTR;
#[cfg(not(target_os = "macos"))]
const NO_ATTRIBUTE: i32 = libc::ENODATA;

fn absent(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(code) if code == NO_ATTRIBUTE || code == libc::ENOTSUP)
}

pub fn get(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    loop {
        let size = call(&path, &name, std::ptr::null_mut(), 0);
        if size < 0 {
            let error = io::Error::last_os_error();
            return if absent(&error) { Ok(None) } else { Err(error) };
        }
        let mut buffer = vec![0_u8; usize::try_from(size).map_err(|_| io::ErrorKind::InvalidData)?];
        let read = call(&path, &name, buffer.as_mut_ptr().cast(), buffer.len());
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ERANGE) {
                continue;
            }
            return if absent(&error) { Ok(None) } else { Err(error) };
        }
        buffer.truncate(usize::try_from(read).map_err(|_| io::ErrorKind::InvalidData)?);
        return Ok(Some(buffer));
    }
}
