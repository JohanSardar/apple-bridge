use crate::http::Client;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

const SCOPE: &str = "reminders:read,reminders:write,system:read";
static GATEWAY: OnceLock<PathBuf> = OnceLock::new();
static CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());
static ENDPOINTS: Mutex<Vec<Endpoint>> = Mutex::new(Vec::new());

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub home: PathBuf,
    pub address: String,
    pub token: String,
    pub scope: String,
}
impl Endpoint {
    #[must_use]
    pub fn client(&self) -> Client {
        Client::at(&self.address, &self.token)
    }
}
pub fn configure(gateway: &Path) {
    let _ = GATEWAY.set(gateway.to_owned());
}
pub(crate) fn binary() -> Result<&'static Path> {
    GATEWAY
        .get()
        .map(PathBuf::as_path)
        .ok_or_else(|| Error::Invalid("gateway binary not configured".into()))
}
pub fn cli(home: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(binary()?)
        .args(args)
        .arg("--home")
        .arg(home)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .output()?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "{} exited {}",
            args.first().copied().unwrap_or_default(),
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub fn start(world: &World, count: usize) -> Result<()> {
    start_with(world, count, SCOPE, 0)
}
pub fn start_with(world: &World, count: usize, scope: &str, cap: u64) -> Result<()> {
    start_with_args(world, count, scope, cap, &[])
}
pub fn start_with_args(
    world: &World,
    count: usize,
    scope: &str,
    cap: u64,
    extra: &[String],
) -> Result<()> {
    let peer = world.pair_address()?;
    let base = lock(&ENDPOINTS).len();
    for index in base..base + count {
        let home = world.directory.join(format!("replica{index}"));
        let name = format!("replica{index}");
        let code = world.gateway_cli(&[
            "pair",
            "--peer-addr",
            &peer,
            "--name",
            &name,
            "--scope",
            scope,
        ])?;
        cli(&home, &["join", code.trim()])?;
        let token = cli(&home, &["token", "create", "--scope", scope])?;
        let log = fs::File::create(world.directory.join(format!("{name}.log")))?;
        let child = Command::new(binary()?)
            .args(["start", "--role", "replica", "--home"])
            .arg(&home)
            .args(["--api-addr", "127.0.0.1:0"])
            .args(["--blob-cap", &cap.to_string()])
            .args(extra)
            .env("APPLE_BRIDGE_SECRET_STORE", "file")
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;
        lock(&CHILDREN).push(child);
        let address = await_runtime(&home)?;
        lock(&ENDPOINTS).push(Endpoint {
            home,
            address,
            token,
            scope: scope.to_owned(),
        });
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let connected = endpoints().iter().all(|endpoint| {
            status(endpoint).is_ok_and(|value| value["replica"]["state"] == "connected")
        });
        if connected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Error::Invalid("replicas not connected".into()));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
fn await_runtime(home: &Path) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(bytes) = fs::read(home.join("runtime.json"))
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && let Some(address) = value["api_address"].as_str()
        {
            return Ok(address.to_owned());
        }
        if Instant::now() >= deadline {
            return Err(Error::Invalid("replica readiness timeout".into()));
        }
        thread::sleep(Duration::from_millis(25));
    }
}
#[must_use]
pub fn endpoints() -> Vec<Endpoint> {
    lock(&ENDPOINTS).clone()
}
pub fn status(endpoint: &Endpoint) -> Result<Value> {
    endpoint
        .client()
        .call("sources.status", &json!({}), 10)?
        .ok_or_else(|| Error::Invalid("replica status rejected".into()))
}
pub fn stop_all() {
    lock(&ENDPOINTS).clear();
    for mut child in lock(&CHILDREN).drain(..) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub fn crash(index: usize) -> Result<()> {
    let mut children = lock(&CHILDREN);
    let child = children
        .get_mut(index)
        .ok_or_else(|| Error::Invalid("replica index".into()))?;
    child.kill()?;
    child.wait()?;
    Ok(())
}
pub fn restart(index: usize) -> Result<()> {
    let endpoint = endpoints()
        .get(index)
        .cloned()
        .ok_or_else(|| Error::Invalid("replica index".into()))?;
    let _ = fs::remove_file(endpoint.home.join("runtime.json"));
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(endpoint.home.with_file_name(format!("replica{index}.log")))?;
    let child = Command::new(binary()?)
        .args(["start", "--role", "replica", "--home"])
        .arg(&endpoint.home)
        .args(["--api-addr", &endpoint.address])
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let mut children = lock(&CHILDREN);
    if let Some(slot) = children.get_mut(index) {
        *slot = child;
    }
    drop(children);
    await_runtime(&endpoint.home)?;
    Ok(())
}
pub fn signal(index: usize, name: &str) -> Result<()> {
    let id = lock(&CHILDREN)
        .get(index)
        .map(Child::id)
        .ok_or_else(|| Error::Invalid("replica index".into()))?;
    let status = Command::new("kill")
        .arg(format!("-{name}"))
        .arg(id.to_string())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Invalid(format!("signal {name} failed")))
    }
}
