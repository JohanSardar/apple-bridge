mod proxy;
pub use proxy::FaultProxy;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("lab: {0}")]
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone)]
pub struct WorldConfig {
    pub gateway: PathBuf,
    pub simapple: PathBuf,
    pub directory: PathBuf,
    pub scope: String,
}
#[derive(Debug, Clone)]
pub struct RealConfig {
    pub gateway: PathBuf,
    pub helper_app: Option<PathBuf>,
    pub directory: PathBuf,
    pub scope: String,
}
#[must_use]
pub fn live_homes(home: &str, data_home: Option<&str>) -> Vec<PathBuf> {
    let mut homes = vec![Path::new(home).join("Library/Application Support/AppleBridge")];
    let data = data_home
        .filter(|root| !root.is_empty())
        .map_or_else(|| Path::new(home).join(".local/share"), PathBuf::from);
    homes.push(data.join("apple-bridge"));
    homes
}
fn absolute(path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = joined
        .parent()
        .ok_or_else(|| Error::Invalid("world directory has no parent".into()))?;
    let name = joined
        .file_name()
        .ok_or_else(|| Error::Invalid("world directory has no name".into()))?;
    Ok(parent.canonicalize()?.join(name))
}
pub fn refuse_live_home(directory: &Path, homes: &[PathBuf]) -> Result<()> {
    let directory = absolute(directory)?;
    for home in homes {
        let resolved = home.canonicalize().unwrap_or_else(|_| home.clone());
        if [home, &resolved]
            .iter()
            .any(|root| directory.starts_with(root) || root.starts_with(&directory))
        {
            return Err(Error::Invalid(
                "real level refuses to run in or above the live home".into(),
            ));
        }
    }
    Ok(())
}
pub fn require_signed_helper(app: Option<&Path>) -> Result<&Path> {
    let app = app.ok_or_else(|| {
        Error::Invalid("real level refused: no signed Helper configured (--helper-app)".into())
    })?;
    if !app.join("Contents/MacOS/AppleBridgeHelper").is_file() {
        return Err(Error::Invalid(
            "real level refused: Helper app has no executable".into(),
        ));
    }
    let verified = Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(app)
        .output()
        .is_ok_and(|o| o.status.success());
    let described = Command::new("codesign")
        .args(["-dv", "--verbose=2"])
        .arg(app)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .unwrap_or_default();
    let team = described
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .is_some_and(|team| !team.is_empty() && team != "not set");
    if !verified || !team || described.contains("Signature=adhoc") {
        return Err(Error::Invalid(
            "real level refused: Helper app is not signed with a team identity".into(),
        ));
    }
    Ok(app)
}
#[derive(Debug)]
pub struct World {
    pub directory: PathBuf,
    pub gateway_home: PathBuf,
    pub helper_app: Option<PathBuf>,
    pub runtime: Value,
    config: WorldConfig,
    gateway: Option<Child>,
    peer: Option<Child>,
    proxy: Option<FaultProxy>,
    token: String,
    gateway_env: Vec<(String, String)>,
    gateway_args: Vec<String>,
}
fn output(mut command: Command) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(Error::Invalid(format!("command exited {}", output.status)));
    }
    String::from_utf8(output.stdout).map_err(|_| Error::Invalid("non UTF-8 output".into()))
}
impl World {
    pub fn start(config: WorldConfig) -> Result<Self> {
        fs::create_dir(&config.directory)?;
        fs::set_permissions(&config.directory, fs::Permissions::from_mode(0o700))?;
        let mut world = Self {
            directory: config.directory.clone(),
            gateway_home: config.directory.join("gateway"),
            helper_app: None,
            runtime: Value::Null,
            config,
            gateway: None,
            peer: None,
            proxy: None,
            token: String::new(),
            gateway_env: Vec::new(),
            gateway_args: Vec::new(),
        };
        world.launch_gateway()?;
        world
            .gateway_cli(&["token", "create", "--scope", &world.config.scope])?
            .trim()
            .clone_into(&mut world.token);
        world.launch_peer()?;
        Ok(world)
    }
    pub fn start_real(config: RealConfig) -> Result<Self> {
        let app = require_signed_helper(config.helper_app.as_deref())?.to_path_buf();
        let home = std::env::var("HOME").unwrap_or_default();
        let data = std::env::var("XDG_DATA_HOME").ok();
        refuse_live_home(&config.directory, &live_homes(&home, data.as_deref()))?;
        fs::create_dir(&config.directory)?;
        fs::set_permissions(&config.directory, fs::Permissions::from_mode(0o700))?;
        let mut world = Self {
            directory: config.directory.clone(),
            gateway_home: config.directory.join("gateway"),
            helper_app: Some(app.clone()),
            runtime: Value::Null,
            config: WorldConfig {
                gateway: config.gateway,
                simapple: PathBuf::new(),
                directory: config.directory,
                scope: config.scope,
            },
            gateway: None,
            peer: None,
            proxy: None,
            token: String::new(),
            gateway_env: Vec::new(),
            gateway_args: vec![
                "--helper-app".into(),
                app.to_string_lossy().into_owned(),
                "--helper-lab-commands".into(),
            ],
        };
        world.launch_gateway()?;
        world
            .gateway_cli(&["token", "create", "--scope", &world.config.scope])?
            .trim()
            .clone_into(&mut world.token);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !world.gateway_home.join("helper/helper.pid").is_file() {
            world.check_children()?;
            if Instant::now() >= deadline {
                return Err(Error::Invalid("helper readiness timeout".into()));
            }
            thread::sleep(Duration::from_millis(25));
        }
        Ok(world)
    }
    fn launch_peer(&mut self) -> Result<()> {
        let socket = self.runtime["socket"]
            .as_str()
            .ok_or_else(|| Error::Invalid("runtime socket".into()))?
            .to_owned();
        let log = fs::File::options()
            .create(true)
            .append(true)
            .open(self.directory.join("simapple.log"))?;
        self.peer = Some(
            Command::new(&self.config.simapple)
                .args(["simapple", "--socket", &socket, "--secret-file"])
                .arg(self.gateway_home.join("apple-secret"))
                .arg("--control")
                .arg(self.directory.join("sim.sock"))
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if self.control("/inspect", &Value::Null).is_ok() {
                return Ok(());
            }
            self.check_children()?;
            if Instant::now() >= deadline {
                return Err(Error::Invalid("peer readiness timeout".into()));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    pub fn sibling(&self, suffix: &str) -> Result<Self> {
        let name = self
            .directory
            .file_name()
            .ok_or_else(|| Error::Invalid("world directory has no name".into()))?
            .to_string_lossy()
            .into_owned();
        Self::start(WorldConfig {
            directory: self.directory.with_file_name(format!("{name}-{suffix}")),
            ..self.config.clone()
        })
    }
    pub fn restart_peer(&mut self) -> Result<()> {
        stop(&mut self.peer);
        let _ = fs::remove_file(self.directory.join("sim.sock"));
        self.launch_peer()
    }
    fn launch_gateway(&mut self) -> Result<()> {
        let peer = self.runtime["peer_address"]
            .as_str()
            .unwrap_or("127.0.0.1:0")
            .to_owned();
        let api = self.runtime["api_address"]
            .as_str()
            .unwrap_or("127.0.0.1:0")
            .to_owned();
        let log = fs::File::options()
            .create(true)
            .append(true)
            .open(self.directory.join("gateway.log"))?;
        self.gateway = Some(
            Command::new(&self.config.gateway)
                .args(["start", "--role", "gateway", "--home"])
                .arg(&self.gateway_home)
                .args(["--peer-addr", &peer, "--api-addr", &api])
                .args(&self.gateway_args)
                .envs(self.gateway_env.iter().map(|(k, v)| (k, v)))
                .env("APPLE_BRIDGE_SECRET_STORE", "file")
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(bytes) = fs::read(self.gateway_home.join("runtime.json"))
                && let Ok(runtime) = serde_json::from_slice(&bytes)
            {
                self.runtime = runtime;
                return Ok(());
            }
            self.check_children()?;
            if Instant::now() >= deadline {
                return Err(Error::Invalid("gateway readiness timeout".into()));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn check_children(&mut self) -> Result<()> {
        for child in [&mut self.gateway, &mut self.peer].into_iter().flatten() {
            if let Some(status) = child.try_wait()? {
                return Err(Error::Invalid(format!("child exited {status}")));
            }
        }
        Ok(())
    }
    pub fn gateway_cli(&self, args: &[&str]) -> Result<String> {
        let mut command = Command::new(&self.config.gateway);
        command
            .args(args)
            .arg("--home")
            .arg(&self.gateway_home)
            .env("APPLE_BRIDGE_SECRET_STORE", "file");
        output(command)
    }
    pub fn control(&self, route: &str, input: &Value) -> Result<Value> {
        self.control_for(route, input, 25)
    }
    pub fn control_for(&self, route: &str, input: &Value, seconds: u64) -> Result<Value> {
        let mut command = Command::new("curl");
        command
            .args(["--silent", "--show-error", "--fail", "--max-time"])
            .arg(seconds.to_string())
            .arg("--unix-socket")
            .arg(self.directory.join("sim.sock"));
        if !input.is_null() {
            command.args([
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
            ]);
        }
        command.arg(format!("http://sim{route}"));
        let fail = |e: String| Error::Invalid(format!("control {route}: {e}"));
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = command.spawn()?;
        if let Some(mut stdin) = child.stdin.take()
            && !input.is_null()
        {
            stdin.write_all(&serde_json::to_vec(input)?)?;
        }
        let result = child.wait_with_output()?;
        if !result.status.success() {
            return Err(fail(format!("command exited {}", result.status)));
        }
        Ok(serde_json::from_slice(&result.stdout)?)
    }
    pub fn query(&self, operation: &str, input: &Value) -> Result<Value> {
        self.query_with_token(operation, input, &self.token)
    }
    pub fn query_with_token(&self, operation: &str, input: &Value, token: &str) -> Result<Value> {
        let address = self.runtime["api_address"]
            .as_str()
            .ok_or_else(|| Error::Invalid("runtime API address".into()))?;
        let mut command = Command::new("curl");
        command
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "--max-time",
                "25",
                "-H",
                "Content-Type: application/json",
                "-H",
                &format!("Authorization: Bearer {token}"),
                "--data-binary",
                &serde_json::to_string(input)?,
            ])
            .arg(format!("http://{address}/v1/op/{operation}"));
        let value: Value = serde_json::from_str(&output(command)?)?;
        if !value["error"].is_null() {
            return Err(Error::Invalid("public operation rejected".into()));
        }
        Ok(value)
    }
    pub fn install_proxy(&mut self) -> Result<()> {
        let target = self.runtime["peer_address"]
            .as_str()
            .ok_or_else(|| Error::Invalid("runtime peer address".into()))?
            .to_owned();
        if let Some(old) = self.proxy.take() {
            old.close();
        }
        self.proxy = Some(FaultProxy::start(&target)?);
        Ok(())
    }
    pub fn pair_address(&self) -> Result<String> {
        if let Some(proxy) = &self.proxy {
            return Ok(proxy.address().to_owned());
        }
        self.runtime["peer_address"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Invalid("runtime peer address".into()))
    }
    pub fn partition(&self) -> Result<()> {
        self.proxy
            .as_ref()
            .map(FaultProxy::partition)
            .ok_or_else(|| Error::Invalid("no fault proxy".into()))
    }
    pub fn heal(&self) -> Result<()> {
        self.proxy
            .as_ref()
            .map(FaultProxy::heal)
            .ok_or_else(|| Error::Invalid("no fault proxy".into()))
    }
    pub fn blob_control(&self, input: &Value) -> Result<Value> {
        let mut command = Command::new("curl");
        command
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "--max-time",
                "5",
                "--unix-socket",
            ])
            .arg(self.gateway_home.join("run/blobs-lab.sock"))
            .args(["--data-binary", &serde_json::to_string(input)?])
            .arg("http://lab/");
        Ok(serde_json::from_str(&output(command).map_err(|e| {
            Error::Invalid(format!("blob control: {e}"))
        })?)?)
    }
    pub fn configure_gateway(
        &mut self,
        env: Vec<(String, String)>,
        args: Vec<String>,
    ) -> Result<()> {
        self.gateway_env = env;
        self.gateway_args = args;
        self.restart_gateway()
    }
    pub fn kill_gateway(&mut self) -> Result<()> {
        let Some(child) = self.gateway.as_mut() else {
            return Err(Error::Invalid("gateway not running".into()));
        };
        child.kill()?;
        Ok(())
    }
    pub fn stop_gateway(&mut self) {
        stop(&mut self.gateway);
        let _ = fs::remove_file(self.gateway_home.join("runtime.json"));
    }
    pub fn stop_helper(&mut self) {
        stop(&mut self.peer);
    }
    pub fn clock_offset(&self, milliseconds: i64) -> Result<()> {
        fs::write(
            self.gateway_home.join("clock-offset"),
            milliseconds.to_string(),
        )?;
        Ok(())
    }
    pub fn start_gateway(&mut self) -> Result<()> {
        self.launch_gateway()
    }
    pub fn restart_gateway(&mut self) -> Result<()> {
        self.stop_gateway();
        self.start_gateway()
    }
    pub fn close(&mut self) {
        if let Some(proxy) = self.proxy.take() {
            proxy.close();
        }
        stop(&mut self.peer);
        stop(&mut self.gateway);
    }
}
fn stop(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
impl Drop for World {
    fn drop(&mut self) {
        self.close();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_refuses_without_helper() {
        let refused = require_signed_helper(None)
            .err()
            .is_some_and(|e| e.to_string().contains("no signed Helper configured"));
        assert!(refused);
    }
    #[test]
    fn real_refuses_unsigned_helper() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("lab-world-unsigned-{}", std::process::id()));
        let executable = directory.join("Contents/MacOS");
        fs::create_dir_all(&executable)?;
        fs::write(executable.join("AppleBridgeHelper"), b"x")?;
        let result = require_signed_helper(Some(&directory)).is_err();
        fs::remove_dir_all(&directory)?;
        assert!(result);
        Ok(())
    }
    #[test]
    fn real_refuses_live_home() -> Result<()> {
        let base = std::env::temp_dir().join(format!("lab-world-live-{}", std::process::id()));
        let home = base.join("home");
        let live = home.join("Library/Application Support/AppleBridge");
        fs::create_dir_all(&live)?;
        let homes = live_homes(&home.to_string_lossy(), None);
        let refused = [
            live.join("world"),
            live.clone(),
            home.join("Library/Application Support"),
        ]
        .iter()
        .all(|directory| refuse_live_home(directory, &homes).is_err());
        let allowed = refuse_live_home(&base.join("elsewhere"), &homes).is_ok();
        fs::remove_dir_all(&base)?;
        assert!(refused);
        assert!(allowed);
        assert_eq!(
            live_homes("/Users/example", Some("/data"))[1],
            Path::new("/data/apple-bridge")
        );
        Ok(())
    }
    #[test]
    fn real_allows_unrelated_directory() {
        let homes = live_homes("/Users/example", None);
        let directory = std::env::temp_dir().join("lab-world-allowed");
        assert!(refuse_live_home(&directory, &homes).is_ok());
    }
}
