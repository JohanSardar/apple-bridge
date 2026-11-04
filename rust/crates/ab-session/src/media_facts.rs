use crate::Error;
use ab_native::notes_reader::{DerivativeFacts, Facts, Variant};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const HELPER_NAME: &str = "AppleBridgeHelper";
const DEADLINE: Duration = Duration::from_secs(120);
const MAX_VARIANT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const DERIVATIVE_CAP: usize = 64;

#[derive(Debug, Clone)]
pub struct MediaFacts {
    helper: PathBuf,
    root: PathBuf,
}

impl MediaFacts {
    #[must_use]
    pub fn new(helper: PathBuf, root: PathBuf) -> Self {
        Self { helper, root }
    }

    #[must_use]
    pub fn locate(configured: Option<PathBuf>, root: PathBuf) -> Option<Self> {
        let helper = configured.or_else(|| {
            std::env::current_exe()
                .ok()?
                .parent()
                .map(|dir| dir.join(HELPER_NAME))
        })?;
        helper.is_file().then(|| Self::new(helper, root))
    }

    fn call(&self, request: &Value) -> Result<Value, Error> {
        std::fs::create_dir_all(&self.root)?;
        let mut child = Command::new(&self.helper)
            .arg("media-facts")
            .env("AB_MEDIA_DERIVATIVES_ROOT", &self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or(Error::Invalid)?;
        stdin.write_all(&serde_json::to_vec(request)?)?;
        drop(stdin);
        let stdout = child.stdout.take().ok_or(Error::Invalid)?;
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(MAX_RESPONSE_BYTES).read_to_end(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes)
        });
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() > DEADLINE {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Reader("media facts timed out".into()));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let bytes = reader.join().map_err(|_| Error::WorkerPanicked)??;
        if !status.success() {
            return Err(Error::Reader("media facts failed".into()));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn resolve(&self, fact: &str, staging: &Path, facts: &mut Facts) -> Result<(), Error> {
        let (kind, argument) = fact.split_once(':').ok_or(Error::Invalid)?;
        match kind {
            "mime" => {
                let response = self.call(&json!({"kind":"mime","uti":argument}))?;
                let mime = response["mime"].as_str().map(str::to_owned);
                facts.mime.insert(argument.to_owned(), mime);
            }
            "supports" => {
                let (uti, extension) = argument.split_once('|').ok_or(Error::Invalid)?;
                let response = self.call(&json!({"kind":"supports","uti":uti,"ext":extension}))?;
                let supported = response["supports"].as_bool().ok_or(Error::Invalid)?;
                facts.supports.insert(argument.to_owned(), supported);
            }
            "derivatives" => {
                if !argument.bytes().all(|b| b.is_ascii_hexdigit()) || argument.len() != 64 {
                    return Err(Error::Invalid);
                }
                let file = staging.join(argument);
                if !file.is_file() {
                    return Err(Error::Reader("staged original missing".into()));
                }
                let response = self.call(&json!({
                    "kind":"derive",
                    "digest":argument,
                    "file":file.to_str().ok_or(Error::Invalid)?,
                }))?;
                if facts.derivatives.len() >= DERIVATIVE_CAP {
                    facts.derivatives.clear();
                }
                facts
                    .derivatives
                    .insert(argument.to_owned(), self.derivatives(&response)?);
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }

    fn derivatives(&self, response: &Value) -> Result<DerivativeFacts, Error> {
        let status = response["status"]
            .as_str()
            .ok_or(Error::Invalid)?
            .to_owned();
        let mut variants = std::collections::BTreeMap::new();
        if let Some(items) = response["variants"].as_object() {
            for (name, item) in items {
                let path = PathBuf::from(item["path"].as_str().ok_or(Error::Invalid)?);
                if !path.starts_with(&self.root) {
                    return Err(Error::Invalid);
                }
                let metadata = std::fs::metadata(&path)?;
                if !metadata.is_file() || metadata.len() > MAX_VARIANT_BYTES {
                    return Err(Error::Invalid);
                }
                let dimension = |key: &str| {
                    item[key]
                        .as_u64()
                        .and_then(|value| u32::try_from(value).ok())
                };
                variants.insert(
                    name.clone(),
                    Variant {
                        bytes_base64: STANDARD.encode(std::fs::read(&path)?),
                        mime: item["mime"].as_str().ok_or(Error::Invalid)?.to_owned(),
                        width: dimension("width"),
                        height: dimension("height"),
                    },
                );
            }
        }
        Ok(DerivativeFacts { status, variants })
    }
}
