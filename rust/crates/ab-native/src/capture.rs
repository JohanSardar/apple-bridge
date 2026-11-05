use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde_json::Value;
use std::os::unix::fs::MetadataExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("source IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("source SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("source JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("native record encoding: {0}")]
    NativeJson(#[from] crate::native_json::Error),
    #[error("source decoder: {0}")]
    Decode(#[from] crate::DecodeError),
    #[error("incomplete source scan")]
    Incomplete,
    #[error("source changed during scan")]
    Conflict,
    #[error("unsupported source schema or value")]
    Unsupported,
    #[error("native Helper fact required: {0}")]
    FactsRequired(String),
    #[error("corrupt source data")]
    Corrupt,
}
impl Error {
    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Sqlite(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, serde::Serialize)]
pub enum Cell {
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    Null,
}
impl Cell {
    pub fn text(&self) -> Option<&str> {
        if let Self::Text(v) = self {
            Some(v)
        } else {
            None
        }
    }
    #[allow(clippy::cast_precision_loss)]
    pub fn number(&self) -> f64 {
        match self {
            Self::Integer(v) => *v as f64,
            Self::Real(v) => *v,
            _ => 0.0,
        }
    }
}
pub type Row = BTreeMap<String, Cell>;
pub fn text<'a>(row: &'a Row, key: &str) -> &'a str {
    row.get(key).and_then(Cell::text).unwrap_or_default()
}
pub fn number(row: &Row, key: &str) -> f64 {
    row.get(key).map_or(0.0, Cell::number)
}

#[derive(Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    created: Option<SystemTime>,
}
fn identity(path: &Path) -> Result<Identity> {
    let m = std::fs::metadata(path)?;
    Ok(Identity {
        device: m.dev(),
        inode: m.ino(),
        created: m.created().ok(),
    })
}

