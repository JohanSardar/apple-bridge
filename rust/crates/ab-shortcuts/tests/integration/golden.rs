use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ab-shortcuts-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_default();
    dir
}

fn build(directory: &Path, extra: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_ab-shortcuts"))
        .current_dir(directory)
        .arg("build")
        .arg("-unsigned")
        .args(extra)
        .output()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn same_tree(actual: &Path, golden: &Path) {
    let mut names: Vec<_> = fs::read_dir(golden)
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    names.sort();
    assert_ne!(names.len(), 0);
    for name in &names {
        let left = fs::read(actual.join(name)).unwrap_or_default();
        let right = fs::read(golden.join(name)).unwrap_or_default();
        assert!(
            left == right,
            "{} differs from the Go output",
            name.to_string_lossy()
        );
    }
    assert_eq!(fs::read_dir(actual).map_or(0, Iterator::count), names.len());
}

#[test]
fn bundled_packs_match_go_byte_for_byte() {
    let directory = scratch("bundled");
    let sources = directory.join("packaging/shortcuts");
    fs::create_dir_all(&sources).unwrap_or_default();
    for entry in fs::read_dir(root().join("packaging/shortcuts")).unwrap_or_else(|e| panic!("{e}"))
    {
        let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
        if path.extension().is_some_and(|x| x == "json") {
            fs::copy(&path, sources.join(path.file_name().unwrap_or_default())).unwrap_or_default();
        }
    }
    build(&directory, &[]);
    same_tree(
        &directory.join(".build/shortcuts"),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"),
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn edge_source_matches_go_byte_for_byte() {
    let directory = scratch("edge");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edge.json"),
        directory.join("edge.json"),
    )
    .unwrap_or_default();
    build(&directory, &["-source", "edge.json"]);
    same_tree(
        &directory.join(".build/shortcuts"),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden-edge"),
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn invalid_sources_are_rejected() {
    let directory = scratch("invalid");
    for (index, source) in [
        r#"{"version":2,"name":"x","actions":[]}"#,
        r#"{"version":1,"name":" ","actions":[]}"#,
        r#"{"version":1,"name":"x"}"#,
        r#"{"version":1,"name":"x","actions":[],"extra":1}"#,
        r#"{"version":1,"name":"x","actions":[]} {}"#,
        r#"{"version":1,"name":"x","actions":[{"key":"a","identifier":"i","parameters":{"UUID":"u"}}]}"#,
        r#"{"version":1,"name":"x","actions":[{"key":"a","identifier":"i"},{"key":"a","identifier":"i"}]}"#,
        r#"{"version":1,"name":"x","actions":[{"key":"a","identifier":"i","parameters":{"p":{"$output":"later"}}}]}"#,
        r#"{"version":1,"name":"x","actions":[{"key":"a","identifier":"i","parameters":{"$bad":1}}]}"#,
        r#"{"version":1,"name":"x","actions":[{"key":"a","identifier":"i","parameters":{"p":null}}]}"#,
    ]
    .iter()
    .enumerate()
    {
        let path = directory.join(format!("case{index}.json"));
        fs::write(&path, source).unwrap_or_default();
        assert!(ab_shortcuts::pack::compile(&path).is_err(), "case {index}");
    }
    let _ = fs::remove_dir_all(directory);
}

#[cfg(target_os = "macos")]
#[test]
fn bundled_packs_are_valid_property_lists() {
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"))
        .unwrap_or_else(|e| panic!("{e}"))
    {
        let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
        if path.extension().is_some_and(|x| x == "shortcut") {
            let status = Command::new("/usr/bin/plutil")
                .arg("-lint")
                .arg(&path)
                .output()
                .unwrap_or_else(|e| panic!("{e}"));
            assert!(status.status.success(), "{path:?}");
        }
    }
}
