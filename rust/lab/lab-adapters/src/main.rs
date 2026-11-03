use lab_runner::Scenario;
use lab_world::{World, WorldConfig};
use std::{env, fs, path::PathBuf, process::ExitCode};
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("run") {
        return Err(
            "usage: lab-families run YAML... --gateway BIN --simapple BIN --output DIR".into(),
        );
    }
    let mut paths = vec![];
    let mut gateway = None;
    let mut simapple = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gateway" => gateway = args.next().map(PathBuf::from),
            "--simapple" => simapple = args.next().map(PathBuf::from),
            "--output" => output = args.next().map(PathBuf::from),
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}").into()),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    let gateway = gateway.ok_or("--gateway required")?.canonicalize()?;
    let simapple = simapple.ok_or("--simapple required")?.canonicalize()?;
    let output = output.ok_or("--output required")?;
    fs::create_dir_all(&output)?;
    lab_adapters::replicas::configure(&gateway);
    let registry = lab_adapters::registry();
    let mut passed = !paths.is_empty();
    for (index, path) in paths.iter().enumerate() {
        let scenario: Scenario = serde_saphyr::from_slice(&fs::read(path)?)?;
        let mut world = World::start(WorldConfig {
            gateway: gateway.clone(),
            simapple: simapple.clone(),
            directory: output.join(format!("world-{index}")),
            scope:
                "messages:read,reminders:read,calendar:read,contacts:read,notes:read,system:read"
                    .into(),
        })?;
        let evidence = registry.run(&scenario, &mut world);
        lab_adapters::replicas::stop_all();
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
