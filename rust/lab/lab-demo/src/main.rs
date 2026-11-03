use std::{env, io::Write, process::ExitCode};

fn main() -> ExitCode {
    let mut seed = 1_i64;
    let mut huge = false;
    let mut digest = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--huge" => huge = true,
            "--digest" => digest = true,
            _ => {
                eprintln!("usage: lab-demo [--seed N] [--huge] [--digest]");
                return ExitCode::FAILURE;
            }
        }
    }
    let dataset = lab_demo::generate(lab_demo::Options { seed, huge });
    let out = if digest {
        format!("{}\n", dataset.digest())
    } else {
        let mut value = dataset.control_json();
        value["counts"] = serde_json::json!(dataset.counts);
        value["seed"] = serde_json::json!(dataset.seed);
        let blobs: serde_json::Map<String, serde_json::Value> = dataset
            .blobs
            .iter()
            .map(|(digest, data)| (digest.clone(), serde_json::json!(data.len())))
            .collect();
        value["blobs"] = serde_json::Value::Object(blobs);
        format!("{value}\n")
    };
    if std::io::stdout().write_all(out.as_bytes()).is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
