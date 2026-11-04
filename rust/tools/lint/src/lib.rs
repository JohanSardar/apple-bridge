mod boundary;
mod registries;
mod rustsrc;
mod swift;

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

pub use boundary::{ALLOWLIST, lint_boundary};
pub use registries::{lint_bootstrap_registrations, lint_source_registries};
pub use rustsrc::{lint_cargo, lint_rust};
pub use swift::lint_swift;

const SKIP_DIRS: [&str; 9] = [
    ".git",
    "target",
    ".worktrees",
    ".build",
    "bin",
    "node_modules",
    "evidence",
    "fixtures",
    "testdata",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub message: String,
}

impl Finding {
    pub fn new(path: &str, line: usize, message: impl Into<String>) -> Self {
        Self {
            path: path.to_owned(),
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.path, self.line, self.message)
    }
}

#[must_use]
pub fn lint_tree(root: &Path) -> Vec<Finding> {
    let mut findings = lint_source_registries(root);
    let mut files = Vec::new();
    walk(root, root, &mut files);
    let swift: Vec<(String, PathBuf)> = files
        .iter()
        .filter(|(_, path)| path.extension().is_some_and(|ext| ext == "swift"))
        .cloned()
        .collect();
    findings.extend(lint_boundary(root, &swift));
    for (relative, path) in files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == "Cargo.toml" {
            findings.extend(lint_cargo(&relative, &path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            findings.extend(lint_rust(&relative, &path));
        } else if path.extension().is_some_and(|ext| ext == "swift") {
            findings.extend(lint_swift(&relative, &path));
        }
    }
    findings
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let name = entry.file_name();
            if SKIP_DIRS.iter().any(|skip| name == *skip) {
                continue;
            }
            walk(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            files.push((relative, path));
        }
    }
}
