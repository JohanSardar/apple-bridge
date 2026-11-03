#![allow(unsafe_code)]

use std::path::Path;

pub const NOT_FOUND: i32 = -25300;
pub const AUTH_FAILED: i32 = -25293;
pub const INTERACTION_NOT_ALLOWED: i32 = -25308;
pub const UNSUPPORTED: i32 = -4;

#[cfg(target_os = "macos")]
mod ffi {
    use std::ffi::{c_char, c_void};

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        pub fn SecKeychainSetUserInteractionAllowed(allowed: bool) -> i32;
        pub fn SecKeychainOpen(path: *const c_char, keychain: *mut *mut c_void) -> i32;
        pub fn SecKeychainFindGenericPassword(
            keychain: *mut c_void,
            service_length: u32,
            service: *const c_char,
            account_length: u32,
            account: *const c_char,
            password_length: *mut u32,
            password: *mut *mut c_void,
            item: *mut *mut c_void,
        ) -> i32;
        pub fn SecKeychainAddGenericPassword(
            keychain: *mut c_void,
            service_length: u32,
            service: *const c_char,
            account_length: u32,
            account: *const c_char,
            password_length: u32,
            password: *const c_void,
            item: *mut *mut c_void,
        ) -> i32;
        pub fn SecKeychainItemDelete(item: *mut c_void) -> i32;
        pub fn SecKeychainItemFreeContent(attributes: *mut c_void, data: *mut c_void) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFRelease(reference: *const c_void);
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::ffi;
    use std::{
        ffi::{CString, c_void},
        os::unix::ffi::OsStrExt,
        path::Path,
        ptr,
    };

    struct Handle(*mut c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer is a retained CoreFoundation reference returned by Security.
                unsafe { ffi::CFRelease(self.0) };
            }
        }
    }

    pub struct Entry {
        keychain: Handle,
        service: String,
        account: String,
    }

    pub fn open(path: &Path, service: &str, account: &str) -> Result<Entry, i32> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| -50)?;
        // SAFETY: the call takes a plain boolean and no pointers.
        let code = unsafe { ffi::SecKeychainSetUserInteractionAllowed(false) };
        if code != 0 {
            return Err(code);
        }
        let mut keychain = ptr::null_mut();
        // SAFETY: path is a NUL-terminated string and keychain is a valid out pointer.
        let code = unsafe { ffi::SecKeychainOpen(path.as_ptr(), &raw mut keychain) };
        let keychain = Handle(keychain);
        if code != 0 {
            return Err(code);
        }
        Ok(Entry {
            keychain,
            service: service.to_owned(),
            account: account.to_owned(),
        })
    }

    fn length(text: &str) -> Result<u32, i32> {
        u32::try_from(text.len()).map_err(|_| -50)
    }

    impl Entry {
        fn find(&self, want_data: bool) -> Result<(Option<Vec<u8>>, Handle), i32> {
            let mut size = 0u32;
            let mut data = ptr::null_mut();
            let mut item = ptr::null_mut();
            // SAFETY: service and account buffers outlive the call; all out pointers are valid.
            let code = unsafe {
                ffi::SecKeychainFindGenericPassword(
                    self.keychain.0,
                    length(&self.service)?,
                    self.service.as_ptr().cast(),
                    length(&self.account)?,
                    self.account.as_ptr().cast(),
                    &raw mut size,
                    &raw mut data,
                    &raw mut item,
                )
            };
            let item = Handle(item);
            let value = if code == 0 && want_data && !data.is_null() {
                // SAFETY: Security returned a buffer of exactly size bytes.
                let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }
                    .to_vec();
                Some(bytes)
            } else {
                None
            };
            if !data.is_null() {
                // SAFETY: data was allocated by SecKeychainFindGenericPassword.
                unsafe { ffi::SecKeychainItemFreeContent(ptr::null_mut(), data) };
            }
            if code == 0 {
                Ok((value, item))
            } else {
                Err(code)
            }
        }

        pub fn get(&self) -> Result<Vec<u8>, i32> {
            self.find(true).map(|(value, _)| value.unwrap_or_default())
        }

        fn add(&self, value: &[u8]) -> Result<(), i32> {
            let size = u32::try_from(value.len()).map_err(|_| -50)?;
            let mut item = ptr::null_mut();
            // SAFETY: all buffers outlive the call and item is a valid out pointer.
            let code = unsafe {
                ffi::SecKeychainAddGenericPassword(
                    self.keychain.0,
                    length(&self.service)?,
                    self.service.as_ptr().cast(),
                    length(&self.account)?,
                    self.account.as_ptr().cast(),
                    size,
                    value.as_ptr().cast(),
                    &raw mut item,
                )
            };
            drop(Handle(item));
            if code == 0 { Ok(()) } else { Err(code) }
        }

        pub fn put(&self, value: &[u8]) -> Result<(), i32> {
            match self.find(false) {
                Ok((_, item)) => {
                    // SAFETY: item is a live keychain item.
                    let code = unsafe { ffi::SecKeychainItemDelete(item.0) };
                    if code != 0 {
                        return Err(code);
                    }
                    self.add(value)
                }
                Err(super::NOT_FOUND) => self.add(value),
                Err(code) => Err(code),
            }
        }

        pub fn delete(&self) -> Result<(), i32> {
            let (_, item) = self.find(false)?;
            // SAFETY: item is a live keychain item.
            let code = unsafe { ffi::SecKeychainItemDelete(item.0) };
            if code == 0 { Ok(()) } else { Err(code) }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn get(keychain: &Path, service: &str, account: &str) -> Result<Vec<u8>, i32> {
    imp::open(keychain, service, account)?.get()
}

#[cfg(target_os = "macos")]
pub fn put(keychain: &Path, service: &str, account: &str, value: &[u8]) -> Result<(), i32> {
    imp::open(keychain, service, account)?.put(value)
}

#[cfg(target_os = "macos")]
pub fn delete(keychain: &Path, service: &str, account: &str) -> Result<(), i32> {
    imp::open(keychain, service, account)?.delete()
}

#[cfg(not(target_os = "macos"))]
pub fn get(_: &Path, _: &str, _: &str) -> Result<Vec<u8>, i32> {
    Err(UNSUPPORTED)
}

#[cfg(not(target_os = "macos"))]
pub fn put(_: &Path, _: &str, _: &str, _: &[u8]) -> Result<(), i32> {
    Err(UNSUPPORTED)
}

#[cfg(not(target_os = "macos"))]
pub fn delete(_: &Path, _: &str, _: &str) -> Result<(), i32> {
    Err(UNSUPPORTED)
}
