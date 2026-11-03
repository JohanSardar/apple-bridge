use crate::infocus::{self, InFocusEvent};
use ab_wire::gojson;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

const LOCKS: &[&str] = &[
    "loginwindow",
    "com.apple.loginwindow",
    "com.apple.ScreenSaver.Engine",
    "bridge.power.sleep",
];
const EXCLUDED: &[&str] = &[
    "com.apple.universalcontrol",
    "com.apple.UserNotificationCenter",
    "com.apple.systempreferences",
    "com.apple.controlcenter",
    "com.apple.notificationcenterui",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usage input IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("usage SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("usage JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("usage wire JSON: {0}")]
    Wire(#[from] gojson::Error),
    #[error("native JSON: {0}")]
    NativeJson(#[from] crate::native_json::Error),
    #[error("invalid usage time or timezone")]
    Time,
    #[error("incomplete usage input")]
    Incomplete,
    #[error("stale usage checkpoint")]
    StaleCheckpoint,
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Interval {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Context {
    pub generation: String,
    pub checkpoint: String,
    pub scan_id: String,
    pub observed_at: String,
    pub zone: String,
    pub local_name: String,
    #[serde(default)]
    pub blocked: Vec<Interval>,
    #[serde(default)]
    pub power: Vec<InFocusEvent>,
    #[serde(default)]
    pub history: Option<HistoryContext>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HistoryContext {
    pub database: String,
    pub hardware_id: String,
    pub now: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Observation {
    pub kind: &'static str,
    pub collection: &'static str,
    pub native_key: String,
    pub scoped_alias: String,
    pub observed_at: String,
    pub data: Box<serde_json::value::RawValue>,
}

#[derive(Debug, serde::Serialize)]
pub struct Scan {
    pub source: &'static str,
    pub generation: String,
    pub checkpoint: String,
    pub scan_id: String,
    pub scan_started_at: String,
    pub scope: [&'static str; 2],
    pub completion_evidence: &'static str,
    pub coverage: Value,
    pub observations: Vec<Observation>,
    pub live: bool,
    #[serde(skip)]
    current: BTreeMap<String, Value>,
    #[serde(skip)]
    base_checkpoint: String,
}

#[derive(Debug, Default)]
pub struct Reader {
    committed: BTreeMap<String, Value>,
    generation: String,
    checkpoint: String,
}

fn data_bytes(value: &Value) -> Result<Box<serde_json::value::RawValue>> {
    Ok(serde_json::from_slice(&crate::native_json::record_data(
        value,
    )?)?)
}

#[allow(clippy::cast_possible_truncation)]
fn timestamp(time: f64) -> Result<jiff::Timestamp> {
    if !time.is_finite() || !(-1e12..1e12).contains(&time) {
        return Err(Error::Time);
    }
    let seconds = time.floor() as i64;
    let nanos = ((time - time.floor()) * 1e9).round() as i32;
    jiff::Timestamp::new(seconds, nanos).map_err(|_| Error::Time)
}
fn stamp(time: f64) -> Result<String> {
    let t = timestamp(time)?;
    Ok(format!(
        "{}.{:03}Z",
        t.strftime("%Y-%m-%dT%H:%M:%S"),
        t.subsec_nanosecond() / 1_000_000
    ))
}

struct Rollup<'a> {
    device: &'a str,
    zone: &'a str,
    blocked: &'a [Interval],
    current: Option<InFocusEvent>,
    locked_at: Option<f64>,
    daily: BTreeMap<String, f64>,
    result: BTreeMap<String, Value>,
}
impl Rollup<'_> {
    fn close(&mut self, end: f64) -> Result<()> {
        let Some(current) = &self.current else {
            return Ok(());
        };
        if end <= current.time {
            return Ok(());
        }
        let capped = end.min(current.time + 14400.0);
        let credited = capped.min(self.locked_at.unwrap_or(capped));
        let mut cursor = current.time;
        while cursor < capped {
            let zoned = timestamp(cursor)?
                .in_tz(self.zone)
                .map_err(|_| Error::Time)?;
            let next = zoned
                .date()
                .tomorrow()
                .and_then(|d| d.in_tz(self.zone))
                .map_err(|_| Error::Time)?
                .timestamp()
                .as_second();
            #[allow(clippy::cast_precision_loss)]
            let until = capped.min(next as f64);
            let day = format!("{}@{}", zoned.date(), self.zone);
            let key = format!("{}/{day}/{}", self.device, current.bundle);
            let value=self.result.entry(key).or_insert_with(|| json!({"device_id":{"$ref":{"collection":"usage.device","native_key":self.device}},"day":day,"bundle_id":current.bundle,"provenance":"biome.app_infocus","decoder_version":infocus::VERSION,"uncertainty":"focus estimates, not active interaction; settings_parity: unproven; unclosed sessions omitted; four-hour cap; system UI excluded; historical lock/sleep gaps; remote batches may lag"}));
            let upper = until.min(credited);
            let mut available = (upper - cursor).max(0.0);
            let mut covered_until = cursor;
            for interval in self
                .blocked
                .iter()
                .filter(|i| i.start < upper && i.end > cursor)
            {
                let lower = cursor.max(covered_until).max(interval.start);
                let end = upper.min(interval.end);
                available -= (end - lower).max(0.0);
                covered_until = covered_until.max(end);
            }
            let excluded = EXCLUDED.contains(&current.bundle.as_str());
            if excluded {
                available = 0.0;
            }
            let daily = self.daily.entry(day).or_default();
            let seconds = available.max(0.0).min((86400.0 - *daily).max(0.0));
            *daily += seconds;
            value["usage_class"] = json!(if excluded {
                "excluded_system_ui"
            } else {
                "foreground_estimate"
            });
            value["clipping_provenance"] = json!(if self.blocked.is_empty() {
                "focus_lock_and_live_power_only"
            } else {
                "focus_lock_live_power_and_local_knowledgeC_display_lock"
            });
            value["seconds_estimated"] =
                json!(value["seconds_estimated"].as_f64().unwrap_or(0.0) + seconds);
            value["clipped_seconds"] =
                json!(value["clipped_seconds"].as_f64().unwrap_or(0.0) + until - cursor - seconds);
            value["sessions"] = json!(value["sessions"].as_u64().unwrap_or(0) + 1);
            value["latest_event_at"] = json!(stamp(end)?);
            cursor = until;
        }
        Ok(())
    }
}

pub fn records(
    input: &[InFocusEvent],
    device: &str,
    mac: bool,
    zone: &str,
    blocked: &[Interval],
) -> Result<BTreeMap<String, Value>> {
    jiff::tz::TimeZone::get(zone).map_err(|_| Error::Time)?;
    if input.iter().any(|e| !e.time.is_finite())
        || blocked
            .iter()
            .any(|i| !i.start.is_finite() || !i.end.is_finite() || i.end < i.start)
    {
        return Err(Error::Time);
    }
    let mut events = input.to_vec();
    events.sort_by(|a, b| {
        a.time
            .total_cmp(&b.time)
            .then(a.starting.cmp(&b.starting))
            .then(a.bundle.cmp(&b.bundle))
    });
    events.dedup();
    let mut barriers = blocked.to_vec();
    barriers.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut state = Rollup {
        device,
        zone,
        blocked: &barriers,
        current: None,
        locked_at: None,
        daily: BTreeMap::new(),
        result: BTreeMap::new(),
    };
    for e in events {
        if mac && LOCKS.contains(&e.bundle.as_str()) {
            if e.starting && state.current.is_some() && state.locked_at.is_none() {
                state.locked_at = Some(e.time);
            }
            continue;
        }
        if e.starting {
            state.close(e.time)?;
            state.current = Some(e);
            state.locked_at = None;
        } else if state.current.as_ref().is_some_and(|c| c.bundle == e.bundle) {
            state.close(e.time)?;
            state.current = None;
            state.locked_at = None;
        }
    }
    if let Some(locked) = state.locked_at {
        state.close(locked)?;
    }
    Ok(state.result)
}
fn events(path: &Path) -> Result<Vec<InFocusEvent>> {
    let mut output = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "tombstone" {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            output.extend(events(&entry.path())?);
        } else if kind.is_file() && !name.is_empty() && name.chars().all(char::is_numeric) {
            output.extend(infocus::decode(&std::fs::read(entry.path())?));
        }
    }
    Ok(output)
}

impl Reader {
    pub fn scan(&self, root: &Path, context: &Context, live: bool) -> Result<Scan> {
        let peers = peers(root)?;
        if !peers.iter().any(|p| p.3) {
            return Err(Error::Incomplete);
        }
        let history = context
            .history
            .as_ref()
            .map(|h| blocked_intervals(&root.join(&h.database), &h.hardware_id, h.now));
        let blocked = history.as_deref().unwrap_or(&context.blocked);
        let base = root.join("streams/restricted/App.InFocus");
        let mut current = BTreeMap::new();
        for (id, name, platform, local, model) in peers {
            let kind = if platform == 3 {
                "mac"
            } else if model.to_lowercase().contains("ipad") {
                "ipad"
            } else if platform == 2 {
                "iphone"
            } else {
                "unknown"
            };
            let fallback = format!("{kind} {}", id.chars().take(8).collect::<String>());
            let name = name.trim();
            let name = if name.is_empty() {
                if local {
                    context.local_name.as_str()
                } else {
                    &fallback
                }
            } else {
                name
            };
            let directory = if local {
                base.join("local")
            } else {
                base.join("remote").join(&id)
            };
            if !directory.exists() {
                if local {
                    return Err(Error::Incomplete);
                }
                continue;
            }
            let mut input = events(&directory)?;
            if local {
                input.extend(context.power.clone());
            }
            let latest = input
                .iter()
                .map(|e| e.time)
                .max_by(f64::total_cmp)
                .map(stamp)
                .transpose()?;
            current.insert(format!("usage.device/{id}"),json!({"name":name,"name_status":if local{"local_host_or_peer"}else if name==fallback{"platform_id_fallback"}else{"peer_metadata"},"platform":kind,"build":"unavailable","is_local":local,"latest_event_at":latest,"provenance":"biome.app_infocus","decoder_version":infocus::VERSION}));
            for (key, value) in records(
                &input,
                &id,
                kind == "mac",
                &context.zone,
                if local { blocked } else { &[] },
            )? {
                current.insert(format!("usage.app_day/{key}"), value);
            }
        }
        let mut keys = current.keys().collect::<Vec<_>>();
        keys.sort_by_key(|k| (!k.starts_with("usage.device/"), *k));
        let mut observations = Vec::new();
        for key in keys {
            if !live || self.committed.get(key) != current.get(key) {
                let (collection, native) = key.split_once('/').ok_or(Error::Incomplete)?;
                observations.push(Observation {
                    kind: "upsert",
                    collection: if collection == "usage.device" {
                        "usage.device"
                    } else {
                        "usage.app_day"
                    },
                    native_key: native.into(),
                    scoped_alias: format!("usage/{key}"),
                    observed_at: context.observed_at.clone(),
                    data: data_bytes(current.get(key).ok_or(Error::Incomplete)?)?,
                });
            }
        }
        Ok(Scan {
            source: "usage",
            generation: context.generation.clone(),
            checkpoint: context.checkpoint.clone(),
            scan_id: context.scan_id.clone(),
            scan_started_at: context.observed_at.clone(),
            scope: ["usage.device", "usage.app_day"],
            completion_evidence: "complete",
            coverage: json!({"fields":"estimated","missing":["settings_parity: unproven","historical lock/sleep gaps outside local knowledgeC evidence","remote names unavailable where absent from DevicePeer; distinct platform/id fallback","remote batches may lag"]}),
            observations,
            live,
            current,
            base_checkpoint: self.checkpoint.clone(),
        })
    }
    pub fn commit(&mut self, scan: Scan) -> Result<()> {
        if scan.checkpoint == self.checkpoint || scan.base_checkpoint != self.checkpoint {
            return Err(Error::StaleCheckpoint);
        }
        self.committed = scan.current;
        self.generation = scan.generation;
        self.checkpoint = scan.checkpoint;
        Ok(())
    }
}

type Peer = (String, String, i64, bool, String);

fn peers(root: &Path) -> Result<Vec<Peer>> {
    let connection = Connection::open_with_flags(
        root.join("sync/sync.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.execute_batch("BEGIN")?;
    let mut statement =
        connection.prepare("SELECT CAST(device_identifier AS TEXT),CAST(name AS TEXT),(CAST(platform AS INTEGER) & 4294967295),(CAST(me AS INTEGER) & 4294967295),CAST(model AS TEXT) FROM DevicePeer")?;
    let peers = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                r.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(peers)
}

pub fn blocked_intervals(path: &Path, hardware_id: &str, now: f64) -> Vec<Interval> {
    fn read(path: &Path, hardware_id: &str, now: f64) -> Result<Vec<Interval>> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut statement=connection.prepare("SELECT o.ZSTARTDATE,o.ZENDDATE FROM ZOBJECT o LEFT JOIN ZSOURCE s ON s.Z_PK=o.ZSOURCE WHERE ((o.ZSTREAMNAME='/device/isLocked' AND o.ZVALUEINTEGER=1) OR (o.ZSTREAMNAME='/display/isBacklit' AND o.ZVALUEINTEGER=0)) AND (o.ZSOURCE IS NULL OR s.ZDEVICEID=?)")?;
        let rows = statement.query_map([hardware_id], |r| {
            Ok((
                r.get::<_, Option<f64>>(0)?.unwrap_or(0.0) + 978_307_200.0,
                r.get::<_, Option<f64>>(1)?.unwrap_or(0.0) + 978_307_200.0,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (start, end) = row?;
            if start.is_finite()
                && end.is_finite()
                && end > start
                && start > 1.6e9
                && end < now + 60.0
            {
                result.push(Interval { start, end });
            }
        }
        Ok(result)
    }
    read(path, hardware_id, now).unwrap_or_default()
}
