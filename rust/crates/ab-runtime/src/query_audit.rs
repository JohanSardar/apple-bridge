use ab_operations::{Code, Principal, QueryActivity, QueryAudit, QueryAuditSink};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Mutex,
};

#[derive(Debug)]
pub struct Audit {
    file: Mutex<File>,
    activity: QueryActivity,
}
impl Audit {
    pub fn open(home: &Path) -> Result<Self, crate::Error> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(home.join("commands-audit.jsonl"))?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            file: Mutex::new(file),
            activity: QueryActivity::open(home)?,
        })
    }
}
impl QueryAuditSink for Audit {
    fn record(
        &self,
        principal: &Principal,
        id: &str,
        audit: &QueryAudit,
        completed: bool,
    ) -> Result<(), ab_operations::Error> {
        let mut bytes = serde_json::to_vec(audit).map_err(|_| Code::Corrupt)?;
        bytes.push(b'\n');
        self.file
            .lock()
            .map_err(|_| Code::Unavailable)?
            .write_all(&bytes)
            .map_err(|_| Code::Unavailable)?;
        self.activity.record(principal, id, audit, completed)
    }
}
