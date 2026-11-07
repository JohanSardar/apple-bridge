use crate::capture::{self, Context, Error, Pending, Result, ScanInput, Snapshot, State};
use serde_json::json;
use std::{path::Path, time::Duration};
const GENERATION: &str = "SELECT Z_UUID FROM Z_METADATA";
const TRANSCRIPT_DECODE_BYTES: usize = 1024 * 1024;
const VOICEMAIL_RECORD_BYTES: usize = 128 * 1024;
const SCAN_TRANSCRIPT_BYTES: usize = 4 * 1024 * 1024;
type Stage<'a> =
    dyn FnMut(&mut crate::assets::Store, &Path, &Path) -> Result<crate::assets::Reference> + 'a;

#[cfg(test)]
mod tests;

const CALL_COLUMNS: &[&str] = &[
    "Z_PK",
    "ZADDRESS",
    "ZNAME",
    "ZDATE",
    "ZDURATION",
    "ZORIGINATED",
    "ZANSWERED",
    "ZSERVICE_PROVIDER",
    "ZCALLTYPE",
];
const VOICEMAIL_COLUMNS: &[&str] = &[
    "Z_PK",
    "ZRECORDUUID",
    "ZFROM",
    "ZDATECREATED",
    "ZDURATION",
    "ZISREAD",
];

#[must_use]
pub fn calls_readable(path: &Path) -> bool {
    Snapshot::open(path, GENERATION)
        .is_ok_and(|reader| reader.require(&[("ZCALLRECORD", CALL_COLUMNS)]).is_ok())
}

#[must_use]
pub fn voicemail_readable(path: &Path) -> bool {
    Snapshot::open(path, GENERATION).is_ok_and(|reader| {
        reader
            .require(&[("ZSTOREDMESSAGE", VOICEMAIL_COLUMNS)])
            .is_ok()
    })
}

pub fn calls(
    path: &Path,
    state: &State,
    context: &Context,
    live: bool,
    max_rows: usize,
) -> Result<Pending> {
    let reader = Snapshot::open(path, GENERATION)?;
    reader.require(&[("ZCALLRECORD", CALL_COLUMNS)])?;
    let rows=reader.query("SELECT CAST(Z_PK AS TEXT) AS id,ZADDRESS,ZNAME,ZDATE,ZDURATION,ZORIGINATED,ZANSWERED,ZSERVICE_PROVIDER,ZCALLTYPE FROM ZCALLRECORD",&[],max_rows,Duration::from_secs(5))?;
    let mut observations = Vec::new();
    for row in rows {
        let n = |k| capture::number(&row, k);
        let t = |k| capture::text(&row, k);
        let originated = n("ZORIGINATED") != 0.0;
        let answered = n("ZANSWERED") != 0.0;
        let provider = t("ZSERVICE_PROVIDER").to_lowercase();
        #[allow(clippy::float_cmp)]
        let video = n("ZCALLTYPE") == 8.0;
        let service = if provider.contains("facetime") {
            if video {
                "FaceTime video"
            } else {
                "FaceTime audio"
            }
        } else if provider.is_empty() || provider == "com.apple.telephony" {
            "cellular"
        } else {
            return Err(Error::Unsupported);
        };
        if !n("ZDURATION").is_finite() || n("ZDURATION") < 0.0 {
            return Err(Error::Corrupt);
        }
        let id = t("id");
        let data = json!({"direction":if originated{"outgoing"}else if answered{"incoming"}else{"missed"},"handle":t("ZADDRESS"),"name":t("ZNAME"),"date":capture::timestamp(n("ZDATE")+978_307_200.0)?,"duration":n("ZDURATION"),"service":service,"answered":answered});
        observations.push(capture::upsert(
            "phone.call",
            id,
            Some(format!("phone/{}/{id}", reader.generation)),
            data,
            context,
        )?);
    }
    observations.sort_by(|a, b| a.native_key.cmp(&b.native_key));
    reader.finish(GENERATION)?;
    state.prepare(ScanInput {
        source: "phone",
        generation: &reader.generation,
        records: observations,
        scope: vec!["phone.call".into()],
        coverage: json!({"fields":"complete"}),
        context,
        live,
    })
}

