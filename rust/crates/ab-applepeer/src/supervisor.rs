use ab_operations::{Code, Error};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};
use tokio::sync::watch;
#[derive(Debug, Clone)]
pub struct HelperConfig {
    pub app: PathBuf,
    pub socket: PathBuf,
    pub secret_file: PathBuf,
    pub home: PathBuf,
    pub lab_commands: bool,
}
pub async fn supervise(config: HelperConfig, mut stop: watch::Receiver<bool>) -> Result<(), Error> {
    if !cfg!(target_os = "macos") {
        return Err(Code::Unsupported.into());
    }
    let home = config.home.join("helper");
    std::fs::create_dir_all(&home).map_err(|_| Code::Unavailable)?;
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| Code::Unavailable)?;
    while !*stop.borrow() {
        let mut command = tokio::process::Command::new("open");
        command
            .args(["-n", "-W", "-g", "-a"])
            .arg(&config.app)
            .args(["--args", "session", "--socket"])
            .arg(&config.socket)
            .arg("--secret-file")
            .arg(&config.secret_file)
            .arg("--home")
            .arg(&home)
            .arg("--gateway-pid")
            .arg(std::process::id().to_string())
            .kill_on_drop(true);
        if config.lab_commands {
            command.arg("--lab-commands");
        }
        if let Ok(mut child) = command.spawn() {
            tokio::select! { _ = stop.changed() => { let _ = child.kill().await; return Ok(()); }, _ = child.wait() => {} }
        }
        tokio::select! { _ = stop.changed() => return Ok(()), () = tokio::time::sleep(std::time::Duration::from_millis(250)) => {} }
    }
    Ok(())
}
