use crate::Finding;
use regex::bytes::Regex;
use std::{fs, path::Path, sync::LazyLock};

static DOC_ATTRIBUTE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?-u)^#[\t\n\f\r ]*!?[\t\n\f\r ]*\[[\t\n\f\r ]*doc[\t\n\f\r ]*(?:=|\()").ok()
});

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[must_use]
pub fn lint_rust(relative: &str, path: &Path) -> Vec<Finding> {
    match fs::read(path) {
        Ok(source) => scan_rust(relative, &source),
        Err(error) => vec![Finding::new(relative, 0, error.to_string())],
    }
}

enum Raw {
    Plain,
    Unterminated,
    Ends(usize),
}

fn raw_string_end(s: &[u8], at: usize) -> Raw {
    if s.get(at) != Some(&b'r') {
        return Raw::Plain;
    }
    let mut j = at + 1;
    while s.get(j) == Some(&b'#') {
        j += 1;
    }
    if s.get(j) != Some(&b'"') {
        return Raw::Plain;
    }
    let mut end = vec![b'"'];
    end.extend(std::iter::repeat_n(b'#', j - at - 1));
    find(&s[j + 1..], &end).map_or(Raw::Unterminated, |k| Raw::Ends(j + 1 + k + end.len()))
}

fn scan_rust(relative: &str, s: &[u8]) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut line = 1;
    let n = s.len();
    let mut i = 0;
    while i < n {
        if s[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if matches!(s[i], b'r' | b'b' | b'c') {
            let start = i;
            if s[i] != b'r' {
                i += 1;
            }
            match raw_string_end(s, i) {
                Raw::Ends(next) => {
                    line += s[start..next]
                        .iter()
                        .fold(0, |n, b| n + usize::from(*b == b'\n'));
                    i = next;
                    continue;
                }
                Raw::Unterminated => break,
                Raw::Plain => i = start,
            }
        }
        let quote_like = s[i] == b'"'
            || (s[i] == b'\'' && i + 2 < n && (s[i + 2] == b'\'' || s[i + 1] == b'\\'));
        if quote_like {
            let q = s[i];
            i += 1;
            while i < n {
                if s[i] == b'\n' {
                    line += 1;
                }
                if s[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if s[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if s[i..].starts_with(b"//") {
            let end = s[i..].iter().position(|b| *b == b'\n').unwrap_or(n - i);
            let safety = relative.starts_with("rust/crates/ab-sys/")
                && s[i..i + end].starts_with(b"// SAFETY:");
            if !safety {
                out.push(Finding::new(relative, line, "comment"));
            }
            i += end;
            continue;
        }
        if s[i..].starts_with(b"/*") {
            out.push(Finding::new(relative, line, "comment"));
            i += 2;
            continue;
        }
        if s[i] == b'#'
            && DOC_ATTRIBUTE
                .as_ref()
                .is_some_and(|re| re.is_match(&s[i..]))
        {
            out.push(Finding::new(relative, line, "doc attribute"));
        }
        i += 1;
    }
    out
}

#[must_use]
pub fn lint_cargo(relative: &str, path: &Path) -> Vec<Finding> {
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) => return vec![Finding::new(relative, 0, error.to_string())],
    };
    let mut out = Vec::new();
    for feature in ["arbitrary_precision", "preserve_order"] {
        let double = format!("\"{feature}\"");
        let single = format!("'{feature}'");
        if find(&source, double.as_bytes()).is_some() || find(&source, single.as_bytes()).is_some()
        {
            out.push(Finding::new(
                relative,
                0,
                format!("banned JSON feature: {feature}"),
            ));
        }
    }
    out
}
