use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command},
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
#[derive(Debug)]
pub struct World {
    pub directory: PathBuf,
    pub gateway_home: PathBuf,
    pub runtime: Value,
    config: WorldConfig,
    gateway: Option<Child>,
    peer: Option<Child>,
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
            runtime: Value::Null,
            config,
            gateway: None,
            peer: None,
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
        let mut command = Command::new("curl");
        command
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "--max-time",
                "25",
                "--unix-socket",
            ])
            .arg(self.directory.join("sim.sock"));
        if !input.is_null() {
            command.args([
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                &serde_json::to_string(input)?,
            ]);
        }
        command.arg(format!("http://sim{route}"));
        Ok(serde_json::from_str(&output(command).map_err(|e| {
            Error::Invalid(format!("control {route}: {e}"))
        })?)?)
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
