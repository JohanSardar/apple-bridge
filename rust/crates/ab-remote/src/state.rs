use crate::{Error, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeMap, fs, io::Write, os::unix::fs::PermissionsExt, path::Path};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub domain: String,
    pub id: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub backend: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

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

pub fn command(home: &Path, action: &str, via: &str) -> Result<serde_json::Value> {
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
    let backend = match via {
        "relay" => "relay",
        "private" => "private",
        "tailscale-app" | "system" => "system",
        "both" => "both",
        _ => {
            return Err(Error::Contract(
                "via must be relay, tailscale-app, private or both",
            ));
        }
    };
    let mut selections: BTreeMap<String, bool> = match read(home, "ingresses") {
        Ok(value) => value,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut entries = BTreeMap::new();
            match read::<Config>(home, "config") {
                Ok(config) => {
                    entries.insert(config.backend, config.enabled);
                }
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            entries
        }
        Err(e) => return Err(e),
    };
    if action == "status" {
        if selections.get("private") == Some(&true) {
            return read(home, "status");
        }
        return Ok(serde_json::to_value(selections)?);
    }
    if action != "on" && action != "off" {
        return Err(Error::Contract("action must be on, off or status"));
    }
    for entry in if backend == "both" {
        vec!["relay", "system"]
    } else {
        vec![backend]
    } {
        selections.insert(entry.to_owned(), action == "on");
    }
    write(home, "ingresses", &selections)?;
    let selected = if selections.get("relay") == Some(&true) {
        "relay"
    } else if selections.get("private") == Some(&true) {
        "private"
    } else {
        "system"
    };
    let mut config = match read::<Config>(home, "config") {
        Ok(value) => value,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e),
    };
    config.id = token();
    config.backend = selected.into();
    config.enabled = selections.values().any(|v| *v);
    write(home, "config", &config)?;
    Ok(serde_json::to_value(selections)?)
}
