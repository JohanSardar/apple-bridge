use super::{
    CaptureReader, Debug, Error, Path, PathBuf, Readiness, ReadinessContext, ScanContext,
    Signature, Value, capture_value, reader_error, signature,
};
use ab_native::{capture, mail};

const MAIL_DATABASE: &str = "Envelope Index";
const MAIL_MAX_ROWS: usize = 1_000_000;

fn data_absent(root: &Path) -> bool {
    let mut missing = root.join(MAIL_DATABASE);
    while let Some(parent) = missing.parent() {
        match std::fs::read_dir(parent) {
            Ok(entries) => {
                return entries
                    .collect::<Result<Vec<_>, _>>()
                    .is_ok_and(|entries| entries.iter().all(|entry| entry.path() != missing));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing = parent.to_path_buf();
            }
            Err(_) => return false,
        }
    }
    false
}

const OPERATIONS: [&str; 6] = [
    "mail.create_draft",
    "mail.send",
    "mail.move",
    "mail.set_flag",
    "mail.mark_read",
    "mail.delete",
];

#[derive(Debug)]
pub struct MailReader {
    root: PathBuf,
    reader: mail::Reader,
    pending: Option<(mail::Pending, Signature)>,
    committed: Option<Signature>,
}

impl MailReader {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            reader: mail::Reader::default(),
            pending: None,
            committed: None,
        }
    }
}

impl CaptureReader for MailReader {
    fn source(&self) -> &'static str {
        "mail"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        ["Envelope Index", "Envelope Index-wal", "Envelope Index-shm"]
            .into_iter()
            .map(|name| self.root.join(name))
            .collect()
    }

    fn token(&self) -> Option<String> {
        None
    }

    fn resume(&mut self, _token: &str) -> bool {
        false
    }

    fn readiness(&mut self, _context: &ReadinessContext<'_>) -> Option<Readiness> {
        Some(if mail::readable(&self.root.join(MAIL_DATABASE)) {
            Readiness::ready(mail::coverage(), &OPERATIONS)
        } else if data_absent(&self.root) {
            Readiness::not("unavailable", mail::coverage(), "no_local_mail_data")
        } else {
            Readiness::not(
                "permission_missing",
                mail::coverage(),
                "Full Disk Access and a supported V10 Envelope Index required",
            )
        })
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending.is_some() {
            return Err(Error::Conflict);
        }
        let database = self.root.join(MAIL_DATABASE);
        if !database.is_file() {
            return Ok(None);
        }
        let current = signature(&self.root)?;
        if self.committed.as_ref() == Some(&current) {
            return Ok(None);
        }
        let read = capture::Context {
            observed_at: context.observed_at()?,
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
        };
        let pending = match self.reader.poll(&database, &read, MAIL_MAX_ROWS) {
            Ok(Some(pending)) => pending,
            Ok(None) => {
                self.committed = Some(current);
                return Ok(None);
            }
            Err(_) => return Ok(None),
        };
        let value = capture_value(&pending)?;
        self.pending = Some((pending, current));
        Ok(Some(value))
    }

    fn commit(&mut self) -> Result<(), Error> {
        let (pending, current) = self.pending.take().ok_or(Error::Conflict)?;
        self.reader.commit(pending).map_err(reader_error)?;
        self.committed = Some(current);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_mail_data_is_not_reported_as_a_missing_permission()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory =
            std::env::temp_dir().join(format!("ab-mail-readiness-{}", std::process::id()));
        std::fs::create_dir_all(&directory)?;
        let root = directory.join("Mail/V10/MailData");
        let context = ReadinessContext {
            fence: 1,
            os_build: "test",
        };
        let mut reader = MailReader::new(root.clone());
        assert_eq!(
            reader.readiness(&context).ok_or("readiness")?.state,
            "unavailable"
        );
        std::fs::create_dir_all(&root)?;
        assert!(data_absent(&root));
        std::fs::write(root.join(MAIL_DATABASE), b"not sqlite")?;
        assert_eq!(
            reader.readiness(&context).ok_or("readiness")?.state,
            "permission_missing"
        );
        std::fs::remove_file(root.join(MAIL_DATABASE))?;
        std::os::unix::fs::symlink(directory.join("missing"), root.join(MAIL_DATABASE))?;
        assert!(!data_absent(&root));
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