#[derive(Debug)]
pub struct Query {
    pub rows: Vec<Row>,
    pub complete: bool,
}
struct DeadlineReset<'a>(&'a std::sync::atomic::AtomicU64);
impl Drop for DeadlineReset<'_> {
    fn drop(&mut self) {
        self.0.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct Snapshot {
    connection: Connection,
    origin: Instant,
    busy_timeout: Duration,
    deadline: std::sync::Arc<std::sync::atomic::AtomicU64>,
    path: PathBuf,
    identity: Identity,
    pub generation: String,
}
impl Snapshot {
    pub fn open(path: &Path, generation_sql: &str) -> Result<Self> {
        Self::open_with_busy_timeout(path, generation_sql, Duration::from_millis(250))
    }
    pub(crate) fn open_with_busy_timeout(
        path: &Path,
        generation_sql: &str,
        busy_timeout: Duration,
    ) -> Result<Self> {
        let initial = identity(path)?;
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;
        connection.busy_timeout(busy_timeout)?;
        connection.execute_batch("BEGIN")?;
        let origin = Instant::now();
        let deadline = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
        let bound = deadline.clone();
        connection.progress_handler(
            1000,
            Some(move || {
                origin.elapsed().as_nanos()
                    >= u128::from(bound.load(std::sync::atomic::Ordering::Relaxed))
            }),
        )?;
        let mut snapshot = Self {
            connection,
            origin,
            busy_timeout,
            deadline,
            path: path.into(),
            identity: initial,
            generation: String::new(),
        };
        let meta = snapshot.query(generation_sql, &[], 2, Duration::from_secs(5))?;
        if meta.len() != 1 || meta[0].len() != 1 {
            return Err(Error::Unsupported);
        }
        let value = meta[0]
            .values()
            .next()
            .and_then(Cell::text)
            .filter(|s| !s.is_empty())
            .ok_or(Error::Unsupported)?;
        let schema = snapshot.query(
            "SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name",
            &[],
            100_000,
            Duration::from_secs(5),
        )?;
        let version = snapshot.query("PRAGMA schema_version", &[], 2, Duration::from_secs(5))?;
        let material = format!(
            "rust-sqlite-1|{:?}|{value}|{}|{}",
            snapshot.identity,
            serde_json::to_string(&version)?,
            serde_json::to_string(&schema)?
        );
        snapshot.generation = crate::body_digest(material.as_bytes());
        if snapshot.identity != identity(path)? {
            return Err(Error::Conflict);
        }
        Ok(snapshot)
    }
    pub fn query(
        &self,
        sql: &str,
        params: &[String],
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Vec<Row>> {
        let result = self.query_partial(sql, params, max_rows, timeout)?;
        if !result.complete {
            return Err(Error::Incomplete);
        }
        Ok(result.rows)
    }
    pub fn query_partial(
        &self,
        sql: &str,
        params: &[String],
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Query> {
        self.query_kind(sql, params, max_rows, timeout, false)
    }
    pub(crate) fn query_notes(
        &self,
        sql: &str,
        params: &[String],
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Vec<Row>> {
        let result = self.query_kind(sql, params, max_rows, timeout, true)?;
        if !result.complete {
            return Err(Error::Incomplete);
        }
        Ok(result.rows)
    }
    fn query_kind(
        &self,
        sql: &str,
        params: &[String],
        max_rows: usize,
        timeout: Duration,
        notes: bool,
    ) -> Result<Query> {
        use std::sync::atomic::Ordering;
        let start = Instant::now();
        let bound = self
            .origin
            .elapsed()
            .as_nanos()
            .saturating_add(timeout.as_nanos());
        self.deadline
            .store(u64::try_from(bound).unwrap_or(u64::MAX), Ordering::Relaxed);
        let _reset = DeadlineReset(&self.deadline);
        let mut statement = self.connection.prepare(sql)?;
        if !statement.readonly() || statement.parameter_count() != params.len() {
            return Err(Error::Unsupported);
        }
        let columns = statement
            .column_names()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut result = Vec::new();
        let mut rows = statement.query(rusqlite::params_from_iter(params))?;
        loop {
            if start.elapsed() >= timeout {
                return Ok(Query {
                    rows: result,
                    complete: false,
                });
            }
            let row = match rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => {
                    return Ok(Query {
                        rows: result,
                        complete: true,
                    });
                }
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::OperationInterrupted =>
                {
                    return Ok(Query {
                        rows: result,
                        complete: false,
                    });
                }
                Err(error) => return Err(error.into()),
            };
            if result.len() >= max_rows {
                return Ok(Query {
                    rows: result,
                    complete: false,
                });
            }
            let mut value = BTreeMap::new();
            for (index, name) in columns.iter().enumerate() {
                let cell = match row.get_ref(index)? {
                    ValueRef::Null => Cell::Null,
                    ValueRef::Integer(v) => Cell::Integer(v),
                    ValueRef::Real(v) => Cell::Real(v),
                    ValueRef::Text(v) => Cell::Text(if notes {
                        String::from_utf8_lossy(v.split(|b| *b == 0).next().unwrap_or_default())
                            .into_owned()
                    } else {
                        std::str::from_utf8(v)
                            .map_err(|_| Error::Corrupt)?
                            .to_owned()
                    }),
                    ValueRef::Blob(v) => Cell::Blob(v.to_vec()),
                };
                value.insert(name.clone(), cell);
            }
            result.push(value);
        }
    }
    pub fn has_column(&self, table: &str, column: &str) -> Result<bool> {
        let rows = self.query(
            &format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\"")),
            &[],
            100_000,
            Duration::from_secs(5),
        )?;
        Ok(rows.iter().any(|row| text(row, "name") == column))
    }
    pub fn require(&self, required: &[(&str, &[&str])]) -> Result<()> {
        for (table, columns) in required {
            let rows = self.query(
                &format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\"")),
                &[],
                100_000,
                Duration::from_secs(5),
            )?;
            let found = rows
                .iter()
                .map(|r| text(r, "name"))
                .collect::<BTreeSet<_>>();
            if rows.is_empty() || columns.iter().any(|c| !found.contains(c)) {
                return Err(Error::Unsupported);
            }
        }
        Ok(())
    }
    pub fn is_current(&self, generation_sql: &str) -> Result<bool> {
        if self.identity != identity(&self.path)? {
            return Ok(false);
        }
        Ok(
            Self::open_with_busy_timeout(&self.path, generation_sql, self.busy_timeout)?.generation
                == self.generation,
        )
    }
    pub fn finish(&self, generation_sql: &str) -> Result<()> {
        if !self.is_current(generation_sql)? {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Context {
    pub observed_at: String,
    pub checkpoint: String,
    pub scan_id: String,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Observation {
    pub kind: &'static str,
    pub collection: String,
    pub native_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoped_alias: Option<String>,
    pub observed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Box<serde_json::value::RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revalidated: Option<bool>,
}
#[allow(clippy::needless_pass_by_value)]
pub fn upsert(
    collection: &str,
    key: &str,
    alias: Option<String>,
    data: Value,
    context: &Context,
) -> Result<Observation> {
    let bytes = crate::native_json::record_data(&data)?;
    Ok(Observation {
        kind: "upsert",
        collection: collection.into(),
        native_key: key.into(),
        scoped_alias: alias,
        observed_at: context.observed_at.clone(),
        data: Some(serde_json::from_slice(&bytes)?),
        revalidated: None,
    })
}
pub fn absent(
    collection: &str,
    key: &str,
    alias: Option<String>,
    context: &Context,
) -> Observation {
    Observation {
        kind: "absent",
        collection: collection.into(),
        native_key: key.into(),
        scoped_alias: alias,
        observed_at: context.observed_at.clone(),
        data: None,
        revalidated: Some(true),
    }
}
#[allow(clippy::cast_possible_truncation)]
pub fn timestamp(time: f64) -> Result<String> {
    if !time.is_finite() || !(-1e12..1e12).contains(&time) {
        return Err(Error::Corrupt);
    }
    let t = jiff::Timestamp::new(
        time.floor() as i64,
        ((time - time.floor()) * 1e9).round() as i32,
    )
    .map_err(|_| Error::Corrupt)?;
    Ok(format!(
        "{}.{:03}Z",
        t.strftime("%Y-%m-%dT%H:%M:%S"),
        t.subsec_nanosecond() / 1_000_000
    ))
}

#[derive(Debug, serde::Serialize)]
pub struct Batch {
    pub source: String,
    pub generation: String,
    pub checkpoint: String,
    pub scan_id: String,
    pub scan_started_at: String,
    pub scope: Vec<String>,
    pub completion_evidence: &'static str,
    pub coverage: Value,
    pub observations: Vec<Observation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blob_handles: BTreeMap<String, String>,
    pub live: bool,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct State {
    #[serde(default)]
    source: String,
    generation: String,
    checkpoint: String,
    records: BTreeMap<String, Stored>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Stored {
    collection: String,
    native_key: String,
    scoped_alias: Option<String>,
    data: String,
}
#[derive(Debug, serde::Serialize)]
pub struct Pending {
    #[serde(flatten)]
    pub batch: Batch,
    #[serde(skip)]
    previous_checkpoint: String,
    #[serde(skip)]
    next: State,
}
#[derive(Debug)]
pub struct ScanInput<'a> {
    pub source: &'a str,
    pub generation: &'a str,
    pub records: Vec<Observation>,
    pub scope: Vec<String>,
    pub coverage: Value,
    pub context: &'a Context,
    pub live: bool,
}
impl State {
    pub fn prepare(&self, input: ScanInput<'_>) -> Result<Pending> {
        self.build(input, true)
    }
    pub fn prepare_incomplete(&self, input: ScanInput<'_>) -> Result<Pending> {
        self.build(input, false)
    }
    fn build(&self, input: ScanInput<'_>, complete: bool) -> Result<Pending> {
        let ScanInput {
            source,
            generation,
            records,
            scope,
            coverage,
            context,
            live,
        } = input;
        if source.is_empty()
            || generation.is_empty()
            || context.checkpoint.is_empty()
            || (!self.source.is_empty() && self.source != source)
            || scope.iter().collect::<BTreeSet<_>>().len() != scope.len()
            || self
                .records
                .values()
                .any(|row| !scope.contains(&row.collection))
        {
            return Err(Error::Corrupt);
        }
        let same = self.generation == generation;
        let mut current = BTreeMap::new();
        let mut changes = Vec::new();
        for row in records {
            let key = format!("{}/{}", row.collection, row.native_key);
            let data = row.data.as_ref().ok_or(Error::Corrupt)?.get().to_owned();
            if current.contains_key(&key)
                || row.native_key.is_empty()
                || !scope.contains(&row.collection)
            {
                return Err(Error::Corrupt);
            }
            let stored = Stored {
                collection: row.collection.clone(),
                native_key: row.native_key.clone(),
                scoped_alias: row.scoped_alias.clone(),
                data,
            };
            if !live
                || !same
                || self
                    .records
                    .get(&key)
                    .is_none_or(|old| !data_equal(&old.data, &stored.data))
            {
                changes.push(row);
            }
            current.insert(key, stored);
        }
        if same {
            for (key, old) in &self.records {
                if !current.contains_key(key) {
                    if complete {
                        changes.push(absent(
                            &old.collection,
                            &old.native_key,
                            old.scoped_alias.clone(),
                            context,
                        ));
                    } else {
                        current.insert(key.clone(), old.clone());
                    }
                }
            }
        }
        let next = Self {
            source: source.into(),
            generation: generation.into(),
            checkpoint: context.checkpoint.clone(),
            records: current,
        };
        Ok(Pending {
            batch: Batch {
                source: source.into(),
                generation: generation.into(),
                checkpoint: context.checkpoint.clone(),
                scan_id: context.scan_id.clone(),
                scan_started_at: context.observed_at.clone(),
                scope,
                completion_evidence: if complete { "complete" } else { "" },
                coverage,
                observations: changes,
                blob_handles: BTreeMap::new(),
                live: live && same,
            },
            previous_checkpoint: self.checkpoint.clone(),
            next,
        })
    }
    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        if pending.previous_checkpoint != self.checkpoint
            || pending.next.checkpoint == self.checkpoint
        {
            return Err(Error::Conflict);
        }
        *self = pending.next;
        Ok(())
    }
}

pub(crate) fn data_equal(left: &str, right: &str) -> bool {
    fn equivalent(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => {
                if a.is_f64() && b.is_f64() {
                    return a.as_f64() == b.as_f64();
                }
                if !a.is_f64() && !b.is_f64() {
                    return if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
                        a == b
                    } else {
                        a.as_u64() == b.as_u64()
                    };
                }
                let (integer, float) = if a.is_f64() { (b, a) } else { (a, b) };
                let Some(f) = float.as_f64() else {
                    return false;
                };
                if let Some(u) = integer.as_u64() {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let exact =
                        if (0.0..18_446_744_073_709_551_616.0).contains(&f) && f.fract() == 0.0 {
                            Some(f as u64)
                        } else {
                            None
                        };
                    exact == Some(u)
                } else {
                    #[allow(clippy::cast_precision_loss)]
                    let value = integer.as_i64().map(|v| v as f64);
                    value == Some(f)
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b))
            }
            (Value::Object(a), Value::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).is_some_and(|b| equivalent(v, b)))
            }
            _ => left == right,
        }
    }
    match (
        serde_json::from_str::<Value>(left),
        serde_json::from_str::<Value>(right),
    ) {
        (Ok(a), Ok(b)) => equivalent(&a, &b),
        _ => false,
    }
}
