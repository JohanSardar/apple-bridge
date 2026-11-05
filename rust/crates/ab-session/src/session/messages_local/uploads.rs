use super::{Config, INVALID, Local, UNAVAILABLE, sha, write_private};
use crate::session::owned::text;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub(super) fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

pub(super) fn confined_upload(path: &str) -> Option<PathBuf> {
    let source = fs::canonicalize(path).ok()?;
    let parent = source.parent()?;
    if parent.file_name()?.to_str()? != "message-uploads" {
        return None;
    }
    let uid = ab_sys::user_id();
    let directory = fs::metadata(parent).ok()?;
    let file = fs::symlink_metadata(&source).ok()?;
    (directory.mode() & 0o777 == 0o700
        && directory.uid() == uid
        && file.uid() == uid
        && file.file_type().is_file())
    .then_some(source)
}

pub(super) fn copy_new(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()
}

pub(super) fn file_digest(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Some(format!("{:x}", digest.finalize()))
}

pub(super) fn selftest_upload(config: &Config, args: &Value) -> Result<Value, String> {
    let name = text(args, "name");
    if !plain_name(name) {
        return Err(INVALID.into());
    }
    let directory = Local::uploads(config);
    fs::create_dir_all(&directory).map_err(|_| UNAVAILABLE.to_owned())?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .map_err(|_| UNAVAILABLE.to_owned())?;
    let bytes = text(args, "text").as_bytes();
    let path = directory.join(name);
    write_private(&path, bytes)?;
    Ok(json!({"path": path.to_string_lossy(), "digest": sha(bytes)}))
}

pub(super) fn selftest_upload_remove(config: &Config, args: &Value) -> Result<Value, String> {
    let uploads = fs::canonicalize(Local::uploads(config)).map_err(|_| INVALID.to_owned())?;
    let file = fs::canonicalize(text(args, "path")).map_err(|_| INVALID.to_owned())?;
    if file.parent() != Some(uploads.as_path()) {
        return Err(INVALID.into());
    }
    let _ = fs::remove_file(file);
    Ok(json!({}))
}
