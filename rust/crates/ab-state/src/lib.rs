#![forbid(unsafe_code)]
mod completion;
mod protection;
pub use protection::{BlobProtection, ReferenceSet};
mod gateway;
mod health;
mod ledger;
pub use gateway::{CommitResult, FrozenSnapshot, restore};
pub use ledger::{BlobReference, Command, CommandError, CommandEvent, CommandPart, CommandRecord};
mod models;
mod notes;
mod projection;
mod query;
mod reminders;
mod scope;
mod specialized;
use ab_domain::{Collection, ErrorCode, Record};
use ab_wire::{
    gojson::{self, EscapeMode},
    messages::{Batch, FeedFrame},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::io::Read;
use std::{collections::HashMap, marker::PhantomData, path::Path};

pub type Visibility = HashMap<String, bool>;
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("command ledger missing; run bridge restore --accept-ledger-loss")]
    LedgerMissing,
    #[error("id_reused: command ID already identifies another request")]
    IdReused,
    #[error("{0}")]
    Contract(#[from] ErrorCode),
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Wire(#[from] gojson::Error),
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub epoch: String,
    pub subscription_revision: u64,
    pub sequence: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub sequence: u64,
    pub op: String,
    pub record: Record,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReadQuery {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub include_aliases: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    pub collection: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub include_deleted: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub after: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub limit: usize,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(count: &usize) -> bool {
    *count == 0
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alias {
    pub source: String,
    pub generation: String,
    pub collection: String,
    pub native_key: String,
    pub record_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadResult {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<Alias>,
    pub records: Vec<Record>,
    pub cursor: Cursor,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next: String,
}
#[derive(Debug)]
pub enum Replica {}
#[derive(Debug)]
pub enum Gateway {}
#[derive(Debug)]
pub struct State<Role> {
    db: Connection,
    ledger: Option<Connection>,
    blob_protection: Option<std::sync::Arc<dyn BlobProtection>>,
    capacity: HashMap<String, bool>,
    capture_failures: HashMap<String, (u64, String, bool)>,
    snapshot_refs: HashMap<String, Vec<serde_json::Value>>,
    leases: HashMap<String, (u64, jiff::Timestamp)>,
    pending: Option<(u64, u64)>,
    generation: u64,
    role: PhantomData<Role>,
    replica_status: Option<serde_json::Value>,
}
#[derive(Debug)]
pub struct SnapshotInstall {
    id: String,
    cursor: Cursor,
    count: i64,
    digest: String,
    expires: jiff::Timestamp,
    ordinal: i64,
    generation: u64,
}
#[derive(Debug)]
pub struct Changes {
    parts: bool,
    visibility: Visibility,
    cursor: Cursor,
}
fn encoded<T: Serialize>(visibility: &T) -> Result<Vec<u8>> {
    Ok(gojson::legacy_json(visibility, EscapeMode::Html)?)
}
fn head(db: &Connection) -> Result<Cursor> {
    let raw: String = db.query_row("SELECT value FROM meta WHERE key='cursor'", [], |record| {
        record.get(0)
    })?;
    Ok(serde_json::from_str(&raw)?)
}
fn set_meta(db: &Connection, key: &str, value: &str) -> Result<()> {
    db.execute(
        "INSERT INTO meta VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}
fn set_cursor(db: &Connection, c: &Cursor) -> Result<()> {
    set_meta(db, "cursor", &String::from_utf8_lossy(&encoded(c)?))?;
    set_meta(db, "epoch", &c.epoch)?;
    if c.sequence > 0 {
        db.execute(
            "INSERT OR IGNORE INTO journal_commits VALUES(?,?)",
            params![c.epoch, number(c.sequence)?],
        )?;
    }
    Ok(())
}
fn source(record: &Record) -> Result<String> {
    Ok(serde_json::from_value(serde_json::to_value(
        record.source,
    )?)?)
}
pub(crate) fn allows(visibility: &Visibility, record: &Record) -> Result<bool> {
    if !visibility.get(&source(record)?).copied().unwrap_or(false) {
        return Ok(false);
    }
    if record.collection != Collection::SystemCommand
        || visibility.get("commands:all") == Some(&true)
    {
        return Ok(true);
    }
    let fields: serde_json::Value = serde_json::from_str(record.data.get())?;
    let p = fields["principal"].as_str().unwrap_or("");
    Ok(visibility.get(&format!("principal:{p}")) == Some(&true)
        || p.split_once('/')
            .is_some_and(|(_, p)| visibility.get(&format!("principal:{p}")) == Some(&true)))
}
pub(crate) fn project(mut record: Record, visibility: &Visibility) -> Result<Record> {
    if record.collection == Collection::MessagesHandle && visibility.get("contacts") != Some(&true)
    {
        let mut fields: serde_json::Value = serde_json::from_str(record.data.get())?;
        if let Some(fields) = fields.as_object_mut() {
            fields.remove("contact_id");
        }
        record.data = serde_json::value::RawValue::from_string(
            String::from_utf8_lossy(&gojson::record_data(
                &serde_json::to_vec(&fields)?,
                EscapeMode::Html,
            )?)
            .into_owned(),
        )?;
    }
    Ok(record)
}
pub fn digest(records: &[Record]) -> String {
    let mut records: Vec<_> = records.iter().collect();
    records
        .sort_by_key(|record| format!("{}/{}", collection(record.collection), record.id.as_str()));
    let mut h = Sha256::new();
    for record in records {
        h.update(format!(
            "{}\0{}\0{}\n",
            collection(record.collection),
            record.id.as_str(),
            record.version
        ));
    }
    format!("{:x}", h.finalize())
}
pub(crate) fn records(
    db: &Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<Record>> {
    let mut stmt = db.prepare_cached(sql)?;
    let mut rows = stmt.query(args)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let raw = row.get_ref(0)?;
        let (rusqlite::types::ValueRef::Text(bytes) | rusqlite::types::ValueRef::Blob(bytes)) = raw
        else {
            return Err(ErrorCode::Corrupt.into());
        };
        out.push(serde_json::from_slice(bytes)?);
    }
    Ok(out)
}
fn upgrade_schema(db: &mut Connection, home: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let backup = home.join("before-schema-2.db");
    if !backup.exists() {
        db.execute(
            "VACUUM INTO ?",
            [backup.to_str().ok_or(ErrorCode::Invalid)?],
        )?;
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))?;
    }
    let tx = db.transaction()?;
    let columns: i64 = tx.query_row(
        "SELECT count(*) FROM pragma_table_info('journal') WHERE name='committed_at'",
        [],
        |r| r.get(0),
    )?;
    if columns == 0 {
        tx.execute_batch("ALTER TABLE journal ADD COLUMN committed_at INTEGER NOT NULL DEFAULT 0")?;
    }
    tx.execute_batch("UPDATE journal SET committed_at=unixepoch() WHERE committed_at=0; UPDATE meta SET value='2' WHERE key='schema_version'")?;
    tx.commit()?;
    Ok(())
}
impl State<Replica> {
    pub fn open(home: impl AsRef<Path>) -> Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        let home = home.as_ref();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(home)?;
        let path = home.join("state.db");
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        let existing: Option<String> = db
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |record| record.get(0),
            )
            .optional()
            .or_else(|e| {
                if e.to_string().contains("no such table") {
                    Ok(None)
                } else {
                    Err(e)
                }
            })?;
        if existing
            .as_deref()
            .is_some_and(|visibility| visibility != "2" && visibility != "1")
        {
            return Err(ErrorCode::Unsupported.into());
        }
        if existing.as_deref() == Some("1") {
            upgrade_schema(&mut db, home)?;
        }
        let tx = db.transaction()?;
        tx.execute_batch(include_str!("schema.sql"))?;
        if tx
            .query_row("SELECT value FROM meta WHERE key='cursor'", [], |record| {
                record.get::<_, String>(0)
            })
            .optional()?
            .is_none()
        {
            let mut random = [0u8; 16];
            std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
            let mut epoch = String::new();
            for byte in random {
                let _ = write!(epoch, "{byte:02X}");
            }
            set_cursor(
                &tx,
                &Cursor {
                    epoch: epoch.clone(),
                    subscription_revision: 1,
                    sequence: 0,
                },
            )?;
            for (k, visibility) in [
                ("owner_fence", "0"),
                ("schema_version", "2"),
                ("retention_floor", "0"),
                ("admission_era", epoch.as_str()),
            ] {
                set_meta(&tx, k, visibility)?;
            }
        }
        let cursor = head(&tx)?;
        tx.execute("DELETE FROM journal_commits WHERE epoch<>? OR through_seq>? OR through_seq<=CAST((SELECT value FROM meta WHERE key='retention_floor') AS INTEGER)",params![cursor.epoch,number(cursor.sequence)?])?;
        tx.execute("INSERT OR IGNORE INTO journal_commits SELECT ?,? WHERE ?>CAST((SELECT value FROM meta WHERE key='retention_floor') AS INTEGER) AND EXISTS(SELECT 1 FROM journal)",params![cursor.epoch,number(cursor.sequence)?,number(cursor.sequence)?])?;
        projection::initialize(&tx)?;
        tx.execute_batch(
            "DELETE FROM blob_refs WHERE owner LIKE 'capture-stage:%' OR owner LIKE 'staging:%';",
        )?;
        commit(tx)?;
        Ok(Self {
            db,
            ledger: None,
            blob_protection: None,
            capacity: HashMap::new(),
            capture_failures: HashMap::new(),
            snapshot_refs: HashMap::new(),
            leases: HashMap::new(),
            pending: None,
            generation: 0,
            role: PhantomData,
            replica_status: None,
        })
    }
}
impl<Role> State<Role> {
    pub fn head(&self) -> Result<Cursor> {
        head(&self.db)
    }
    pub fn retention_floor(&self) -> Result<u64> {
        Ok(u64::try_from(self.db.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='retention_floor'",
            [],
            |record| record.get::<_, i64>(0),
        )?)
        .map_err(|_| ErrorCode::Corrupt)?)
    }
    pub fn discard_capture(&mut self) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute_batch("DELETE FROM pending_changes; DELETE FROM blob_refs WHERE owner LIKE 'capture-stage:%';")?;
        commit(tx)?;
        self.pending = None;
        Ok(())
    }
}
impl State<Replica> {
    pub fn begin_install(
        &mut self,
        begin: &FeedFrame,
        epoch: &str,
        revision: u64,
    ) -> Result<SnapshotInstall> {
        self.discard_capture()?;
        let expires: jiff::Timestamp = begin
            .expires_at
            .as_str()
            .parse()
            .map_err(|_| ErrorCode::Invalid)?;
        if begin.id.is_empty()
            || begin.count < 0
            || begin.digest.is_empty()
            || expires < jiff::Timestamp::now()
        {
            return Err(ErrorCode::Invalid.into());
        }
        self.db.execute("DELETE FROM staging_records", [])?;
        self.generation += 1;
        Ok(SnapshotInstall {
            id: begin.id.clone(),
            cursor: Cursor {
                epoch: epoch.into(),
                subscription_revision: revision,
                sequence: begin.s,
            },
            count: begin.count,
            digest: begin.digest.clone(),
            expires,
            ordinal: 0,
            generation: self.generation,
        })
    }
    pub fn snapshot_page(&mut self, i: &mut SnapshotInstall, page: &FeedFrame) -> Result<()> {
        if i.generation != self.generation
            || page.id != i.id
            || page.ordinal != i.ordinal
            || i.expires < jiff::Timestamp::now()
        {
            return Err(ErrorCode::Corrupt.into());
        }
        let tx = self.db.transaction()?;
        for raw in page.records.iter().flatten() {
            let record: Record = serde_json::from_str(raw.get())?;
            if record.version == 0 || record.deleted {
                return Err(ErrorCode::Corrupt.into());
            }
            tx.execute(
                "INSERT INTO staging_records VALUES(?,?,?,?)",
                params![
                    i.id,
                    collection(record.collection),
                    record.id.as_str(),
                    encoded(&record)?
                ],
            )?;
            projection::blob_refs(
                &tx,
                &format!("staging:{}:{}", i.id, record.id.as_str()),
                &record,
            )?;
        }
        commit(tx)?;
        i.ordinal += 1;
        i.expires = jiff::Timestamp::now()
            .checked_add(std::time::Duration::from_secs(600))
            .map_err(|_| ErrorCode::Invalid)?;
        Ok(())
    }
    pub fn install_snapshot(&mut self, i: SnapshotInstall) -> Result<Cursor> {
        if i.generation != self.generation || i.expires < jiff::Timestamp::now() {
            return Err(ErrorCode::Corrupt.into());
        }
        let tx = self.db.transaction()?;
        let rs = records(
            &tx,
            "SELECT payload FROM staging_records WHERE snapshot=? ORDER BY collection,id",
            &[&i.id],
        )?;
        if i64::try_from(rs.len()).ok() != Some(i.count) || digest(&rs) != i.digest {
            return Err(ErrorCode::Corrupt.into());
        }
        tx.execute_batch("DELETE FROM handle_address; DELETE FROM chat_participant; DELETE FROM msg_by_chat; DELETE FROM reaction_by_message; DELETE FROM attachment_by_message; DELETE FROM activity_by_chat; DELETE FROM chat_summary; DELETE FROM records; DELETE FROM journal; DELETE FROM journal_commits; DELETE FROM applied_batches; DELETE FROM blob_refs WHERE owner LIKE 'record:%';")?;
        for record in rs {
            projection::write(&tx, &record, i.cursor.sequence)?;
        }
        set_meta(&tx, "replica_base", &i.cursor.sequence.to_string())?;
        set_meta(&tx, "retention_floor", &i.cursor.sequence.to_string())?;
        set_cursor(&tx, &i.cursor)?;
        tx.execute_batch(
            "DELETE FROM staging_records; DELETE FROM blob_refs WHERE owner LIKE 'staging:%';",
        )?;
        commit(tx)?;
        Ok(i.cursor)
    }
    pub fn apply_batch(&mut self, b: &Batch) -> Result<Cursor> {
        let tx = self.db.transaction()?;
        let mut c = head(&tx)?;
        if b.epoch != c.epoch || b.subscription_revision != c.subscription_revision {
            return Err(ErrorCode::Reset.into());
        }
        if b.through <= c.sequence {
            return Ok(c);
        }
        let expected = self.pending.map_or(c.sequence, |(_, through)| through);
        if b.after != expected || b.through <= b.after {
            return Err(ErrorCode::Conflict.into());
        }
        let staged = self.pending.is_some() || b.partial;
        let mut previous = b.after;
        for raw in b.changes.iter().flatten() {
            let change: Change = serde_json::from_str(raw.get())?;
            if change.sequence <= previous
                || change.sequence > b.through
                || change.record.version == 0
                || !["upsert", "delete", "evict"].contains(&change.op.as_str())
            {
                return Err(ErrorCode::Corrupt.into());
            }
            previous = change.sequence;
            if staged {
                tx.execute(
                    "INSERT INTO pending_changes VALUES(?,?)",
                    params![number(change.sequence)?, encoded(&change)?],
                )?;
                projection::blob_refs(
                    &tx,
                    &format!("capture-stage:{}", change.sequence),
                    &change.record,
                )?;
            } else {
                apply(&tx, &c.epoch, &change)?;
            }
        }
        if b.partial {
            commit(tx)?;
            self.pending = Some((self.pending.map_or(b.after, |(after, _)| after), b.through));
            return Ok(c);
        }
        let after = self.pending.map_or(b.after, |(after, _)| after);
        if staged {
            let mut stmt = tx.prepare("SELECT payload FROM pending_changes ORDER BY seq")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let raw: Vec<u8> = row.get(0)?;
                apply(&tx, &c.epoch, &serde_json::from_slice(&raw)?)?;
            }
            drop(rows);
            drop(stmt);
            tx.execute_batch("DELETE FROM pending_changes; DELETE FROM blob_refs WHERE owner LIKE 'capture-stage:%';")?;
        }
        #[cfg(feature = "lab")]
        if staged
            || b.changes
                .as_ref()
                .is_some_and(|changes| !changes.is_empty())
        {
            apply_boundary(&tx)?;
        }
        tx.execute(
            "INSERT INTO applied_batches VALUES(?,?)",
            params![number(after)?, number(b.through)?],
        )?;
        c.sequence = b.through;
        set_cursor(&tx, &c)?;
        commit(tx)?;
        self.pending = None;
        Ok(c)
    }
}
impl<Role> State<Role> {
    pub fn read(&self, visibility: &Visibility, query: &ReadQuery) -> Result<ReadResult> {
        if query.limit > 10000
            || (!query.collection.is_empty()
                && !Collection::ALL
                    .iter()
                    .any(|c| collection(*c) == query.collection))
        {
            return Err(ErrorCode::Invalid.into());
        }
        let tx = self.db.unchecked_transaction()?;
        let mut out = ReadResult {
            records: Vec::new(),
            cursor: head(&tx)?,
            next: String::new(),
            aliases: Vec::new(),
        };
        let (ac, ai) = query.after.split_once('/').unwrap_or((&query.after, ""));
        let sources: Vec<_> = visibility
            .iter()
            .filter(|(_, allowed)| **allowed)
            .map(|(s, _)| s)
            .collect();
        let sources = serde_json::to_string(&sources)?;
        let mut sql="SELECT payload FROM records WHERE (?1='' OR collection=?1) AND (?2='' OR id=?2) AND (deleted=0 OR ?3) AND (collection,id)>(?4,?5) AND source IN (SELECT value FROM json_each(?6))".to_owned();
        let terms = query
            .text
            .split_whitespace()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        if !query.text.is_empty() {
            sql.push_str(" AND (collection,id) IN (SELECT collection,id FROM records_fts WHERE records_fts MATCH ?7)");
        }
        sql.push_str(" ORDER BY collection,id");
        let mut stmt = tx.prepare_cached(&sql)?;
        let base: [&dyn rusqlite::ToSql; 7] = [
            &query.collection,
            &query.id,
            &query.include_deleted,
            &ac,
            &ai,
            &sources,
            &terms,
        ];
        let mut rows = stmt.query(&base[..if query.text.is_empty() { 6 } else { 7 }])?;
        let limit = if query.limit == 0 { 1000 } else { query.limit };
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(0)?;
            let record: Record = serde_json::from_slice(&raw)?;
            if !allows(visibility, &record)?
                || format!("{}/{}", collection(record.collection), record.id.as_str())
                    <= query.after
            {
                continue;
            }
            if out.records.len() == limit {
                if let Some(last) = out.records.last() {
                    out.next = format!("{}/{}", collection(last.collection), last.id.as_str());
                }
                break;
            }
            out.records.push(project(record, visibility)?);
        }
        drop(rows);
        drop(stmt);
        if query.include_aliases {
            for record in &out.records {
                let mut stmt=tx.prepare_cached("SELECT a.source,a.generation,a.collection,a.native_key,a.record_id FROM aliases a JOIN checkpoints c ON a.source=c.source AND a.generation=c.generation WHERE a.collection=? AND a.record_id=? ORDER BY a.source,a.generation,a.native_key")?;
                let aliases = stmt.query_map(
                    params![collection(record.collection), record.id.as_str()],
                    |record| {
                        Ok(Alias {
                            source: record.get(0)?,
                            generation: record.get(1)?,
                            collection: record.get(2)?,
                            native_key: record.get(3)?,
                            record_id: record.get(4)?,
                        })
                    },
                )?;
                for a in aliases {
                    out.aliases.push(a?);
                }
            }
        }
        if !query.id.is_empty() && out.records.is_empty() {
            return Err(ErrorCode::NotFound.into());
        }
        Ok(out)
    }
    pub fn changes(&self, visibility: Visibility, cursor: Cursor) -> Result<Changes> {
        self.check_cursor(&cursor)?;
        Ok(Changes {
            parts: false,
            visibility,
            cursor,
        })
    }
    pub fn feed_changes(&self, visibility: Visibility, cursor: Cursor) -> Result<Changes> {
        let mut changes = self.changes(visibility, cursor)?;
        changes.parts = true;
        Ok(changes)
    }
    fn check_cursor(&self, c: &Cursor) -> Result<()> {
        let h = self.head()?;
        if c.epoch != h.epoch
            || c.subscription_revision != h.subscription_revision
            || c.sequence > h.sequence
            || c.sequence < self.retention_floor()?
        {
            return Err(ErrorCode::Reset.into());
        }
        Ok(())
    }
    pub fn next_changes(&self, i: &mut Changes) -> Result<Option<Batch>> {
        let tx = self.db.unchecked_transaction()?;
        self.check_cursor(&i.cursor)?;
        let h = head(&tx)?;
        if h.sequence <= i.cursor.sequence {
            return Ok(None);
        }
        let limit = h.sequence.min(i.cursor.sequence.saturating_add(128));
        let boundary:Option<i64>=tx.query_row("SELECT MAX(through_seq) FROM journal_commits WHERE epoch=? AND through_seq>? AND through_seq<=?",params![h.epoch,number(i.cursor.sequence)?,number(limit)?],|record|record.get(0))?;
        let mut b = Batch {
            epoch: h.epoch.clone(),
            subscription_revision: h.subscription_revision,
            after: i.cursor.sequence,
            through: boundary
                .map(u64::try_from)
                .transpose()
                .map_err(|_| ErrorCode::Corrupt)?
                .unwrap_or(limit),
            partial: boundary.is_none(),
            changes: Some(Vec::new()),
        };
        let mut stmt =
            tx.prepare("SELECT payload FROM journal WHERE seq>? AND seq<=? ORDER BY seq")?;
        let mut rows = stmt.query(params![number(b.after)?, number(b.through)?])?;
        let mut size = 0;
        while let Some(row) = rows.next()? {
            let raw: Vec<u8> = row.get(0)?;
            let mut ch: Change = serde_json::from_slice(&raw)?;
            if !allows(&i.visibility, &ch.record)? {
                continue;
            }
            if size + raw.len() > 262_144 - 2048 && size > 0 {
                let bound:Option<i64>=tx.query_row("SELECT MAX(through_seq) FROM journal_commits WHERE epoch=? AND through_seq>? AND through_seq<?",params![h.epoch,number(b.after)?,number(ch.sequence)?],|record|record.get(0))?;
                b.through = bound
                    .map(u64::try_from)
                    .transpose()
                    .map_err(|_| ErrorCode::Corrupt)?
                    .unwrap_or(ch.sequence - 1);
                b.partial = bound.is_none();
                if let Some(changes) = &mut b.changes {
                    changes.retain(|raw| {
                        serde_json::from_str::<Change>(raw.get())
                            .is_ok_and(|ch| ch.sequence <= b.through)
                    });
                }
                break;
            }
            size += raw.len();
            ch.record = project(ch.record, &i.visibility)?;
            if let Some(changes) = &mut b.changes {
                changes.push(serde_json::value::RawValue::from_string(
                    String::from_utf8_lossy(&encoded(&ch)?).into_owned(),
                )?);
            }
        }
        if b.partial && !i.parts {
            return Err(ErrorCode::Reset.into());
        }
        i.cursor.sequence = b.through;
        Ok(Some(b))
    }
}
fn apply(tx: &Transaction<'_>, epoch: &str, ch: &Change) -> Result<()> {
    projection::write(tx, &ch.record, ch.sequence)?;
    tx.execute(
        "INSERT INTO journal(seq,epoch,source,payload,committed_at) VALUES(?,?,?,?,unixepoch())",
        params![
            number(ch.sequence)?,
            epoch,
            source(&ch.record)?,
            encoded(ch)?
        ],
    )?;
    Ok(())
}

