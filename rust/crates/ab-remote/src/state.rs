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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ct_warning: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub last_error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default)]
    pub pending_link: String,
    #[serde(default)]
    pub tls_termination: String,
    #[serde(default = "zero_time")]
    pub certificate_expiry: String,
}
pub const ZERO_TIME: &str = "0001-01-01T00:00:00Z";
fn zero_time() -> String {
    ZERO_TIME.into()
}
impl Default for Status {
    fn default() -> Self {
        Self {
            ct_warning: String::new(),
            id: String::new(),
            state: String::new(),
            backend: String::new(),
            url: String::new(),
            last_error: String::new(),
            message: String::new(),
            pending_link: String::new(),
            tls_termination: String::new(),
            certificate_expiry: zero_time(),
        }
    }
}

pub fn status(home: &Path) -> Result<Status> {
    match read(home, "status") {
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(Status {
            state: "off".into(),
            backend: "embedded".into(),
            tls_termination: "on this Mac".into(),
            ..Status::default()
        }),
        other => other,
    }
}

pub fn select(home: &Path, action: &str, via: &str, id: &str) -> Result<Config> {
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
                    let backend = if config.backend == "embedded" {
                        "relay".to_owned()
                    } else {
                        config.backend
                    };
                    entries.insert(backend, config.enabled);
                }
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            entries
        }
        Err(e) => return Err(e),
    };
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
    config.id = id.into();
    config.backend = selected.into();
    config.enabled = selections.values().any(|v| *v);
    write(home, "config", &config)?;
    Ok(config)
}

pub fn entries(via: &str) -> Vec<&'static str> {
    match via {
        "relay" => vec!["relay"],
        "private" => vec!["private"],
        "both" => vec!["relay", "system"],
        _ => vec!["system"],
    }
}

pub fn cancel(home: &Path, id: &str, via: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = home.join("remote");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join("config.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let mut config = read::<Config>(home, "config")?;
    if config.id != id {
        return Ok(());
    }
    let mut selections: BTreeMap<String, bool> = read(home, "ingresses")?;
    for entry in entries(via) {
        selections.insert(entry.to_owned(), false);
    }
    config.enabled = selections.values().any(|enabled| *enabled);
    write(home, "config", &config)?;
    write(home, "ingresses", &selections)
}

pub fn command(home: &Path, action: &str, via: &str) -> Result<serde_json::Value> {
    if action != "status" {
        select(home, action, via, &token())?;
    }
    match read::<BTreeMap<String, bool>>(home, "ingresses") {
        Ok(selections) => Ok(serde_json::to_value(selections)?),
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(e),
    }
}

pub fn annotate(home: &Path, name: &str) -> Result<()> {
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
    let mut config = match read::<Config>(home, "config") {
        Ok(value) => value,
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e),
    };
    config.name = name.into();
    write(home, "config", &config)
}

pub fn snippets(home: &Path) -> Result<String> {
    let status: Status = read(home, "status")?;
    if status.state != "on" || status.url.is_empty() {
        return Err(Error::Contract("remote is not ready"));
    }
    let url = &status.url;
    Ok(format!(
        "Claude Code: claude mcp add --transport http apple-bridge {url}/mcp\nclaude.ai connector: {url}/mcp\nChatGPT connector: {url}/mcp\nCodex config.toml:\n[mcp_servers.apple-bridge]\nurl = \"{url}/mcp\"\nGrok connector URL: {url}/mcp\nGrok (xai_sdk.tools): mcp(server_url=\"{url}/mcp\", server_label=\"apple-bridge\", authorization=\"Bearer <agent-token>\")\n"
    ))
}

#[cfg(test)]
mod flag_tests {
    use super::*;
    type Check = std::result::Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn snippets_require_a_ready_remote_and_match_go_text() -> Check {
        let home = tempfile::tempdir()?;
        assert!(snippets(home.path()).is_err());
        let mut status = Status {
            state: "starting".into(),
            url: "https://h.example".into(),
            ..Status::default()
        };
        write(home.path(), "status", &status)?;
        assert!(snippets(home.path()).is_err());
        status.state = "on".into();
        write(home.path(), "status", &status)?;
        assert_eq!(
            snippets(home.path())?,
            "Claude Code: claude mcp add --transport http apple-bridge https://h.example/mcp\nclaude.ai connector: https://h.example/mcp\nChatGPT connector: https://h.example/mcp\nCodex config.toml:\n[mcp_servers.apple-bridge]\nurl = \"https://h.example/mcp\"\nGrok connector URL: https://h.example/mcp\nGrok (xai_sdk.tools): mcp(server_url=\"https://h.example/mcp\", server_label=\"apple-bridge\", authorization=\"Bearer <agent-token>\")\n"
        );
        Ok(())
    }

    #[test]
    fn annotate_keeps_the_name_through_selection() -> Check {
        let home = tempfile::tempdir()?;
        annotate(home.path(), "laptop")?;
        let config = select(home.path(), "on", "relay", "id1")?;
        assert_eq!(config.name, "laptop");
        assert_eq!(read::<Config>(home.path(), "config")?.name, "laptop");
        Ok(())
    }
}
