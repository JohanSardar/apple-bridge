use crate::capture::{Error, Result};
use base64::Engine as _;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::SystemTime,
};

type Row = Map<String, Value>;

const FIELDS: [&str; 16] = [
    "store_status",
    "store_coverage",
    "store_identifier_match",
    "travel_time",
    "travel_advisory_behavior",
    "structured_location",
    "invitation_status",
    "owner_participation_status",
    "privacy_level",
    "priority",
    "conference_url",
    "attachments",
    "start_location",
    "attendee_proposals",
    "participant_comments",
    "store_alarm_details",
];

const REQUIRED: [(&str, &[&str]); 7] = [
    (
        "CalendarItem",
        &[
            "ROWID",
            "unique_identifier",
            "UUID",
            "external_id",
            "calendar_id",
            "location_id",
            "self_attendee_id",
            "travel_time",
            "travel_advisory_behavior",
            "invitation_status",
            "privacy_level",
            "priority",
            "conference_url",
        ],
    ),
    ("Calendar", &["ROWID", "UUID"]),
    (
        "Location",
        &[
            "ROWID",
            "title",
            "address",
            "latitude",
            "longitude",
            "radius",
        ],
    ),
    ("Participant", &["ROWID", "status"]),
    ("Conference", &["owner_id", "url"]),
    ("Attachment", &["owner_id", "file_id"]),
    (
        "AttachmentFile",
        &[
            "ROWID",
            "UUID",
            "filename",
            "local_path",
            "file_size",
            "format",
        ],
    ),
];

#[derive(Debug, Default)]
pub struct Snapshot {
    events: BTreeMap<String, Vec<Row>>,
    attachments: BTreeMap<i64, Vec<Row>>,
}

fn rows(connection: &Connection, sql: &str) -> Result<Vec<Row>> {
    let mut statement = connection.prepare(sql).map_err(|_| Error::Corrupt)?;
    let names: Vec<String> = statement.column_names().map(str::to_owned).collect();
    let mut cursor = statement.query([]).map_err(|_| Error::Corrupt)?;
    let mut result = Vec::new();
    while let Some(row) = cursor.next().map_err(|_| Error::Corrupt)? {
        let mut object = Row::new();
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index).map_err(|_| Error::Corrupt)? {
                ValueRef::Text(bytes) => {
                    Value::String(String::from_utf8(bytes.to_vec()).map_err(|_| Error::Corrupt)?)
                }
                ValueRef::Integer(number) => finite(wide(number)),
                ValueRef::Real(number) => finite(number),
                _ => Value::Null,
            };
            object.insert(name.clone(), value);
        }
        result.push(object);
    }
    Ok(result)
}

#[allow(clippy::cast_precision_loss)]
fn wide(number: i64) -> f64 {
    number as f64
}

#[allow(clippy::cast_precision_loss)]
fn wide_unsigned(number: u64) -> f64 {
    number as f64
}

fn finite(number: f64) -> Value {
    if number.is_finite() {
        json!(number)
    } else {
        Value::Null
    }
}

fn text<'a>(row: &'a Row, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}

