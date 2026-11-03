use std::{fs, io::Write, path::PathBuf, process::ExitCode};

use ab_featuregraph::{Checker, render, summarize};

fn main() -> ExitCode {
    let mut root = PathBuf::from(".");
    let mut html: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let flag = arg.trim_start_matches('-');
        let (name, inline) = match flag.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (flag, None),
        };
        let target = match name {
            "root" => &mut root,
            "html" => html.get_or_insert_with(PathBuf::new),
            _ => {
                let _ = writeln!(std::io::stderr(), "flag provided but not defined: {arg}");
                return ExitCode::from(2);
            }
        };
        match inline.or_else(|| args.next()) {
            Some(value) => *target = PathBuf::from(value),
            None => {
                let _ = writeln!(std::io::stderr(), "flag needs an argument: {arg}");
                return ExitCode::from(2);
            }
        }
    }
    let mut checker = match Checker::load(&root) {
        Ok(checker) => checker,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "{error}");
            return ExitCode::from(2);
        }
    };
    let views = checker.check();
    if let Some(path) = html.filter(|p| !p.as_os_str().is_empty()) {
        if let Err(error) = fs::write(&path, render(&checker.graph, &views)) {
            let _ = writeln!(std::io::stderr(), "{error}");
            return ExitCode::from(2);
        }
    }
    println!("{}", summarize(&views));
    if checker.problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        for problem in &checker.problems {
            let _ = writeln!(std::io::stderr(), "problem: {problem}");
        }
        ExitCode::FAILURE
    }
}
