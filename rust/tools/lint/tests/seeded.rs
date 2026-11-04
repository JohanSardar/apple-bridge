use ab_lint::{Finding, lint_tree};
use std::{fs, path::Path};

const PACKAGE: &str = "// swift-tools-version:6.0\nlet x = sourceTargets.map { $0 }\n";
const HELPER: &str = "await SessionSources.register(into: peer)\n";
const SOURCES: &str = "enum SessionSources {}\n";

fn write(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
    }
    fs::write(path, body).unwrap_or_else(|e| panic!("{e}"));
}

fn baseline(root: &Path) {
    write(root, "native/Package.swift", PACKAGE);
    write(
        root,
        "native/Sources/AppleSession/HelperCommand.swift",
        HELPER,
    );
    write(
        root,
        "native/Sources/AppleSession/SessionSources.swift",
        SOURCES,
    );
    write(
        root,
        "rust/crates/ab-sys/src/lib.rs",
        "fn f() {\n// SAFETY: inherited descriptor\n}\n",
    );
    write(
        root,
        "rust/crates/ab-wire/src/lib.rs",
        "const A: &str = \"https://x.invalid/* ok */\";\nconst B: &str = br###\"// /* */\"###;\nconst C: char = '/';\n",
    );
    write(
        root,
        "native/Sources/App/Ok.swift",
        "let url = \"https://x.invalid//ok\"\nlet raw = \"\"\"\n// inside\n\"\"\"\n",
    );
    write(
        root,
        "rust/Cargo.toml",
        "[dependencies]\nserde_json = \"1\"\n",
    );
}

fn lint_with(relative: &str, body: &str) -> Vec<Finding> {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    baseline(dir.path());
    write(dir.path(), relative, body);
    lint_tree(dir.path())
}

fn assert_caught(relative: &str, body: &str, message: &str) {
    let findings = lint_with(relative, body);
    assert!(
        findings
            .iter()
            .any(|f| f.path == relative && f.message == message),
        "{relative} {body:?} not caught: {findings:?}"
    );
}

#[test]
fn clean_baseline_passes() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    baseline(dir.path());
    assert_eq!(lint_tree(dir.path()), Vec::new());
}

#[test]
fn swift_line_comment() {
    assert_caught("native/Sources/App/A.swift", "let a = 1 // x\n", "comment");
}

#[test]
fn swift_block_comment() {
    assert_caught(
        "native/Sources/App/A.swift",
        "/* x */\nlet a = 1\n",
        "comment",
    );
}

#[test]
fn swift_header_allowed_only_in_package_first_line() {
    assert_caught(
        "native/Sources/App/A.swift",
        "// swift-tools-version:6.0\n",
        "comment",
    );
    assert_caught(
        "native/Package.swift",
        "let a = sourceTargets.map { $0 }\n// swift-tools-version:6.0\n",
        "comment",
    );
}

#[test]
fn rust_line_comment() {
    assert_caught(
        "rust/crates/ab-wire/src/a.rs",
        "fn f() {}\n// x\n",
        "comment",
    );
}

#[test]
fn rust_block_comment() {
    assert_caught(
        "rust/crates/ab-wire/src/a.rs",
        "/* x */ fn f() {}\n",
        "comment",
    );
}

#[test]
fn rust_doc_attribute() {
    assert_caught(
        "rust/crates/ab-wire/src/a.rs",
        "#[doc = \"x\"]\nfn f() {}\n",
        "doc attribute",
    );
    assert_caught(
        "rust/crates/ab-wire/src/b.rs",
        "#![doc(hidden)]\n",
        "doc attribute",
    );
}

#[test]
fn rust_safety_comment_only_in_sys() {
    assert_caught(
        "rust/crates/ab-wire/src/a.rs",
        "// SAFETY: inherited descriptor\n",
        "comment",
    );
    assert_caught(
        "rust/crates/ab-sys/src/a.rs",
        "/* SAFETY: x */\n",
        "comment",
    );
}

#[test]
fn cargo_banned_features() {
    for feature in ["arbitrary_precision", "preserve_order"] {
        assert_caught(
            "rust/crates/ab-wire/Cargo.toml",
            &format!(
                "[dependencies]\nserde_json = {{ version = \"1\", features = [\"{feature}\"] }}\n"
            ),
            &format!("banned JSON feature: {feature}"),
        );
    }
}

#[test]
fn registry_requirements() {
    assert_caught(
        "native/Package.swift",
        "// swift-tools-version:6.0\nlet x = 1\n",
        "AppleSession dependencies must derive from source target blocks",
    );
    assert_caught(
        "native/Sources/AppleSession/HelperCommand.swift",
        "let x = 1\n",
        "source registration belongs in SessionSources",
    );
}

