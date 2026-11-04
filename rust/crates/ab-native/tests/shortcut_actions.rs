#![allow(clippy::unwrap_used)]

use ab_native::shortcut_actions::discover;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    root: PathBuf,
    actions: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("ab-shortcut-actions-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let contents = root.join("Synthetic.app/Contents");
        let metadata = contents.join("Resources/Metadata.appintents");
        fs::create_dir_all(&metadata).unwrap();
        fs::write(
            contents.join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.example.synthetic</string></dict></plist>"#,
        )
        .unwrap();
        let actions = metadata.join("extract.actionsdata");
        Self { root, actions }
    }

    fn discover(&self) -> Value {
        discover(std::slice::from_ref(&self.root))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn items(result: &Value) -> &Vec<Value> {
    result["items"].as_array().unwrap()
}

#[test]
fn lists_discoverable_actions_with_app_identifier_and_title() {
    let fixture = Fixture::new("list");
    fs::write(
        &fixture.actions,
        r#"{"actions":{"Export":{"identifier":"Export","title":{"key":"Export"}},"Hidden":{"isDiscoverable":false},"Assistant":{"visibilityMetadata":{"assistantOnly":true}},"Untitled":{}}}"#,
    )
    .unwrap();
    let result = fixture.discover();
    assert_eq!(result["complete"], true);
    assert_eq!(result["schema_guaranteed"], false);
    assert_eq!(
        items(&result),
        &vec![
            json!({"app":"com.example.synthetic","identifier":"Export","title":"Export"}),
            json!({"app":"com.example.synthetic","identifier":"Untitled","title":"Untitled"}),
        ]
    );
}

#[test]
fn corrupt_oversized_and_unreadable_files_are_incomplete_and_empty() {
    let fixture = Fixture::new("bad");
    fs::write(&fixture.actions, "invalid").unwrap();
    let corrupt = fixture.discover();
    assert_eq!(corrupt["complete"], false);
    assert_eq!(items(&corrupt).len(), 0);
    fs::write(&fixture.actions, vec![b' '; (4 << 20) + 1]).unwrap();
    let oversized = fixture.discover();
    assert_eq!(oversized["complete"], false);
    assert_eq!(items(&oversized).len(), 0);
    fs::write(&fixture.actions, "").unwrap();
    assert_eq!(fixture.discover()["complete"], false);
}

#[test]
fn a_bundle_without_an_identifier_is_incomplete() {
    let fixture = Fixture::new("noid");
    fs::write(&fixture.actions, r#"{"actions":{"A":{}}}"#).unwrap();
    fs::write(
        fixture.root.join("Synthetic.app/Contents/Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict></dict></plist>"#,
    )
    .unwrap();
    let result = fixture.discover();
    assert_eq!(result["complete"], false);
    assert_eq!(items(&result).len(), 0);
}

#[test]
fn the_item_count_is_capped_at_two_thousand() {
    let fixture = Fixture::new("cap");
    let actions: serde_json::Map<String, Value> = (0..2001)
        .map(|i| (format!("Action{i}"), json!({"title":{"key":"Synthetic"}})))
        .collect();
    fs::write(
        &fixture.actions,
        serde_json::to_vec(&json!({"actions":actions})).unwrap(),
    )
    .unwrap();
    let result = fixture.discover();
    assert_eq!(result["complete"], false);
    assert_eq!(items(&result).len(), 2000);
}

#[test]
fn symlinks_are_never_followed() {
    let fixture = Fixture::new("link");
    std::os::unix::fs::symlink("/etc/passwd", &fixture.actions).unwrap();
    assert_eq!(items(&fixture.discover()).len(), 0);
    fs::remove_file(&fixture.actions).unwrap();
    fs::write(&fixture.actions, r#"{"actions":{"A":{}}}"#).unwrap();
    let outside = fixture.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let linked = fixture.root.join("Linked.app");
    std::os::unix::fs::symlink(fixture.root.join("Synthetic.app"), &linked).unwrap();
    assert_eq!(items(&fixture.discover()).len(), 1);
}

#[test]
fn files_outside_an_app_or_the_metadata_directory_are_ignored() {
    let fixture = Fixture::new("scope");
    let elsewhere = fixture.root.join("Plain/Metadata.appintents");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(
        elsewhere.join("extract.actionsdata"),
        r#"{"actions":{"A":{}}}"#,
    )
    .unwrap();
    let wrong = fixture.root.join("Synthetic.app/Contents/Resources/Other");
    fs::create_dir_all(&wrong).unwrap();
    fs::write(wrong.join("extract.actionsdata"), r#"{"actions":{"B":{}}}"#).unwrap();
    let result = fixture.discover();
    assert_eq!(items(&result).len(), 0);
    assert_eq!(result["complete"], true);
}

#[test]
fn missing_roots_are_skipped() {
    let fixture = Fixture::new("missing");
    fs::write(&fixture.actions, r#"{"actions":{"A":{}}}"#).unwrap();
    let result = discover(&[
        Path::new("/nonexistent-ab-root").to_path_buf(),
        fixture.root.clone(),
    ]);
    assert_eq!(items(&result).len(), 1);
    assert_eq!(result["complete"], true);
}
