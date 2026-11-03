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
    "mail",
    "safari",
    "usage",
    "notes",
    "phone",
    "voicemail",
    "voice_memos",
];
pub const CONFIG_FILE: &str = "session/rust-capture.json";
pub const STATE_DIRECTORY: &str = "session/capture-state";

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
        readers::from_config(&self.sources)
    }
}

pub fn load(home: &Path) -> Result<Option<Loaded>, Error> {
    let path = home.join(CONFIG_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let file: File = serde_json::from_slice(&bytes)?;
    if file.sources.is_empty() {
        return Ok(None);
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
    let loaded = Loaded {
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
}

pub fn serve(
    gateway: UnixStream,
    helper: UnixStream,
    capture: Option<&Loaded>,
    effects: Option<EffectsLink>,
) -> Result<(), Error> {
    #[cfg(feature = "rust-effects")]
    if let Some(link) = effects {
        let layer = crate::effects::Layer::open(link.native, &link.home)?;
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
