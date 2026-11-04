use crate::{Config, Error};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
fn database(home: &Path) -> Result<Connection, Error> {
    Connection::open_with_flags(home.join("state.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| Error::Invalid)
}
pub fn status(home: &Path) -> Result<Value, Error> {
    let config = Config::load(home)?;
    let db = database(home)?;
    let raw: String = db
        .query_row("SELECT value FROM meta WHERE key='cursor'", [], |row| {
            row.get(0)
        })
        .map_err(|_| Error::Invalid)?;
    Ok(json!({"role": config.role, "cursor": serde_json::from_str::<Value>(&raw)?}))
}
fn private(path: &Path) -> Result<bool, Error> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(false);
    }
    let mut valid = meta.permissions().mode() & 0o777 == if meta.is_dir() { 0o700 } else { 0o600 };
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            valid &= private(&entry?.path())?;
        }
    }
    Ok(valid)
}
pub fn doctor(home: &Path) -> Result<Value, Error> {
    let store = ab_trust::backend_name(&ab_trust::StoreEnvironment::current());
    let last_error = fs::read(home.join("credential-store-error.json"))
        .ok()
        .and_then(|data| serde_json::from_slice::<ab_trust::CredentialStoreError>(&data).ok())
        .filter(|failure| failure.store == store);
    let credentials = match &last_error {
        Some(failure) => json!({"store":store,"last_error":failure}),
        None => json!({"store":store}),
    };
    match doctor_report(home) {
        Ok(mut report) => {
            report["credentials"] = credentials;
            if last_error.is_some() {
                report["ok"] = json!(false);
            }
            Ok(report)
        }
        Err(_) if last_error.is_some() => Ok(json!({"credentials":credentials,"ok":false})),
        Err(error) => Err(error),
    }
}
const SQLITE_BUDGET: Duration = Duration::from_secs(5);
const SHAPE_BUDGET: Duration = Duration::from_secs(5);
fn bounded_integrity(db: &Connection, budget: Duration) -> Result<Option<String>, Error> {
    let handle = db.get_interrupt_handle();
    let (done, finished) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if finished.recv_timeout(budget).is_err() {
            handle.interrupt();
        }
    });
    let outcome = db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0));
    drop(done);
    watchdog.join().map_err(|_| Error::Invalid)?;
    match outcome {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::SqliteFailure(failure, _))
            if failure.code == rusqlite::ErrorCode::OperationInterrupted =>
        {
            Ok(None)
        }
        Err(_) => Err(Error::Invalid),
    }
}
fn doctor_report(home: &Path) -> Result<Value, Error> {
    ab_trust::passkeys::validate_home(home)?;
    let db = database(home)?;
    db.execute_batch("BEGIN").map_err(|_| Error::Invalid)?;
    let started = Instant::now();
    let integrity = bounded_integrity(&db, SQLITE_BUDGET)?;
    let integrity_complete = integrity.is_some();
    let integrity = integrity.unwrap_or_else(|| "ok".into());
    let schema: String = db
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let modes = private(home)?;
    let mut result = json!({"ok":integrity == "ok" && modes,"private_modes":modes,"sqlite":if integrity_complete { integrity.as_str() } else { "incomplete" },"schema":schema,"credentials":{"store":"file"},"disk_encryption":"unverified","clock_skew":"unverified","versions":{"runtime":"rust","ipc":"ipc/1","feed":"feed/1","sqlite":rusqlite::version()},"warnings":[]});
    let cursor_raw: String = db
        .query_row("SELECT value FROM meta WHERE key='cursor'", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::Invalid)?;
    let cursor: Value = serde_json::from_str(&cursor_raw)?;
    let sequence = cursor["sequence"].as_i64().ok_or(Error::Invalid)?;
    let floor: i64 = db
        .query_row(
            "SELECT value FROM meta WHERE key='retention_floor'",
            [],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| Error::Invalid)?
        .parse()
        .map_err(|_| Error::Invalid)?;
    let applied: i64 = db
        .query_row(
            "SELECT count(*) FROM journal WHERE seq>? AND seq<=?",
            [floor, sequence],
            |r| r.get(0),
        )
        .map_err(|_| Error::Invalid)?;
    let inconsistent: i64 = db
        .query_row(
            "SELECT count(*) FROM records WHERE seq>?",
            [sequence],
            |r| r.get(0),
        )
        .map_err(|_| Error::Invalid)?;
    let mut counts = serde_json::Map::new();
    let mut stmt = db
        .prepare("SELECT collection,count(*) FROM records WHERE deleted=0 GROUP BY collection")
        .map_err(|_| Error::Invalid)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|_| Error::Invalid)?;
    for row in rows {
        let (key, count) = row.map_err(|_| Error::Invalid)?;
        counts.insert(key, json!(count));
    }
    let base: Option<String> = db
        .query_row("SELECT value FROM meta WHERE key='replica_base'", [], |r| {
            r.get(0)
        })
        .ok();
    let mut exactly_once = true;
    if let Some(base) = base {
        let mut next: i64 = base.parse().map_err(|_| Error::Invalid)?;
        let mut stmt = db
            .prepare("SELECT after_seq,through_seq FROM applied_batches ORDER BY after_seq")
            .map_err(|_| Error::Invalid)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|_| Error::Invalid)?;
        for row in rows {
            let (after, through) = row.map_err(|_| Error::Invalid)?;
            exactly_once &= after == next && through > after;
            next = through;
        }
        exactly_once &= next == sequence;
    }
    result["records"] = Value::Object(counts);
    result["cursor"] = cursor;
    result["floor"] = json!(floor);
    result["applied"] = json!(applied);
    result["exactly_once"] = json!(exactly_once);
    result["ok"] = json!(integrity == "ok" && modes && inconsistent == 0 && exactly_once);
    let (shape, shape_complete) =
        crate::shape::violations(&db, started + SQLITE_BUDGET + SHAPE_BUDGET)?;
    result["shape"] = shape;
    result["checks"] = json!({"sqlite_integrity": if integrity_complete { "complete" } else { "incomplete" }, "shape_scan": if shape_complete { "complete" } else { "incomplete" }});
    for (complete, warning) in [
        (integrity_complete, "SQLite integrity check did not finish within its time budget on this large database; doctor reports it as incomplete. Run sqlite3 on a stopped copy for a full integrity_check."),
        (shape_complete, "Record shape scan did not finish within its time budget; the reported shape counts cover only the records scanned."),
    ] {
        if !complete {
            result["warnings"].as_array_mut().ok_or(Error::Invalid)?.push(json!(warning));
        }
    }
    encryption(home, &mut result);
    encryption_warning(&mut result)?;
    for warning in ab_remote::ct::doctor(home, ab_remote::ct::now()) {
        result["warnings"]
            .as_array_mut()
            .ok_or(Error::Invalid)?
            .push(json!(warning));
    }
    clock(home, &mut result)?;
    Ok(result)
}
#[cfg(feature = "lab-harness")]
pub fn credential_lifecycle(home: &Path) -> Result<(), Error> {
    use ab_trust::CredentialStore;
    use std::io::Read;
    let store = ab_trust::FileStore::new(home).map_err(|_| Error::Invalid)?;
    let key = format!("lab-lifecycle-{}", std::process::id());
    let result = (|| {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        let mut random = fs::File::open("/dev/urandom")?;
        random.read_exact(&mut a)?;
        random.read_exact(&mut b)?;
        store.put(&key, &a).map_err(|_| Error::Invalid)?;
        if store.get(&key).map_err(|_| Error::Invalid)? != a {
            return Err(Error::Invalid);
        }
        store.put(&key, &b).map_err(|_| Error::Invalid)?;
        if a == b || store.get(&key).map_err(|_| Error::Invalid)? != b {
            return Err(Error::Invalid);
        }
        store.delete(&key).map_err(|_| Error::Invalid)?;
        if !matches!(store.get(&key), Err(ab_trust::Error::Io(e)) if e.kind()==std::io::ErrorKind::NotFound)
        {
            return Err(Error::Invalid);
        }
        println!(
            "{}",
            json!({"backend":"file","created":true,"used":true,"rotated":true,"revoked":true})
        );
        Ok(())
    })();
    store.delete(&key).map_err(|_| Error::Invalid)?;
    result
}