#[test]
fn bootstrap_registration_injection() {
    for relative in [
        "native/Sources/AppleSession/HelperCommand.swift",
        "native/Sources/AppleSession/SessionSources.swift",
    ] {
        let body = format!(
            "{}try? await peer.register(NewSource())\n",
            if relative.ends_with("HelperCommand.swift") {
                HELPER
            } else {
                SOURCES
            }
        );
        assert_caught(
            relative,
            &body,
            "source registration must live in native/Registrations/<target>.swift",
        );
    }
}

#[test]
fn skipped_directories_are_ignored() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    baseline(dir.path());
    for skipped in [
        "target",
        ".build",
        "node_modules",
        "fixtures",
        "testdata",
        "evidence",
        "bin",
        ".git",
        ".worktrees",
    ] {
        write(dir.path(), &format!("rust/{skipped}/a.rs"), "// x\n");
    }
    assert_eq!(lint_tree(dir.path()), Vec::new());
}

fn caught_boundary(relative: &str, body: &str, concept: &str) {
    let findings = lint_with(relative, body);
    assert!(
        findings.iter().any(|f| f.path == relative
            && f.message.starts_with("boundary: ")
            && f.message.contains(concept)),
        "{relative} {body:?} not caught: {findings:?}"
    );
}

#[test]
fn boundary_catches_sqlite_in_swift() {
    caught_boundary(
        "native/Sources/NewSource/Reader.swift",
        "import SQLite3\n",
        "sqlite concept",
    );
    caught_boundary(
        "native/Sources/NewSource/Reader.swift",
        "let rc = sqlite3_open(path, &db)\n",
        "sqlite concept",
    );
}

#[test]
fn boundary_catches_gateway_concepts_in_swift() {
    for (body, token) in [
        ("let key = \"capture_checkpoint\"\n", "capture_checkpoint"),
        ("var credit = 0\n", "credit"),
        ("case \"Committed\":\n", "Committed"),
        ("case \"ScanCommitted\":\n", "ScanCommitted"),
        ("var committed = [String]()\n", "committed"),
        ("let observedBaselines = [:]\n", "Baseline"),
    ] {
        caught_boundary("native/Sources/NewSource/Sink.swift", body, token);
    }
}

#[test]
fn boundary_ignores_unrelated_words() {
    let findings = lint_with(
        "native/Sources/NewSource/Plain.swift",
        "let creditCard = 1\nlet uncommittedChange = 2\n",
    );
    assert_eq!(findings, Vec::new());
}

#[test]
fn boundary_ignores_check_targets_and_tests() {
    for relative in [
        "native/Sources/NewChecks/main.swift",
        "native/Sources/NewOracle/main.swift",
        "native/Tests/NewChecks/Main.swift",
    ] {
        let findings = lint_with(relative, "import SQLite3\nvar credit = 0\n");
        assert_eq!(findings, Vec::new(), "{relative}");
    }
}

#[test]
fn boundary_allowlist_admits_exactly_the_listed_rule() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    baseline(dir.path());
    write(
        dir.path(),
        "native/boundary-allowlist.txt",
        "sqlite native/Sources/Old/Reader.swift moves-to-rust\n",
    );
    write(
        dir.path(),
        "native/Sources/Old/Reader.swift",
        "import SQLite3\nvar credit = 0\n",
    );
    let findings = lint_tree(dir.path());
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].message.contains("gateway concept"));
}

#[test]
fn boundary_stale_allowlist_entry_fails() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    baseline(dir.path());
    write(
        dir.path(),
        "native/boundary-allowlist.txt",
        "gateway native/Sources/Gone/Sink.swift moves-to-rust\nsqlite native/Sources/App/Ok.swift moves-to-rust\n",
    );
    let findings = lint_tree(dir.path());
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(
        findings
            .iter()
            .all(|f| f.path == "native/boundary-allowlist.txt"
                && f.message.starts_with("stale allowlist entry"))
    );
}

#[test]
fn boundary_rejects_malformed_allowlist() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    baseline(dir.path());
    write(
        dir.path(),
        "native/boundary-allowlist.txt",
        "gateway native/Sources/App/Ok.swift\nwhatever native/Sources/App/Ok.swift reason\n",
    );
    let findings = lint_tree(dir.path());
    assert_eq!(findings.len(), 2, "{findings:?}");
}

#[test]
fn boundary_real_allowlist_is_exactly_todays_violations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let findings: Vec<_> = lint_tree(&root)
        .into_iter()
        .filter(|f| f.message.starts_with("boundary:") || f.message.starts_with("stale allowlist"))
        .collect();
    assert_eq!(findings, Vec::new());
}
