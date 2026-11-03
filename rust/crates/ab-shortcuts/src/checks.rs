#![forbid(unsafe_code)]
use ab_shortcuts::fixtures::{
    self, Report, capture_checks, clock_pack_checks, clock_session_checks, echo_check,
    focus_compiled_checks, focus_pack_checks, runner_checks,
};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(&args) {
        Ok(report) => {
            println!("{}", report.to_json());
            0
        }
        Err(message) => {
            eprintln!("{message}");
            2
        }
    };
    std::process::exit(code);
}

fn run(args: &[String]) -> Result<Report, String> {
    let usage = "usage: ab-shortcuts-checks prepare DIR | run ID DIR REPO_ROOT";
    match args.first().map(String::as_str) {
        Some("prepare") if args.len() == 2 => {
            fixtures::prepare(&PathBuf::from(&args[1])).map_err(|e| e.to_string())?;
            Ok(Report::default())
        }
        Some("slow") if args.len() == 2 => {
            let mut report = Report::default();
            report.add(
                "long_runner_output",
                fixtures::slow_run(&PathBuf::from(&args[1]).join("shortcuts")),
            );
            Ok(report)
        }
        Some("run") if args.len() == 4 => {
            let directory = PathBuf::from(&args[2]);
            let root = PathBuf::from(&args[3]);
            let executable = directory.join("shortcuts");
            let mut report = Report::default();
            match args[1].as_str() {
                "SC-RUN-REAL" => echo_check(&mut report, &executable),
                "SC-CAPTURE" => capture_checks(&mut report, &directory, &executable),
                "SC-RUN" => runner_checks(&mut report, &directory, &executable),
                "CLK-SIM" => clock_pack_checks(&mut report, &root, &directory),
                "CLK-ALARM-REAL" => clock_session_checks(&mut report, false),
                "CLK-TIMER-REAL" => clock_session_checks(&mut report, true),
                "FOC-SIM" | "FOC-PHONE-SYNC" => {
                    focus_pack_checks(&mut report);
                    focus_compiled_checks(&mut report, &root, &directory);
                }
                other => return Err(format!("unknown scenario {other}")),
            }
            Ok(report)
        }
        _ => Err(usage.into()),
    }
}
