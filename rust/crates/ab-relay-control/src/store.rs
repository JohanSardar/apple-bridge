use crate::ids::{AccountId, DeviceAction, DeviceId};
use crate::{decode, device_message, encode, hash, random, rotation_message};
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
    #[error("invalid request")]
    Invalid,
    #[error("authentication required")]
    Unauthorized,
    #[error("access denied")]
    Forbidden,
    #[error("capacity exhausted")]
    Capacity,
    #[error("slow_down")]
    SlowDown,
    #[error("authorization_pending")]
    Pending,
    #[error("expired or consumed")]
    Expired,
    #[error("policy unavailable")]
    Unavailable,
    #[error("restore fenced")]
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

impl Control {
    pub fn open(database: &Path, fence: &Path) -> Result<Self> {
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
CREATE TABLE IF NOT EXISTS accounts(id TEXT PRIMARY KEY, subject TEXT UNIQUE NOT NULL);
CREATE TABLE IF NOT EXISTS invites(hash TEXT PRIMARY KEY, expires INTEGER NOT NULL, uses INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions(hash TEXT PRIMARY KEY, account TEXT NOT NULL REFERENCES accounts(id), expires INTEGER NOT NULL, fresh INTEGER NOT NULL, csrf TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS recovery(hash TEXT PRIMARY KEY, account TEXT NOT NULL REFERENCES accounts(id));
CREATE TABLE IF NOT EXISTS passkeys(id TEXT PRIMARY KEY, account TEXT NOT NULL REFERENCES accounts(id), key TEXT NOT NULL, counter INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS challenges(id TEXT PRIMARY KEY, kind TEXT NOT NULL, data TEXT NOT NULL, expires INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS grants(id TEXT PRIMARY KEY, poll TEXT UNIQUE NOT NULL, code TEXT UNIQUE NOT NULL, public TEXT NOT NULL, fingerprint TEXT NOT NULL, slot INTEGER NOT NULL, nonce TEXT NOT NULL, proof_until INTEGER NOT NULL, expires INTEGER NOT NULL, next_poll INTEGER NOT NULL, interval INTEGER NOT NULL, account TEXT, state TEXT NOT NULL CHECK(state IN ('pending','approved','consumed')));
CREATE TABLE IF NOT EXISTS devices(id TEXT PRIMARY KEY, account TEXT NOT NULL REFERENCES accounts(id), public TEXT UNIQUE NOT NULL, slot INTEGER NOT NULL, state TEXT NOT NULL CHECK(state IN ('active','suspended','revoked')), credential TEXT NOT NULL, generation INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS live_slot ON devices(account,slot) WHERE state != 'revoked';
CREATE TABLE IF NOT EXISTS usage(owner TEXT PRIMARY KEY, month TEXT NOT NULL, bytes INTEGER NOT NULL, second INTEGER NOT NULL, rate INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS streams(id TEXT PRIMARY KEY, device TEXT NOT NULL REFERENCES devices(id), expires INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS disconnects(id INTEGER PRIMARY KEY AUTOINCREMENT, device TEXT NOT NULL, epoch INTEGER NOT NULL);
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
        })
    }
    fn with<T>(&self, f: impl FnOnce(&mut Writer) -> Result<T>) -> Result<T> {
        let mut writer = self.writer.lock().map_err(|_| Error::Unavailable)?;
        writer.check()?;
        f(&mut writer)
    }
    pub fn invite(&self, expires: i64, uses: u32) -> Result<String> {
        if uses == 0 {
            return Err(Error::Invalid);
        }
        let secret = random(32)?;
        self.with(|w| {
            w.db.execute(
                "INSERT INTO invites VALUES(?1,?2,?3)",
                params![hash(&secret), expires, uses],
            )?;
            Ok(secret)
        })
    }
    pub fn throttle(&self, source: &str, now: i64) -> Result<()> {
        self.with(|w| {
            w.db.execute("DELETE FROM throttle WHERE window < ?1", [now - 600])?;
            let size: i64 = w.db.query_row("SELECT count(*) FROM throttle", [], |r| r.get(0))?;
            if size >= 4096 { return Err(Error::SlowDown); }
            let key = hash(source);
            w.db.execute("INSERT INTO throttle VALUES(?1,?2,1) ON CONFLICT(source) DO UPDATE SET count=CASE WHEN window < ?3 THEN 1 ELSE count+1 END, window=CASE WHEN window < ?3 THEN ?2 ELSE window END", params![key,now,now-60])?;
            let count: i64 = w.db.query_row("SELECT count FROM throttle WHERE source=?1", [key], |r| r.get(0))?;
            if count > 12 { return Err(Error::SlowDown); }
            Ok(())
        })
    }
    pub(crate) fn challenge(&self, kind: &str, data: &Value, now: i64) -> Result<String> {
        self.with(|w| {
            w.db.execute("DELETE FROM challenges WHERE expires <= ?1", [now])?;
            let count: i64 =
                w.db.query_row("SELECT count(*) FROM challenges", [], |r| r.get(0))?;
            if count >= 4096 {
                return Err(Error::Capacity);
            }
            let id = random(32)?;
            w.db.execute(
                "INSERT INTO challenges VALUES(?1,?2,?3,?4)",
                params![hash(&id), kind, data.to_string(), now + 300],
            )?;
            Ok(id)
        })
    }
    pub(crate) fn take_challenge(&self, id: &str, kind: &str, now: i64) -> Result<Value> {
        self.with(|w| {
            let data: String =
                w.db.query_row(
                    "DELETE FROM challenges WHERE id=?1 AND kind=?2 AND expires>?3 RETURNING data",
                    params![hash(id), kind, now],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(Error::Expired)?;
            Ok(serde_json::from_str(&data)?)
        })
    }
    pub(crate) fn login(&self, subject: &str, invite: &str, now: i64) -> Result<Value> {
        if subject.is_empty() || subject.len() > 128 {
            return Err(Error::Invalid);
        }
        self.with(|w| {
            let tx = w.db.transaction()?;
            let account: Option<String> = tx
                .query_row("SELECT id FROM accounts WHERE subject=?1", [subject], |r| {
                    r.get(0)
                })
                .optional()?;
            let id = if let Some(id) = account {
                id
            } else {
                let count: i64 = tx.query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))?;
                if count >= 25 {
                    return Err(Error::Capacity);
                }
                if tx.execute(
                    "UPDATE invites SET uses=uses-1 WHERE hash=?1 AND expires>?2 AND uses>0",
                    params![invite, now],
                )? != 1
                {
                    return Err(Error::Forbidden);
                }
                let id = random(16)?;
                tx.execute("INSERT INTO accounts VALUES(?1,?2)", params![id, subject])?;
                id
            };
            let session = new_session(&tx, &id, now)?;
            tx.commit()?;
            Ok(session)
        })
    }
    pub(crate) fn session(
        &self,
        secret: &str,
        csrf: Option<&str>,
        fresh: bool,
        now: i64,
    ) -> Result<String> {
        self.with(|w| {
            let (account, refreshed, token): (String, i64, String) =
                w.db.query_row(
                    "SELECT account,fresh,csrf FROM sessions WHERE hash=?1 AND expires>?2",
                    params![hash(secret), now],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?
                .ok_or(Error::Unauthorized)?;
            if csrf.is_some_and(|v| hash(v) != token) {
                return Err(Error::Forbidden);
            }
            if fresh && (refreshed > now || now - refreshed > 300) {
                return Err(Error::Unauthorized);
            }
            Ok(account)
        })
    }
    pub(crate) fn recover(&self, account: &str, code: &str, now: i64) -> Result<Value> {
        self.with(|w| {
            let tx = w.db.transaction()?;
            if tx.execute(
                "DELETE FROM recovery WHERE hash=?1 AND account=?2",
                params![hash(code), account],
            )? != 1
            {
                return Err(Error::Unauthorized);
            }
            tx.execute("DELETE FROM sessions WHERE account=?1", [&account])?;
            let value = new_session(&tx, account, now)?;
            tx.commit()?;
            Ok(value)
        })
    }
    pub(crate) fn save_passkey(
        &self,
        account: &str,
        id: &str,
        key: &str,
        counter: u32,
    ) -> Result<Vec<String>> {
        self.with(|w| {
            let tx = w.db.transaction()?;
            tx.execute(
                "INSERT INTO passkeys VALUES(?1,?2,?3,?4)",
                params![id, account, key, counter],
            )?;
            tx.execute("DELETE FROM recovery WHERE account=?1", [&account])?;
            let codes = (0..8).map(|_| random(24)).collect::<Result<Vec<_>>>()?;
            for code in &codes {
                tx.execute(
                    "INSERT INTO recovery VALUES(?1,?2)",
                    params![hash(code), account],
                )?;
            }
            tx.commit()?;
            Ok(codes)
        })
    }
    pub(crate) fn passkeys(&self, account: &str) -> Result<Value> {
        self.with(|w| {
            let mut q = w.db.prepare("SELECT id FROM passkeys WHERE account=?1")?;
            let ids = q
                .query_map([&account], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(json!(ids))
        })
    }
    pub(crate) fn authenticate_passkey(
        &self,
        account: &str,
        id: &str,
        verify: impl FnOnce(&str, u32) -> Result<u32>,
        now: i64,
    ) -> Result<Value> {
        self.with(|w| {
            let tx = w.db.transaction()?;
            let (key, counter): (String, u32) = tx
                .query_row(
                    "SELECT key,counter FROM passkeys WHERE account=?1 AND id=?2",
                    params![account, id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or(Error::Unauthorized)?;
            let next = verify(&key, counter)?;
            tx.execute(
                "UPDATE passkeys SET counter=?1 WHERE id=?2",
                params![next, id],
            )?;
            let session = new_session(&tx, account, now)?;
            tx.commit()?;
            Ok(session)
        })
    }
    pub fn start_grant(&self, public: &str, slot: u8, now: i64) -> Result<Value> {
        let key: [u8; 32] = decode(public)?.try_into().map_err(|_| Error::Invalid)?;
        VerifyingKey::from_bytes(&key).map_err(|_| Error::Invalid)?;
        if !(1..=2).contains(&slot) {
            return Err(Error::Invalid);
        }
        self.with(|w| {
            w.db.execute("DELETE FROM grants WHERE expires<=?1",[now])?;
            let n:i64=w.db.query_row("SELECT count(*) FROM grants",[],|r|r.get(0))?;
            if n>=4096 { return Err(Error::Capacity); }
            let id=random(24)?; let poll=random(32)?; let code=encode(decode(&random(6)?)?).to_uppercase(); let nonce=random(32)?;
            let fingerprint=hash(key);
            w.db.execute("INSERT INTO grants VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9,5,NULL,'pending')",params![id,hash(&poll),hash(&code),public,fingerprint,slot,nonce,now+600,now+5])?;
            Ok(json!({"transaction":id,"polling_secret":poll,"user_code":code,"fingerprint":fingerprint,"expires_in":600,"interval":5,"verification_uri":"https://app.lexaai.tech/device"}))
        })
    }
    pub(crate) fn confirm_view(&self, code: &str, now: i64) -> Result<Value> {
        self.with(|w| {
            w.db.query_row("SELECT fingerprint,slot FROM grants WHERE code=?1 AND expires>?2 AND state='pending'",params![hash(code),now],|r|Ok(json!({"code":code,"fingerprint":r.get::<_,String>(0)?,"slot":r.get::<_,u8>(1)?}))).optional()?.ok_or(Error::Expired)
        })
    }
    pub(crate) fn confirm(
        &self,
        account: &str,
        code: &str,
        fingerprint: &str,
        now: i64,
    ) -> Result<Value> {
        self.with(|w| {
            if w.db.execute("UPDATE grants SET account=?1,state='approved' WHERE code=?2 AND fingerprint=?3 AND expires>?4 AND state='pending'",params![account,hash(code),fingerprint,now])?!=1 {return Err(Error::Expired);}
            Ok(json!({"approved":true}))
        })
    }
    pub fn poll(&self, poll: &str, now: i64) -> Result<Value> {
        self.with(|w| {
            let (id,_nonce,public,slot,next,interval,state):(String,String,String,u8,i64,i64,String)=w.db.query_row("SELECT id,nonce,public,slot,next_poll,interval,state FROM grants WHERE poll=?1 AND expires>?2",params![hash(poll),now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?.ok_or(Error::Expired)?;
            if state=="consumed" {return Err(Error::Expired);}
            if now<next {
                w.db.execute("UPDATE grants SET interval=min(interval+5,60),next_poll=?1+min(interval+5,60) WHERE id=?2",params![now,id])?;
                return Err(Error::SlowDown);
            }
            w.db.execute("UPDATE grants SET next_poll=?1 WHERE id=?2",params![now+interval,id])?;
            if state!="approved" {return Err(Error::Pending);}
            let nonce=random(32)?;
            w.db.execute("UPDATE grants SET nonce=?1,proof_until=?2 WHERE id=?3",params![nonce,now+30,id])?;
            Ok(json!({"transaction":id,"nonce":nonce,"public_key":public,"slot":slot}))
        })
    }
    pub fn complete(&self, poll: &str, signature: &str, now: i64) -> Result<Value> {
        self.with(|w| {
            let tx=w.db.transaction()?;
            let (id,nonce,public,slot,account):(String,String,String,u8,String)=tx.query_row("SELECT id,nonce,public,slot,account FROM grants WHERE poll=?1 AND expires>?2 AND state='approved' AND proof_until>?2",params![hash(poll),now],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or(Error::Expired)?;
            proof(&public,signature,&device_message(&id,&nonce,&public,slot))?;
            let count:i64=tx.query_row("SELECT count(*) FROM devices WHERE account=?1 AND state!='revoked'",[&account],|r|r.get(0))?;
            let occupied:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE account=?1 AND slot=?2 AND state!='revoked')",params![account,slot],|r|r.get(0))?;
            if count>=2 || occupied {return Err(Error::Capacity);}
            let device = decode(&random(12)?)?.iter().flat_map(|byte| [char::from(b"0123456789abcdef"[usize::from(byte >> 4)]), char::from(b"0123456789abcdef"[usize::from(byte & 15)])]).collect::<String>();
            let credential=random(32)?;
            tx.execute("INSERT INTO devices VALUES(?1,?2,?3,?4,'active',?5,1)",params![device,account,public,slot,hash(&credential)])?;
            tx.execute("UPDATE grants SET state='consumed',nonce='' WHERE id=?1",[id])?;
            tx.commit()?;
            Ok(json!({"device":device,"hostname":format!("{device}.ab.lexaai.tech"),"credential":credential,"generation":1}))
        })
    }
    pub(crate) fn list(&self, account: &str) -> Result<Value> {
        let account = AccountId::parse(account)?;
        self.with(|w| {
            let mut q=w.db.prepare("SELECT id,public,slot,state,generation FROM devices WHERE account=?1")?;
            let rows=q.query_map([&account],|r|Ok(json!({"id":r.get::<_,String>(0)?,"fingerprint":r.get::<_,String>(1).ok().and_then(|v|decode(&v).ok()).map(hash),"slot":r.get::<_,u8>(2)?,"state":r.get::<_,String>(3)?,"generation":r.get::<_,i64>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
            let usage:Option<(String,i64)>=w.db.query_row("SELECT month,bytes FROM usage WHERE owner=?1",[&account],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            Ok(json!({"devices":rows,"usage":usage,"credentials":self::json_credentials(&w.db,&account)?}))
        })
    }
    pub(crate) fn change(
        &self,
        account: &str,
        device: &str,
        action: &str,
        key: &str,
        signature: &str,
        nonce: &str,
    ) -> Result<Value> {
        let account = AccountId::parse(account)?;
        let device = DeviceId::parse(device)?;
        let action = DeviceAction::parse(action)?;
        self.with(|w| {
            let state:Option<String>=w.db.query_row("SELECT state FROM devices WHERE account=?1 AND id=?2",params![account,device],|r|r.get(0)).optional()?;
            let current=state.ok_or(Error::Forbidden)?;
            let state=action.transition(&current)?;
            if action==DeviceAction::Rotate { proof(key,signature,&rotation_message(&device,nonce,key))?; }
            if action==DeviceAction::Rotate {
                let used:bool=w.db.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE public=?1 AND id!=?2)",params![key,device],|r|r.get(0))?;
                if used {return Err(Error::Forbidden);}
            }
            let epoch=w.advance()?;
            let tx=w.db.transaction()?;
            let credential=random(32)?;
            if action==DeviceAction::Rotate {
                tx.execute("UPDATE devices SET public=?1,credential=?2,generation=generation+1 WHERE id=?3",params![key,hash(&credential),device])?;
            } else {
                tx.execute("UPDATE devices SET state=?1,generation=generation+1 WHERE id=?2",params![state,device])?;
            }
            tx.execute("DELETE FROM streams WHERE device=?1",[&device])?;
            tx.execute("UPDATE fence SET epoch=?1",[epoch])?;
            tx.execute("INSERT INTO disconnects(device,epoch) VALUES(?1,?2)",params![device,epoch])?;
            tx.commit()?;
            Ok(json!({"committed":true,"disconnect":device,"epoch":epoch,"credential":if action==DeviceAction::Rotate{Some(credential)}else{None}}))
        })
    }
    pub(crate) fn delete_account(&self, account: &str) -> Result<Value> {
        let account = AccountId::parse(account)?;
        self.with(|w| {
            let epoch=w.advance()?;
            let tx=w.db.transaction()?;
            tx.execute("INSERT INTO disconnects(device,epoch) SELECT id,?1 FROM devices WHERE account=?2 AND state!='revoked'",params![epoch,account])?;
            tx.execute("DELETE FROM streams WHERE device IN (SELECT id FROM devices WHERE account=?1)",[&account])?;
            tx.execute("UPDATE devices SET state='revoked',credential='',generation=generation+1 WHERE account=?1",[&account])?;
            for table in ["sessions","recovery","passkeys","grants"] {
                tx.execute(&format!("DELETE FROM {table} WHERE account=?1"),[&account])?;
            }
            tx.execute("DELETE FROM usage WHERE owner=?1 OR owner IN (SELECT id FROM devices WHERE account=?1)",[&account])?;
            tx.execute("UPDATE accounts SET subject=?1 WHERE id=?2",params![format!("deleted:{}",hash(&*account)),account])?;
            tx.execute("UPDATE fence SET epoch=?1",[epoch])?;
            tx.commit()?;
            Ok(json!({"deleted":true,"epoch":epoch}))
        })
    }
    pub fn pending_disconnects(&self) -> Result<Value> {
        self.with(|w| {
            let mut query=w.db.prepare("SELECT id,device,epoch FROM disconnects ORDER BY id LIMIT 1000")?;
            let rows=query.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"device":r.get::<_,String>(1)?,"epoch":r.get::<_,i64>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
            Ok(json!(rows))
        })
    }
    pub fn ack_disconnect(&self, id: i64) -> Result<()> {
        self.with(|w| {
            w.db.execute("DELETE FROM disconnects WHERE id=?1", [id])?;
            Ok(())
        })
    }
    pub fn admit(
        &self,
        device: &str,
        credential: &str,
        generation: i64,
        now: i64,
    ) -> Result<Value> {
        let device = DeviceId::parse(device)?;
        self.with(|w| {
            let tx=w.db.transaction()?;
            tx.execute("DELETE FROM streams WHERE expires<=?1",[now])?;
            let account:String=tx.query_row("SELECT account FROM devices WHERE id=?1 AND credential=?2 AND generation=?3 AND state='active'",params![device,hash(credential),generation],|r|r.get(0)).optional()?.ok_or(Error::Forbidden)?;
            let n:i64=tx.query_row("SELECT count(*) FROM streams WHERE device=?1",[&device],|r|r.get(0))?;
            let a:i64=tx.query_row("SELECT count(*) FROM streams s JOIN devices d ON s.device=d.id WHERE d.account=?1",[&account],|r|r.get(0))?;
            if n>=32 || a>=64 {return Err(Error::Capacity);}
            let lease=random(32)?;
            tx.execute("INSERT INTO streams VALUES(?1,?2,?3)",params![hash(&lease),device,now+30])?;
            tx.commit()?;
            Ok(json!({"lease":lease,"expires":now+30}))
        })
    }
    pub fn release(&self, lease: &str) -> Result<()> {
        self.with(|w| {
            w.db.execute("DELETE FROM streams WHERE id=?1", [hash(lease)])?;
            Ok(())
        })
    }
    pub fn charge(&self, lease: &str, upstream: u64, downstream: u64, now: i64) -> Result<()> {
        let bytes = upstream.checked_add(downstream).ok_or(Error::Capacity)?;
        if bytes > 1_048_576 {
            return Err(Error::Capacity);
        }
        let stamp = jiff::Timestamp::from_second(now)
            .map_err(|_| Error::Invalid)?
            .to_zoned(jiff::tz::TimeZone::UTC);
        let month = format!("{:04}-{:02}", stamp.year(), stamp.month());
        self.with(|w| {
            let tx=w.db.transaction()?;
            let (device,account):(String,String)=tx.query_row("SELECT d.id,d.account FROM streams s JOIN devices d ON d.id=s.device WHERE s.id=?1 AND s.expires>?2 AND d.state='active'",params![hash(lease),now],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::Forbidden)?;
            for owner in [&device,&account] {
                tx.execute("INSERT INTO usage VALUES(?1,?2,0,?3,0) ON CONFLICT(owner) DO UPDATE SET bytes=CASE WHEN month!=?2 THEN 0 ELSE bytes END, month=?2,rate=CASE WHEN second!=?3 THEN 0 ELSE rate END,second=?3",params![owner,month,now])?;
                if tx.execute("UPDATE usage SET bytes=bytes+?1,rate=rate+?1 WHERE owner=?2 AND bytes+?1<=10737418240 AND rate+?1<=1048576",params![i64::try_from(bytes).map_err(|_|Error::Capacity)?,owner])?!=1 {return Err(Error::Capacity);}
            }
            tx.execute("UPDATE streams SET expires=?1 WHERE id=?2",params![now+30,hash(lease)])?;
            tx.commit()?;
            Ok(())
        })
    }
}
fn new_session(db: &Connection, account: &str, now: i64) -> Result<Value> {
    let secret = random(32)?;
    let csrf = random(32)?;
    db.execute("DELETE FROM sessions WHERE expires<=?1", [now])?;
    db.execute(
        "INSERT INTO sessions VALUES(?1,?2,?3,?4,?5)",
        params![hash(&secret), account, now + 3600, now, hash(&csrf)],
    )?;
    Ok(json!({"session":secret,"csrf":csrf,"account":account}))
}
fn json_credentials(db: &Connection, account: &str) -> Result<Vec<String>> {
    let mut q = db.prepare("SELECT id FROM passkeys WHERE account=?1")?;
    Ok(q.query_map([&account], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}
fn proof(public: &str, signature: &str, message: &[u8]) -> Result<()> {
    let bytes: [u8; 32] = decode(public)?.try_into().map_err(|_| Error::Invalid)?;
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| Error::Invalid)?;
    let sig = Signature::from_slice(&decode(signature)?).map_err(|_| Error::Invalid)?;
    key.verify_strict(message, &sig)
        .map_err(|_| Error::Forbidden)
}
