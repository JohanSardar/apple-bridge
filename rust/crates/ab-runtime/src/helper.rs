use crate::{Config, Error};
use std::{ffi::OsString, os::unix::fs::DirBuilderExt, path::Path, process::Stdio};
use tokio::sync::watch;

pub fn arguments(config: &Config, secret: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-n", "-W", "-g", "-a"]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.extend([
        config.helper_app.clone().into(),
        "--args".into(),
        "session".into(),
        "--socket".into(),
        config.socket.clone().into(),
        "--secret-file".into(),
        secret.as_os_str().to_owned(),
        "--home".into(),
        config.home.join("helper").into_os_string(),
        "--gateway-pid".into(),
        std::process::id().to_string().into(),
    ]);
    if config.helper_lab_commands {
        args.push("--lab-commands".into());
    }
    args
}

pub async fn supervise(
    config: &Config,
    secret: &Path,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    if !cfg!(target_os = "macos") || config.helper_app.is_empty() {
        return Err(Error::Invalid);
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(config.home.join("helper"))?;
    while !*stop.borrow() {
        let mut child = tokio::process::Command::new("open")
            .args(arguments(config, secret))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn();
        if let Ok(child) = child.as_mut() {
            tokio::select! {
                _ = child.wait() => {},
                _ = stop.wait_for(|value| *value) => {
                    child.kill().await?;
                    return Ok(());
                }
            }
        }
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_millis(250)) => {},
            _ = stop.wait_for(|value| *value) => return Ok(()),
        }
    }
    Ok(())
}
