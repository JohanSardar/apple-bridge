use crate::{decode, hash, random};
use ed25519_dalek::{Signature, VerifyingKey};
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Mutex,
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("That doesn't look right. Check it and try again.")]
    Invalid,
    #[error("Sign in first.")]
    Unauthorized,
    #[error("sign in again to do this")]
    SignInAgain,
    #[error("You can't do that from this account.")]
    Forbidden,
    #[error("This account already has all the Macs it can hold. Remove one first.")]
    Capacity,
    #[error("Too many tries. Wait a moment and try again.")]
    SlowDown,
    #[error("authorization_pending")]
    Pending,
    #[error(
        "That code has expired or was already used. Turn Remote on again on your Mac for a new one."
    )]
    Expired,
    #[error("The relay can't do that right now. Try again in a minute.")]
    Unavailable,
    #[error("The relay can't do that right now. Try again in a minute.")]
    Fenced,
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Unavailable
    }
}
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Unavailable
    }
}
impl From<serde_json::Error> for Error {
    fn from(_: serde_json::Error) -> Self {
        Self::Invalid
    }
}

struct Writer {
    db: Connection,
    fence: File,
    _lock: File,
}
pub struct Control {
    writer: Mutex<Writer>,
    public_url: String,
    policy_sync: tokio::sync::Mutex<()>,
}
impl fmt::Debug for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Control([redacted])")
    }
}

impl Writer {
    fn epoch(&mut self) -> Result<i64> {
        self.fence.seek(SeekFrom::Start(0))?;
        let mut text = String::new();
        self.fence.read_to_string(&mut text)?;
        if !text.ends_with('\n') {
            return Err(Error::Fenced);
        }
        let mut current = -1_i64;
        for line in text.lines() {
            let next: i64 = line.parse().map_err(|_| Error::Fenced)?;
            if next != current.checked_add(1).ok_or(Error::Fenced)? {
                return Err(Error::Fenced);
            }
            current = next;
        }
        if current < 0 {
            return Err(Error::Fenced);
        }
        Ok(current)
    }
    fn check(&mut self) -> Result<()> {
        let epoch: i64 = self
            .db
            .query_row("SELECT epoch FROM fence", [], |r| r.get(0))?;
        if epoch != self.epoch()? {
            return Err(Error::Fenced);
        }
        Ok(())
    }
    fn advance(&mut self) -> Result<i64> {
        self.check()?;
        let next = self.epoch()?.checked_add(1).ok_or(Error::Fenced)?;
        self.fence.seek(SeekFrom::End(0))?;
        self.fence.write_all(format!("{next}\n").as_bytes())?;
        self.fence.sync_all()?;
        Ok(next)
    }
}

const DEVICES_PER_ACCOUNT: i64 = 2;
const FRESH_FENCE: &str = "0\n";
const GRANT_SECONDS: i64 = 600;
const COMPLETION_SECONDS: i64 = 30;
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGrant {
    pub id: String,
    pub account: String,
    pub public_key: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyChange {
    pub id: i64,
    pub hostname: String,
    pub disabled: bool,
    pub revoked: bool,
}
impl PolicyChange {
    #[must_use]
    pub fn version(&self) -> i64 {
        self.id
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAction {
    Suspend,
    Resume,
    Revoke,
}

fn user_code() -> Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| Error::Unavailable)?;
    let letter =
        |byte: &u8| char::from(USER_CODE_ALPHABET[usize::from(*byte) % USER_CODE_ALPHABET.len()]);
    let code: String = bytes.iter().map(letter).collect();
    Ok(format!("{}-{}", &code[..4], &code[4..]))
}
pub(crate) fn normalize_code(code: &str) -> String {
    let compact: String = code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if compact.len() == 8 {
        format!("{}-{}", &compact[..4], &compact[4..])
    } else {
        compact
    }
}
fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Mac".to_owned()
    } else {
        cleaned.to_owned()
    }
}
fn proof(public: &str, signature: &str, message: &[u8]) -> Result<()> {
    let bytes: [u8; 32] = decode(public)?.try_into().map_err(|_| Error::Invalid)?;
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| Error::Invalid)?;
    let sig = Signature::from_slice(&decode(signature)?).map_err(|_| Error::Invalid)?;
    key.verify_strict(message, &sig)
        .map_err(|_| Error::Forbidden)
}

