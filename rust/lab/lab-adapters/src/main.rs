use lab_runner::Scenario;
use lab_world::{RealConfig, World, WorldConfig};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};
fn require_lab_gateway(
    gateway: &Path,
    output: &Path,
    real: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if real {
        return Ok(());
    }
    let home = output.join("gateway-capability-probe");
    fs::create_dir_all(&home)?;
    let status = Command::new(gateway)
        .args(["lab-operation-catalog", "--home"])
        .arg(&home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = fs::remove_dir_all(&home);
    if status.is_ok_and(|status| status.success()) {
        return Ok(());
    }
    Err(format!(
        "gateway binary {} was built without the lab harness; build it with `cargo build --manifest-path rust/Cargo.toml -p ab-runtime --features lab-queries --bin bridge`",
        gateway.display()
    )
    .into())
}
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("run") {
        return Err(
            "usage: lab-families run YAML... --gateway BIN --simapple BIN --output DIR [--level sim|real --helper-app APP]".into(),
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
    let real = level == "real";
    let gateway = gateway.ok_or("--gateway required")?.canonicalize()?;
    let simapple = if real {
        PathBuf::new()
    } else {
        simapple.ok_or("--simapple required")?.canonicalize()?
    };
    let output = output.ok_or("--output required")?;
    let phone_only = !paths.is_empty()
        && paths.iter().all(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(lab_adapters::iphone_real::handles)
        });
    if real && !phone_only {
        lab_world::require_signed_helper(helper.as_deref())?;
    }
    fs::create_dir_all(&output)?;
    require_lab_gateway(&gateway, &output, real)?;
    lab_adapters::replicas::configure(&gateway);
    lab_adapters::session_proxy::configure(&simapple);
    let registry = lab_adapters::registry();
    let mut passed = !paths.is_empty();
    for (index, path) in paths.iter().enumerate() {
        let mut scenario: Scenario = serde_saphyr::from_slice(&fs::read(path)?)?;
        lab_adapters::prepare(&mut scenario);
        if scenario.when.is_empty()
            && lab_adapters::readmodels::SCENARIOS.contains(&scenario.id.as_str())
        {
            scenario.when.push(serde_json::json!({"readmodels": {}}));
        }
        if scenario.when.is_empty()
            && let Some(step) = lab_adapters::implied_step(&scenario.id)
        {
            scenario.when.push(serde_json::json!({ step: {} }));
        }
        if real {
            if let Some(evidence) = lab_adapters::iphone_real::run_real(&scenario) {
                passed &= evidence.passed;
                println!("{}", serde_json::to_string(&evidence)?);
                continue;
            }
            let evidence = registry.run_real(
                &scenario,
                RealConfig {
                    gateway: gateway.clone(),
                    helper_app: helper.clone(),
                    directory: output.join(format!("world-{index}")),
                    scope: "location:read,maps:read,maps:write,files:read,files:write,safari:read,system:read".into(),
                },
            )?;
            passed &= evidence.passed;
            println!("{}", serde_json::to_string(&evidence)?);
            continue;
        }
        if let Some(evidence) = lab_adapters::iphone::run(&scenario) {
            passed &= evidence.passed;
            println!("{}", serde_json::to_string(&evidence)?);
            continue;
        }
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
