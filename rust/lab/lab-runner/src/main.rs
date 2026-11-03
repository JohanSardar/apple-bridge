use lab_runner::{Registry, Scenario};
use lab_world::{RealConfig, World, WorldConfig};
use std::{env, fs, path::PathBuf, process::ExitCode};
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("run") {
        return Err(
            "usage: lab-runner run YAML... --gateway BIN --simapple BIN --output DIR [--level sim|real --helper-app APP]".into(),
        );
    }
    let mut paths = vec![];
    let mut gateway = None;
    let mut simapple = None;
    let mut output = None;
    let mut helper = None;
    let mut level = "sim".to_owned();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gateway" => gateway = args.next().map(PathBuf::from),
            "--simapple" => simapple = args.next().map(PathBuf::from),
            "--output" => output = args.next().map(PathBuf::from),
            "--helper-app" => helper = args.next().map(PathBuf::from),
            "--level" => level = args.next().ok_or("--level value required")?,
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}").into()),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if level != "sim" && level != "real" {
        return Err("--level must be sim or real".into());
    }
    let gateway = gateway.ok_or("--gateway required")?.canonicalize()?;
    let output = output.ok_or("--output required")?;
    let real = level == "real";
    let simapple = if real {
        PathBuf::new()
    } else {
        simapple.ok_or("--simapple required")?.canonicalize()?
    };
    if real {
        lab_world::require_signed_helper(helper.as_deref())?;
    }
    fs::create_dir_all(&output)?;
    let registry = Registry::standard();
    let mut passed = !paths.is_empty();
    for (index, path) in paths.iter().enumerate() {
        let scenario: Scenario = serde_saphyr::from_slice(&fs::read(path)?)?;
        let directory = output.join(format!("world-{index}"));
        let scope =
            "messages:read,reminders:read,calendar:read,contacts:read,notes:read,system:read";
        let evidence = if real {
            registry.run_real(
                &scenario,
                RealConfig {
                    gateway: gateway.clone(),
                    helper_app: helper.clone(),
                    directory,
                    scope: scope.into(),
                },
            )?
        } else {
            let mut world = World::start(WorldConfig {
                gateway: gateway.clone(),
                simapple: simapple.clone(),
                directory,
                scope: scope.into(),
            })?;
            registry.run(&scenario, &mut world)
        };
        passed &= evidence.passed;
        println!("{}", serde_json::to_string(&evidence)?);
    }
    Ok(passed)
}
fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
