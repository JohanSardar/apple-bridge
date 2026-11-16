use crate::{Error, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, fs, io::Write, os::unix::fs::PermissionsExt, path::Path};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub id: String,
    pub enabled: bool,
}

pub const ROUTE: &str = "tailnet";

pub fn read<T: DeserializeOwned>(home: &Path, name: &str) -> Result<T> {
    Ok(serde_json::from_slice(&fs::read(
        home.join("remote").join(format!("{name}.json")),
    )?)?)
}

pub fn write<T: Serialize>(home: &Path, name: &str, value: &T) -> Result<()> {
    atomic(
        &home.join("remote").join(format!("{name}.json")),
        &serde_json::to_vec(value)?,
    )
}

pub fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or(Error::Contract("missing parent"))?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let tmp = parent.join(format!(".remote-{}", token()));
    let result = (|| {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result
}

pub fn token() -> String {
    use base64::Engine;
    let mut bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub last_error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default)]
    pub pending_link: String,
    #[serde(default = "zero_time")]
    pub certificate_expiry: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<String, Route>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pending_link: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_error: String,
}
pub const ZERO_TIME: &str = "0001-01-01T00:00:00Z";
fn zero_time() -> String {
    ZERO_TIME.into()
}
impl Default for Status {
    fn default() -> Self {
        Self {
            id: String::new(),
            state: String::new(),
            url: String::new(),
            last_error: String::new(),
            message: String::new(),
            pending_link: String::new(),
            certificate_expiry: zero_time(),
            routes: BTreeMap::new(),
        }
    }
}

pub fn status(home: &Path) -> Result<Status> {
    match read(home, "status") {
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(Status {
            state: "off".into(),
            ..Status::default()
        }),
        other => other,
    }
}

fn lock(home: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = home.join("remote");
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join("config.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    Ok(lock)
}

fn selections(home: &Path) -> Result<BTreeMap<String, bool>> {
    match read(home, "ingresses") {
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        other => other,
    }
}

#[must_use]
pub fn selection(home: &Path) -> Option<bool> {
    selections(home).ok()?.get(ROUTE).copied()
}

#[must_use]
pub fn selected(home: &Path) -> bool {
    selection(home) == Some(true)
}

pub fn select(home: &Path, action: &str, id: &str) -> Result<Config> {
    if action != "on" && action != "off" {
        return Err(Error::Contract("action must be on, off or status"));
    }
    let _lock = lock(home)?;
    let mut selections = selections(home)?;
    selections.retain(|name, _| name == ROUTE);
    selections.insert(ROUTE.to_owned(), action == "on");
    write(home, "ingresses", &selections)?;
    let config = Config {
        id: id.into(),
        enabled: action == "on",
    };
    write(home, "config", &config)?;
    Ok(config)
}

pub fn cancel(home: &Path, id: &str) -> Result<()> {
    let _lock = lock(home)?;
    let config = read::<Config>(home, "config")?;
    if config.id != id {
        return Ok(());
    }
    write(
        home,
        "ingresses",
        &BTreeMap::from([(ROUTE.to_owned(), false)]),
    )?;
    write(
        home,
        "config",
        &Config {
            enabled: false,
            ..config
        },
    )
}
