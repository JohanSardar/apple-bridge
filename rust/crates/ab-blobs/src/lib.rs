#![forbid(unsafe_code)]
pub mod uploads;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Trust(#[from] ab_trust::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid digest or offset")]
    Invalid,
    #[error("digest mismatch")]
    Corrupt,
    #[error("blob cap exceeded")]
    Capacity,
    #[error("blob grant rejected")]
    Unauthorized,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Digest(String);
impl Digest {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for Digest {
    type Error = Error;
    fn try_from(v: String) -> Result<Self, Error> {
        if v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(v))
        } else {
            Err(Error::Invalid)
        }
    }
}
impl From<Digest> for String {
    fn from(d: Digest) -> Self {
        d.0
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlobState {
    Present,
    Pending,
    Offloaded,
    Missing,
    OverCap,
    Failed,
    DiskFull,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub state: BlobState,
    pub size: u64,
    pub received: u64,
    #[serde(default)]
    pub resumed_bytes: u64,
    #[serde(default)]
    pub corruptions: u64,
    #[serde(default)]
    pub retry_attempts: u64,
    #[serde(default = "zero_time")]
    pub next_attempt_at: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub unreferenced_at: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl Default for Entry {
    fn default() -> Self {
        Self {
            state: BlobState::Pending,
            size: 0,
            received: 0,
            resumed_bytes: 0,
            corruptions: 0,
            retry_attempts: 0,
            next_attempt_at: zero_time(),
            reason: String::new(),
            unreferenced_at: "0001-01-01T00:00:00Z".into(),
            extra: BTreeMap::new(),
        }
    }
}
fn zero_time() -> String {
    "0001-01-01T00:00:00Z".into()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Manifest {
    entries: BTreeMap<String, Entry>,
    pins: BTreeMap<String, String>,
}
#[derive(Clone, Debug)]
pub struct RecordGrant {
    digests: BTreeSet<String>,
}
impl RecordGrant {
    pub fn from_record(
        visibility: &ab_trust::Visibility,
        record: &ab_domain::Record,
    ) -> Result<Self, Error> {
        if !visibility.allows(record) || record.deleted {
            return Err(Error::Unauthorized);
        }
        Ok(Self {
            digests: references(record.data.get())?
                .into_iter()
                .map(|r| r.digest)
                .collect(),
        })
    }
    pub fn allows(&self, digest: &Digest) -> bool {
        self.digests.contains(digest.as_str())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Reference {
    pub digest: String,
    pub size: u64,
    pub state: String,
    pub variant: bool,
    pub awaiting_variants: bool,
}
pub fn references(data: &str) -> Result<Vec<Reference>, Error> {
    fn walk(
        v: &serde_json::Value,
        size: u64,
        variant: bool,
        awaiting: bool,
        out: &mut Vec<Reference>,
    ) {
        match v {
            serde_json::Value::Object(m) => {
                let size = m
                    .get("size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(size);
                let awaiting =
                    awaiting || m.contains_key("media") && !m.contains_key("derivatives");
                for key in ["digest", "raw_body_digest"] {
                    if let Some(d) = m.get(key).and_then(serde_json::Value::as_str)
                        && Digest::try_from(d.to_owned()).is_ok()
                    {
                        out.push(Reference {
                            digest: d.into(),
                            size,
                            state: m
                                .get("state")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .into(),
                            variant,
                            awaiting_variants: awaiting && !variant,
                        });
                    }
                }
                if let Some(d) = m.get("derivatives") {
                    walk(d, size, true, awaiting, out);
                }
                for (k, v) in m {
                    if k != "derivatives" {
                        walk(v, size, variant, awaiting, out);
                    }
                }
            }
            serde_json::Value::Array(a) => {
                for v in a {
                    walk(v, size, variant, awaiting, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&serde_json::from_str(data)?, 0, false, false, &mut out);
    Ok(out)
}
#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    cap: u64,
    manifest: Manifest,
    references: BTreeSet<String>,
    protection: rusqlite::Connection,
}
impl Store {
    pub fn open(home: impl AsRef<Path>, cap: u64) -> Result<Self, Error> {
        let root = home.as_ref().join("blobs");
        for p in [&root, &root.join("sha256"), &root.join("staging")] {
            fs::create_dir_all(p)?;
            fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
        }
        let manifest = match fs::read(root.join("manifest.json")) {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Manifest::default(),
            Err(e) => return Err(e.into()),
        };
        let protection_path = root.join("protection.db");
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&protection_path)?;
        let protection = rusqlite::Connection::open(protection_path)?;
        protection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000; CREATE TABLE IF NOT EXISTS protection_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS protection_owners(owner TEXT PRIMARY KEY,payload BLOB NOT NULL); CREATE TABLE IF NOT EXISTS protection_unreferenced(digest TEXT PRIMARY KEY,at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS protection_released_pins(pin TEXT PRIMARY KEY);")?;
        let mut protected = BTreeSet::new();
        {
            let mut stmt = protection.prepare("SELECT payload FROM protection_owners")?;
            for row in stmt.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
                for r in serde_json::from_slice::<Vec<serde_json::Value>>(&row?)? {
                    if let Some(d) = r.get("digest").and_then(serde_json::Value::as_str) {
                        protected.insert(d.to_owned());
                    }
                }
            }
        }
        let mut s = Self {
            root,
            cap: if cap == 0 { 20 << 30 } else { cap },
            manifest,
            references: protected,
            protection,
        };
        {
            let mut stmt = s
                .protection
                .prepare("SELECT pin FROM protection_released_pins")?;
            for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
                s.manifest.pins.remove(&row?);
            }
            let mut stmt = s
                .protection
                .prepare("SELECT digest,at FROM protection_unreferenced")?;
            for row in
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            {
                let (d, at) = row?;
                if !s.references.contains(&d)
                    && let Some(e) = s.manifest.entries.get_mut(&d)
                {
                    e.unreferenced_at = at;
                }
            }
        }
        let keys: Vec<_> = s.manifest.entries.keys().cloned().collect();
        for k in keys {
            let d = Digest::try_from(k.clone())?;
            let present = s.path(&d).is_file();
            let received = part_size(&s.part(&d))?;
            if let Some(e) = s.manifest.entries.get_mut(&k) {
                if e.state == BlobState::Present && !present {
                    e.state = BlobState::Pending;
                }
                if e.state != BlobState::Present {
                    e.received = received;
                }
            }
        }
        Ok(s)
    }
    fn path(&self, d: &Digest) -> PathBuf {
        self.root.join("sha256").join(&d.0[..2]).join(&d.0[2..])
    }
    fn part(&self, d: &Digest) -> PathBuf {
        self.root.join("staging").join(format!("{}.part", d.0))
    }
    fn save(&self) -> Result<(), Error> {
        Ok(ab_trust::atomic_write(
            &self.root.join("manifest.json"),
            &serde_json::to_vec(&self.manifest)?,
        )?)
    }
    pub fn rearm(&mut self, d: &Digest) -> Result<bool, Error> {
        let Some(e) = self.manifest.entries.get_mut(d.as_str()) else {
            return Ok(false);
        };
        if e.state != BlobState::Failed || e.reason != "ipc_retry_exhausted" || e.corruptions > 0 {
            return Ok(false);
        }
        e.state = BlobState::Pending;
        e.reason.clear();
        e.retry_attempts = 7;
        e.next_attempt_at = zero_time();
        self.save()?;
        Ok(true)
    }
    pub fn retry(&mut self, d: &Digest, completed: bool) -> Result<bool, Error> {
        let e = self
            .manifest
            .entries
            .get_mut(d.as_str())
            .ok_or(Error::Invalid)?;
        if completed {
            e.next_attempt_at = zero_time();
        } else if e.retry_attempts >= 8 {
            e.state = BlobState::Failed;
            e.reason = "ipc_retry_exhausted".into();
            e.next_attempt_at = zero_time();
        } else {
            e.retry_attempts += 1;
            let delay = (250_i64 << (e.retry_attempts - 1).min(4)).min(4000);
            e.next_attempt_at =
                (jiff::Timestamp::now() + jiff::SignedDuration::from_millis(delay)).to_string();
        }
        let pending = e.state == BlobState::Pending;
        self.save()?;
        Ok(pending)
    }
    pub fn entries(&self) -> &BTreeMap<String, Entry> {
        &self.manifest.entries
    }
    #[must_use]
    pub fn progress(&self) -> serde_json::Value {
        let entries = &self.manifest.entries;
        serde_json::json!({
            "entries": entries,
            "bytes": self.used(),
            "backlog": entries.values().filter(|e| e.state != BlobState::Present).count(),
            "resumed_bytes": entries.values().map(|e| e.resumed_bytes).sum::<u64>(),
            "corruptions": entries.values().map(|e| e.corruptions).sum::<u64>(),
        })
    }
    pub fn used(&self) -> u64 {
        self.manifest
            .entries
            .values()
            .map(|e| {
                if e.state == BlobState::Present {
                    e.size
                } else {
                    e.received
                }
            })
            .sum()
    }
    pub fn begin(&mut self, d: &Digest, size: u64) -> Result<Option<u64>, Error> {
        if self
            .manifest
            .entries
            .get(&d.0)
            .is_some_and(|e| e.state == BlobState::Present)
        {
            return Ok(None);
        }
        let received = part_size(&self.part(d))?;
        let over = self
            .used()
            .saturating_sub(received)
            .saturating_add(size.max(received))
            > self.cap;
        let e = self.manifest.entries.entry(d.0.clone()).or_default();
        e.size = size;
        e.received = received;
        e.resumed_bytes += received;
        e.state = if over {
            BlobState::OverCap
        } else {
            BlobState::Pending
        };
        self.save()?;
        if over {
            Err(Error::Capacity)
        } else {
            Ok(Some(received))
        }
    }
    pub fn chunk(&mut self, d: &Digest, offset: u64, bytes: &[u8]) -> Result<(), Error> {
        let e = self.manifest.entries.get(&d.0).ok_or(Error::Invalid)?;
        if e.state != BlobState::Pending || offset != e.received || bytes.len() > 1 << 20 {
            return Err(Error::Invalid);
        }
        if self.used().saturating_add(bytes.len() as u64) > self.cap {
            if let Some(e) = self.manifest.entries.get_mut(&d.0) {
                e.state = BlobState::OverCap;
            }
            self.save()?;
            return Err(Error::Capacity);
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(self.part(d))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        if let Some(e) = self.manifest.entries.get_mut(&d.0) {
            e.received += bytes.len() as u64;
        }
        self.save()
    }
    pub fn finish(&mut self, d: &Digest) -> Result<(), Error> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.part(d))?;
        let mut hash = Sha256::new();
        let mut buf = vec![0; 65536];
        let mut size = 0;
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
            size += n as u64;
        }
        if format!("{:x}", hash.finalize()) != d.0 {
            file.set_len(0)?;
            file.sync_all()?;
            if let Some(e) = self.manifest.entries.get_mut(&d.0) {
                e.state = BlobState::Failed;
                e.received = 0;
                e.corruptions += 1;
            }
            self.save()?;
            return Err(Error::Corrupt);
        }
        file.sync_all()?;
        let path = self.path(d);
        let parent = path.parent().ok_or(Error::Invalid)?;
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        fs::rename(self.part(d), &path)?;
        File::open(parent)?.sync_all()?;
        File::open(self.root.join("staging"))?.sync_all()?;
        let e = self.manifest.entries.get_mut(&d.0).ok_or(Error::Invalid)?;
        e.state = BlobState::Present;
        e.size = size;
        e.received = size;
        self.save()
    }
    pub fn open_authorized(&self, grant: &RecordGrant, d: &Digest) -> Result<File, Error> {
        if !grant.allows(d) {
            return Err(Error::Unauthorized);
        }
        if !self
            .manifest
            .entries
            .get(&d.0)
            .is_some_and(|e| e.state == BlobState::Present)
        {
            return Err(Error::Invalid);
        }
        Ok(File::open(self.path(d))?)
    }
    pub fn protect(&mut self, digests: BTreeSet<String>) -> Result<(), Error> {
        let updates = [(
            "replica:current".into(),
            digests
                .into_iter()
                .map(|digest| Reference {
                    digest,
                    ..Reference::default()
                })
                .collect(),
        )]
        .into_iter()
        .collect();
        self.update_protection(&updates, false)
    }
    pub fn protect_references(&mut self, references: Vec<Reference>) -> Result<(), Error> {
        let updates = [("replica:current".into(), references)]
            .into_iter()
            .collect();
        self.update_protection(&updates, false)
    }
    pub fn unavailable(&mut self, d: &Digest, reason: &str) -> Result<(), Error> {
        let Some(entry) = self.manifest.entries.get_mut(&d.0) else {
            return Ok(());
        };
        if entry.state == BlobState::Present {
            return Ok(());
        }
        match reason {
            "pending" | "unavailable" => return Ok(()),
            "offloaded" => entry.state = BlobState::Offloaded,
            "failed" => {
                entry.state = BlobState::Failed;
                "source_failed".clone_into(&mut entry.reason);
            }
            _ => entry.state = BlobState::Missing,
        }
        self.save()
    }
    pub fn update_protection(
        &mut self,
        updates: &BTreeMap<String, Vec<Reference>>,
        release_pins: bool,
    ) -> Result<(), Error> {
        if updates.is_empty() {
            return Ok(());
        }
        for refs in updates.values() {
            for r in refs {
                Digest::try_from(r.digest.clone())?;
            }
        }
        let now = jiff::Timestamp::now().to_string();
        let tx = self.protection.transaction()?;
        for (owner, refs) in updates {
            if refs.is_empty() {
                tx.execute("DELETE FROM protection_owners WHERE owner=?", [owner])?;
            } else {
                tx.execute("INSERT INTO protection_owners(owner,payload) VALUES(?,?) ON CONFLICT(owner) DO UPDATE SET payload=excluded.payload", rusqlite::params![owner, serde_json::to_vec(refs)?])?;
            }
        }
        let mut protected = BTreeSet::new();
        {
            let mut statement = tx.prepare("SELECT payload FROM protection_owners")?;
            for row in statement.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
                for r in serde_json::from_slice::<Vec<Reference>>(&row?)? {
                    protected.insert(r.digest);
                }
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO protection_meta(key,value) VALUES('initialized','1')",
            [],
        )?;
        let mut entries = self.manifest.entries.clone();
        for r in updates.values().flatten() {
            let entry = entries.entry(r.digest.clone()).or_insert_with(|| Entry {
                size: r.size,
                state: match r.state.as_str() {
                    "missing" => BlobState::Missing,
                    "offloaded" => BlobState::Offloaded,
                    _ => BlobState::Pending,
                },
                ..Entry::default()
            });
            if matches!(entry.state, BlobState::Offloaded | BlobState::Missing) {
                entry.state = match r.state.as_str() {
                    "offloaded" => BlobState::Offloaded,
                    "missing" => BlobState::Missing,
                    _ => BlobState::Pending,
                };
            }
        }
        for (d, e) in &mut entries {
            if protected.contains(d) {
                tx.execute("DELETE FROM protection_unreferenced WHERE digest=?", [d])?;
                e.unreferenced_at = "0001-01-01T00:00:00Z".into();
            } else {
                if e.unreferenced_at.is_empty() || e.unreferenced_at == "0001-01-01T00:00:00Z" {
                    e.unreferenced_at.clone_from(&now);
                }
                tx.execute(
                    "INSERT OR IGNORE INTO protection_unreferenced(digest,at) VALUES(?,?)",
                    rusqlite::params![d, e.unreferenced_at],
                )?;
            }
        }
        let released: Vec<_> = self
            .manifest
            .pins
            .iter()
            .filter(|(_, d)| release_pins && protected.contains(*d))
            .map(|(pin, _)| pin.clone())
            .collect();
        for pin in &released {
            tx.execute(
                "INSERT OR IGNORE INTO protection_released_pins(pin) VALUES(?)",
                [pin],
            )?;
        }
        tx.commit()?;
        self.manifest.entries = entries;
        self.references = protected;
        for pin in released {
            self.manifest.pins.remove(&pin);
        }
        self.save()
    }
    pub fn stage(&mut self, bytes: &[u8]) -> Result<(Digest, String), Error> {
        let d = Digest::try_from(format!("{:x}", Sha256::digest(bytes)))?;
        if self.begin(&d, bytes.len() as u64)?.is_some() {
            let part = self.part(&d);
            if part.exists() {
                fs::remove_file(&part)?;
            }
            if let Some(e) = self.manifest.entries.get_mut(d.as_str()) {
                e.received = 0;
            }
            for (index, chunk) in bytes.chunks(1 << 20).enumerate() {
                self.chunk(&d, (index * (1 << 20)) as u64, chunk)?;
            }
            self.finish(&d)?;
        }
        let pin = format!(
            "pin_{}",
            ab_trust::fingerprint(format!("{}:{}", d.as_str(), jiff::Timestamp::now()).as_bytes())
        );
        self.manifest.pins.insert(pin.clone(), d.as_str().into());
        self.save()?;
        Ok((d, pin))
    }
    pub fn release_pin(&mut self, pin: &str) -> Result<(), Error> {
        self.protection.execute(
            "INSERT OR IGNORE INTO protection_released_pins(pin) VALUES(?)",
            [pin],
        )?;
        self.manifest.pins.remove(pin);
        self.save()
    }
    pub fn gc(&mut self, now: jiff::Timestamp, grace_seconds: i64) -> Result<usize, Error> {
        let remove: Vec<_> = self
            .manifest
            .entries
            .iter()
            .filter(|(d, e)| {
                !self.references.contains(*d)
                    && !self.manifest.pins.values().any(|p| p == *d)
                    && e.unreferenced_at
                        .parse::<jiff::Timestamp>()
                        .is_ok_and(|t| now.as_second() - t.as_second() >= grace_seconds)
            })
            .map(|(d, _)| d.clone())
            .collect();
        for d in &remove {
            let digest = Digest::try_from(d.clone())?;
            for p in [self.path(&digest), self.part(&digest)] {
                match fs::remove_file(p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            self.manifest.entries.remove(d);
        }
        if !remove.is_empty() {
            self.save()?;
        }
        Ok(remove.len())
    }
}

fn part_size(path: &Path) -> Result<u64, Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(meta.len()),
        Ok(_) => Err(Error::Invalid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e.into()),
    }
}

pub trait BlobRead {
    fn read(
        &self,
        handle: &str,
        offset: u64,
        length: usize,
    ) -> impl std::future::Future<Output = Result<(Vec<u8>, bool), Error>> + Send;
}
impl Store {
    pub async fn transfer(
        &mut self,
        source: &impl BlobRead,
        handle: &str,
        digest: &Digest,
        size: u64,
    ) -> Result<(), Error> {
        let Some(mut offset) = self.begin(digest, size)? else {
            return Ok(());
        };
        loop {
            let (bytes, done) = source.read(handle, offset, 256 << 10).await?;
            if bytes.len() > 256 << 10 || bytes.is_empty() && !done {
                return Err(Error::Invalid);
            }
            if size > 0 && offset.saturating_add(bytes.len() as u64) > size {
                return Err(Error::Corrupt);
            }
            self.chunk(digest, offset, &bytes)?;
            offset += bytes.len() as u64;
            if done {
                if size > 0 && offset != size {
                    return Err(Error::Corrupt);
                }
                return self.finish(digest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rearm_gives_an_exhausted_blob_exactly_one_more_attempt() -> Result<(), Box<dyn std::error::Error>>
    {
        let home = tempfile::tempdir()?;
        let digest = Digest::try_from("b".repeat(64))?;
        let mut store = Store::open(home.path(), 0)?;
        assert!(!store.rearm(&digest)?);
        store.begin(&digest, 1024)?;
        for _ in 0..8 {
            store.retry(&digest, false)?;
        }
        assert_eq!(store.entries()[digest.as_str()].state, BlobState::Failed);
        assert!(store.rearm(&digest)?);
        assert!(!store.rearm(&digest)?);
        assert_eq!(store.entries()[digest.as_str()].state, BlobState::Pending);
        assert!(store.retry(&digest, false)?);
        assert!(!store.retry(&digest, false)?);
        assert_eq!(store.entries()[digest.as_str()].state, BlobState::Failed);
        Ok(())
    }

    #[test]
    fn retry_budget_and_deadline_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let digest = Digest::try_from("a".repeat(64))?;
        let mut store = Store::open(home.path(), 0)?;
        store.begin(&digest, 1024)?;
        for _ in 0..5 {
            assert!(store.retry(&digest, false)?);
        }
        let deadline = store.entries()[digest.as_str()].next_attempt_at.clone();
        drop(store);
        let mut store = Store::open(home.path(), 0)?;
        assert_eq!(store.entries()[digest.as_str()].retry_attempts, 5);
        assert_eq!(store.entries()[digest.as_str()].next_attempt_at, deadline);
        for _ in 5..8 {
            assert!(store.retry(&digest, false)?);
        }
        assert!(!store.retry(&digest, false)?);
        drop(store);
        let store = Store::open(home.path(), 0)?;
        let entry = &store.entries()[digest.as_str()];
        assert_eq!(entry.state, BlobState::Failed);
        assert_eq!(entry.retry_attempts, 8);
        assert_eq!(entry.reason, "ipc_retry_exhausted");
        Ok(())
    }
}