fn encryption_warning(result: &mut Value) -> Result<(), Error> {
    if result["disk_encryption"] == "enabled" {
        return Ok(());
    }
    let warning = if result["disk_encryption"] == "warning_not_enabled" {
        "The data directory is not on detected encrypted storage; enable FileVault or move the directory onto LUKS/dm-crypt storage."
    } else {
        "Disk encryption could not be checked; install findmnt/lsblk on Linux or check FileVault status on macOS, and verify encryption for the data directory."
    };
    result["warnings"]
        .as_array_mut()
        .ok_or(Error::Invalid)?
        .push(json!(warning));
    Ok(())
}
fn encryption(home: &Path, result: &mut Value) {
    if cfg!(target_os = "macos")
        && let Ok(output) = std::process::Command::new("fdesetup")
            .arg("status")
            .output()
        && output.status.success()
    {
        result["disk_encryption"] = json!(if String::from_utf8_lossy(&output.stdout)
            .contains("FileVault is On")
        {
            "enabled"
        } else {
            "warning_not_enabled"
        });
    }
    if cfg!(target_os = "linux")
        && let Ok(output) = std::process::Command::new("findmnt")
            .args(["-n", "-o", "SOURCE", "-T"])
            .arg(home)
            .output()
    {
        let device = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if output.status.success()
            && !device.is_empty()
            && let Ok(output) = std::process::Command::new("lsblk")
                .args(["--inverse", "--noheadings", "--output", "TYPE", &device])
                .output()
            && output.status.success()
        {
            result["disk_encryption"] = json!(if String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .any(|s| s == "crypt")
            {
                "enabled"
            } else {
                "warning_not_enabled"
            });
        }
    }
}

