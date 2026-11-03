use crate::Finding;
use std::{fs, path::Path};

#[must_use]
pub fn lint_swift(relative: &str, path: &Path) -> Vec<Finding> {
    match fs::read(path) {
        Ok(source) => scan_swift(relative, path, &source),
        Err(error) => vec![Finding::new(relative, 0, error.to_string())],
    }
}

fn tools_version_header(line: &[u8]) -> bool {
    line.strip_prefix(b"// swift-tools-version:")
        .is_some_and(|rest| !rest.is_empty() && rest.iter().all(|b| b.is_ascii_digit() || *b == b'.'))
}

fn scan_swift(relative: &str, path: &Path, src: &[u8]) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut line = 1;
    let n = src.len();
    let is_package = path.file_name().is_some_and(|name| name == "Package.swift");
    let mut i = 0;
    while i < n {
        let c = src[i];
        if c == b'\n' {
            line += 1;
        } else if c == b'"' {
            (i, line) = skip_string(src, i, line);
        } else if c == b'/' && i + 1 < n && src[i + 1] == b'/' {
            let mut end = i;
            while end < n && src[end] != b'\n' {
                end += 1;
            }
            if !(i == 0 && line == 1 && is_package && tools_version_header(&src[i..end])) {
                out.push(Finding::new(relative, line, "comment"));
            }
            while i < n && src[i] != b'\n' {
                i += 1;
            }
            line += 1;
        } else if c == b'/' && i + 1 < n && src[i + 1] == b'*' {
            out.push(Finding::new(relative, line, "comment"));
            i += 2;
            while i + 1 < n && !(src[i] == b'*' && src[i + 1] == b'/') {
                if src[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 1;
        }
        i += 1;
    }
    out
}

fn skip_string(src: &[u8], start: usize, mut line: usize) -> (usize, usize) {
    let n = src.len();
    let mut i = start;
    if i + 2 < n && src[i + 1] == b'"' && src[i + 2] == b'"' {
        i += 3;
        while i + 2 < n && !(src[i] == b'"' && src[i + 1] == b'"' && src[i + 2] == b'"') {
            if src[i] == b'\n' {
                line += 1;
            }
            i += 1;
        }
        return (i + 2, line);
    }
    i += 1;
    while i < n && src[i] != b'"' {
        if src[i] == b'\\' {
            i += 1;
        }
        if i < n && src[i] == b'\n' {
            line += 1;
        }
        i += 1;
    }
    (i, line)
}
