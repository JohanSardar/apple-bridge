use crate::Finding;
use regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const ALLOWLIST: &str = "native/boundary-allowlist.txt";

const SQLITE: &str = r"import SQLite3|sqlite3_|\bSQLiteSnapshot\b";
const GATEWAY: &str =
    r"capture_checkpoint|byte_credit|ScanCommitted|\bcredit\b|\b[Cc]ommitted\b|(?i:baseline)";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Rule {
    Sqlite,
    Gateway,
}

impl Rule {
    const ALL: [Rule; 2] = [Rule::Sqlite, Rule::Gateway];

    fn name(self) -> &'static str {
        match self {
            Rule::Sqlite => "sqlite",
            Rule::Gateway => "gateway",
        }
    }

    fn parse(name: &str) -> Option<Rule> {
        Rule::ALL.into_iter().find(|rule| rule.name() == name)
    }
}

fn checks_target(name: &str) -> bool {
    name.ends_with("Checks") || name.ends_with("Oracle")
}

fn shipped(relative: &str) -> bool {
    let parts: Vec<&str> = relative.split('/').collect();
    match parts.as_slice() {
        ["native", "Sources", target, _, ..] => !checks_target(target),
        ["packaging", .., file] => !file.contains("Check") && !file.contains("Fixture"),
        _ => false,
    }
}

fn first_match(pattern: &Regex, text: &str) -> Option<(usize, String)> {
    pattern.find(text).map(|found| {
        let line = text[..found.start()]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
            + 1;
        (line, found.as_str().to_owned())
    })
}

struct Entry {
    rule: Rule,
    path: String,
    line: usize,
}

fn parse_allowlist(text: &str, findings: &mut Vec<Finding>) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let mut fields = raw.split_whitespace();
        let (Some(rule), Some(path), Some(_reason)) = (fields.next(), fields.next(), fields.next())
        else {
            if !raw.trim().is_empty() {
                findings.push(Finding::new(
                    ALLOWLIST,
                    line,
                    "allowlist line needs: rule path reason",
                ));
            }
            continue;
        };
        match Rule::parse(rule) {
            Some(rule) => entries.push(Entry {
                rule,
                path: path.to_owned(),
                line,
            }),
            None => findings.push(Finding::new(
                ALLOWLIST,
                line,
                format!("unknown boundary rule: {rule}"),
            )),
        }
    }
    entries
}

#[must_use]
pub fn lint_boundary(root: &Path, swift: &[(String, PathBuf)]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let allowed = match fs::read_to_string(root.join(ALLOWLIST)) {
        Ok(text) => parse_allowlist(&text, &mut findings),
        Err(_) => Vec::new(),
    };
    let patterns: BTreeMap<Rule, Regex> = [(Rule::Sqlite, SQLITE), (Rule::Gateway, GATEWAY)]
        .into_iter()
        .filter_map(|(rule, pattern)| Regex::new(pattern).ok().map(|regex| (rule, regex)))
        .collect();
    let mut hits: BTreeSet<(Rule, String)> = BTreeSet::new();
    for (relative, path) in swift {
        if !shipped(relative) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for (rule, pattern) in &patterns {
            let Some((line, token)) = first_match(pattern, &text) else {
                continue;
            };
            hits.insert((*rule, relative.clone()));
            if !allowed
                .iter()
                .any(|entry| entry.rule == *rule && entry.path == *relative)
            {
                findings.push(Finding::new(
                    relative,
                    line,
                    format!(
                        "boundary: {} concept `{token}` outside the allowlist",
                        rule.name()
                    ),
                ));
            }
        }
    }
    for entry in &allowed {
        if !hits.contains(&(entry.rule, entry.path.clone())) {
            findings.push(Finding::new(
                ALLOWLIST,
                entry.line,
                format!(
                    "stale allowlist entry: {} no longer uses {} concepts, remove it",
                    entry.path,
                    entry.rule.name()
                ),
            ));
        }
    }
    findings
}
