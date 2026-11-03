use crate::Error;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize)]
pub struct EffectPermit {
    command_id: String,
    owner_fence: u64,
    ticket: String,
    request_hash: String,
    operation: String,
    expires_at: u64,
    nonce: String,
}

#[derive(Debug)]
pub struct PreparedEffect {
    pub command_id: String,
    pub owner_fence: u64,
    pub ticket: String,
    pub request_hash: String,
    pub operation: String,
    pub expires_at: u64,
    pub prepared: Value,
}

#[derive(Debug)]
pub struct DispatchGuard {
    file: File,
    outcomes: BTreeMap<String, Value>,
    poisoned: bool,
}

impl DispatchGuard {
    pub fn open(home: &Path) -> Result<Self, Error> {
        std::fs::create_dir_all(home)?;
        let info = std::fs::symlink_metadata(home)?;
        if !info.is_dir() || info.file_type().is_symlink() {
            return Err(Error::Invalid);
        }
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))?;
        let mut file = ab_sys::open_guard(&home.join("dispatch-guard.log"))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            let boundary = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
            bytes.truncate(boundary);
            file.set_len(boundary as u64)?;
        }
        let mut outcomes = BTreeMap::new();
        for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let entry: Value = serde_json::from_slice(line)?;
            let id = entry["command_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(Error::Invalid)?
                .to_owned();
            match entry["t"].as_str() {
                Some("Outcome") => {
                    outcomes.insert(id, entry);
                }
                Some("DispatchStarted") => {
                    outcomes.insert(id.clone(), json!({"t":"Outcome","command_id":id,"disposition":"indeterminate","evidence":{"reason":"dispatch_started_without_outcome","prepared":entry["prepared"]}}));
                }
                _ => return Err(Error::Invalid),
            }
        }
        file.seek(SeekFrom::End(0))?;
        file.sync_all()?;
        File::open(home)?.sync_all()?;
        Ok(Self {
            file,
            outcomes,
            poisoned: false,
        })
    }

    fn append(&mut self, value: &Value) -> Result<(), Error> {
        if self.poisoned {
            return Err(Error::Conflict);
        }
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        self.poisoned = true;
        self.file.write_all(&bytes)?;
        self.file.sync_all()?;
        self.poisoned = false;
        Ok(())
    }

    pub fn unresolved(&self) -> Vec<Value> {
        self.outcomes
            .values()
            .filter(|v| v["disposition"] == "indeterminate")
            .cloned()
            .collect()
    }

    pub fn outcome(&self, id: &str) -> Option<&Value> {
        self.outcomes.get(id)
    }

    pub fn begin(&mut self, prepared: PreparedEffect) -> Result<EffectPermit, Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Invalid)?
            .as_secs();
        if prepared.command_id.is_empty()
            || prepared.ticket.is_empty()
            || prepared.request_hash.is_empty()
            || prepared.operation.is_empty()
            || prepared.owner_fence == 0
            || prepared.expires_at <= now
        {
            return Err(Error::Invalid);
        }
        if self.outcomes.contains_key(&prepared.command_id) {
            return Err(Error::AlreadyStarted);
        }
        let mut random = [0_u8; 32];
        OpenOptions::new()
            .read(true)
            .custom_flags(0)
            .open("/dev/urandom")?
            .read_exact(&mut random)?;
        let nonce: String = random
            .iter()
            .flat_map(|b| {
                let hex = b"0123456789abcdef";
                [
                    char::from(hex[usize::from(b >> 4)]),
                    char::from(hex[usize::from(b & 15)]),
                ]
            })
            .collect();
        let entry = json!({"t":"DispatchStarted","command_id":prepared.command_id,"ticket":prepared.ticket,"prepared":prepared.prepared});
        self.append(&entry)?;
        self.outcomes.insert(prepared.command_id.clone(), json!({"t":"Outcome","command_id":prepared.command_id,"disposition":"indeterminate","evidence":{"prepared":entry["prepared"]}}));
        Ok(EffectPermit {
            command_id: prepared.command_id,
            owner_fence: prepared.owner_fence,
            ticket: prepared.ticket,
            request_hash: prepared.request_hash,
            operation: prepared.operation,
            expires_at: prepared.expires_at,
            nonce,
        })
    }

    pub fn finish(&mut self, id: &str, disposition: &str, evidence: &Value) -> Result<(), Error> {
        if !self.outcomes.contains_key(id)
            || !["locally_observed", "indeterminate"].contains(&disposition)
        {
            return Err(Error::Invalid);
        }
        let entry =
            json!({"t":"Outcome","command_id":id,"disposition":disposition,"evidence":evidence});
        self.append(&entry)?;
        self.outcomes.insert(id.to_owned(), entry);
        Ok(())
    }
}