fn clock(home: &Path, result: &mut Value) -> Result<(), Error> {
    if Config::load(home)?.role == "gateway" {
        result["clock_skew"] = json!("n/a");
        return Ok(());
    }
    let sample = fs::read(home.join("gateway-clock.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let times = sample.as_ref().and_then(|s| {
        ["t0", "t1", "t2", "t3"]
            .iter()
            .map(|key| s[*key].as_str()?.parse::<jiff::Timestamp>().ok())
            .collect::<Option<Vec<_>>>()
    });
    if let Some(t) = times.filter(|t| t[3] >= t[0] && t[2] >= t[1]) {
        let rtt = (t[3].as_nanosecond() - t[0].as_nanosecond())
            - (t[2].as_nanosecond() - t[1].as_nanosecond());
        if rtt >= 0 {
            let offset = i128::midpoint(
                t[1].as_nanosecond() - t[0].as_nanosecond(),
                t[2].as_nanosecond() - t[3].as_nanosecond(),
            ) / 1_000_000;
            let uncertainty = (rtt / 2 + 999_999) / 1_000_000;
            let age = jiff::Timestamp::now().as_millisecond() - t[3].as_millisecond();
            result["clock_method"] = json!("authenticated_feed_ntp_four_timestamps");
            result["clock_offset_ms"] = json!(i64::try_from(offset).map_err(|_| Error::Invalid)?);
            result["clock_skew_ms"] = result["clock_offset_ms"].clone();
            result["clock_uncertainty_ms"] =
                json!(i64::try_from(uncertainty).map_err(|_| Error::Invalid)?);
            result["clock_rtt_ms"] =
                json!(i64::try_from(rtt / 1_000_000).map_err(|_| Error::Invalid)?);
            result["clock_sample_age_ms"] = json!(age);
            result["clock_skew"] = json!("ok");
            if !(0..=15000).contains(&age) {
                result["clock_skew"] = json!("warning_stale_sample");
            } else if uncertainty > 1000 {
                result["clock_skew"] = json!("warning_clock_uncertainty");
            }
            if offset - uncertainty > 1000 || offset + uncertainty < -1000 {
                result["clock_skew"] = json!("warning_clock_skew");
                result["warnings"].as_array_mut().ok_or(Error::Invalid)?.push(json!("Clock offset exceeds one second after accounting for round-trip uncertainty; enable network time synchronization on both hosts."));
            }
            return Ok(());
        }
    }
    result["warnings"].as_array_mut().ok_or(Error::Invalid)?.push(json!("Authenticated gateway clock exchange unavailable; reconnect peers with clock-exchange support, then rerun doctor."));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integrity_check_is_bounded_and_reports_incomplete() -> Result<(), Box<dyn std::error::Error>>
    {
        let db = Connection::open_in_memory()?;
        db.execute_batch(
            "CREATE TABLE t(a INTEGER, b TEXT); CREATE INDEX t_b ON t(b);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<400000)
             INSERT INTO t SELECT i, hex(randomblob(16)) FROM n;",
        )?;
        let started = Instant::now();
        assert_eq!(bounded_integrity(&db, Duration::from_millis(1))?, None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            bounded_integrity(&db, Duration::from_secs(60))?,
            Some("ok".into())
        );
        Ok(())
    }
}
