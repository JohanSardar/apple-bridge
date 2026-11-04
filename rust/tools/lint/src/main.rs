use std::{path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let root = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let findings = ab_lint::lint_tree(&root);
    for finding in &findings {
        println!("{finding}");
    }
    if findings.is_empty() {
        println!("lint: ok");
        ExitCode::SUCCESS
    } else {
        println!("lint: {} problem(s)", findings.len());
        ExitCode::FAILURE
    }
}
