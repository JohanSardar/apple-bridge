use crate::media_facts::MediaFacts;

use super::{
    AgendaReader, Arc, BTreeMap, CaptureReader, Debug, Deserialize, Duration, Error, FilesReader,
    FocusReader, MacReader, MailReader, MessagesReader, NotesReader, Path, PathBuf, PdfReader,
    SafariReader, ShortcutsReader, TelephonyReader, UsageReader, VoiceMemosReader, native_facts,
    shortcuts,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceConfig {
    root: PathBuf,
    #[serde(default = "default_generation")]
    generation: String,
    #[serde(default = "default_zone")]
    zone: String,
    #[serde(default = "default_local_name")]
    local_name: String,
    #[serde(default)]
    hydration_hold: Option<PathBuf>,
    #[serde(default)]
    staging: Option<PathBuf>,
    #[serde(default)]
    home: Option<PathBuf>,
    #[serde(default)]
    facts: Option<FactsLink>,
    #[serde(default = "default_facts_poll")]
    facts_poll_ms: u64,
    #[serde(default)]
    store: Option<PathBuf>,
    #[serde(default)]
    lookups: Option<PathBuf>,
    #[serde(default)]
    media_helper: Option<PathBuf>,
    #[serde(default)]
    media_root: Option<PathBuf>,
    #[serde(default)]
    roots: Vec<PathBuf>,
    #[serde(default = "default_media_retry")]
    media_retry_ms: u64,
    #[serde(default = "default_tabs_poll")]
    tabs_poll_ms: u64,
    #[serde(default)]
    executable: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FactsLink {
    Native(NativeMarker),
    Socket { socket: PathBuf },
}

#[derive(Debug, Deserialize)]
enum NativeMarker {
    #[serde(rename = "native")]
    Native,
}

fn default_media_retry() -> u64 {
    60_000
}

fn default_tabs_poll() -> u64 {
    5_000
}

fn default_facts_poll() -> u64 {
    15_000
}

fn lexical(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute() {
        return Err(Error::Invalid);
    }
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                if !out.pop() {
                    return Err(Error::Invalid);
                }
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

fn resolved(path: &Path) -> Result<PathBuf, Error> {
    let path = lexical(path)?;
    let mut tail = Vec::new();
    let mut head = path.as_path();
    loop {
        if let Ok(mut base) = head.canonicalize() {
            for part in tail.iter().rev() {
                base.push(part);
            }
            return Ok(base);
        }
        let name = head.file_name().ok_or(Error::Invalid)?;
        tail.push(name.to_owned());
        head = head.parent().ok_or(Error::Invalid)?;
    }
}

pub fn confine_staging(root: &Path, staging: &Path) -> Result<(), Error> {
    let root = resolved(root)?;
    let staging = resolved(staging)?;
    if staging.starts_with(&root) || root.starts_with(&staging) {
        return Err(Error::Invalid);
    }
    Ok(())
}

fn default_generation() -> String {
    "rust-native/1".into()
}

fn default_zone() -> String {
    "UTC".into()
}

fn default_local_name() -> String {
    "local".into()
}

pub fn from_config(config: &str) -> Result<BTreeMap<String, Box<dyn CaptureReader>>, Error> {
    from_config_linked(config, None)
}

#[must_use]
pub fn wants_inherited_facts(config: &str) -> bool {
    serde_json::from_str::<BTreeMap<String, SourceConfig>>(config).is_ok_and(|sources| {
        sources
            .values()
            .any(|source| matches!(source.facts, Some(FactsLink::Native(_))))
    })
}

fn linked_client(
    facts: Option<&FactsLink>,
    inherited: Option<&Arc<native_facts::Client>>,
) -> Result<Option<Arc<native_facts::Client>>, Error> {
    match facts {
        None => Ok(None),
        Some(FactsLink::Native(_)) => Ok(Some(Arc::clone(inherited.ok_or(Error::Invalid)?))),
        Some(FactsLink::Socket { socket }) => Ok(Some(Arc::new(native_facts::Client::socket(
            socket.clone(),
        )?))),
    }
}

#[allow(clippy::too_many_lines)]
pub fn from_config_linked(
    config: &str,
    inherited: Option<&Arc<native_facts::Client>>,
) -> Result<BTreeMap<String, Box<dyn CaptureReader>>, Error> {
    let sources: BTreeMap<String, SourceConfig> = serde_json::from_str(config)?;
    let mut readers: BTreeMap<String, Box<dyn CaptureReader>> = BTreeMap::new();
    for (source, config) in sources {
        let reader: Box<dyn CaptureReader> = match source.as_str() {
            "safari" => {
                let client = linked_client(config.facts.as_ref(), inherited)?;
                Box::new(
                    SafariReader::new(config.root, config.generation)
                        .with_tabs(client, Duration::from_millis(config.tabs_poll_ms.max(1))),
                )
            }
            "focus" => {
                let client =
                    linked_client(config.facts.as_ref(), inherited)?.ok_or(Error::Invalid)?;
                let home = config.home.clone().ok_or(Error::Invalid)?;
                Box::new(FocusReader::new(
                    config.root,
                    &home,
                    client,
                    Duration::from_millis(config.facts_poll_ms.max(1)),
                ))
            }
            "pdf" => Box::new(PdfReader::new(config.root)),
            "shortcuts" => Box::new(ShortcutsReader::new(
                config.root,
                config
                    .executable
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(shortcuts::EXECUTABLE)),
                Duration::from_millis(config.facts_poll_ms.max(1)),
            )),
            "mail" => Box::new(MailReader::new(config.root)),
            "usage" => Box::new(UsageReader::new(
                config.root,
                config.generation,
                config.zone,
                config.local_name,
            )),
            "phone" => Box::new(TelephonyReader::calls(&config.root)),
            "voicemail" => Box::new(TelephonyReader::voicemail(&config.root)),
            "voice_memos" => {
                let staging = config.staging.clone().ok_or(Error::Invalid)?;
                Box::new(VoiceMemosReader::new(&config.root, &staging)?)
            }
            "notes" => {
                let staging = config.staging.clone().ok_or(Error::Invalid)?;
                let media = config
                    .media_root
                    .clone()
                    .and_then(|root| MediaFacts::locate(config.media_helper.clone(), root));
                let client = linked_client(config.facts.as_ref(), inherited)?;
                Box::new(
                    NotesReader::new(config.root, &staging)?
                        .holding(config.hydration_hold)
                        .with_media(media)
                        .with_status(client, config.home.clone()),
                )
            }
            "messages" => {
                let home = match config.home.clone() {
                    Some(home) => home,
                    None if config.root.ends_with("Library/Messages") => config
                        .root
                        .parent()
                        .and_then(Path::parent)
                        .ok_or(Error::Invalid)?
                        .to_path_buf(),
                    None => return Err(Error::Invalid),
                };
                if !home.is_absolute() {
                    return Err(Error::Invalid);
                }
                let client = linked_client(config.facts.as_ref(), inherited)?;
                Box::new(
                    MessagesReader::new(config.root, home)
                        .with_media(client, Duration::from_millis(config.media_retry_ms)),
                )
            }
            "files" | "mac" => {
                let client =
                    linked_client(config.facts.as_ref(), inherited)?.ok_or(Error::Invalid)?;
                if source == "mac" {
                    Box::new(MacReader::new(
                        config.root,
                        client,
                        Duration::from_millis(config.facts_poll_ms.max(1)),
                    ))
                } else {
                    Box::new(FilesReader::new(
                        config.root,
                        config.roots,
                        config.home.as_deref(),
                        client,
                    )?)
                }
            }
            "calendar" | "contacts" | "reminders" => {
                if config.store.is_some() && !["calendar", "reminders"].contains(&source.as_str()) {
                    return Err(Error::Invalid);
                }
                let client =
                    linked_client(config.facts.as_ref(), inherited)?.ok_or(Error::Invalid)?;
                let reader = match source.as_str() {
                    "calendar" => match config.store {
                        Some(store) => AgendaReader::calendar(config.root)
                            .with_calendar_store(lexical(&store)?),
                        None => AgendaReader::calendar(config.root),
                    },
                    "contacts" => AgendaReader::contacts(config.root),
                    _ => match config.store {
                        Some(store) => AgendaReader::reminders(config.root)
                            .with_reminders_store(lexical(&store)?),
                        None => AgendaReader::reminders(config.root),
                    },
                };
                let reader = match config.lookups.filter(|_| source == "calendar") {
                    Some(path) => reader.with_lookups(lexical(&path)?)?,
                    None => reader,
                };
                Box::new(reader.paged(client, Duration::from_millis(config.facts_poll_ms.max(1))))
            }
            _ => return Err(Error::Invalid),
        };
        readers.insert(source, reader);
    }
    Ok(readers)
}
