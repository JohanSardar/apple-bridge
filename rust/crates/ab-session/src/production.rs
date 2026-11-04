use crate::{
    Error,
    ownership::{ConnectionState, Producer, Selection, SourceId},
    readers::{self, CaptureReader},
    routed::{self, Options},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

pub const SUPPORTED: &[&str] = &[
    "calendar",
    "contacts",
    "mail",
    "messages",
    "safari",
    "usage",
    "notes",
    "phone",
    "voicemail",
    "voice_memos",
];
pub const CONFIG_FILE: &str = "session/rust-capture.json";
pub const STATE_DIRECTORY: &str = "session/capture-state";
pub const STAGING_DIRECTORY: &str = "session/staging";
const STAGED: &[&str] = &["notes", "voice_memos"];

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

#[derive(Debug)]
pub struct Loaded {
    sources: String,
    names: Vec<String>,
    options: Options,
    notes_root: Option<PathBuf>,
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

    #[must_use]
    pub fn notes_root(&self) -> Option<&Path> {
        self.notes_root.as_deref()
    }

    pub fn readers(&self) -> Result<BTreeMap<String, Box<dyn CaptureReader>>, Error> {
        readers::from_config(&self.sources)
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
    let mut selection = Selection::default();
    for name in file.sources.keys() {
        selection.select_disconnected(
            SourceId::try_from(name.clone())?,
            Producer::Rust,
            &ConnectionState::Disconnected,
        )?;
    }
    selection.validate_routed(SUPPORTED)?;
    let notes_root = file
        .sources
        .get("notes")
        .and_then(|source| source["root"].as_str())
        .map(PathBuf::from);
    let loaded = Loaded {
        notes_root,
        sources: serde_json::to_string(&file.sources)?,
        names: file.sources.keys().cloned().collect(),
        options: Options {
            poll: Duration::from_millis(file.poll_ms),
            watch: file.watch,
            state: Some(home.join(STATE_DIRECTORY)),
        },
    };
    loaded.readers()?;
    Ok(Some(loaded))
}

#[derive(Debug)]
pub struct EffectsLink {
    pub native: UnixStream,
    pub home: PathBuf,
    #[cfg(feature = "rust-effects")]
    pub routing: crate::effects::Routing,
}

pub const EFFECTS_DIRECTORY: &str = "native-effects";
pub const EFFECTS_FILE: &str = "session/rust-effects.json";

#[cfg(feature = "rust-effects")]
fn notes_database(capture: Option<&Loaded>) -> Option<PathBuf> {
    let root = capture
        .and_then(Loaded::notes_root)
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("AB_SESSION_NOTES_ROOT")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| {
                        PathBuf::from(home).join("Library/Group Containers/group.com.apple.notes")
                    })
                })
        })?;
    let database = root.join("NoteStore.sqlite");
    database.exists().then_some(database)
}

#[cfg(feature = "rust-effects")]
pub fn effects_routing(home: &Path) -> Result<crate::effects::Routing, Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Effects {
        sources: Vec<String>,
    }
    if let Ok(list) = std::env::var("AB_SESSION_EFFECTS_SOURCES") {
        return Ok(crate::effects::Routing::parse(&list));
    }
    match std::fs::read(home.join(EFFECTS_FILE)) {
        Ok(bytes) => {
            let file: Effects = serde_json::from_slice(&bytes)?;
            Ok(crate::effects::Routing::only(file.sources))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(crate::effects::Routing::all())
        }
        Err(error) => Err(error.into()),
    }
}

pub fn serve(
    gateway: UnixStream,
    helper: UnixStream,
    capture: Option<&Loaded>,
    effects: Option<EffectsLink>,
) -> Result<(), Error> {
    #[cfg(feature = "rust-effects")]
    if let Some(link) = effects {
        let mut config = crate::effects::Config {
            routing: link.routing,
            ..crate::effects::Config::default()
        };
        if let Some(loaded) = capture {
            config.rust_sources = loaded.sources().iter().cloned().collect();
        }
        config.notes_database = notes_database(capture);
        let layer = crate::effects::Layer::open_with(
            link.native,
            &link.home.join(EFFECTS_DIRECTORY),
            config,
        )?;
        let (readers, options) = match capture {
            Some(loaded) => (loaded.readers()?, loaded.options().clone()),
            None => (BTreeMap::new(), Options::default()),
        };
        return routed::relay_layered(gateway, helper, readers, &options, Some(&layer));
    }
    #[cfg(not(feature = "rust-effects"))]
    if effects.is_some() {
        return Err(Error::Conflict);
    }
    match capture {
        Some(loaded) => {
            routed::relay_layered(gateway, helper, loaded.readers()?, loaded.options(), None)
        }
        None => crate::proxy::relay(gateway, helper),
    }
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
