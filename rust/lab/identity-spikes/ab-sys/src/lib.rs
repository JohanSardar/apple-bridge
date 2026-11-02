pub fn parent_pid() -> i32 {
    unsafe { libc::getppid() }
}

pub fn responsible_pid() -> Option<i32> {
    unsafe {
        let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"responsibility_get_pid_responsible_for_pid".as_ptr());
        if symbol.is_null() {
            return None;
        }
        let function: unsafe extern "C" fn(i32) -> i32 = std::mem::transmute(symbol);
        Some(function(libc::getpid()))
    }
}

pub fn write_report(fd: i32, bytes: &[u8]) -> std::io::Result<()> {
    let mut rest = bytes;
    while !rest.is_empty() {
        let count = unsafe { libc::write(fd, rest.as_ptr().cast(), rest.len()) };
        if count < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if count == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        rest = &rest[count as usize..];
    }
    Ok(())
}
