#![allow(clippy::unwrap_used)]

use ab_native::{
    capture::{Context, Snapshot},
    safari::{Kind, ReadContext, Reader},
};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ab-native-race-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}
#[test]
fn snapshot_rejects_replacement_before_completeness() {
    let root = directory();
    let path = root.join("store.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE rows(key TEXT);INSERT INTO rows VALUES('synthetic');")
        .unwrap();
    drop(db);
    let scan = Snapshot::open(&path, "SELECT 'fixture-1'").unwrap();
    assert_eq!(
        scan.query("SELECT * FROM rows", &[], 1, Duration::from_secs(1))
            .unwrap()
            .len(),
        1
    );
    let replacement = root.join("replacement.db");
    let db = rusqlite::Connection::open(&replacement).unwrap();
    db.execute_batch("CREATE TABLE rows(key TEXT);").unwrap();
    drop(db);
    std::fs::rename(replacement, &path).unwrap();
    assert!(scan.finish("SELECT 'fixture-1'").is_err());
    drop(scan);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn pending_safari_scans_require_ordered_acknowledgements() {
    let root = directory();
    let path = root.join("Bookmarks.plist");
    let bytes=b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Children</key><array/></dict></plist>";
    std::fs::write(&path, bytes).unwrap();
    let context = ReadContext {
        capture: Context {
            observed_at: "2026-01-01T00:00:00.000Z".into(),
            checkpoint: "a".into(),
            scan_id: "a".into(),
        },
        generation: "synthetic".into(),
        now: 0.0,
        days: 90.0,
        max_rows: 100_000,
        automation_permitted: false,
    };
    let mut reader = Reader::default();
    let first = reader
        .poll(&root, Kind::Bookmarks, &context)
        .unwrap()
        .unwrap();
    let second = reader
        .poll(&root, Kind::Bookmarks, &context)
        .unwrap()
        .unwrap();
    reader.commit(second).unwrap();
    assert!(reader.commit(first).is_err());
    std::fs::write(&path, b"broken").unwrap();
    assert!(reader.poll(&root, Kind::Bookmarks, &context).is_err());
    std::fs::write(&path, bytes).unwrap();
    assert!(
        reader
            .poll(&root, Kind::Bookmarks, &context)
            .unwrap()
            .is_none()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn xml_plist_limits_apply_before_constructing_nested_values() {
    use ab_native::plist;
    let deeply_nested = format!(
        "<plist>{}<string>synthetic</string>{}</plist>",
        "<array>".repeat(1_000),
        "</array>".repeat(1_000)
    );
    assert!(matches!(
        plist::decode(deeply_nested.as_bytes()),
        Err(ab_native::DecodeError::Limit)
    ));
    let ascii = b"{ fixture = (one, two); bytes = <0102>; }";
    let value = plist::decode(ascii).unwrap();
    assert_eq!(
        value.dictionary().unwrap()["fixture"],
        plist::Value::Array(vec![
            plist::Value::String("one".into()),
            plist::Value::String("two".into())
        ])
    );
}