impl Snapshot {
    pub fn read(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| Error::Incomplete)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(25))
            .map_err(|_| Error::Incomplete)?;
        connection
            .execute_batch("BEGIN")
            .map_err(|_| Error::Incomplete)?;
        for (table, fields) in REQUIRED {
            let columns: BTreeSet<String> =
                rows(&connection, &format!("PRAGMA table_info({table})"))?
                    .iter()
                    .map(|row| text(row, "name").to_owned())
                    .collect();
            if !fields.iter().all(|field| columns.contains(*field)) {
                return Err(Error::Unsupported);
            }
        }
        let mut items = rows(
            &connection,
            "SELECT i.ROWID AS item_id,i.unique_identifier,i.UUID,i.external_id,c.UUID AS calendar_uuid,i.travel_time,i.travel_advisory_behavior,i.invitation_status,i.privacy_level,i.priority,i.conference_url,p.status AS self_status,l.title AS location_title,l.address AS location_address,l.latitude AS latitude,l.longitude AS longitude,l.radius AS radius FROM CalendarItem i JOIN Calendar c ON c.ROWID=i.calendar_id LEFT JOIN Participant p ON p.ROWID=i.self_attendee_id LEFT JOIN Location l ON l.ROWID=i.location_id",
        )?;
        let mut conferences: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
        for conference in rows(
            &connection,
            "SELECT owner_id,url FROM Conference WHERE url IS NOT NULL",
        )? {
            let owner = conference.get("owner_id").and_then(Value::as_f64);
            let link = text(&conference, "url");
            if let (Some(owner), false) = (owner.and_then(exact), link.is_empty()) {
                conferences
                    .entry(owner)
                    .or_default()
                    .insert(link.to_owned());
            }
        }
        for item in &mut items {
            if item.get("conference_url") != Some(&Value::Null) {
                continue;
            }
            let owner = item.get("item_id").and_then(Value::as_f64).and_then(exact);
            if let Some(links) = owner.and_then(|owner| conferences.get(&owner))
                && links.len() == 1
                && let Some(link) = links.iter().next()
            {
                item.insert("conference_url".into(), Value::String(link.clone()));
            }
        }
        let mut snapshot = Self::default();
        for item in items {
            let keys: BTreeSet<&str> = ["unique_identifier", "UUID", "external_id"]
                .iter()
                .map(|key| text(&item, key))
                .filter(|key| !key.is_empty())
                .collect();
            for key in keys {
                snapshot
                    .events
                    .entry(key.to_owned())
                    .or_default()
                    .push(item.clone());
            }
        }
        for row in rows(
            &connection,
            "SELECT a.owner_id,f.UUID,f.filename,f.local_path,f.file_size,f.format FROM Attachment a JOIN AttachmentFile f ON f.ROWID=a.file_id",
        )? {
            if let Some(owner) = row.get("owner_id").and_then(Value::as_f64).and_then(exact) {
                snapshot.attachments.entry(owner).or_default().push(row);
            }
        }
        Ok(snapshot)
    }

    #[must_use]
    pub fn event(
        &self,
        identifier: &str,
        external: &str,
        calendar: &str,
        detached: bool,
    ) -> Option<Row> {
        if detached {
            return None;
        }
        for key in [identifier, external] {
            if key.is_empty() {
                continue;
            }
            let candidates: Vec<&Row> = self
                .events
                .get(key)
                .into_iter()
                .flatten()
                .filter(|row| same_uuid(text(row, "calendar_uuid"), calendar))
                .collect();
            if candidates.len() > 1 {
                return None;
            }
            if let Some(found) = candidates.first() {
                let mut row = (*found).clone();
                let column = ["unique_identifier", "UUID", "external_id"]
                    .iter()
                    .find(|column| text(&row, column) == key)
                    .copied()
                    .unwrap_or("unknown");
                let prefix = if key == identifier {
                    "calendarItemIdentifier:"
                } else {
                    "calendarItemExternalIdentifier:"
                };
                row.insert(
                    "matched_by".into(),
                    Value::String(format!("{prefix}{column}")),
                );
                return Some(row);
            }
        }
        None
    }
}

#[allow(clippy::cast_possible_truncation)]
fn exact(number: f64) -> Option<i64> {
    (number.fract() == 0.0 && number.abs() < 9.0e15).then_some(number as i64)
}

