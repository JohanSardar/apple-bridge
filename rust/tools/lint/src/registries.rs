use crate::Finding;
use regex::Regex;
use std::{fs, path::Path};

const REQUIRED: [(&str, &str, &str); 2] = [
    (
        "native/Package.swift",
        "sourceTargets.map",
        "AppleSession dependencies must derive from source target blocks",
    ),
    (
        "native/Sources/AppleSession/HelperCommand.swift",
        "SessionSources.register(",
        "source registration belongs in SessionSources",
    ),
];

const BOOTSTRAP_FILES: [&str; 2] = [
    "native/Sources/AppleSession/HelperCommand.swift",
    "native/Sources/AppleSession/SessionSources.swift",
];

#[must_use]
pub fn lint_source_registries(root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    for (path, required, message) in REQUIRED {
        match fs::read(root.join(path)) {
            Ok(data) => {
                if !String::from_utf8_lossy(&data).contains(required) {
                    out.push(Finding::new(path, 1, message));
                }
            }
            Err(error) => out.push(Finding::new(path, 0, error.to_string())),
        }
    }
    for relative in BOOTSTRAP_FILES {
        if let Ok(body) = fs::read(root.join(relative)) {
            out.extend(lint_bootstrap_registrations(relative, &body));
        }
    }
    out
}

#[must_use]
pub fn lint_bootstrap_registrations(relative: &str, source: &[u8]) -> Vec<Finding> {
    if !Path::new(relative)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("swift"))
    {
        return Vec::new();
    }
    let (Ok(allowed), Ok(any)) = (
        Regex::new(r"SessionSources\s*\.\s*register\s*\("),
        Regex::new(r"\.\s*register\s*\("),
    ) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(source);
    let body = allowed.replace_all(&text, "");
    if any.is_match(&body) {
        return vec![Finding::new(
            relative,
            1,
            "source registration must live in native/Registrations/<target>.swift",
        )];
    }
    Vec::new()
}
