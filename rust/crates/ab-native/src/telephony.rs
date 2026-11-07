use crate::capture::{self, Context, Error, Pending, Result, ScanInput, Snapshot, State};
use serde_json::json;
use std::{path::Path, time::Duration};
const GENERATION: &str = "SELECT Z_UUID FROM Z_METADATA";

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
    let root = plist::archive_root(bytes)?;
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

#[must_use]
pub fn voicemail_coverage() -> serde_json::Value {
    json!({"fields":"metadata_and_local_assets","transcript":"archived_if_present","audio":"local_if_present"})
}

#[derive(Debug)]
pub struct VoicemailReader {
    state: State,
    assets: crate::assets::Store,
    tracked: std::collections::BTreeSet<std::path::PathBuf>,
}

impl VoicemailReader {
    pub fn open(staging: &Path) -> Result<Self> {
        Ok(Self {
            state: State::default(),
            assets: crate::assets::Store::open(staging, "voicemail-blob:")?,
            tracked: std::collections::BTreeSet::new(),
        })
    }

    pub fn tracked_audio(&self) -> impl Iterator<Item = &Path> {
        self.tracked.iter().map(std::path::PathBuf::as_path)
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
        self.tracked.insert(file.clone());
        if let Ok(reference) = self.assets.stage(&file, root) {
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
        self.tracked.clear();
        let mut handles = std::collections::BTreeMap::new();
        let mut observations = Vec::new();
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
            self.audio(&row, &root, &mut data, &mut handles)?;
            let id = t("id");
            observations.push(capture::upsert(
                "voicemail.message",
                id,
                Some(format!("voicemail/{}/{id}", reader.generation)),
                data,
                context,
            )?);
        }
        observations.sort_by(|a, b| a.native_key.cmp(&b.native_key));
        reader.finish(GENERATION)?;
        let mut pending = self.state.prepare(ScanInput {
            source: "voicemail",
            generation: &reader.generation,
            records: observations,
            scope: vec!["voicemail.message".into()],
            coverage: voicemail_coverage(),
            context,
            live,
        })?;
        pending.batch.blob_handles = handles;
        Ok(pending)
    }

    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        self.state.commit(pending)
    }
}