pub fn voicemail_transcript(bytes: &[u8]) -> Result<serde_json::Value> {
    use crate::plist::{self, Value};
    let root = plist::archive_root_limited(bytes, TRANSCRIPT_DECODE_BYTES)?;
    let (text, elements, confidence) = match &root {
        Value::Dictionary(fields)
            if fields.contains_key("transcriptionString") || fields.contains_key("segments") =>
        {
            let elements = match fields.get("segments") {
                Some(Value::Array(values)) => values.as_slice(),
                None => &[],
                _ => return Err(Error::Corrupt),
            };
            (
                fields.get("transcriptionString").and_then(Value::string),
                elements,
                fields.get("confidence").and_then(Value::number),
            )
        }
        Value::Array(values) => (None, values.as_slice(), None),
        _ => return Err(Error::Unsupported),
    };
    let mut segments = Vec::new();
    for element in elements {
        let fields = element.dictionary().ok_or(Error::Corrupt)?;
        let text = fields
            .get("text")
            .and_then(Value::string)
            .unwrap_or_default();
        let mut segment = json!({"text": text});
        if let Some(confidence) = fields.get("confidence").and_then(Value::number) {
            if !confidence.is_finite() {
                return Err(Error::Corrupt);
            }
            segment["confidence"] = json!(confidence);
        }
        segments.push(segment);
    }
    let text = text.map_or_else(
        || {
            segments
                .iter()
                .filter_map(|v| v["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        },
        str::to_owned,
    );
    let mut result = json!({"text": text, "segments": segments});
    if let Some(confidence) = confidence {
        if !confidence.is_finite() {
            return Err(Error::Corrupt);
        }
        result["confidence"] = json!(confidence);
    }
    Ok(result)
}

fn voicemail_observation(
    id: &str,
    generation: &str,
    data: serde_json::Value,
    context: &Context,
) -> Result<capture::Observation> {
    let mut observation = capture::upsert(
        "voicemail.message",
        id,
        Some(format!("voicemail/{generation}/{id}")),
        data,
        context,
    )?;
    let size = |observation: &capture::Observation| -> Result<usize> {
        Ok(crate::native_json::record_data(&serde_json::to_value(observation)?)?.len())
    };
    if size(&observation)? > VOICEMAIL_RECORD_BYTES {
        let mut data: serde_json::Value =
            serde_json::from_str(observation.data.as_ref().ok_or(Error::Corrupt)?.get())?;
        let fields = data.as_object_mut().ok_or(Error::Corrupt)?;
        if fields.remove("transcript").is_none() {
            return Err(Error::Unsupported);
        }
        fields.insert("transcript_state".into(), json!("unsupported"));
        observation.data = Some(serde_json::from_slice(&crate::native_json::record_data(
            &data,
        )?)?);
    }
    if size(&observation)? > VOICEMAIL_RECORD_BYTES {
        return Err(Error::Unsupported);
    }
    Ok(observation)
}

#[must_use]
pub fn voicemail_coverage() -> serde_json::Value {
    json!({"fields":"metadata_and_local_assets","transcript":"archived_if_present","audio":"local_if_present"})
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AudioVersion {
    device: u64,
    inode: u64,
    size: u64,
    modified: Option<std::time::SystemTime>,
    changed: (i64, i64),
    dataless: bool,
}

fn audio_version(path: &Path) -> Option<AudioVersion> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some(AudioVersion {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified: metadata.modified().ok(),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
        dataless: crate::assets::dataless(&metadata),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSignature(std::collections::BTreeMap<std::path::PathBuf, Option<AudioVersion>>);

impl AudioSignature {
    #[must_use]
    pub fn current(&self) -> Self {
        Self(
            self.0
                .keys()
                .map(|path| (path.clone(), audio_version(path)))
                .collect(),
        )
    }
}

#[derive(Debug)]
pub struct VoicemailReader {
    state: State,
    assets: crate::assets::Store,
    tracked: AudioSignature,
    retry: bool,
}

impl VoicemailReader {
    pub fn open(staging: &Path) -> Result<Self> {
        Ok(Self {
            state: State::default(),
            assets: crate::assets::Store::open(staging, "voicemail-blob:")?,
            tracked: AudioSignature::default(),
            retry: false,
        })
    }

    #[must_use]
    pub const fn needs_retry(&self) -> bool {
        self.retry
    }

    fn retain_transcript(
        &self,
        observation: &mut capture::Observation,
        generation: &str,
        live: bool,
        remaining: &mut usize,
    ) -> Result<bool> {
        if live && self.state.reuse(generation, observation) {
            return Ok(false);
        }
        let raw = observation.data.as_ref().ok_or(Error::Corrupt)?.get();
        let mut data: serde_json::Value = serde_json::from_str(raw)?;
        if data.get("transcript").is_none() {
            return Ok(false);
        }
        if raw.len() <= *remaining {
            *remaining -= raw.len();
            return Ok(false);
        }
        let fields = data.as_object_mut().ok_or(Error::Corrupt)?;
        fields.remove("transcript");
        fields.insert("transcript_state".into(), json!("pending"));
        observation.data = Some(serde_json::from_slice(&crate::native_json::record_data(
            &data,
        )?)?);
        Ok(true)
    }

    pub fn audio_signature(&self) -> &AudioSignature {
        &self.tracked
    }

    pub fn read_blob(&self, handle: &str, offset: u64, length: usize) -> Result<serde_json::Value> {
        self.assets.read(handle, offset, length)
    }

    fn audio(
        &mut self,
        row: &capture::Row,
        root: &Path,
        data: &mut serde_json::Value,
        handles: &mut std::collections::BTreeMap<String, String>,
        stage: &mut Stage<'_>,
    ) -> Result<()> {
        use capture::Cell;
        let Some(Cell::Text(extension)) = row.get("ZFILETYPE") else {
            return Ok(());
        };
        data["audio_state"] = json!("missing");
        let mime = match (
            data["type"] == "faceTimeVideo",
            extension.to_ascii_lowercase().as_str(),
        ) {
            (false, "amr") => "audio/amr",
            (false, "m4a" | "mov") => "audio/mp4",
            _ => {
                data["audio_state"] = json!("unsupported");
                return Ok(());
            }
        };
        let uuid = match row.get("ZRECORDUUID") {
            Some(Cell::Blob(bytes)) if bytes.len() == 16 => {
                crate::reminders_store::uuid_text(bytes)
            }
            Some(Cell::Text(text)) => {
                let hex = text.replace('-', "");
                if hex.len() != 32 || !hex.bytes().all(|v| v.is_ascii_hexdigit()) {
                    return Ok(());
                }
                format!(
                    "{}-{}-{}-{}-{}",
                    &hex[..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..]
                )
                .to_ascii_uppercase()
            }
            _ => return Ok(()),
        };
        let file = root
            .join("Assets")
            .join(&uuid[..2])
            .join(format!("{uuid}.{extension}"));
        let before = audio_version(&file);
        let staged = stage(&mut self.assets, &file, root);
        if before != audio_version(&file) || matches!(staged, Err(Error::Conflict)) {
            return Err(Error::Conflict);
        }
        if let Err(error) = &staged {
            let missing = matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound)
                && before.is_none()
                && std::fs::symlink_metadata(&file)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
            let unavailable = matches!(error, Error::Corrupt | Error::Incomplete);
            if !missing && !unavailable {
                return Err(Error::Staging);
            }
        }
        self.tracked.0.insert(file, before);
        if let Ok(reference) = staged {
            handles.insert(
                reference.digest.clone(),
                self.assets
                    .handle(&reference.digest)
                    .ok_or(Error::Incomplete)?,
            );
            let mut reference = serde_json::to_value(reference)?;
            reference["mime_type"] = json!(mime);
            data["audio"] = reference;
            data["audio_state"] = json!("present");
        }
        Ok(())
    }

    pub fn scan(
        &mut self,
        path: &Path,
        context: &Context,
        live: bool,
        max_rows: usize,
    ) -> Result<Pending> {
        self.scan_staged(
            path,
            context,
            live,
            max_rows,
            &mut crate::assets::Store::stage,
        )
    }

    fn scan_staged(
        &mut self,
        path: &Path,
        context: &Context,
        live: bool,
        max_rows: usize,
        stage: &mut Stage<'_>,
    ) -> Result<Pending> {
        use capture::Cell;
        let reader = Snapshot::open(path, GENERATION)?;
        reader.require(&[("ZSTOREDMESSAGE", VOICEMAIL_COLUMNS)])?;
        let columns = reader.query(
            "PRAGMA table_info(ZSTOREDMESSAGE)",
            &[],
            100_000,
            Duration::from_secs(5),
        )?;
        let has = |name: &str| columns.iter().any(|row| capture::text(row, "name") == name);
        let optional = [
            "ZMAILBOXTYPE",
            "ZPROVIDER",
            "ZMESSAGETYPE",
            "ZTRANSCRIPT",
            "ZFILETYPE",
        ]
        .map(|name| {
            if has(name) {
                name.to_owned()
            } else {
                format!("NULL AS {name}")
            }
        })
        .join(",");
        let filter = if has("ZDATEDELETED") {
            " WHERE ZDATEDELETED IS NULL"
        } else {
            ""
        };
        let rows = reader.query(&format!("SELECT CASE WHEN typeof(ZRECORDUUID)='blob' THEN lower(hex(ZRECORDUUID)) ELSE COALESCE(NULLIF(ZRECORDUUID,''),CAST(Z_PK AS TEXT)) END AS id,ZRECORDUUID,ZFROM,ZDATECREATED,ZDURATION,ZISREAD,{optional} FROM ZSTOREDMESSAGE{filter}"), &[], max_rows, Duration::from_secs(5))?;
        let root = path.parent().ok_or(Error::Corrupt)?.canonicalize()?;
        self.tracked.0.clear();
        let mut handles = std::collections::BTreeMap::new();
        let mut observations = Vec::new();
        let mut remaining = SCAN_TRANSCRIPT_BYTES;
        let mut retry = false;
        for row in rows {
            let n = |key| capture::number(&row, key);
            let t = |key| capture::text(&row, key);
            if !n("ZDURATION").is_finite() || !n("ZDATECREATED").is_finite() || n("ZDURATION") < 0.0
            {
                return Err(Error::Corrupt);
            }
            let provider = t("ZPROVIDER").to_ascii_lowercase();
            let kind = if provider == "com.apple.coretelephony" {
                "carrierVoicemail"
            } else if provider.ends_with(".facetimeprovider") {
                if matches!(row.get("ZMESSAGETYPE"), Some(Cell::Integer(2))) {
                    "faceTimeVideo"
                } else {
                    "faceTimeAudio"
                }
            } else {
                "unknown"
            };
            let mailbox = match row.get("ZMAILBOXTYPE") {
                Some(Cell::Integer(0)) => "inbox",
                Some(Cell::Integer(1)) => "blocked",
                Some(Cell::Integer(2)) => "deleted",
                _ => "unknown",
            };
            let mut data = json!({"sender":t("ZFROM"),"date":capture::timestamp(n("ZDATECREATED")+978_307_200.0)?,"duration":n("ZDURATION"),"read":n("ZISREAD")!=0.0,"mailbox":mailbox,"type":kind,"transcript_state":"unavailable","audio_state":"unavailable"});
            if has("ZTRANSCRIPT") {
                data["transcript_state"] = json!("missing");
                if let Some(Cell::Blob(bytes)) = row.get("ZTRANSCRIPT")
                    && !bytes.is_empty()
                {
                    match voicemail_transcript(bytes) {
                        Ok(transcript) => {
                            data["transcript"] = transcript;
                            data["transcript_state"] = json!("present");
                        }
                        Err(_) => data["transcript_state"] = json!("unsupported"),
                    }
                }
            }
            self.audio(&row, &root, &mut data, &mut handles, stage)?;
            let id = t("id");
            let mut observation = voicemail_observation(id, &reader.generation, data, context)?;
            retry |=
                self.retain_transcript(&mut observation, &reader.generation, live, &mut remaining)?;
            observations.push(observation);
        }
        observations.sort_by(|a, b| a.native_key.cmp(&b.native_key));
        reader.finish(GENERATION)?;
        let mut pending = self.state.prepare_fingerprinted(ScanInput {
            source: "voicemail",
            generation: &reader.generation,
            records: observations,
            scope: vec!["voicemail.message".into()],
            coverage: voicemail_coverage(),
            context,
            live,
        })?;
        pending.batch.blob_handles = handles;
        self.retry = retry;
        Ok(pending)
    }

    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        self.state.commit(pending)
    }
}
