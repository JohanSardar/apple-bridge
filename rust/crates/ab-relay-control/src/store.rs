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
const GRANT_SECONDS: i64 = 600;
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
        if !exists {
            if fence_file.metadata()?.len() != 0 {
                return Err(Error::Fenced);
            }
            fence_file.write_all(b"0\n")?;
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
CREATE TABLE IF NOT EXISTS throttle(source TEXT PRIMARY KEY, window INTEGER NOT NULL, count INTEGER NOT NULL);")?;
        if !exists {
            db.execute("INSERT INTO fence VALUES(0)", [])?;
        }
        let mut writer = Writer {
            db,
            fence: fence_file,
            _lock: lock,
        };
        writer.check()?;
        Ok(Self {
            writer: Mutex::new(writer),
            public_url: public_url.trim_end_matches('/').to_owned(),
        })
    }
    fn with<T>(&self, f: impl FnOnce(&mut Writer) -> Result<T>) -> Result<T> {
        let mut writer = self.writer.lock().map_err(|_| Error::Unavailable)?;
        writer.check()?;
        f(&mut writer)
    }
    pub fn public_url(&self) -> &str {
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
    pub fn start_grant(&self, public: &str, name: &str, now: i64) -> Result<Value> {
        let key: [u8; 32] = decode(public)?.try_into().map_err(|_| Error::Invalid)?;
        VerifyingKey::from_bytes(&key).map_err(|_| Error::Invalid)?;
        let name = clean_name(name);
        let verification = format!("{}/device", self.public_url);
        self.with(|w| {
            w.db.execute("DELETE FROM grants WHERE expires<=?1", [now])?;
            let waiting: i64 =
                w.db.query_row("SELECT count(*) FROM grants", [], |r| r.get(0))?;
            if waiting >= 4096 {
                return Err(Error::Capacity);
            }
            let id = random(24)?;
            let poll = random(32)?;
            let code = user_code()?;
            let nonce = random(32)?;
            let fingerprint = ab_wire::framing::device_key_fingerprint(&key);
            w.db.execute(
                "INSERT INTO grants VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9,5,NULL,'pending')",
                params![
                    id,
                    hash(&poll),
                    hash(&code),
                    public,
                    fingerprint,
                    name,
                    nonce,
                    now + GRANT_SECONDS,
                    now + 5
                ],
            )?;
            Ok(json!({
                "transaction": id,
                "polling_secret": poll,
                "user_code": code,
                "fingerprint": fingerprint,
                "expires_in": GRANT_SECONDS,
                "interval": 5,
                "verification_uri": verification,
                "verification_uri_complete": format!("{verification}?code={code}"),
            }))
        })
    }
    pub fn grant_view(&self, code: &str, now: i64) -> Result<Value> {
        let code = normalize_code(code);
        self.with(|w| {
            w.db.query_row(
                "SELECT fingerprint,name FROM grants WHERE code=?1 AND expires>?2 AND state='pending'",
                params![hash(&code), now],
                |r| {
                    Ok(json!({
                        "code": code,
                        "fingerprint": r.get::<_, String>(0)?,
                        "name": r.get::<_, String>(1)?,
                    }))
                },
            )
            .optional()?
            .ok_or(Error::Expired)
        })
    }
    pub fn confirm(&self, account: &str, code: &str, fingerprint: &str, now: i64) -> Result<Value> {
        let code = normalize_code(code);
        self.with(|w| {
            let held = held_devices(&w.db, account)?;
            if held >= DEVICES_PER_ACCOUNT {
                return Err(Error::Capacity);
            }
            if w.db.execute(
                "UPDATE grants SET account=?1,state='approved' WHERE code=?2 AND fingerprint=?3 AND expires>?4 AND state='pending'",
                params![account, hash(&code), fingerprint, now],
            )? != 1
            {
                return Err(Error::Expired);
            }
            Ok(json!({"approved": true}))
        })
    }
    pub fn poll(&self, poll: &str, now: i64) -> Result<Value> {
        self.with(|w| {
            let row: (String, String, i64, i64, String) = w
                .db
                .query_row(
                    "SELECT id,public,next_poll,interval,state FROM grants WHERE poll=?1 AND expires>?2",
                    params![hash(poll), now],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?
                .ok_or(Error::Expired)?;
            let (id, public, next, interval, state) = row;
            if state == "consumed" {
                return Err(Error::Expired);
            }
            if now < next {
                w.db.execute(
                    "UPDATE grants SET interval=min(interval+5,60),next_poll=?1+min(interval+5,60) WHERE id=?2",
                    params![now, id],
                )?;
                return Err(Error::SlowDown);
            }
            w.db.execute("UPDATE grants SET next_poll=?1 WHERE id=?2", params![now + interval, id])?;
            if state != "approved" {
                return Err(Error::Pending);
            }
            let nonce = random(32)?;
            w.db.execute(
                "UPDATE grants SET nonce=?1,proof_until=?2 WHERE id=?3",
                params![nonce, now + 30, id],
            )?;
            Ok(json!({"transaction": id, "nonce": nonce, "public_key": public}))
        })
    }
    pub fn verify_grant(&self, poll: &str, signature: &str, now: i64) -> Result<VerifiedGrant> {
        self.with(|w| {
            let row: (String, String, String, String, String) = w
                .db
                .query_row(
                    "SELECT id,nonce,public,name,account FROM grants WHERE poll=?1 AND expires>?2 AND state='approved' AND proof_until>?2",
                    params![hash(poll), now],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?
                .ok_or(Error::Expired)?;
            let (id, nonce, public_key, name, account) = row;
            let message =
                ab_wire::framing::device_link_message(&self.public_url, &id, &nonce, &public_key);
            proof(&public_key, signature, &message)?;
            if held_devices(&w.db, &account)? >= DEVICES_PER_ACCOUNT {
                return Err(Error::Capacity);
            }
            Ok(VerifiedGrant { id, account, public_key, name })
        })
    }
    pub fn finish_grant(&self, grant: &VerifiedGrant, hostname: &str, now: i64) -> Result<()> {
        self.with(|w| {
            let tx = w.db.transaction()?;
            tx.execute(
                "INSERT INTO devices VALUES(?1,?2,?3,?4,'active',?5)",
                params![hostname, grant.account, grant.public_key, grant.name, now],
            )?;
            tx.execute(
                "UPDATE grants SET state='consumed',nonce='' WHERE id=?1",
                [&grant.id],
            )?;
            tx.commit()?;
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
                "SELECT id,hostname,disabled,revoked FROM policy_outbox ORDER BY id LIMIT 100",
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
    pub fn policy_applied(&self, id: i64) -> Result<()> {
        self.with(|w| {
            w.db.execute("DELETE FROM policy_outbox WHERE id=?1", [id])?;
            Ok(())
        })
    }
}
fn held_devices(db: &Connection, account: &str) -> Result<i64> {
    Ok(db.query_row(
        "SELECT count(*) FROM devices WHERE account=?1 AND state!='revoked'",
        [account],
        |r| r.get(0),
    )?)
}
