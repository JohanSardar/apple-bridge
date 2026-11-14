use crate::{assertions::await_until, http::Client};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn native_directory() -> Option<PathBuf> {
    env::var("AB_LAB_NATIVE_OUTPUT")
        .ok()
        .or_else(|| {
            fs::read_to_string("native/.build/native-output-1")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .map(PathBuf::from)
}

fn newest(path: &Path) -> Option<SystemTime> {
    let meta = fs::metadata(path).ok()?;
    let mut latest = meta.modified().ok()?;
    if meta.is_dir() {
        for entry in fs::read_dir(path).ok()? {
            latest = latest.max(newest(&entry.ok()?.path())?);
        }
    }
    Some(latest)
}

fn fresh(binary: &Path, inputs: &[&str]) -> bool {
    let Some(built) = fs::metadata(binary).ok().and_then(|m| m.modified().ok()) else {
        return false;
    };
    inputs
        .iter()
        .all(|input| newest(Path::new(input)).is_some_and(|t| t <= built))
}

fn native_binary(
    evidence: &mut Evidence,
    target: &str,
    inputs: &[&str],
    always_output: bool,
) -> Option<PathBuf> {
    let binary = native_directory().map(|d| d.join(target));
    if binary.as_ref().is_some_and(|b| fresh(b, inputs)) {
        if always_output {
            evidence.check("native_output", true, "native output directory");
        }
        return binary;
    }
    let built = Command::new("packaging/dev/build-native.sh")
        .env("APPLE_BRIDGE_NATIVE_TARGET", target)
        .output();
    let ok = built.as_ref().is_ok_and(|o| o.status.success());
    evidence.check("native_build", ok, "native checks build");
    if !ok {
        return None;
    }
    let directory = native_directory();
    evidence.check(
        "native_output",
        directory.is_some(),
        "native output directory",
    );
    directory.map(|d| d.join(target))
}

fn run_native(binary: &Path, args: &[&str]) -> (bool, String) {
    match Command::new(binary).args(args).output() {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ),
        Err(error) => (false, error.to_string()),
    }
}

fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn put(world: &World, kind: &str, version: u64) -> Result<()> {
    let collection = "files.item";
    let fields = json!({"root":"/Bridge Test","path":"fixture.txt","name":"fixture.txt","kind":"file","size":0,"created":null,"modified":null,"tags":[],"downloaded":true,"icloud_status":"local","fixture_version":version});
    let at = now();
    let mut capture = json!({"source":"files","generation":"native-media-fixture","observations":[{"kind":kind,"collection":collection,"native_key":"native-media-fixture","data":fields,"observed_at":at,"revalidated":true}]});
    if kind == "absent" {
        capture["scan_id"] = json!("native-media-fixture");
        capture["scan_started_at"] = json!(at);
        capture["scope"] = json!([collection]);
        capture["completion_evidence"] = json!("complete");
    }
    world.control("/capture", &capture)?;
    Ok(())
}

fn records(client: &Client) -> Option<Vec<Value>> {
    let out = client
        .call(
            "records.query",
            &json!({"collection":"files.item","limit":10}),
            10,
        )
        .ok()??;
    out["records"].as_array().cloned()
}

fn gateway_flow(world: &World, evidence: &mut Evidence) -> Result<()> {
    let client = Client::new(world, "admin")?;
    evidence.check("selected_gateway_started", true, "gateway ready");
    let mut id: Option<String> = None;
    for version in 1..=2u64 {
        let sent = put(world, "upsert", version);
        evidence.check(
            &format!("selected_gateway_capture_{version}"),
            sent.is_ok(),
            &sent
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        if sent.is_err() {
            return Ok(());
        }
        let seen = await_until(5, || {
            let Some(rows) = records(&client).filter(|r| r.len() == 1) else {
                return false;
            };
            let row = &rows[0];
            if row["data"]["fixture_version"].as_u64() != Some(version)
                || row["version"].as_u64() != Some(version)
            {
                return false;
            }
            let current = row["id"].as_str().map(str::to_owned);
            if id.is_some() && id != current {
                return false;
            }
            id = current;
            true
        });
        evidence.check(
            &format!("selected_gateway_readback_{version}"),
            seen,
            "record visible at the expected version",
        );
        if !seen {
            return Ok(());
        }
    }
    let sent = put(world, "absent", 3);
    evidence.check("selected_gateway_absent_capture", sent.is_ok(), "");
    if sent.is_err() {
        return Ok(());
    }
    evidence.check(
        "selected_gateway_absent_readback",
        await_until(5, || records(&client).is_some_and(|r| r.is_empty())),
        "record removed",
    );
    Ok(())
}

pub fn files_transform(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let Some(binary) = native_binary(
        evidence,
        "FilesChecks",
        &[
            "native/Package.swift",
            "native/Sources/FilesSource",
            "native/Sources/FilesChecks",
            "native/Sources/SourceSupport",
        ],
        false,
    ) else {
        return Ok(());
    };
    let (ok, _) = run_native(&binary, &["FILES-TRANSFORM"]);
    evidence.check("native_files_checks", ok, "native Files checks");
    if evidence.passed {
        gateway_flow(world, evidence)?;
    }
    Ok(())
}
