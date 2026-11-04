use crate::{
    Error,
    native_facts::Client,
    queries::{MailBodies, NativeQueries},
    readers::{self, CaptureReader},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const SUPPORTED: &[&str] = &[
    "calendar",
    "contacts",
    "reminders",
    "mail",
    "messages",
    "safari",
    "usage",
    "notes",
    "phone",
    "voicemail",
    "voice_memos",
    "files",
    "mac",
];
pub const CONFIG_FILE: &str = "session/rust-capture.json";
pub const STATE_DIRECTORY: &str = "session/capture-state";
pub const STAGING_DIRECTORY: &str = "session/staging";
const STAGED: &[&str] = &["notes", "voice_memos"];
pub const MEDIA_DIRECTORY: &str = "session/media-derivatives";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    sources: BTreeMap<String, serde_json::Value>,
    #[serde(default = "default_watch")]
    watch: bool,
    #[serde(default = "default_poll")]
    poll_ms: u64,
}

fn default_watch() -> bool {
    true
}

fn default_poll() -> u64 {
    5_000
}

#[derive(Debug, Clone)]
pub struct Options {
    pub poll: Duration,
    pub watch: bool,
    pub state: Option<PathBuf>,
    pub queries: NativeQueries,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(100),
            watch: false,
            state: None,
            queries: NativeQueries::default(),
        }
    }
}

#[derive(Debug)]
pub struct Loaded {
    sources: String,
    names: Vec<String>,
    options: Options,
}

impl Loaded {
    #[must_use]
    pub fn sources(&self) -> &[String] {
        &self.names
    }

    #[must_use]
    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn readers(&self) -> Result<BTreeMap<String, Box<dyn CaptureReader>>, Error> {
        self.readers_with(None)
    }

    pub fn readers_with(
        &self,
        facts: Option<&Arc<Client>>,
    ) -> Result<BTreeMap<String, Box<dyn CaptureReader>>, Error> {
        readers::from_config_linked(&self.sources, facts)
    }

    #[must_use]
    pub fn wants_inherited_facts(&self) -> bool {
        readers::wants_inherited_facts(&self.sources)
    }
}

fn zone_from_link(target: &Path) -> Option<String> {
    let text = target.to_str()?;
    let (_, zone) = text.rsplit_once("zoneinfo/")?;
    (!zone.is_empty()).then(|| zone.to_owned())
}

fn system_zone() -> String {
    if let Some(zone) = std::env::var("TZ")
        .ok()
        .map(|value| value.trim_start_matches(':').to_owned())
        .filter(|value| !value.is_empty() && !value.starts_with('/'))
    {
        return zone;
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|target| zone_from_link(&target))
        .unwrap_or_else(|| "UTC".into())
}

fn computer_name() -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()?;
    let name = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !name.is_empty()).then_some(name)
}

pub fn load(home: &Path) -> Result<Option<Loaded>, Error> {
    let path = home.join(CONFIG_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut file: File = serde_json::from_slice(&bytes)?;
    if file.sources.is_empty() {
        return Ok(None);
    }
    for name in STAGED {
        if let Some(source) = file.sources.get_mut(*name) {
            let object = source.as_object_mut().ok_or(Error::Invalid)?;
            if !object.contains_key("staging") {
                let path = home.join(STAGING_DIRECTORY).join(name);
                let text = path.to_str().ok_or(Error::Invalid)?;
                object.insert("staging".into(), text.into());
            }
        }
    }
    if let Some(source) = file.sources.get_mut("notes") {
        let object = source.as_object_mut().ok_or(Error::Invalid)?;
        if !object.contains_key("media_root") {
            let path = home.join(MEDIA_DIRECTORY);
            let text = path.to_str().ok_or(Error::Invalid)?;
            object.insert("media_root".into(), text.into());
        }
    }
    if let Some(source) = file.sources.get_mut("usage") {
        let object = source.as_object_mut().ok_or(Error::Invalid)?;
        if !object.contains_key("zone") {
            object.insert("zone".into(), system_zone().into());
        }
        if !object.contains_key("local_name")
            && let Some(name) = computer_name()
        {
            object.insert("local_name".into(), name.into());
        }
    }
    if file
        .sources
        .keys()
        .any(|name| !SUPPORTED.contains(&name.as_str()))
    {
        return Err(Error::Conflict);
    }
    let mail_bodies = file
        .sources
        .get("mail")
        .and_then(|source| source["root"].as_str())
        .map(|root| {
            let root = Path::new(root);
            let root = if root.ends_with("MailData") {
                root.parent().unwrap_or(root)
            } else {
                root
            };
            Arc::new(MailBodies::new(root.to_path_buf()))
        });
    let loaded = Loaded {
        sources: serde_json::to_string(&file.sources)?,
        names: file.sources.keys().cloned().collect(),
        options: Options {
            poll: Duration::from_millis(file.poll_ms),
            watch: file.watch,
            state: Some(home.join(STATE_DIRECTORY)),
            queries: NativeQueries {
                mail_bodies,
                shortcut_roots: None,
            },
        },
    };
    loaded.readers_with(Some(&Arc::new(Client::detached())))?;
    Ok(Some(loaded))
}

#[cfg(test)]
mod zone_tests {
    use super::zone_from_link;
    use std::path::Path;

    #[test]
    fn zone_is_the_path_below_zoneinfo() {
        assert_eq!(
            zone_from_link(Path::new("/var/db/timezone/zoneinfo/America/Chicago")).as_deref(),
            Some("America/Chicago")
        );
        assert_eq!(
            zone_from_link(Path::new("/usr/share/zoneinfo/UTC")).as_deref(),
            Some("UTC")
        );
        assert_eq!(zone_from_link(Path::new("/etc/other")), None);
    }
}
