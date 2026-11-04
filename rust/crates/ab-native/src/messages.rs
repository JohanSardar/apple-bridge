use ab_wire::gojson::{self, EscapeMode};
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(target_os = "macos")]
use std::os::macos::fs::MetadataExt as _;

mod observations;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Messages SQLite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("Messages filesystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("Messages JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Messages encoding: {0}")]
    Wire(#[from] gojson::Error),
    #[error("Messages database lacks message.guid")]
    Schema,
    #[error("Messages database changed during scan")]
    Replaced,
    #[error("invalid blob range or unavailable attachment")]
    Blob,
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct MediaSource {
    pub path: PathBuf,
    pub sticker: bool,
}

#[derive(Debug, Default)]
struct Row {
    attachments: Vec<Row>,
    values: BTreeMap<String, String>,
    blobs: BTreeMap<String, Vec<u8>>,
}
impl Row {
    fn text(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }
    fn int(&self, key: &str) -> i64 {
        self.text(key).parse().unwrap_or(0)
    }
    fn flag(&self, key: &str) -> bool {
        self.int(key) != 0
    }
    fn hash(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        for (key, value) in &self.values {
            hash.update(key.len().to_le_bytes());
            hash.update(key);
            hash.update(value.len().to_le_bytes());
            hash.update(value);
        }
        for (key, value) in &self.blobs {
            hash.update(key.len().to_le_bytes());
            hash.update(key);
            hash.update(Sha256::digest(value));
        }
        hash.finalize().into()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Observation {
    pub kind: String,
    pub collection: String,
    pub native_key: String,
    pub data: Value,
}
impl Observation {
    fn new(collection: &str, key: &str, data: Value) -> Self {
        Self {
            kind: "upsert".into(),
            collection: format!("messages.{collection}"),
            native_key: key.into(),
            data,
        }
    }
    fn key(&self) -> String {
        format!("{}/{}", self.collection, self.native_key)
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        Ok(gojson::record_data(
            &serde_json::to_vec(self)?,
            EscapeMode::Html,
        )?)
    }
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Batch {
    pub generation: String,
    pub live: bool,
    pub complete: bool,
    pub observations: Vec<Observation>,
}
#[derive(Debug)]
struct Scan {
    cursor: i64,
    seen: BTreeSet<String>,
}
#[derive(Debug)]
pub struct Reader {
    pending_scan: Option<Scan>,
    reconciliation_requested: bool,
    pins: Option<Vec<String>>,
    pins_stamp: Option<(u64, i64, i64, u64)>,
    path: PathBuf,
    root: PathBuf,
    home: PathBuf,
    cwd_name: String,
    db: Connection,
    identity: (u64, u64),
    wal_identity: Option<u64>,
    schema_version: i64,
    columns: BTreeMap<String, BTreeSet<String>>,
    pub generation: String,
    version: Option<i64>,
    reconciled: Option<i64>,
    last_row: i64,
    native_hashes: BTreeMap<String, [u8; 32]>,
    hashes: BTreeMap<String, [u8; 32]>,
    candidates: BTreeSet<String>,
    recoverable: BTreeSet<String>,
    attachment_parents: BTreeMap<String, String>,
    rows_read: Cell<u64>,
    fingerprints: Cell<u64>,
    undo_hashes: BTreeMap<String, Option<[u8; 32]>>,
    undo_native: BTreeMap<String, Option<[u8; 32]>>,
}
impl Reader {
    pub fn open(path: &Path, home: &Path) -> Result<Self> {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(250))?;
        let mut columns = BTreeMap::new();
        for table in [
            "message",
            "chat",
            "handle",
            "chat_handle_join",
            "chat_message_join",
            "message_attachment_join",
            "attachment",
            "chat_recoverable_message_join",
            "recoverable_message_part",
        ] {
            let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
            let fields = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<std::result::Result<BTreeSet<_>, _>>()?;
            columns.insert(table.to_string(), fields);
        }
        if !columns.get("message").is_some_and(|c| c.contains("guid")) {
            return Err(Error::Schema);
        }
        let meta = fs::metadata(path)?;
        let identity = (meta.dev(), meta.ino());
        let signature = columns
            .iter()
            .map(|(k, v)| format!("{k}:{}", v.iter().cloned().collect::<Vec<_>>().join(",")))
            .collect::<Vec<_>>()
            .join("\n");
        let generation =
            crate::body_digest(format!("{}:{}{signature}", identity.0, identity.1).as_bytes());
        let schema_version = db.query_row("PRAGMA schema_version", [], |r| r.get(0))?;
        Ok(Self {
            pending_scan: None,
            reconciliation_requested: false,
            pins: None,
            pins_stamp: None,
            path: path.into(),
            home: home.into(),
            cwd_name: std::env::current_dir()
                .ok()
                .map(|p| {
                    p.file_name()
                        .map_or_else(|| "/".into(), |n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_default(),
            root: home.join("Library/Messages/Attachments"),
            db,
            identity,
            wal_identity: fs::metadata(format!("{}-wal", path.display()))
                .ok()
                .map(|m| m.ino()),
            schema_version,
            columns,
            generation,
            version: None,
            reconciled: None,
            last_row: 0,
            native_hashes: BTreeMap::new(),
            hashes: BTreeMap::new(),
            candidates: BTreeSet::new(),
            recoverable: BTreeSet::new(),
            attachment_parents: BTreeMap::new(),
            rows_read: Cell::new(0),
            fingerprints: Cell::new(0),
            undo_hashes: BTreeMap::new(),
            undo_native: BTreeMap::new(),
        })
    }
    fn has(&self, table: &str, fields: &[&str]) -> bool {
        self.columns
            .get(table)
            .is_some_and(|c| fields.iter().all(|f| c.contains(*f)))
    }
    fn refresh_pins(&mut self) -> bool {
        let path = self
            .home
            .join("Library/Preferences/com.apple.messages.pinning.plist");
        let stamp = fs::metadata(&path)
            .ok()
            .map(|m| (m.ino(), m.mtime(), m.mtime_nsec(), m.len()));
        if stamp == self.pins_stamp {
            return false;
        }
        self.pins_stamp = stamp;
        let pins = (|| {
            if stamp?.3 > 16 * 1024 * 1024 {
                return None;
            }
            let bytes = fs::read(path).ok()?;
            let root = crate::plist::decode(&bytes).ok()?;
            let crate::plist::Value::Array(values) =
                root.dictionary()?.get("pD")?.dictionary()?.get("pP")?
            else {
                return None;
            };
            let pins = values
                .iter()
                .map(|v| {
                    v.string()
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                })
                .collect::<Option<Vec<_>>>()?;
            (pins.len() <= 9 && pins.iter().collect::<BTreeSet<_>>().len() == pins.len())
                .then_some(pins)
        })();
        let changed = pins != self.pins;
        self.pins = pins;
        changed
    }
    pub fn resume_candidates(
        &mut self,
        collection: &str,
        keys: impl IntoIterator<Item = String>,
    ) -> Result<()> {
        if ![
            "messages.chat",
            "messages.handle",
            "messages.message",
            "messages.reaction",
            "messages.attachment",
        ]
        .contains(&collection)
        {
            return Err(Error::Schema);
        }
        self.candidates
            .extend(keys.into_iter().map(|key| format!("{collection}/{key}")));
        self.reconciled = None;
        Ok(())
    }
    pub fn invalidate<'a>(&mut self, identities: impl IntoIterator<Item = (&'a str, &'a str)>) {
        for (collection, key) in identities {
            self.hashes.remove(&format!("{collection}/{key}"));
            self.native_hashes.remove(key);
        }
        if self.version.is_some() {
            self.version = Some(i64::MIN);
        }
    }
    pub fn work_counts(&self) -> (u64, u64) {
        (self.rows_read.get(), self.fingerprints.get())
    }
    pub fn coverage_gaps(&self) -> Vec<String> {
        let expected = [
            (
                "message",
                "guid,date,text,attributedBody,handle_id,is_from_me,date_delivered,date_read,is_delivered,is_read,error,is_finished,item_type,date_edited,date_retracted,message_summary_info,associated_message_guid,associated_message_type,associated_message_emoji,reply_to_guid,thread_originator_guid,group_action_type,group_title,other_handle,expressive_send_style_id,balloon_bundle_id,payload_data,is_audio_message,date_played,is_expirable,expire_state,schedule_type,schedule_state,subject,was_delivered_quietly,did_notify_recipient,is_spam,has_unseen_mention,is_stewie,is_sos,sent_or_received_off_grid",
            ),
            (
                "chat",
                "guid,service_name,display_name,is_archived,last_read_message_timestamp,is_filtered,properties",
            ),
            ("handle", "id,service"),
            ("chat_message_join", "chat_id,message_id"),
            ("chat_handle_join", "chat_id,handle_id"),
            (
                "attachment",
                "guid,filename,mime_type,uti,total_bytes,transfer_state,is_sticker,sticker_user_info,hide_attachment,emoji_image_content_identifier,emoji_image_short_description",
            ),
            ("message_attachment_join", "message_id,attachment_id"),
            (
                "chat_recoverable_message_join",
                "chat_id,message_id,delete_date",
            ),
        ];
        expected
            .iter()
            .flat_map(|(t, fields)| {
                fields
                    .split(',')
                    .filter(|f| !self.has(t, &[f]))
                    .map(|f| format!("{t}.{f}"))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
    fn current(&self) -> Result<bool> {
        let m = fs::metadata(&self.path)?;
        let wal = fs::metadata(format!("{}-wal", self.path.display()))
            .ok()
            .map(|m| m.ino());
        Ok(wal == self.wal_identity
            && (m.dev(), m.ino()) == self.identity
            && self
                .db
                .query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))?
                == self.schema_version)
    }
    fn query(&self, sql: &str, bindings: &[String]) -> Result<Vec<Row>> {
        let mut stmt = self.db.prepare_cached(sql)?;
        let names: Vec<String> = stmt.column_names().map(ToString::to_string).collect();
        let mut rows = stmt.query(rusqlite::params_from_iter(bindings))?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            self.rows_read.set(self.rows_read.get() + 1);
            let mut out = Row::default();
            for (i, name) in names.iter().enumerate() {
                let text = match row.get_ref(i)? {
                    ValueRef::Null => continue,
                    ValueRef::Blob(b) => {
                        out.blobs.insert(name.clone(), b.to_vec());
                        continue;
                    }
                    ValueRef::Text(b) => {
                        if let Ok(s) = std::str::from_utf8(b) {
                            s.to_owned()
                        } else {
                            out.blobs.insert(name.clone(), b.to_vec());
                            continue;
                        }
                    }
                    ValueRef::Integer(n) => n.to_string(),
                    ValueRef::Real(n) => self
                        .db
                        .prepare_cached("SELECT CAST(? AS TEXT)")?
                        .query_row([n], |r| r.get::<_, String>(0))?,
                };
                out.values.insert(name.clone(), text);
            }
            result.push(out);
        }
        Ok(result)
    }
    fn deletion(&self, chat: bool) -> String {
        if !self.has(
            "chat_recoverable_message_join",
            &["chat_id", "message_id", "delete_date"],
        ) || !self.has("chat_message_join", &["chat_id", "message_id"])
        {
            return "NULL".into();
        }
        if chat {
            "(SELECT MAX(r.delete_date) FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND NOT EXISTS(SELECT 1 FROM chat_message_join j WHERE j.chat_id=c.ROWID AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join visible WHERE visible.chat_id=c.ROWID AND visible.message_id=j.message_id)))".into()
        } else {
            "(SELECT MAX(r.delete_date) FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND NOT EXISTS(SELECT 1 FROM chat_message_join j WHERE j.message_id=m.ROWID AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join visible WHERE visible.chat_id=j.chat_id AND visible.message_id=m.ROWID)))".into()
        }
    }
    fn projected(
        &self,
        selection: &str,
        bindings: &[String],
        descending: bool,
    ) -> Result<Vec<Row>> {
        let chat = |table: &str| {
            if self.has("chat", &["guid"]) && self.has(table, &["chat_id", "message_id"]) {
                format!(
                    "(SELECT c.guid FROM {table} j JOIN chat c ON c.ROWID=j.chat_id WHERE j.message_id=m.ROWID ORDER BY c.guid LIMIT 1)"
                )
            } else {
                "NULL".into()
            }
        };
        let handle = |col: &str| {
            if self.has("message", &[col]) && self.has("handle", &["id", "service"]) {
                format!("(SELECT h.service || ':' || h.id FROM handle h WHERE h.ROWID=m.{col})")
            } else {
                "NULL".into()
            }
        };
        let fields = [
            "guid",
            "filename",
            "mime_type",
            "uti",
            "emoji_image_content_identifier",
            "emoji_image_short_description",
        ]
        .into_iter()
        .filter(|f| self.has("attachment", &[f]))
        .map(|f| format!("quote(a.{f})"))
        .collect::<Vec<_>>()
        .join("||':'||");
        let attachments = if self.has("attachment", &["guid"])
            && self.has("message_attachment_join", &["message_id", "attachment_id"])
        {
            format!(
                "(SELECT group_concat(signature,'|') FROM (SELECT {fields} AS signature FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID WHERE j.message_id=m.ROWID ORDER BY a.ROWID))"
            )
        } else {
            "NULL".into()
        };
        let mut ctes = vec![format!("capture_batch AS MATERIALIZED ({selection})")];
        for t in [
            "chat_message_join",
            "chat_recoverable_message_join",
            "message_attachment_join",
        ] {
            if self.has(t, &["message_id"]) {
                ctes.push(format!("{t} AS MATERIALIZED (SELECT * FROM main.{t} WHERE message_id IN (SELECT row_position FROM capture_batch))"));
            }
        }
        let rows = self.query(&format!("WITH {} SELECT m.ROWID AS row_position,m.*,COALESCE({},{}) AS capture_chat_guid,{} AS capture_sender_key,{} AS capture_deleted_date,{attachments} AS capture_attachment_signature,{} AS capture_member_key FROM message m WHERE m.ROWID IN (SELECT row_position FROM capture_batch) ORDER BY m.ROWID {}",ctes.join(","),chat("chat_message_join"),chat("chat_recoverable_message_join"),handle("handle_id"),self.deletion(false),handle("other_handle"),if descending {"DESC"}else{"ASC"}),bindings)?;
        self.with_attachments(rows)
    }
    fn with_attachments(&self, mut rows: Vec<Row>) -> Result<Vec<Row>> {
        if !self.has("attachment", &["guid"])
            || !self.has("message_attachment_join", &["message_id", "attachment_id"])
        {
            return Ok(rows);
        }
        let keys = rows
            .iter()
            .filter(|r| {
                !r.text("balloon_bundle_id").is_empty() || r.blobs.contains_key("attributedBody")
            })
            .map(|r| r.text("row_position").to_string())
            .collect::<Vec<_>>();
        let mut attachments: BTreeMap<i64, Vec<Row>> = BTreeMap::new();
        for keys in keys.chunks(128) {
            let sql = format!(
                "SELECT j.message_id AS capture_parent_position,a.* FROM message_attachment_join j JOIN attachment a ON a.ROWID=j.attachment_id WHERE j.message_id IN ({}) ORDER BY a.ROWID",
                vec!["?"; keys.len()].join(",")
            );
            for attachment in self.query(&sql, keys)? {
                attachments
                    .entry(attachment.int("capture_parent_position"))
                    .or_default()
                    .push(attachment);
            }
        }
        for row in &mut rows {
            row.attachments = attachments
                .remove(&row.int("row_position"))
                .unwrap_or_default();
        }
        Ok(rows)
    }
    fn emit(&mut self, observations: Vec<Observation>) -> Result<Vec<Observation>> {
        let mut output = Vec::new();
        for observation in observations {
            if observation.native_key.is_empty() {
                continue;
            }
            let hash = Sha256::digest(gojson::record_data(
                &serde_json::to_vec(&observation.data)?,
                EscapeMode::Html,
            )?)
            .into();
            if self.hashes.get(&observation.key()) != Some(&hash) {
                self.undo_hashes
                    .entry(observation.key())
                    .or_insert_with(|| self.hashes.get(&observation.key()).copied());
                self.hashes.insert(observation.key(), hash);
                output.push(observation);
            }
        }
        Ok(output)
    }
    fn messages(&mut self, rows: &[Row]) -> Result<Vec<Observation>> {
        let mut items = Vec::new();
        for row in rows {
            self.fingerprints.set(self.fingerprints.get() + 1);
            let hash = row.hash();
            if self.native_hashes.get(row.text("guid")) != Some(&hash) {
                items.extend(self.message_observations(row));
                self.undo_native
                    .entry(row.text("guid").into())
                    .or_insert_with(|| self.native_hashes.get(row.text("guid")).copied());
                self.native_hashes.insert(row.text("guid").into(), hash);
            }
        }
        self.emit(items)
    }
    fn inserts(
        &mut self,
        batches: &mut Vec<Batch>,
        referenced: &mut BTreeSet<String>,
    ) -> Result<()> {
        let max: i64 =
            self.db
                .query_row("SELECT COALESCE(MAX(ROWID),0) FROM message", [], |r| {
                    r.get(0)
                })?;
        while self.last_row < max {
            let rows = self.projected("SELECT ROWID AS row_position FROM message WHERE ROWID>? AND ROWID<=? ORDER BY ROWID LIMIT 128",&[self.last_row.to_string(),max.to_string()],false)?;
            let Some(last) = rows.last() else { break };
            let cursor = last.int("row_position");
            for row in &rows {
                for field in [
                    "associated_message_guid",
                    "reply_to_guid",
                    "thread_originator_guid",
                ] {
                    if !row.text(field).is_empty() {
                        referenced.insert(target(row.text(field)).to_string());
                    }
                }
            }
            let observations = self.messages(&rows)?;
            self.batch(batches, observations, true, false);
            self.last_row = cursor;
        }
        Ok(())
    }
    fn batch(
        &self,
        batches: &mut Vec<Batch>,
        observations: Vec<Observation>,
        live: bool,
        complete: bool,
    ) {
        let mut items = observations.into_iter().peekable();
        if items.peek().is_none() && complete {
            batches.push(Batch {
                generation: self.generation.clone(),
                live,
                complete: true,
                observations: Vec::new(),
            });
        }
        while items.peek().is_some() {
            let observations = items.by_ref().take(128).collect();
            batches.push(Batch {
                generation: self.generation.clone(),
                live,
                complete: complete && items.peek().is_none(),
                observations,
            });
        }
    }
    pub fn poll(&mut self, reconcile: bool) -> Result<Vec<Batch>> {
        if !self.current()? {
            let fresh = Self::open(&self.path, &self.home)?;
            let prior = std::mem::take(&mut self.hashes);
            let mut candidates = std::mem::take(&mut self.candidates);
            let changed = fresh.generation != self.generation;
            if changed {
                candidates.extend(prior.keys().cloned());
            }
            *self = fresh;
            self.candidates = candidates;
            if !changed {
                self.hashes = prior;
            }
            self.version = None;
        }
        let pins_changed = self.refresh_pins();
        let version: i64 = self.db.query_row("PRAGMA data_version", [], |r| r.get(0))?;
        if self.version == Some(version) && (!reconcile || self.reconciled == Some(version)) {
            if pins_changed {
                let small = self.small_observations()?;
                let observations = self.emit(small)?;
                let mut batches = Vec::new();
                self.batch(&mut batches, observations, true, false);
                self.undo_hashes.clear();
                return Ok(batches);
            }
            return Ok(Vec::new());
        }
        self.db.execute_batch("BEGIN")?;
        let saved_row = self.last_row;
        let saved_recoverable = self.recoverable.clone();
        let saved_parents = self.attachment_parents.clone();
        let result = (|| {
            if self.version.is_none() {
                self.last_row =
                    self.db
                        .query_row("SELECT COALESCE(MAX(ROWID),0) FROM message", [], |r| {
                            r.get(0)
                        })?;
            }
            self.scan(version, reconcile)
        })();
        let end = self.db.execute_batch("ROLLBACK");
        if result.is_err() || end.is_err() {
            for (key, old) in std::mem::take(&mut self.undo_hashes) {
                if let Some(hash) = old {
                    self.hashes.insert(key, hash);
                } else {
                    self.hashes.remove(&key);
                }
            }
            for (key, old) in std::mem::take(&mut self.undo_native) {
                if let Some(hash) = old {
                    self.native_hashes.insert(key, hash);
                } else {
                    self.native_hashes.remove(&key);
                }
            }
            self.last_row = saved_row;
            self.recoverable = saved_recoverable;
            self.attachment_parents = saved_parents;
        }
        self.undo_hashes.clear();
        self.undo_native.clear();
        end?;
        result
    }
    fn scan(&mut self, version: i64, reconcile: bool) -> Result<Vec<Batch>> {
        let mut batches = Vec::new();
        let mut referenced = BTreeSet::new();
        self.inserts(&mut batches, &mut referenced)?;
        let recovery = if self.has("chat_recoverable_message_join", &["message_id"]) {
            self.query("SELECT DISTINCT m.guid FROM message m JOIN chat_recoverable_message_join r ON r.message_id=m.ROWID",&[])?.iter().map(|r|r.text("guid").to_string()).collect()
        } else {
            BTreeSet::new()
        };
        referenced.extend(self.recoverable.union(&recovery).cloned());
        self.recoverable = recovery;
        let recent = self.projected(
            "SELECT ROWID AS row_position FROM message ORDER BY ROWID DESC LIMIT 2000",
            &[],
            true,
        )?;
        for rows in recent.chunks(128) {
            let items = self.messages(rows)?;
            self.batch(&mut batches, items, true, false);
        }
        let small = self.small_observations()?;
        let parents: BTreeMap<String, String> = small
            .iter()
            .filter(|o| o.collection == "messages.attachment")
            .filter_map(|o| {
                o.data
                    .pointer("/message_id/$ref/native_key")
                    .and_then(Value::as_str)
                    .map(|p| (o.native_key.clone(), p.to_string()))
            })
            .collect();
        for (key, parent) in &self.attachment_parents {
            if parents.get(key) != Some(parent) {
                referenced.insert(parent.clone());
            }
        }
        self.attachment_parents = parents;
        let small = self.emit(small)?;
        for item in &small {
            if let Some(key) = item
                .data
                .pointer("/message_id/$ref/native_key")
                .and_then(Value::as_str)
            {
                referenced.insert(key.into());
            }
        }
        self.batch(&mut batches, small, true, false);
        let keys = referenced.into_iter().collect::<Vec<_>>();
        for keys in keys.chunks(128) {
            let marks = vec!["?"; keys.len()].join(",");
            let rows = self.projected(
                &format!("SELECT ROWID AS row_position FROM message WHERE guid IN ({marks})"),
                keys,
                false,
            )?;
            let items = self.messages(&rows)?;
            self.batch(&mut batches, items, true, false);
        }
        if reconcile {
            self.reconcile(version, &mut batches)?;
        }
        self.version = Some(version);
        Ok(batches)
    }
    fn reconcile(&mut self, version: i64, batches: &mut Vec<Batch>) -> Result<()> {
        let mut seen: BTreeSet<String> = self
            .small_observations()?
            .iter()
            .map(Observation::key)
            .collect();
        let mut cursor = 0;
        loop {
            let rows = self.projected(
                "SELECT ROWID AS row_position FROM message WHERE ROWID>? ORDER BY ROWID LIMIT 128",
                &[cursor.to_string()],
                false,
            )?;
            let Some(last) = rows.last() else { break };
            cursor = last.int("row_position");
            for row in &rows {
                seen.insert(format!(
                    "messages.{}/{}",
                    if reaction(row).is_some() {
                        "reaction"
                    } else {
                        "message"
                    },
                    row.text("guid")
                ));
            }
            let items = self.messages(&rows)?;
            self.batch(batches, items, false, false);
        }
        self.finish_reconciliation(version, &seen, batches)
    }
    fn finish_reconciliation(
        &mut self,
        version: i64,
        seen: &BTreeSet<String>,
        batches: &mut Vec<Batch>,
    ) -> Result<()> {
        if !self.current()? {
            return Err(Error::Replaced);
        }
        let missing = self
            .hashes
            .keys()
            .chain(self.candidates.iter())
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect::<Vec<_>>();
        let mut retained = BTreeSet::new();
        let guids = missing
            .iter()
            .filter_map(|k| k.split_once('/'))
            .filter(|(c, _)| *c == "messages.message" || *c == "messages.reaction")
            .map(|(_, k)| k.to_string())
            .collect::<Vec<_>>();
        for keys in guids.chunks(128) {
            let rows = self.query(
                &format!(
                    "SELECT guid FROM message WHERE guid IN ({})",
                    vec!["?"; keys.len()].join(",")
                ),
                keys,
            )?;
            retained.extend(rows.iter().map(|r| r.text("guid").to_string()));
        }
        let final_small = self
            .small_observations()?
            .iter()
            .map(Observation::key)
            .collect::<BTreeSet<_>>();
        let mut absent = Vec::new();
        for key in missing.into_iter().collect::<BTreeSet<_>>() {
            let Some((collection, native_key)) = key.split_once('/') else {
                continue;
            };
            let supported = match collection {
                "messages.chat" => self.has("chat", &["guid"]),
                "messages.handle" => self.has("handle", &["id", "service"]),
                "messages.attachment" => self.has("attachment", &["guid"]),
                _ => true,
            };
            if !supported
                || final_small.contains(&key)
                || ((collection == "messages.message" || collection == "messages.reaction")
                    && retained.contains(native_key))
            {
                continue;
            }
            absent.push(Observation {
                kind: "absent".into(),
                collection: collection.into(),
                native_key: native_key.into(),
                data: json!({}),
            });
            self.undo_hashes
                .entry(key.clone())
                .or_insert_with(|| self.hashes.get(&key).copied());
            self.undo_native
                .entry(native_key.into())
                .or_insert_with(|| self.native_hashes.get(native_key).copied());
            self.hashes.remove(&key);
            self.native_hashes.remove(native_key);
        }
        self.batch(batches, absent, false, true);
        self.candidates
            .retain(|key| !seen.contains(key) && !final_small.contains(key));
        for item in batches
            .last()
            .map_or(&[][..], |b| b.observations.as_slice())
        {
            self.candidates.remove(&item.key());
        }
        self.reconciled = Some(version);

        Ok(())
    }
    pub fn reconciliation_pending(&self) -> bool {
        self.reconciliation_requested || self.pending_scan.is_some()
    }
    pub fn step(&mut self, request_reconciliation: bool) -> Result<Vec<Batch>> {
        let requested = self.reconciliation_requested || request_reconciliation;
        let live = self.poll(false)?;
        self.reconciliation_requested |= requested;
        if self.reconciled == self.version {
            self.reconciliation_requested = false;
            self.pending_scan = None;
            return Ok(live);
        }
        if !live.is_empty() {
            return Ok(live);
        }
        if !self.reconciliation_pending() {
            return Ok(live);
        }
        let version = self.version.ok_or(Error::Replaced)?;
        let mut scan = match self.pending_scan.take() {
            Some(scan) => scan,
            None => Scan {
                cursor: 0,
                seen: self
                    .small_observations()?
                    .iter()
                    .map(Observation::key)
                    .collect(),
            },
        };
        let page_start = scan.cursor;
        self.db.execute_batch("BEGIN")?;
        let result = self.history_page(&mut scan);
        let end = self.db.execute_batch("ROLLBACK");
        let (mut batches, done) = match result {
            Ok(page) if end.is_ok() => page,
            failed => {
                for (key, old) in std::mem::take(&mut self.undo_hashes) {
                    if let Some(h) = old {
                        self.hashes.insert(key, h);
                    } else {
                        self.hashes.remove(&key);
                    }
                }
                for (key, old) in std::mem::take(&mut self.undo_native) {
                    if let Some(h) = old {
                        self.native_hashes.insert(key, h);
                    } else {
                        self.native_hashes.remove(&key);
                    }
                }
                self.reconciliation_requested = true;
                scan.cursor = page_start;
                self.pending_scan = Some(scan);
                end?;
                return failed.map(|(batches, _)| batches);
            }
        };
        self.undo_hashes.clear();
        self.undo_native.clear();
        if !self.current()? {
            self.reconciliation_requested = true;
            return Ok(batches);
        }
        if done {
            self.finish_reconciliation(version, &scan.seen, &mut batches)?;
            self.undo_hashes.clear();
            self.undo_native.clear();
            self.reconciliation_requested = false;
        } else {
            self.pending_scan = Some(scan);
        }
        Ok(batches)
    }
    pub fn scan_cursor(&self) -> Option<i64> {
        self.pending_scan.as_ref().map(|scan| scan.cursor)
    }
    pub fn resume_scan(&mut self, cursor: i64) -> Result<()> {
        if cursor < 0 {
            return Err(Error::Replaced);
        }
        let seen = self
            .small_observations()?
            .iter()
            .map(Observation::key)
            .collect();
        self.pending_scan = Some(Scan { cursor, seen });
        self.reconciliation_requested = true;
        Ok(())
    }
    fn history_page(&mut self, scan: &mut Scan) -> Result<(Vec<Batch>, bool)> {
        let rows = self.projected(
            "SELECT ROWID AS row_position FROM message WHERE ROWID>? ORDER BY ROWID LIMIT 128",
            &[scan.cursor.to_string()],
            false,
        )?;
        if let Some(last) = rows.last() {
            scan.cursor = last.int("row_position");
        }
        for row in &rows {
            scan.seen.insert(format!(
                "messages.{}/{}",
                if reaction(row).is_some() {
                    "reaction"
                } else {
                    "message"
                },
                row.text("guid")
            ));
        }
        let observations = self.messages(&rows)?;
        let mut batches = Vec::new();
        self.batch(&mut batches, observations, false, false);
        Ok((batches, rows.len() < 128))
    }
    pub fn attachment_path(&self, value: &str) -> Option<PathBuf> {
        if value.is_empty() {
            return None;
        }
        let path = if let Some(suffix) = value.strip_prefix("~/") {
            self.home.join(suffix)
        } else {
            PathBuf::from(value)
        };
        let root = fs::canonicalize(&self.root).ok()?;
        let resolved = fs::canonicalize(&path)
            .or_else(|_| {
                path.parent()
                    .and_then(|p| fs::canonicalize(p).ok())
                    .map(|p| p.join(path.file_name().unwrap_or_default()))
                    .ok_or(std::io::ErrorKind::NotFound)
            })
            .ok()?;
        (resolved.starts_with(&root) && resolved != root).then_some(resolved)
    }
    pub fn media_source(&self, guid: &str) -> Result<Option<MediaSource>> {
        if !self.has("attachment", &["guid", "filename"]) {
            return Ok(None);
        }
        let sticker = if self.has("attachment", &["is_sticker"]) {
            "is_sticker"
        } else {
            "0 AS is_sticker"
        };
        let rows = self.query(
            &format!("SELECT filename,{sticker} FROM attachment WHERE guid=?"),
            &[guid.into()],
        )?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let Some(path) = self.attachment_path(row.text("filename")) else {
            return Ok(None);
        };
        if fs::metadata(&path).is_ok_and(|meta| dataless(&meta)) {
            return Ok(None);
        }
        Ok(Some(MediaSource {
            path,
            sticker: row.flag("is_sticker"),
        }))
    }
    pub fn live_companions(&self, guid: &str) -> Result<Vec<PathBuf>> {
        if !self.has("attachment", &["guid", "filename"]) {
            return Ok(Vec::new());
        }
        let position = self.query(
            "SELECT ROWID AS position FROM attachment WHERE guid=?",
            &[guid.into()],
        )?;
        let Some(position) = position.first().map(|row| row.text("position").to_owned()) else {
            return Ok(Vec::new());
        };
        let rows = self.query(
            "SELECT filename FROM attachment WHERE lower(filename) LIKE '%.mov' ORDER BY abs(ROWID-?) LIMIT 16",
            &[position],
        )?;
        Ok(rows
            .iter()
            .filter_map(|row| self.attachment_path(row.text("filename")))
            .filter(|path| fs::metadata(path).is_ok_and(|meta| !dataless(&meta)))
            .collect())
    }
    pub fn read_blob(&self, handle: &str, offset: u64, length: usize) -> Result<Vec<u8>> {
        if length == 0 || length > 128 * 1024 {
            return Err(Error::Blob);
        }
        if !self.has("attachment", &["guid", "filename"]) {
            return Err(Error::Blob);
        }
        let rows = self.query(
            "SELECT filename FROM attachment WHERE guid=?",
            &[handle.into()],
        )?;
        let path = rows
            .first()
            .and_then(|r| self.attachment_path(r.text("filename")))
            .ok_or(Error::Blob)?;
        if dataless(&fs::metadata(&path)?) {
            return Err(Error::Blob);
        }
        let mut file = fs::File::open(path)?;
        if dataless(&file.metadata()?) || offset > file.metadata()?.len() {
            return Err(Error::Blob);
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.take(u64::try_from(length).map_err(|_| Error::Blob)?)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}
fn target(value: &str) -> &str {
    if value.starts_with("p:") {
        value.split_once('/').map_or(value, |(_, s)| s)
    } else {
        value.strip_prefix("bp:").unwrap_or(value)
    }
}
fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref":{"collection":format!("messages.{collection}"),"native_key":key}})
}
fn reaction(row: &Row) -> Option<(i64, &str, bool, &str)> {
    let raw = row.int("associated_message_type");
    let removed = (3000..=3007).contains(&raw);
    let n = raw - if removed { 3000 } else { 2000 };
    let index = usize::try_from(n).ok()?;
    let kind = [
        "love",
        "like",
        "dislike",
        "laugh",
        "emphasize",
        "question",
        "emoji",
        "sticker",
    ]
    .get(index)?;
    let value = row.text("associated_message_guid");
    let guid = target(value);
    let part = if value.starts_with("p:") {
        value
            .split_once('/')?
            .0
            .strip_prefix("p:")?
            .parse::<i64>()
            .ok()?
    } else {
        0
    };
    (!guid.is_empty() && part >= 0).then_some((part, guid, removed, *kind))
}
#[cfg(target_os = "macos")]
fn dataless(meta: &fs::Metadata) -> bool {
    meta.st_flags() & 0x4000_0000 != 0
}
#[cfg(not(target_os = "macos"))]
fn dataless(_: &fs::Metadata) -> bool {
    false
}
