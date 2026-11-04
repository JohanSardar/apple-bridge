#![allow(clippy::unwrap_used)]

use ab_native::{capture::Context, files};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ab-files-walk-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn context(checkpoint: &str) -> Context {
    Context {
        observed_at: "2026-11-03T00:00:00.000Z".into(),
        checkpoint: checkpoint.into(),
        scan_id: format!("scan-{checkpoint}"),
    }
}

#[test]
fn depth_over_sixteen_is_incomplete_and_never_listed() {
    let root = scratch("depth");
    let mut deep = root.clone();
    for level in 0..17 {
        deep.push(format!("d{level}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("hidden.txt"), "x").unwrap();
    let walk = files::walk(&root, &|_, _| None).unwrap();
    assert!(!walk.complete);
    assert_eq!(walk.entries.len(), 16);
    assert!(
        walk.entries
            .iter()
            .all(|entry| entry.path.split('/').count() <= 16)
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn symlinks_and_their_targets_are_not_followed() {
    let root = scratch("link");
    let outside = scratch("outside");
    fs::write(outside.join("secret"), "s").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    let walk = files::walk(&root, &|_, _| None).unwrap();
    assert!(walk.complete);
    assert_eq!(walk.entries.len(), 0);
    fs::remove_dir_all(&root).unwrap();
    fs::remove_dir_all(&outside).unwrap();
}

#[test]
fn an_incomplete_walk_never_reports_a_file_absent() {
    let root = scratch("partial");
    fs::write(root.join("a"), "1").unwrap();
    fs::write(root.join("b"), "2").unwrap();
    let mut reader = files::Reader::default();
    let walk = files::walk(&root, &|_, _| None).unwrap();
    let first = reader
        .prepare(&walk, &BTreeMap::new(), &context("c1"))
        .unwrap();
    assert_eq!(first.batch.batch.observations.len(), 2);
    assert_eq!(first.batch.batch.completion_evidence, "complete");
    reader.commit(first).unwrap();

    let mut partial = files::walk(&root, &|_, _| None).unwrap();
    partial.entries.retain(|entry| entry.name == "a");
    partial.complete = false;
    let second = reader
        .prepare(&partial, &BTreeMap::new(), &context("c2"))
        .unwrap();
    assert!(second.batch.batch.observations.is_empty());
    assert_eq!(second.batch.batch.completion_evidence, "");
    reader.commit(second).unwrap();

    let mut complete = files::walk(&root, &|_, _| None).unwrap();
    complete.entries.retain(|entry| entry.name == "a");
    let third = reader
        .prepare(&complete, &BTreeMap::new(), &context("c3"))
        .unwrap();
    let kinds: Vec<_> = third
        .batch
        .batch
        .observations
        .iter()
        .map(|o| o.kind)
        .collect();
    assert_eq!(kinds, ["absent"]);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn state_round_trips_through_a_token() {
    let root = scratch("token");
    fs::write(root.join("a"), "1").unwrap();
    let mut reader = files::Reader::default();
    let walk = files::walk(&root, &|_, _| None).unwrap();
    let first = reader
        .prepare(&walk, &BTreeMap::new(), &context("c1"))
        .unwrap();
    reader.commit(first).unwrap();
    let restored: files::Reader =
        serde_json::from_str(&serde_json::to_string(&reader).unwrap()).unwrap();
    let again = restored
        .prepare(&walk, &BTreeMap::new(), &context("c2"))
        .unwrap();
    assert!(again.batch.batch.observations.is_empty());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn finder_tags_drop_their_colour_suffix_and_come_from_the_attribute_reader() {
    let plist = br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><array><string>Red
6</string><string>Work</string></array></plist>"#;
    assert_eq!(files::parse_tags(plist), ["Red", "Work"]);
    assert_eq!(files::parse_tags(b"not a plist"), Vec::<String>::new());
    let root = scratch("tags");
    fs::write(root.join("a"), "1").unwrap();
    let reader = |path: &std::path::Path, name: &str| {
        (name == files::TAGS_ATTRIBUTE && path.ends_with("a")).then(|| plist.to_vec())
    };
    let walk = files::walk(&root, &reader).unwrap();
    assert_eq!(walk.entries[0].tags, ["Red", "Work"]);
    fs::remove_dir_all(&root).unwrap();
}
