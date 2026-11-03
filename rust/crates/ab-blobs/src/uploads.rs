use crate::{Digest, Error, Store};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

const LIMIT: i64 = 100 << 20;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UploadRequest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chunk_id: String,
    pub action: String,
    pub upload_id: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: i64,
    pub size: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &i64) -> bool {
    *value == 0
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UploadStatus {
    pub upload_id: String,
    pub offset: i64,
    pub size: i64,
    pub state: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct Lease {
    #[serde(flatten)]
    status: UploadStatus,
    pin: String,
    owner: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    expected_digest: String,
}
impl Store {
    fn upload_path(&self, owner: &str, id: &str) -> PathBuf {
        self.root
            .join("upload-leases")
            .join(ab_trust::fingerprint(format!("{owner}/{id}").as_bytes()))
    }
    fn save_upload(&self, lease: &Lease) -> Result<(), Error> {
        let path = self
            .upload_path(&lease.owner, &lease.status.upload_id)
            .with_extension("json");
        fs::create_dir_all(path.parent().ok_or(Error::Invalid)?)?;
        ab_trust::atomic_write(&path, &serde_json::to_vec(lease)?)?;
        Ok(())
    }
    fn load_upload(&mut self, owner: &str, id: &str) -> Result<Lease, Error> {
        let lease: Lease = serde_json::from_slice(&fs::read(
            self.upload_path(owner, id).with_extension("json"),
        )?)?;
        if lease.owner != owner || lease.status.upload_id != id || lease.pin.is_empty() {
            return Err(Error::Corrupt);
        }
        if lease
            .status
            .expires_at
            .parse::<jiff::Timestamp>()
            .map_err(|_| Error::Corrupt)?
            <= jiff::Timestamp::now()
        {
            self.discard_upload(&lease)?;
            return Err(Error::Invalid);
        }
        Ok(lease)
    }
    fn discard_upload(&mut self, lease: &Lease) -> Result<(), Error> {
        self.release_pin(&lease.pin)?;
        let path = self.upload_path(&lease.owner, &lease.status.upload_id);
        for extension in ["json", "part"] {
            match fs::remove_file(path.with_extension(extension)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_lines)]
    pub fn upload(&mut self, owner: &str, request: &UploadRequest) -> Result<UploadStatus, Error> {
        if owner.is_empty()
            || request.upload_id.len() > 128
            || !(request.upload_id.starts_with("upload_") || request.upload_id.starts_with("pin_"))
            || !request
                .upload_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            || request.size > LIMIT
        {
            return Err(Error::Invalid);
        }
        let loaded = self.load_upload(owner, &request.upload_id);
        let mut lease = match loaded {
            Ok(lease) => lease,
            Err(Error::Io(e))
                if e.kind() == std::io::ErrorKind::NotFound && request.action == "begin" =>
            {
                if request.size < 0 {
                    return Err(Error::Invalid);
                }
                Digest::try_from(request.digest.clone())?;
                let path = self.upload_path(owner, &request.upload_id);
                fs::create_dir_all(path.parent().ok_or(Error::Invalid)?)?;
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path.with_extension("part"))?
                    .sync_all()?;
                let lease = Lease {
                    status: UploadStatus {
                        upload_id: request.upload_id.clone(),
                        offset: 0,
                        size: request.size,
                        state: "uploading".into(),
                        expires_at: expiry()?,
                        digest: String::new(),
                    },
                    pin: format!(
                        "upload-pin:{}",
                        path.file_name().ok_or(Error::Invalid)?.to_string_lossy()
                    ),
                    owner: owner.into(),
                    expected_digest: request.digest.clone(),
                };
                self.save_upload(&lease)?;
                return Ok(lease.status);
            }
            Err(e) => return Err(e),
        };
        match request.action.as_str() {
            "begin"
                if request.size != lease.status.size || request.digest != lease.expected_digest =>
            {
                return Err(Error::Invalid);
            }
            "begin" | "status" => {}
            "prepare" => {
                if lease.status.state != "uploading"
                    || request.offset != lease.status.offset
                    || request.digest != lease.expected_digest
                {
                    return Err(Error::Invalid);
                }
            }
            "commit" => {
                if request.digest != lease.expected_digest {
                    return Err(Error::Corrupt);
                }
                if lease.status.state == "ready" {
                    return Ok(lease.status);
                }
                if lease.status.offset != lease.status.size {
                    return Err(Error::Invalid);
                }
                let mut file = fs::OpenOptions::new().read(true).write(true).open(
                    self.upload_path(owner, &request.upload_id)
                        .with_extension("part"),
                )?;
                file.set_len(u64::try_from(lease.status.offset).map_err(|_| Error::Invalid)?)?;
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                if ab_trust::fingerprint(&bytes) != request.digest {
                    self.discard_upload(&lease)?;
                    return Err(Error::Corrupt);
                }
                let (digest, temporary_pin) = self.stage(&bytes)?;
                self.manifest
                    .pins
                    .insert(lease.pin.clone(), digest.as_str().into());
                self.save()?;
                self.release_pin(&temporary_pin)?;
                lease.status.digest = digest.as_str().into();
                lease.status.state = "ready".into();
                let alias = self
                    .root
                    .join("message-uploads")
                    .join(ab_trust::fingerprint(
                        format!("{owner}/{}", request.upload_id).as_bytes(),
                    ));
                fs::create_dir_all(alias.parent().ok_or(Error::Invalid)?)?;
                match fs::hard_link(self.path(&digest), &alias) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.into()),
                }
                self.save_upload(&lease)?;
            }
            "abandon" => self.discard_upload(&lease)?,
            _ => return Err(Error::Invalid),
        }
        Ok(lease.status)
    }
    pub fn append_upload(
        &mut self,
        owner: &str,
        id: &str,
        offset: i64,
        bytes: &[u8],
    ) -> Result<UploadStatus, Error> {
        let mut lease = self.load_upload(owner, id)?;
        let next = offset
            .checked_add(i64::try_from(bytes.len()).map_err(|_| Error::Invalid)?)
            .ok_or(Error::Invalid)?;
        if offset < 0
            || offset != lease.status.offset
            || lease.status.state != "uploading"
            || bytes.is_empty()
            || bytes.len() > 1 << 20
            || next > LIMIT
            || next > lease.status.size
        {
            return Err(Error::Invalid);
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(self.upload_path(owner, id).with_extension("part"))?;
        let offset = u64::try_from(offset).map_err(|_| Error::Invalid)?;
        file.set_len(offset)?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        lease.status.offset = next;
        lease.status.expires_at = expiry()?;
        self.save_upload(&lease)?;
        Ok(lease.status)
    }
}
fn expiry() -> Result<String, Error> {
    Ok(jiff::Timestamp::now()
        .checked_add(std::time::Duration::from_secs(600))
        .map_err(|_| Error::Invalid)?
        .to_string())
}
