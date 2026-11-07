use crate::{
    Change, Collection, Connection, Cursor, Deserialize, ErrorCode, EscapeMode, Gateway, HashMap,
    OptionalExtension, Path, PhantomData, Read, Record, Replica, Result, Serialize, State,
    Visibility, allows, apply, digest, encoded, gojson, head, known_records, number, params,
    project, projection, records, set_cursor, set_meta,
};
use ab_domain::{Coverage, Source};
use ab_wire::messages::{Capture, Checkpoint, Observation};
use serde_json::{Value, value::RawValue};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommitResult {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub duplicate: bool,
    pub after: u64,
    pub through: u64,
    pub degraded: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct FrozenSnapshot {
    pub id: String,
    pub cursor: Cursor,
    pub count: usize,
    pub digest: String,
    pub expires_at: String,
    pub records: Vec<Record>,
}
pub(crate) fn new_id(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let millis = crate::replay::now().as_millisecond().to_be_bytes();
    bytes[..6].copy_from_slice(&millis[2..]);
    let mut n = u128::from_be_bytes(bytes);
    let alphabet = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut out = [b'0'; 26];
    for byte in out.iter_mut().rev() {
        *byte = alphabet[(n & 31) as usize];
        n >>= 5;
    }
    let text = String::from_utf8_lossy(&out);
    Ok(if prefix.is_empty() {
        text.into_owned()
    } else {
        format!("{prefix}_{text}")
    })
}
fn timestamp(raw: &str) -> Result<jiff::Timestamp> {
    raw.parse().map_err(|_| ErrorCode::Invalid.into())
}
pub(crate) fn resolve(
    db: &Connection,
    c: &Capture,
    o: &Observation,
    create: bool,
) -> Result<Option<String>> {
    let source = serde_json::to_value(c.source)?
        .as_str()
        .unwrap_or("")
        .to_owned();
    let mut id: Option<String> = db.query_row("SELECT record_id FROM aliases WHERE source=? AND generation=? AND collection=? AND native_key=?", params![source,c.generation,o.collection,o.native_key], |r|r.get(0)).optional()?;
    if id.is_some() {
        return Ok(id);
    }
    if !o.scoped_alias.is_empty() {
        id = db.query_row("SELECT record_id FROM aliases WHERE source=? AND collection=? AND scoped_alias=? LIMIT 1", params![source,o.collection,o.scoped_alias], |r|r.get(0)).optional()?;
    }
    if id.is_none() && create {
        let col: Collection = serde_json::from_value(Value::String(o.collection.clone()))
            .map_err(|_| ErrorCode::Invalid)?;
        id = Some(
            match crate::replay::record_id(&source, &c.generation, &o.collection, &o.native_key) {
                Some(id) => id,
                None => new_id(col.prefix())?,
            },
        );
    }
    if let Some(id) = &id {
        db.execute(
            "INSERT INTO aliases VALUES(?,?,?,?,?,?)",
            params![
                source,
                c.generation,
                o.collection,
                o.native_key,
                o.scoped_alias,
                id
            ],
        )?;
    }
    Ok(id)
}
fn references(db: &Connection, c: &Capture, value: &mut Value) -> Result<()> {
    match value {
        Value::Object(fields) => {
            if let Some(Value::Object(r)) = fields.get("$ref") {
                let o = Observation {
                    collection: r
                        .get("collection")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    native_key: r
                        .get("native_key")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    ..Observation::default()
                };
                *value = Value::String(
                    resolve(db, c, &o, false)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?,
                );
            } else {
                for item in fields.values_mut() {
                    references(db, c, item)?;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                references(db, c, item)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn data(value: &Value) -> Result<Box<RawValue>> {
    Ok(RawValue::from_string(
        String::from_utf8_lossy(&gojson::record_data(
            &serde_json::to_vec(value)?,
            EscapeMode::Html,
        )?)
        .into_owned(),
    )?)
}
fn keeps_hydrated_note(old: Option<&Record>, observation: &Observation) -> Result<bool> {
    let Some(old) = old.filter(|r| !r.deleted) else {
        return Ok(false);
    };
    let Some(data) = observation.data.as_ref() else {
        return Ok(false);
    };
    let next: Value = serde_json::from_str(data.get())?;
    if next.get("body_status").and_then(Value::as_str) != Some("pending") {
        return Ok(false);
    }
    let previous: Value = serde_json::from_str(old.data.get())?;
    let fingerprint = next.get("native_fingerprint").and_then(Value::as_str);
    Ok(
        previous.get("body_status").and_then(Value::as_str) != Some("pending")
            && fingerprint.is_some_and(|f| !f.is_empty())
            && previous.get("native_fingerprint").and_then(Value::as_str) == fingerprint,
    )
}
fn note_version(old: Option<&Record>, value: &mut Value) -> Result<()> {
    let next = value.as_object_mut().ok_or(ErrorCode::Corrupt)?;
    next.remove("content_version");
    let mut version = old.map_or(1, |r| r.version + 1);
    if let Some(old) = old.filter(|r| !r.deleted) {
        let mut previous: Value = serde_json::from_str(old.data.get())?;
        let saved = previous
            .as_object_mut()
            .and_then(|p| p.remove("content_version"))
            .and_then(|v| v.as_u64());
        let identical = Value::Object(next.clone()) == previous;
        let same = next
            .get("native_fingerprint")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
            && [
                "native_fingerprint",
                "title",
                "plain_text",
                "locked",
                "folder_id",
                "recently_deleted",
                "read_only",
            ]
            .iter()
            .all(|k| next.get(*k) == previous.get(*k));
        if identical || same {
            version = saved
                .filter(|v| *v > 0 && *v <= old.version)
                .unwrap_or(old.version);
        }
    }
    next.insert("content_version".into(), version.into());
    Ok(())
}
fn contact_index(db: &Connection) -> Result<HashMap<String, String>> {
    let mut index = HashMap::new();
    for r in records(
        db,
        "SELECT payload FROM records WHERE collection='contacts.contact' AND deleted=0",
        &[],
    )? {
        let value: Value = serde_json::from_str(r.data.get())?;
        for key in ["phones", "emails"] {
            for item in value[key].as_array().into_iter().flatten() {
                let key = projection::normalize(item["value"].as_str().unwrap_or(""));
                if !key.is_empty() {
                    index
                        .entry(key)
                        .and_modify(|id| {
                            if *id != r.id.as_str() {
                                *id = String::new();
                            }
                        })
                        .or_insert_with(|| r.id.as_str().to_owned());
                }
            }
        }
    }
    Ok(index)
}
fn link(value: &mut Value, index: &HashMap<String, String>) -> Result<()> {
    let value = value.as_object_mut().ok_or(ErrorCode::Corrupt)?;
    if value
        .get("address")
        .is_some_and(|v| !v.is_string() && !v.is_null())
    {
        return Err(ErrorCode::Corrupt.into());
    }
    let address = projection::normalize(value.get("address").and_then(Value::as_str).unwrap_or(""));
    value.remove("contact_id");
    if let Some(id) = index.get(&address).filter(|id| !id.is_empty()) {
        value.insert("contact_id".into(), id.clone().into());
    }
    Ok(())
}
impl State<Gateway> {
    pub fn open(home: impl AsRef<Path>) -> Result<Self> {
        let home = home.as_ref();
        let fresh = !home.join("state.db").try_exists()?;
        let replica = State::<Replica>::open(home)?;
        let mut state = Self {
            db: replica.db,
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
        };
        if fresh {
            set_cursor(
                &state.db,
                &Cursor {
                    epoch: new_id("")?,
                    subscription_revision: 1,
                    sequence: 0,
                },
            )?;
        }
        state.init_commands(home)?;
        Ok(state)
    }
    #[cfg(feature = "lab")]
    pub fn lab_max_pages(&mut self, pages: u32) -> Result<()> {
        self.db.pragma_update(None, "max_page_count", pages)?;
        Ok(())
    }
    pub fn next_fence(&mut self) -> Result<u64> {
        let n: i64 = self.db.query_row("UPDATE meta SET value=CAST(value AS INTEGER)+1 WHERE key='owner_fence' RETURNING value",[],|r|r.get::<_,String>(0)?.parse().map_err(|_|rusqlite::Error::InvalidQuery))?;
        u64::try_from(n).map_err(|_| ErrorCode::Corrupt.into())
    }
    pub fn checkpoints(&self) -> Result<HashMap<String, Checkpoint>> {
        let mut stmt = self
            .db
            .prepare("SELECT source,generation,checkpoint FROM checkpoints")?;
        Ok(stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    Checkpoint {
                        generation: r.get(1)?,
                        checkpoint: r.get(2)?,
                        refresh_blobs: false,
                    },
                ))
            })?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn native_key(&self, collection: &str, id: &str) -> Result<String> {
        self.db.query_row("SELECT native_key FROM aliases WHERE collection=? AND record_id=? ORDER BY rowid DESC LIMIT 1",params![collection,id],|r|r.get(0)).optional()?.ok_or(ErrorCode::NotFound.into())
    }
    pub fn observed_record_id(
        &self,
        collection: &str,
        native_key: &str,
        cursor: &Cursor,
    ) -> Result<String> {
        self.db.query_row("SELECT a.record_id FROM aliases a JOIN checkpoints c ON c.source=a.source AND c.generation=a.generation JOIN records r ON r.collection=a.collection AND r.id=a.record_id WHERE a.source=? AND a.collection=? AND a.native_key=? AND EXISTS(SELECT 1 FROM meta WHERE key='epoch' AND value=?)",params![collection.split('.').next().unwrap_or(""),collection,native_key,cursor.epoch],|r|r.get(0)).optional()?.ok_or(ErrorCode::NotFound.into())
    }
    #[allow(clippy::too_many_lines)]
    pub fn commit_capture(&mut self, c: &Capture) -> Result<CommitResult> {
        let result = self.commit_capture_inner(c);
        if let Some(source) = c.source {
            let name = serde_json::to_value(source)?
                .as_str()
                .unwrap_or("")
                .to_owned();
            match &result {
                Ok(_) => {
                    if let Some(failure) = self.capture_failures.get_mut(&name) {
                        failure.2 = false;
                    }
                }
                Err(error) if !matches!(error, crate::Error::Contract(ErrorCode::Reset)) => {
                    let code = match error {
                        crate::Error::Sqlite(rusqlite::Error::SqliteFailure(code, _)) => {
                            format!("sqlite_{}", code.extended_code)
                        }
                        _ => "capture_error".into(),
                    };
                    let failure = self.capture_failures.entry(name.clone()).or_default();
                    failure.0 += 1;
                    failure.1.clone_from(&code);
                    failure.2 = true;
                    eprintln!(
                        "{}",
                        serde_json::json!({"stage":"gateway_capture_rejected","source":name,"error_code":code,"observations":c.observations.as_ref().map_or(0, Vec::len)})
                    );
                }
                Err(_) => {}
            }
            if result.as_ref().is_err_and(crate::is_capacity) {
                self.capacity.insert(name, true);
            } else if result.is_ok() {
                self.capacity.clear();
            }
        }
        result
    }
    #[allow(clippy::too_many_lines)]
    fn commit_capture_inner(&mut self, c: &Capture) -> Result<CommitResult> {
        let source = c.source.ok_or(ErrorCode::Invalid)?;
        if c.generation.is_empty() || c.checkpoint.is_empty() {
            return Err(ErrorCode::Invalid.into());
        }
        let observations = c.observations.as_deref().unwrap_or(&[]);
        for o in observations {
            let col: Collection = serde_json::from_value(o.collection.clone().into())
                .map_err(|_| ErrorCode::Invalid)?;
            if (col.source() != source && col != Collection::SystemSource)
                || o.native_key.is_empty()
                || o.observed_at.as_str() == "0001-01-01T00:00:00Z"
                || !matches!(o.kind.as_str(), "upsert" | "alias" | "absent" | "evict")
                || (o.kind == "evict"
                    && (source != Source::Calendar || col != Collection::CalendarEvent))
            {
                return Err(ErrorCode::Invalid.into());
            }
            if o.kind == "upsert"
                && !o
                    .data
                    .as_ref()
                    .and_then(|v| serde_json::from_str::<Value>(v.get()).ok())
                    .is_some_and(|v| v.is_object())
            {
                return Err(ErrorCode::Invalid.into());
            }
        }
        let tx = self.db.transaction()?;
        let mut cursor = head(&tx)?;
        let mut result = CommitResult {
            after: cursor.sequence,
            through: cursor.sequence,
            ..CommitResult::default()
        };
        let source_name = serde_json::to_value(source)?
            .as_str()
            .unwrap_or("")
            .to_owned();
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT generation,checkpoint FROM checkpoints WHERE source=?",
                [&source_name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (generation, checkpoint) = prior.unwrap_or_default();
        if generation == c.generation
            && checkpoint == c.checkpoint
            && (source != Source::Contacts
                || observations.is_empty()
                || c.predecessor != checkpoint)
        {
            result.duplicate = true;
            return Ok(result);
        }
        if c.predecessor
            != if generation == c.generation {
                &checkpoint
            } else {
                ""
            }
        {
            return Err(ErrorCode::Reset.into());
        }
        for o in observations
            .iter()
            .filter(|o| !matches!(o.kind.as_str(), "absent" | "evict"))
        {
            resolve(&tx, c, o, true)?;
        }
        let index = contact_index(&tx)?;
        let mut refresh = false;
        for o in observations {
            if o.kind == "alias" {
                continue;
            }
            let deleted = matches!(o.kind.as_str(), "absent" | "evict");
            if deleted
                && (c.scan_id.is_empty()
                    || c.completion_evidence != "complete"
                    || c.scan_started_at.as_str() == "0001-01-01T00:00:00Z"
                    || !o.revalidated
                    || !c.scope.as_ref().is_some_and(|scope| {
                        scope.iter().any(|v| {
                            serde_json::from_str::<String>(v.get()).is_ok_and(|s| s == o.collection)
                        })
                    }))
            {
                result.degraded = true;
                continue;
            }
            let Some(id) = resolve(&tx, c, o, !deleted)? else {
                continue;
            };
            let old = records(
                &tx,
                "SELECT payload FROM records WHERE collection=? AND id=?",
                &[&o.collection, &id],
            )?
            .pop();
            if old.is_some() {
                let latest: String = tx.query_row(
                    "SELECT observed_at FROM records WHERE collection=? AND id=?",
                    params![o.collection, id],
                    |r| r.get(0),
                )?;
                let latest = timestamp(&latest)?;
                if timestamp(o.observed_at.as_str())? < latest
                    || (deleted && latest > timestamp(c.scan_started_at.as_str())?)
                {
                    continue;
                }
            }
            if !deleted && o.collection == "notes.note" && keeps_hydrated_note(old.as_ref(), o)? {
                tx.execute(
                    "UPDATE records SET observed_at=? WHERE collection=? AND id=?",
                    params![o.observed_at.as_str(), o.collection, id],
                )?;
                continue;
            }
            let raw = if deleted {
                let Some(old) = &old else {
                    continue;
                };
                old.data.clone()
            } else {
                let mut value: Value =
                    serde_json::from_str(o.data.as_ref().ok_or(ErrorCode::Invalid)?.get())?;
                references(&tx, c, &mut value)?;
                if o.collection == "messages.handle" {
                    link(&mut value, &index)?;
                }
                if o.collection == "notes.note" {
                    note_version(old.as_ref(), &mut value)?;
                }
                if o.collection == "messages.attachment" {
                    let previous = match old.as_ref().filter(|r| !r.deleted) {
                        Some(r) => Some(serde_json::from_str::<Value>(r.data.get())?),
                        None => None,
                    };
                    crate::attachments::keep_known(previous.as_ref(), &mut value);
                }
                data(&value)?
            };
            if old
                .as_ref()
                .is_some_and(|r| r.deleted == deleted && r.data.get() == raw.get())
            {
                tx.execute(
                    "UPDATE records SET observed_at=? WHERE collection=? AND id=?",
                    params![o.observed_at.as_str(), o.collection, id],
                )?;
                continue;
            }
            let record = Record {
                collection: serde_json::from_value(o.collection.clone().into())?,
                id: id.try_into()?,
                version: old.as_ref().map_or(1, |r| r.version + 1),
                source,
                observed_at: o.observed_at.as_str().into(),
                deleted,
                coverage: Coverage {
                    permissions: Default::default(),
                    pack_version: String::new(),
                    operations: Vec::new(),
                    fields: "complete".into(),
                    missing: Some(Vec::new()),
                },
                data: raw,
                tombstone_reason: if o.kind == "evict" {
                    "window_evicted".into()
                } else {
                    String::new()
                },
            };
            if encoded(&record)?.len() > 262_144 - 2048 {
                return Err(ErrorCode::Capacity.into());
            }
            refresh |= record.collection == Collection::ContactsContact;
            cursor.sequence += 1;
            apply(
                &tx,
                &cursor.epoch,
                &Change {
                    sequence: cursor.sequence,
                    op: if deleted {
                        if o.kind == "evict" { "evict" } else { "delete" }
                    } else {
                        "upsert"
                    }
                    .into(),
                    record,
                },
            )?;
        }
        if refresh {
            let index = contact_index(&tx)?;
            for mut r in records(
                &tx,
                "SELECT payload FROM records WHERE collection='messages.handle' AND deleted=0 ORDER BY id",
                &[],
            )? {
                let mut value: Value = serde_json::from_str(r.data.get())?;
                link(&mut value, &index)?;
                let raw = data(&value)?;
                if raw.get() == r.data.get() {
                    continue;
                }
                r.data = raw;
                r.version += 1;
                r.observed_at = crate::replay::now().to_string();
                cursor.sequence += 1;
                apply(
                    &tx,
                    &cursor.epoch,
                    &Change {
                        sequence: cursor.sequence,
                        op: "upsert".into(),
                        record: r,
                    },
                )?;
            }
        }
        tx.execute("INSERT INTO checkpoints VALUES(?,?,?) ON CONFLICT(source) DO UPDATE SET generation=excluded.generation,checkpoint=excluded.checkpoint",params![source_name,c.generation,c.checkpoint])?;
        set_cursor(&tx, &cursor)?;
        crate::protection::commit(tx, self.blob_protection.as_ref())?;
        result.through = cursor.sequence;
        Ok(result)
    }
    pub fn freeze_snapshot(
        &mut self,
        visibility: &Visibility,
        revision: u64,
    ) -> Result<FrozenSnapshot> {
        let tx = self.db.unchecked_transaction()?;
        let mut cursor = head(&tx)?;
        cursor.subscription_revision = revision;
        let mut visible = Vec::new();
        for record in known_records(
            &tx,
            "SELECT payload FROM records WHERE deleted=0 ORDER BY collection,id",
            &[],
        )? {
            if allows(visibility, &record)? {
                visible.push(project(record, visibility)?);
            }
        }
        let expires = crate::replay::now()
            .checked_add(Duration::from_secs(600))
            .map_err(|_| ErrorCode::Invalid)?;
        let id = new_id("snap")?;
        let mut refs = Vec::new();
        for r in &visible {
            let owner = format!("snapshot:{}", r.id.as_str());
            projection::blob_refs(&tx, &owner, r)?;
            refs.extend(crate::protection::owner(&tx, &owner)?);
        }
        if let Some(hook) = &self.blob_protection {
            hook.delta(
                &HashMap::from([(format!("snapshot:{id}"), refs.clone())]),
                false,
            )?;
        }
        self.snapshot_refs.insert(id.clone(), refs);
        self.leases.insert(id.clone(), (cursor.sequence, expires));
        Ok(FrozenSnapshot {
            id,
            cursor,
            count: visible.len(),
            digest: digest(&visible),
            expires_at: expires.to_string(),
            records: visible,
        })
    }
    pub fn renew_snapshot(&mut self, id: &str) -> Result<()> {
        let lease = self
            .leases
            .get_mut(id)
            .filter(|(_, expires)| *expires >= crate::replay::now())
            .ok_or(ErrorCode::Expired)?;
        lease.1 = crate::replay::now()
            .checked_add(Duration::from_secs(600))
            .map_err(|_| ErrorCode::Invalid)?;
        Ok(())
    }
    pub fn close_snapshot(&mut self, id: &str) -> Result<()> {
        if let Some(hook) = &self.blob_protection {
            hook.delta(
                &HashMap::from([(format!("snapshot:{id}"), Vec::new())]),
                false,
            )?;
        }
        self.leases.remove(id);
        self.snapshot_refs.remove(id);
        Ok(())
    }
    pub fn prune(&mut self, rows: u64, age: Duration) -> Result<()> {
        let tx = self.db.transaction()?;
        let h = head(&tx)?;
        if h.sequence <= rows {
            return Ok(());
        }
        let cutoff = crate::replay::now()
            .as_second()
            .saturating_sub(i64::try_from(age.as_secs()).map_err(|_| ErrorCode::Invalid)?);
        let old: Option<i64> = tx.query_row(
            "SELECT MAX(seq) FROM journal WHERE committed_at<?",
            [cutoff],
            |r| r.get(0),
        )?;
        let Some(old) = old else {
            return Ok(());
        };
        let mut floor =
            (h.sequence - rows).min(u64::try_from(old).map_err(|_| ErrorCode::Corrupt)?);
        self.leases
            .retain(|_, (_, expires)| *expires >= crate::replay::now());
        for (seq, _) in self.leases.values() {
            floor = floor.min(*seq);
        }
        let boundary: Option<i64> = tx.query_row(
            "SELECT MAX(through_seq) FROM journal_commits WHERE epoch=? AND through_seq<=?",
            params![h.epoch, number(floor)?],
            |r| r.get(0),
        )?;
        let current: String = tx.query_row(
            "SELECT value FROM meta WHERE key='retention_floor'",
            [],
            |r| r.get(0),
        )?;
        if let Some(floor) = boundary.filter(|floor| *floor > current.parse().unwrap_or(0)) {
            tx.execute("DELETE FROM journal WHERE seq<=?", [floor])?;
            tx.execute("DELETE FROM journal_commits WHERE through_seq<=?", [floor])?;
            set_meta(&tx, "retention_floor", &floor.to_string())?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn backup(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let staged = parent.join(format!("backup-{}.db", new_id("")?));
        let result = (|| {
            self.db.execute(
                "VACUUM INTO ?",
                [staged.to_str().ok_or(ErrorCode::Invalid)?],
            )?;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))?;
            std::fs::File::open(&staged)?.sync_all()?;
            std::fs::rename(&staged, path)?;
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if staged.exists() {
            let _ = std::fs::remove_file(staged);
        }
        result
    }
}
pub fn restore(
    home: impl AsRef<Path>,
    backup: impl AsRef<Path>,
    accept_ledger_loss: bool,
) -> Result<()> {
    let home = home.as_ref();
    let backup = backup.as_ref();
    let current = home.join("state.db");
    let backup = if backup.as_os_str().is_empty() {
        if !accept_ledger_loss {
            return Err(ErrorCode::Invalid.into());
        }
        current.as_path()
    } else {
        backup
    };
    let source = Connection::open_with_flags(backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = source.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(ErrorCode::Corrupt.into());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(home)?;
    let path = home.join(format!("restore-{}.db", new_id("")?));
    let result = (|| {
        source.execute("VACUUM INTO ?", [path.to_str().ok_or(ErrorCode::Invalid)?])?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let mut staged = Connection::open(&path)?;
        let tx = staged.transaction()?;
        tx.execute_batch(include_str!("schema.sql"))?;
        set_cursor(
            &tx,
            &Cursor {
                epoch: new_id("")?,
                subscription_revision: 1,
                sequence: 0,
            },
        )?;
        tx.execute_batch("DELETE FROM journal; DELETE FROM journal_commits; DELETE FROM staging_records; DELETE FROM applied_batches; UPDATE records SET seq=0; DELETE FROM meta WHERE key='replica_base'; UPDATE meta SET value='0' WHERE key='retention_floor';")?;
        tx.commit()?;
        let current_path = home.join("state.db");
        if current_path.exists() {
            let current = Connection::open(&current_path)?;
            if !accept_ledger_loss {
                crate::ledger::check_ledger(&current, home)?;
            }
            let busy: i64 =
                current.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
            if busy != 0 {
                return Err(ErrorCode::Conflict.into());
            }
        }
        if !accept_ledger_loss {
            crate::ledger::check_ledger(&staged, home)?;
        }
        let ledger = crate::ledger::open_ledger(home)?;
        let era = new_id("")?;
        ledger.execute("UPDATE meta SET value=? WHERE key='admission_era'", [&era])?;
        set_meta(&staged, "admission_era", &era)?;
        drop(ledger);
        drop(staged);
        for suffix in ["-wal", "-shm"] {
            let p = home.join(format!("state.db{suffix}"));
            if p.exists() {
                std::fs::remove_file(p)?;
            }
        }
        std::fs::rename(&path, &current_path)?;
        std::fs::File::open(home)?.sync_all()?;
        Ok(())
    })();
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    result
}
