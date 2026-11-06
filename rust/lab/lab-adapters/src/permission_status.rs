use crate::http::random_id;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn home() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
}
fn fingerprint() -> Option<String> {
    let mut values = vec![];
    for file in [
        home().join("Library/Application Support/com.apple.TCC/TCC.db"),
        PathBuf::from("/Library/Application Support/com.apple.TCC/TCC.db"),
    ] {
        for suffix in ["", "-wal"] {
            let mut path = file.clone().into_os_string();
            path.push(suffix);
            match fs::read(Path::new(&path)) {
                Ok(data) => values.push(hex::encode(Sha256::digest(&data))),
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && !suffix.is_empty() => {}
                Err(_) => return None,
            }
        }
    }
    Some(values.join(":"))
}
fn helper_app(scratch: &Path) -> Result<PathBuf> {
    if let Some(app) = env::var_os("AB_PERMISSION_HELPER_APP") {
        return Ok(PathBuf::from(app));
    }
    let app = scratch.join("AppleBridgeHelper.app");
    let status = Command::new("packaging/dev/build-helper.sh")
        .arg(&app)
        .status()?;
    if status.success() {
        Ok(app)
    } else {
        Err(Error::Invalid("signed helper build failed".into()))
    }
}
fn report(app: &Path, command: &str, name: &str) -> Result<BTreeMap<String, String>> {
    let directory = home().join("Library/Application Support/AppleBridgeDev");
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("permissions-{}.json", random_id("run")?));
    let status = Command::new("open")
        .args(["-n", "-W", "-g", "-a"])
        .arg(app)
        .args(["--args", command, name, "--report"])
        .arg(&path)
        .status()?;
    let data = fs::read(&path);
    let _ = fs::remove_file(&path);
    if !status.success() {
        return Err(Error::Invalid("helper permission report failed".into()));
    }
    Ok(serde_json::from_slice(&data?)?)
}

pub fn real(evidence: &mut Evidence) -> Result<()> {
    let scratch = env::temp_dir().join(random_id("permission-helper-")?);
    fs::create_dir_all(&scratch)?;
    let app = helper_app(&scratch);
    let Ok(app) = app else {
        evidence.check("signed_helper_build", false, "build-helper.sh failed");
        let _ = fs::remove_dir_all(&scratch);
        return Ok(());
    };
    let before = fingerprint();
    let states = report(&app, "permission-status-proof", "");
    let _ = fs::remove_dir_all(&scratch);
    let mut states = states.unwrap_or_default();
    let calls = states
        .remove("_request_api_calls")
        .and_then(|v| v.parse::<u64>().ok());
    evidence.check(
        "zero_native_request_api_calls",
        calls == Some(0),
        "no native permission request API was called",
    );
    let mut counts: BTreeMap<&str, Option<BTreeMap<String, u64>>> = BTreeMap::new();
    for phase in ["before", "after", "peak"] {
        let values = states
            .remove(&format!("_windows_{phase}"))
            .and_then(|v| serde_json::from_str::<BTreeMap<String, u64>>(&v).ok());
        evidence.check(
            &format!("window_observation_{phase}"),
            values.is_some(),
            "window owner counts decoded",
        );
        counts.insert(phase, values);
    }
    let no_creation = counts["peak"].as_ref().is_some_and(|peak| {
        let before = counts["before"].clone().unwrap_or_default();
        peak.iter()
            .all(|(owner, count)| *count <= before.get(owner).copied().unwrap_or(0))
    });
    evidence.check(
        "no_new_dialog_windows",
        no_creation,
        "the report opened no permission dialog",
    );
    evidence.check(
        "helper_report",
        states.len() == 11,
        &format!("{} permission states", states.len()),
    );
    for (key, state) in &states {
        evidence.check(
            key,
            matches!(
                state.as_str(),
                "not_determined" | "denied" | "authorized" | "unknown"
            ),
            state,
        );
    }
    let after = fingerprint();
    evidence.check(
        "tcc_database_readable",
        before.is_some() && after.is_some(),
        "TCC databases readable",
    );
    evidence.check(
        "tcc_database_unchanged",
        before.is_some() && before == after,
        "TCC databases unchanged by the report",
    );
    Ok(())
}

pub fn sim(_: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let scratch = env::temp_dir().join(random_id("permission-helper-")?);
    fs::create_dir_all(&scratch)?;
    let Ok(app) = helper_app(&scratch) else {
        evidence.check("signed_helper_build", false, "build-helper.sh failed");
        let _ = fs::remove_dir_all(&scratch);
        return Ok(());
    };
    let states = report(&app, "permission-fake", "fake");
    let _ = fs::remove_dir_all(&scratch);
    let states = states.unwrap_or_default();
    evidence.check(
        "helper_report",
        states.len() == 15,
        &format!("{} permission states", states.len()),
    );
    for (key, state) in &states {
        evidence.check(key, state == "1", state);
    }
    Ok(())
}