impl Control {
    pub fn open(database: &Path, fence: &Path, public_url: &str) -> Result<Self> {
        if database == fence {
            return Err(Error::Invalid);
        }
        let exists = database.exists();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .truncate(false)
            .open(database.with_extension("writer-lock"))?;
        lock.try_lock_exclusive().map_err(|_| Error::Unavailable)?;
        let mut fence_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(!exists)
            .mode(0o600)
            .truncate(false)
            .open(fence)?;
        fence_file
            .try_lock_exclusive()
            .map_err(|_| Error::Unavailable)?;
        if !exists && fence_file.metadata()?.len() != 0 {
            let mut written = String::new();
            fence_file.read_to_string(&mut written)?;
            if written != FRESH_FENCE {
                return Err(Error::Fenced);
            }
        } else if !exists {
            fence_file.write_all(FRESH_FENCE.as_bytes())?;
            fence_file.sync_all()?;
            if let Some(parent) = fence.parent() {
                File::open(parent)?.sync_all()?;
            }
        }
        let db = Connection::open(database)?;
        std::fs::set_permissions(database, std::fs::Permissions::from_mode(0o600))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS fence(epoch INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS grants(id TEXT PRIMARY KEY, poll TEXT UNIQUE NOT NULL, code TEXT UNIQUE NOT NULL, public TEXT NOT NULL, fingerprint TEXT NOT NULL, name TEXT NOT NULL, nonce TEXT NOT NULL, proof_until INTEGER NOT NULL, expires INTEGER NOT NULL, next_poll INTEGER NOT NULL, interval INTEGER NOT NULL, account TEXT, state TEXT NOT NULL CHECK(state IN ('pending','approved','consumed')));
CREATE TABLE IF NOT EXISTS devices(hostname TEXT PRIMARY KEY, account TEXT NOT NULL, public TEXT UNIQUE NOT NULL, name TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('active','suspended','revoked')), linked_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS policy_outbox(id INTEGER PRIMARY KEY AUTOINCREMENT, hostname TEXT NOT NULL, disabled INTEGER NOT NULL, revoked INTEGER NOT NULL, epoch INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS throttle(source TEXT PRIMARY KEY, window INTEGER NOT NULL, count INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS grant_completions(grant_id TEXT PRIMARY KEY, account TEXT NOT NULL, until INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS grant_sources(grant_id TEXT PRIMARY KEY, source TEXT NOT NULL, created INTEGER NOT NULL);")?;
        let mut writer = Writer {
            db,
            fence: fence_file,
            _lock: lock,
        };
        let recorded: i64 = writer
            .db
            .query_row("SELECT count(*) FROM fence", [], |r| r.get(0))?;
        if recorded == 0 {
            if writer.epoch()? != 0 {
                return Err(Error::Fenced);
            }
            writer.db.execute("INSERT INTO fence VALUES(0)", [])?;
        }
        writer.check()?;
        Ok(Self {
            writer: Mutex::new(writer),
            public_url: public_url.trim_end_matches('/').to_owned(),
            policy_sync: tokio::sync::Mutex::new(()),
        })
    }
    fn with<T>(&self, f: impl FnOnce(&mut Writer) -> Result<T>) -> Result<T> {
        let mut writer = self.writer.lock().map_err(|_| Error::Unavailable)?;
        writer.check()?;
        f(&mut writer)
    }
    pub(crate) fn public_url(&self) -> &str {
        &self.public_url
    }
    pub fn throttle(&self, source: &str, now: i64) -> Result<()> {
        self.with(|w| {
            w.db.execute("DELETE FROM throttle WHERE window < ?1", [now - 600])?;
            let size: i64 = w.db.query_row("SELECT count(*) FROM throttle", [], |r| r.get(0))?;
            if size >= 4096 {
                return Err(Error::SlowDown);
            }
            let key = hash(source);
            w.db.execute(
                "INSERT INTO throttle VALUES(?1,?2,1) ON CONFLICT(source) DO UPDATE SET count=CASE WHEN window < ?3 THEN 1 ELSE count+1 END, window=CASE WHEN window < ?3 THEN ?2 ELSE window END",
                params![key, now, now - 60],
            )?;
            let count: i64 =
                w.db.query_row("SELECT count FROM throttle WHERE source=?1", [key], |r| r.get(0))?;
            if count > 60 {
                return Err(Error::SlowDown);
            }
            Ok(())
        })
    }
    pub fn list(&self, account: &str) -> Result<Value> {
        self.with(|w| {
            let mut query = w.db.prepare(
                "SELECT hostname,public,name,state,linked_at FROM devices WHERE account=?1 AND state!='revoked' ORDER BY linked_at",
            )?;
            let rows = query
                .query_map([account], |r| {
                    let public: String = r.get(1)?;
                    let fingerprint = decode(&public).ok().and_then(|key| <[u8; 32]>::try_from(key).ok()).map(|key| ab_wire::framing::device_key_fingerprint(&key));
                    Ok(json!({
                        "hostname": r.get::<_, String>(0)?,
                        "fingerprint": fingerprint,
                        "name": r.get::<_, String>(2)?,
                        "state": r.get::<_, String>(3)?,
                        "linked_at": r.get::<_, i64>(4)?,
                    }))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(json!({"devices": rows, "limit": DEVICES_PER_ACCOUNT}))
        })
    }
    pub fn change(&self, account: &str, hostname: &str, action: DeviceAction) -> Result<()> {
        self.with(|w| {
            let current: String =
                w.db.query_row(
                    "SELECT state FROM devices WHERE account=?1 AND hostname=?2",
                    params![account, hostname],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(Error::Forbidden)?;
            let (next, disabled, revoked) = match (action, current.as_str()) {
                (DeviceAction::Suspend, "active" | "suspended") => ("suspended", true, false),
                (DeviceAction::Resume, "active" | "suspended") => ("active", false, false),
                (DeviceAction::Revoke, "active" | "suspended") => ("revoked", true, true),
                _ => return Err(Error::Invalid),
            };
            let epoch = w.advance()?;
            let tx = w.db.transaction()?;
            tx.execute(
                "UPDATE devices SET state=?1 WHERE hostname=?2",
                params![next, hostname],
            )?;
            tx.execute(
                "INSERT INTO policy_outbox(hostname,disabled,revoked,epoch) VALUES(?1,?2,?3,?4)",
                params![hostname, disabled, revoked, epoch],
            )?;
            tx.execute("UPDATE fence SET epoch=?1", [epoch])?;
            tx.commit()?;
            Ok(())
        })
    }
    pub fn revoke_all(&self, account: &str) -> Result<()> {
        self.with(|w| {
            let epoch = w.advance()?;
            let tx = w.db.transaction()?;
            tx.execute(
                "INSERT INTO policy_outbox(hostname,disabled,revoked,epoch) SELECT hostname,1,1,?1 FROM devices WHERE account=?2 AND state!='revoked'",
                params![epoch, account],
            )?;
            tx.execute("UPDATE devices SET state='revoked' WHERE account=?1", [account])?;
            tx.execute("DELETE FROM grants WHERE account=?1", [account])?;
            tx.execute("UPDATE fence SET epoch=?1", [epoch])?;
            tx.commit()?;
            Ok(())
        })
    }
    pub fn pending_policy(&self) -> Result<Vec<PolicyChange>> {
        self.with(|w| {
            let mut query = w.db.prepare(
                "SELECT o.id,o.hostname,o.disabled,o.revoked FROM policy_outbox o WHERE o.id=(SELECT max(id) FROM policy_outbox WHERE hostname=o.hostname) ORDER BY o.id LIMIT 100",
            )?;
            let rows = query
                .query_map([], |r| {
                    Ok(PolicyChange {
                        id: r.get(0)?,
                        hostname: r.get(1)?,
                        disabled: r.get(2)?,
                        revoked: r.get(3)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    pub(crate) fn policy_applied(&self, change: &PolicyChange) -> Result<()> {
        self.with(|w| {
            w.db.execute(
                "DELETE FROM policy_outbox WHERE hostname=?1 AND id<=?2",
                params![change.hostname, change.id],
            )?;
            Ok(())
        })
    }
    pub(crate) async fn lock_policy_sync(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.policy_sync.lock().await
    }
    pub(crate) fn try_lock_policy_sync(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        self.policy_sync.try_lock().ok()
    }
}
fn held_devices(db: &Connection, account: &str) -> Result<i64> {
    Ok(db.query_row(
        "SELECT count(*) FROM devices WHERE account=?1 AND state!='revoked'",
        [account],
        |r| r.get(0),
    )?)
}
fn network_of(address: &str) -> Option<std::net::IpAddr> {
    match address.parse::<std::net::IpAddr>().ok()?.to_canonical() {
        std::net::IpAddr::V6(address) => {
            let mut segments = address.segments();
            segments[4..].fill(0);
            Some(std::net::IpAddr::from(segments))
        }
        address @ std::net::IpAddr::V4(_) => Some(address),
    }
}
fn same_network(left: &str, right: &str) -> bool {
    network_of(left).is_some_and(|left| network_of(right) == Some(left))
}

mod grants;