fn same_uuid(stored: &str, calendar: &str) -> bool {
    if stored == calendar {
        return true;
    }
    match (normalized_uuid(stored), normalized_uuid(calendar)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn normalized_uuid(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let dashes = [8, 13, 18, 23];
    if bytes.len() != 36 {
        return None;
    }
    let mut result = String::with_capacity(32);
    for (index, byte) in bytes.iter().enumerate() {
        if dashes.contains(&index) {
            if *byte != b'-' {
                return None;
            }
        } else if byte.is_ascii_hexdigit() {
            result.push(byte.to_ascii_lowercase() as char);
        } else {
            return None;
        }
    }
    Some(result)
}

#[derive(Debug, Clone)]
struct Blob {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
    digest: String,
    handle: String,
    mime: &'static str,
}

#[derive(Debug)]
pub struct Enrichment {
    root: PathBuf,
    snapshot: Option<Snapshot>,
    signature: Vec<String>,
    blobs: BTreeMap<String, Blob>,
    files: BTreeMap<PathBuf, Blob>,
    handles: BTreeMap<String, String>,
    counter: u64,
}

fn dataless(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        metadata.st_flags() & 0x4000_0000 != 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = metadata;
        false
    }
}

fn put(
    data: &mut Map<String, Value>,
    coverage: &mut Map<String, Value>,
    key: &str,
    value: Value,
    state: &str,
    reason: &str,
) {
    data.insert(key.to_owned(), value);
    coverage.insert(key.to_owned(), status(state, reason));
}

fn status(status: &str, reason: &str) -> Value {
    json!({"status": status, "reason": reason})
}

fn mime_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

impl Enrichment {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            snapshot: None,
            signature: Vec::new(),
            blobs: BTreeMap::new(),
            files: BTreeMap::new(),
            handles: BTreeMap::new(),
            counter: 0,
        }
    }

    #[must_use]
    pub fn handles(&self) -> &BTreeMap<String, String> {
        &self.handles
    }

    fn database(&self) -> PathBuf {
        self.root.join("Calendar.sqlitedb")
    }

    fn current_signature(&self) -> Vec<String> {
        let path = self.database();
        let wal = PathBuf::from(format!("{}-wal", path.display()));
        [path, wal]
            .iter()
            .map(|path| match fs::symlink_metadata(path) {
                Ok(metadata) => format!(
                    "{}:{}:{}:{}",
                    metadata.ino(),
                    metadata.size(),
                    metadata.mtime(),
                    metadata.mtime_nsec()
                ),
                Err(_) => "missing".to_owned(),
            })
            .collect()
    }

    fn refresh(&mut self, force: bool) -> bool {
        let current = self.current_signature();
        if !force && self.snapshot.is_some() && current == self.signature {
            return true;
        }
        if let Ok(snapshot) = Snapshot::read(&self.database()) {
            self.snapshot = Some(snapshot);
            self.signature = current;
            true
        } else {
            self.snapshot = None;
            false
        }
    }

    pub fn enrich(
        &mut self,
        observations: &[Value],
        external: &dyn Fn(&str) -> String,
        force: bool,
    ) -> Vec<Value> {
        let ready = self.refresh(force);
        observations
            .iter()
            .map(|observation| {
                if observation["collection"] != "calendar.event" {
                    return observation.clone();
                }
                self.event(observation, external, ready)
            })
            .collect()
    }

    fn event(
        &mut self,
        observation: &Value,
        external: &dyn Fn(&str) -> String,
        ready: bool,
    ) -> Value {
        let mut value = observation.clone();
        let mut data = observation["data"].as_object().cloned().unwrap_or_default();
        let native = observation["native_key"].as_str().unwrap_or_default();
        let identifier = native.split('|').next().unwrap_or_default();
        let calendar = observation["data"]["calendar_id"]["$ref"]["native_key"]
            .as_str()
            .unwrap_or_default();
        let detached = observation["data"]["is_exception"]
            .as_bool()
            .unwrap_or(false);
        let row = if ready {
            self.snapshot.as_ref().and_then(|snapshot| {
                snapshot.event(identifier, &external(native), calendar, detached)
            })
        } else {
            None
        };
        let mut coverage = Map::new();
        if let Some(row) = row {
            self.matched(&mut data, &mut coverage, &row);
        } else {
            let reason = if ready {
                "calendar_store_identity_unverified"
            } else {
                "calendar_store_unavailable_or_schema_unverified"
            };
            for field in FIELDS
                .iter()
                .filter(|field| !["store_status", "store_coverage"].contains(field))
            {
                put(
                    &mut data,
                    &mut coverage,
                    field,
                    Value::Null,
                    "unsupported",
                    reason,
                );
            }
            data.insert("store_status".into(), json!("unsupported"));
        }
        data.insert("store_coverage".into(), Value::Object(coverage));
        value["data"] = Value::Object(data);
        value
    }

    fn matched(
        &mut self,
        data: &mut Map<String, Value>,
        coverage: &mut Map<String, Value>,
        row: &Row,
    ) {
        let get = |key: &str| row.get(key).cloned().unwrap_or(Value::Null);
        put(
            data,
            coverage,
            "store_identifier_match",
            get("matched_by"),
            "complete",
            "",
        );
        for field in [
            "start_location",
            "attendee_proposals",
            "participant_comments",
            "store_alarm_details",
        ] {
            put(
                data,
                coverage,
                field,
                Value::Null,
                "unsupported",
                "positive_native_fixture_not_verified_on_this_os",
            );
        }
        for field in [
            "travel_advisory_behavior",
            "invitation_status",
            "privacy_level",
            "priority",
        ] {
            put(data, coverage, field, get(field), "complete", "");
        }
        put(
            data,
            coverage,
            "owner_participation_status",
            get("self_status"),
            "complete",
            "",
        );
        Self::travel(data, coverage, row);
        self.attachments(data, coverage, row);
    }

    fn travel(
        &self,
        data: &mut Map<String, Value>,
        coverage: &mut Map<String, Value>,
        row: &Row,
    ) {
        let get = |key: &str| row.get(key).cloned().unwrap_or(Value::Null);
        let travel = get("travel_time");
        let known_zero = travel.as_f64() == Some(0.0);
        let reason = if known_zero {
            ""
        } else if travel.is_null() {
            "travel_time_unavailable"
        } else {
            "positive_travel_time_units_not_verified_on_this_os"
        };
        put(
            data,
            coverage,
            "travel_time",
            if known_zero { travel } else { Value::Null },
            if known_zero {
                "complete"
            } else {
                "unsupported"
            },
            reason,
        );
        if get("location_title").is_null() && get("location_address").is_null() {
            put(
                data,
                coverage,
                "structured_location",
                Value::Null,
                "complete",
                "",
            );
        } else {
            put(
                data,
                coverage,
                "structured_location",
                json!({"title": get("location_title"), "address": get("location_address"), "latitude": null, "longitude": null, "radius": null}),
                "partial",
                "coordinate_units_not_verified_on_this_os",
            );
        }
        let conference = get("conference_url");
        let link = conference.as_str().unwrap_or_default();
        if conference.is_null() || link.is_empty() {
            put(
                data,
                coverage,
                "conference_url",
                Value::Null,
                "complete",
                "",
            );
        } else if link_scheme_allowed(link) {
            put(
                data,
                coverage,
                "conference_url",
                conference.clone(),
                "complete",
                "",
            );
        } else {
            put(
                data,
                coverage,
                "conference_url",
                Value::Null,
                "unsupported",
                "unsupported_conference_url_scheme",
            );
        }
    }

    fn attachments(
        &mut self,
        data: &mut Map<String, Value>,
        coverage: &mut Map<String, Value>,
        row: &Row,
    ) {
        let get = |key: &str| row.get(key).cloned().unwrap_or(Value::Null);
        let owner = get("item_id").as_f64().and_then(exact).unwrap_or(0);
        let attachments = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.attachments.get(&owner))
            .cloned()
            .unwrap_or_default();
        let mut described = Vec::new();
        let mut complete = true;
        for item in &attachments {
            match self.attachment(item) {
                Ok(blob) => {
                    described.push(json!({
                        "filename": item.get("filename").filter(|name| !name.is_null()).cloned().unwrap_or_else(|| json!("Attachment")),
                        "mime_type": blob.mime,
                        "blob": {"state": "present", "digest": blob.digest, "size": wide_unsigned(blob.size)},
                        "derivatives": {},
                    }));
                    self.handles
                        .insert(blob.digest.clone(), blob.handle.clone());
                }
                Err(_) => complete = false,
            }
        }
        if !attachments.is_empty() {
            coverage.insert(
                "attachment_derivatives".into(),
                status("unsupported", "native_calendar_derivatives_not_verified"),
            );
        }
        let state = if complete { "complete" } else { "partial" };
        let reason = if complete {
            ""
        } else {
            "attachment_bytes_not_locally_available_or_path_unverified"
        };
        put(
            data,
            coverage,
            "attachments",
            Value::Array(described),
            state,
            reason,
        );
        let all_complete = coverage.values().all(|entry| entry["status"] == "complete");
        data.insert(
            "store_status".into(),
            json!(if all_complete { "complete" } else { "partial" }),
        );
    }

    fn handle(&mut self, digest: &str) -> String {
        self.counter += 1;
        let mut hash = Sha256::new();
        hash.update(digest.as_bytes());
        hash.update(self.counter.to_be_bytes());
        hash.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
                .to_be_bytes(),
        );
        format!("calendar-attachment:{:x}", hash.finalize())[..52].to_owned()
    }

    fn resolve(&self, row: &Row) -> Result<PathBuf> {
        let path = text(row, "local_path");
        if path.is_empty() {
            return Err(Error::Incomplete);
        }
        let candidate = if let Some(rest) = path.strip_prefix("file://") {
            PathBuf::from(rest)
        } else if path.starts_with('/') {
            PathBuf::from(path)
        } else {
            self.root.join(path)
        };
        let resolved = fs::canonicalize(&candidate).map_err(|_| Error::Incomplete)?;
        let root = fs::canonicalize(&self.root).map_err(|_| Error::Incomplete)?;
        if resolved.starts_with(&root) && resolved != root {
            Ok(resolved)
        } else {
            Err(Error::Unsupported)
        }
    }

    fn attachment(&mut self, row: &Row) -> Result<Blob> {
        let path = self.resolve(row)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| Error::Incomplete)?;
        if !metadata.file_type().is_file() || dataless(&metadata) {
            return Err(Error::Incomplete);
        }
        let size = metadata.len();
        let modified = metadata.modified().map_err(|_| Error::Incomplete)?;
        if let Some(cached) = self.files.get(&path)
            && cached.size == size
            && cached.modified == modified
        {
            return Ok(cached.clone());
        }
        let mut file = open_local(&path)?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0_u8; 128 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|_| Error::Incomplete)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        let after = fs::symlink_metadata(&path).map_err(|_| Error::Incomplete)?;
        if after.len() != size || after.modified().ok() != Some(modified) {
            return Err(Error::Incomplete);
        }
        let digest = format!("{:x}", hash.finalize());
        let blob = Blob {
            mime: mime_of(&path),
            handle: self.handle(&digest),
            path: path.clone(),
            size,
            modified,
            digest,
        };
        self.files.insert(path, blob.clone());
        self.blobs.insert(blob.handle.clone(), blob.clone());
        Ok(blob)
    }

    pub fn read_blob(&self, handle: &str, offset: u64, length: usize) -> Result<Value> {
        let blob = self.blobs.get(handle).ok_or(Error::Incomplete)?;
        if offset > blob.size || length == 0 || length > 131_072 {
            return Err(Error::Incomplete);
        }
        let metadata = fs::symlink_metadata(&blob.path).map_err(|_| Error::Incomplete)?;
        let root = fs::canonicalize(&self.root).map_err(|_| Error::Incomplete)?;
        let resolved = fs::canonicalize(&blob.path).map_err(|_| Error::Incomplete)?;
        if dataless(&metadata)
            || !resolved.starts_with(&root)
            || metadata.len() != blob.size
            || metadata.modified().ok() != Some(blob.modified)
        {
            return Err(Error::Incomplete);
        }
        let mut file = open_local(&blob.path)?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|_| Error::Incomplete)?;
        let mut bytes = Vec::new();
        file.take(length as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Incomplete)?;
        Ok(json!({
            "t": "BlobChunk",
            "handle": handle,
            "offset": wide_unsigned(offset),
            "bytes": base64::engine::general_purpose::STANDARD.encode(&bytes),
            "digest": blob.digest,
            "eof": offset + bytes.len() as u64 == blob.size,
        }))
    }
}

fn link_scheme_allowed(link: &str) -> bool {
    let Some((scheme, rest)) = link.split_once(':') else {
        return false;
    };
    ["https", "http"].contains(&scheme.to_ascii_lowercase().as_str()) && !rest.is_empty()
}

fn open_local(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Error::Incomplete)?;
    let metadata = file.metadata().map_err(|_| Error::Incomplete)?;
    if !metadata.file_type().is_file() || dataless(&metadata) {
        return Err(Error::Incomplete);
    }
    Ok(file)
}