pub(crate) fn collection(c: Collection) -> &'static str {
    match c {
        Collection::CalendarCalendar => "calendar.calendar",
        Collection::CalendarEvent => "calendar.event",
        Collection::ContactsContact => "contacts.contact",
        Collection::ContactsContainer => "contacts.container",
        Collection::ContactsGroup => "contacts.group",
        Collection::FilesItem => "files.item",
        Collection::FocusStatus => "focus.status",
        Collection::HealthSample => "health.sample",
        Collection::IphoneLocation => "iphone.location",
        Collection::MacApp => "mac.app",
        Collection::MacAudioDevice => "mac.audio_device",
        Collection::MacBluetoothDevice => "mac.bluetooth_device",
        Collection::MacDisplay => "mac.display",
        Collection::MacNetwork => "mac.network",
        Collection::MacPower => "mac.power",
        Collection::MailAccount => "mail.account",
        Collection::MailAttachment => "mail.attachment",
        Collection::MailMailbox => "mail.mailbox",
        Collection::MailMessage => "mail.message",
        Collection::MessagesAttachment => "messages.attachment",
        Collection::MessagesChat => "messages.chat",
        Collection::MessagesHandle => "messages.handle",
        Collection::MessagesMessage => "messages.message",
        Collection::MessagesReaction => "messages.reaction",
        Collection::NotesFolder => "notes.folder",
        Collection::NotesNote => "notes.note",
        Collection::PdfArtifact => "pdf.artifact",
        Collection::PhoneCall => "phone.call",
        Collection::RemindersAttachment => "reminders.attachment",
        Collection::RemindersList => "reminders.list",
        Collection::RemindersReminder => "reminders.reminder",
        Collection::SafariBookmark => "safari.bookmark",
        Collection::SafariCloudTab => "safari.cloud_tab",
        Collection::SafariHistory => "safari.history",
        Collection::SafariReadingList => "safari.reading_list",
        Collection::SafariTab => "safari.tab",
        Collection::ShortcutsFolder => "shortcuts.folder",
        Collection::ShortcutsShortcut => "shortcuts.shortcut",
        Collection::SystemCommand => "system.command",
        Collection::SystemReplica => "system.replica",
        Collection::SystemSource => "system.source",
        Collection::UsageAppDay => "usage.app_day",
        Collection::UsageDevice => "usage.device",
        Collection::VoiceMemosFolder => "voice_memos.folder",
        Collection::VoiceMemosRecording => "voice_memos.recording",
        Collection::VoicemailMessage => "voicemail.message",
    }
}

pub(crate) fn number(count: u64) -> Result<i64> {
    i64::try_from(count).map_err(|_| ErrorCode::Corrupt.into())
}

pub(crate) fn commit(tx: Transaction<'_>) -> Result<()> {
    tx.execute("DELETE FROM blob_ref_changes", [])?;
    tx.commit()?;
    Ok(())
}

pub fn is_capacity(error: &Error) -> bool {
    match error {
        Error::Sqlite(rusqlite::Error::SqliteFailure(code, _)) => code.extended_code & 255 == 13,
        Error::Io(error) => error.raw_os_error() == Some(28),
        _ => false,
    }
}

#[cfg(feature = "lab")]
fn apply_boundary(db: &Connection) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let home = Path::new(db.path().ok_or(ErrorCode::Invalid)?)
        .parent()
        .ok_or(ErrorCode::Invalid)?;
    match std::fs::remove_file(home.join("lab-apply-armed")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let mut reached = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(home.join("lab-apply-reached"))?;
    std::io::Write::write_all(&mut reached, b"write_started_before_commit")?;
    loop {
        std::thread::park();
    }
}
