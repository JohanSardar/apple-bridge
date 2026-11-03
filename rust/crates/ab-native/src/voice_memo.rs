use crate::capture::{self, Cell, Context, Error, Pending, Result, Snapshot, State};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
const GENERATION: &str = "SELECT Z_UUID FROM Z_METADATA";

pub fn transcript(data: &[u8]) -> Result<Vec<Value>> {
    let mut value: Value = serde_json::from_slice(data)?;
    foundation_numbers(&mut value);
    let mut segments = Vec::new();
    visit(&value, 0, &mut segments);
    segments.sort_by(|a, b| {
        a["start"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&b["start"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(segments)
}
fn foundation_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if number.as_u64().is_none() => {
            if let Some(float) = number.as_f64() {
                if (0.0..18_446_744_073_709_551_616.0).contains(&float) && float.fract() == 0.0 {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let integer = float as u64;
                    *value = Value::from(integer);
                } else {
                    *value = serde_json::json!(float);
                }
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                foundation_numbers(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                foundation_numbers(value);
            }
        }
        _ => {}
    }
}
fn append(text: &str, range: &Value, segments: &mut Vec<Value>) {
    let Some(times) = range.as_array().filter(|v| v.len() == 2) else {
        return;
    };
    let (Some(start), Some(end)) = (times[0].as_f64(), times[1].as_f64()) else {
        return;
    };
    if !text.is_empty() && start.is_finite() && end.is_finite() && start >= 0.0 && end >= start {
        segments.push(json!({"text":text,"start":times[0],"end":times[1]}));
    }
}
fn visit(value: &Value, depth: usize, segments: &mut Vec<Value>) {
    if depth >= 16 || segments.len() >= 100_000 {
        return;
    }
    if let Some(fields) = value.as_object() {
        let runs = fields
            .get("runs")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let table = fields
            .get("attributeTable")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if !table.is_empty() {
            for pair in runs.as_chunks::<2>().0 {
                if let (Some(text), Some(attribute)) = (pair[0].as_str(), pair[1].as_u64())
                    && let Some(attributes) =
                        usize::try_from(attribute).ok().and_then(|i| table.get(i))
                {
                    append(text, &attributes["timeRange"], segments);
                }
            }
        }
        if let Some(text) = fields.get("text") {
            append(
                text.as_str().unwrap_or_default(),
                fields
                    .get("timeRange")
                    .or_else(|| fields.get("attributes").and_then(|v| v.get("timeRange")))
                    .unwrap_or(&Value::Null),
                segments,
            );
        }
        for (key, value) in fields {
            if key != "attributes" {
                visit(value, depth + 1, segments);
            }
        }
    }
    if let Some(values) = value.as_array() {
        let mut i = 0;
        while i < values.len() {
            if let (Some(text), Some(attributes)) = (
                values[i].as_str(),
                values.get(i + 1).filter(|v| v.is_object()),
            ) {
                append(text, &attributes["timeRange"], segments);
                i += 2;
            } else {
                visit(&values[i], depth + 1, segments);
                i += 1;
            }
        }
    }
}
pub fn embedded_transcript(bytes: &[u8]) -> Result<Vec<Value>> {
    atoms(
        &mut std::io::Cursor::new(bytes),
        0,
        u64::try_from(bytes.len()).map_err(|_| Error::Corrupt)?,
        0,
    )
}
pub fn read_transcript(path: &Path) -> Result<Vec<Value>> {
    let mut file = crate::assets::open_regular(path)?;
    let size = file.metadata()?.len();
    atoms(&mut file, 0, size, 0)
}
fn atoms(
    input: &mut (impl std::io::Read + std::io::Seek),
    start: u64,
    end: u64,
    depth: usize,
) -> Result<Vec<Value>> {
    use std::io::SeekFrom;
    if depth >= 8 {
        return Ok(Vec::new());
    }
    let mut offset = start;
    while offset <= end && end - offset >= 8 {
        input.seek(SeekFrom::Start(offset))?;
        let mut header = [0; 8];
        if input.read_exact(&mut header).is_err() {
            return Ok(Vec::new());
        }
        let mut length = u64::from(u32::from_be_bytes(
            header[..4].try_into().map_err(|_| Error::Corrupt)?,
        ));
        let mut header_size = 8;
        if length == 1 {
            let mut extended = [0; 8];
            if input.read_exact(&mut extended).is_err() {
                return Ok(Vec::new());
            }
            length = u64::from_be_bytes(extended);
            header_size = 16;
        }
        if length == 0 {
            length = end - offset;
        }
        if length < header_size || length > end - offset {
            return Ok(Vec::new());
        }
        if &header[4..] == b"tsrp" && length - header_size <= 4 * 1024 * 1024 {
            let mut body =
                vec![0; usize::try_from(length - header_size).map_err(|_| Error::Corrupt)?];
            input.read_exact(&mut body)?;
            return transcript(&body);
        }
        if [b"moov".as_slice(), b"trak".as_slice(), b"udta".as_slice()].contains(&&header[4..]) {
            let result = atoms(input, offset + header_size, offset + length, depth + 1)?;
            if !result.is_empty() {
                return Ok(result);
            }
        }
        offset += length;
    }
    Ok(Vec::new())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    device: u64,
    inode: u64,
    size: u64,
    modified: Option<std::time::SystemTime>,
    ctime: i64,
    ctime_ns: i64,
}
fn version(path: &Path) -> Option<Version> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| Version {
        device: m.dev(),
        inode: m.ino(),
        size: m.len(),
        modified: m.modified().ok(),
        ctime: m.ctime(),
        ctime_ns: m.ctime_nsec(),
    })
}
fn local_audio(root: &Path, value: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let path = if Path::new(value).is_absolute() {
        Path::new(value).to_path_buf()
    } else {
        root.join(value)
    };
    let path = path.canonicalize().unwrap_or_else(|_| {
        let mut output = std::path::PathBuf::new();
        for part in path.components() {
            match part {
                Component::ParentDir => {
                    output.pop();
                }
                Component::CurDir => {}
                other => output.push(other.as_os_str()),
            }
        }
        output
    });
    (path.starts_with(root)
        && path != root
        && path
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("m4a")))
    .then_some(path)
}
#[derive(Debug)]
pub struct Reader {
    state: State,
    assets: crate::assets::Store,
    cache: std::collections::BTreeMap<std::path::PathBuf, (Version, Value, Vec<Value>)>,
    tracked: std::collections::BTreeSet<std::path::PathBuf>,
}
impl Reader {
    pub fn open(staging: &Path) -> Result<Self> {
        Ok(Self {
            state: State::default(),
            assets: crate::assets::Store::open(staging, "voice-memo-blob:")?,
            cache: std::collections::BTreeMap::new(),
            tracked: std::collections::BTreeSet::new(),
        })
    }
    pub fn tracked_audio(&self) -> impl Iterator<Item = &Path> {
        self.tracked.iter().map(std::path::PathBuf::as_path)
    }
    pub fn read_blob(&self, handle: &str, offset: u64, length: usize) -> Result<Value> {
        self.assets.read(handle, offset, length)
    }
    fn audio(&mut self, file: &Path, root: &Path) -> Result<(Value, Vec<Value>)> {
        self.tracked.insert(file.into());
        let current = version(file).ok_or(Error::Incomplete)?;
        if let Some((prior, reference, segments)) = self.cache.get(file)
            && *prior == current
        {
            return Ok((reference.clone(), segments.clone()));
        }
        let reference = self.assets.stage(file, root)?;
        let copied = self
            .assets
            .file(&reference.digest)
            .ok_or(Error::Incomplete)?;
        let segments = read_transcript(copied).unwrap_or_default();
        let reference = serde_json::to_value(reference)?;
        self.cache
            .insert(file.into(), (current, reference.clone(), segments.clone()));
        Ok((reference, segments))
    }
    fn recording(
        &mut self,
        row: &capture::Row,
        root: &Path,
        handles: &mut std::collections::BTreeMap<String, String>,
    ) -> Result<(String, Value)> {
        let n = |k| capture::number(row, k);
        let t = |k| capture::text(row, k);
        if !n("ZDURATION").is_finite() || !n("ZDATE").is_finite() || n("ZDURATION") < 0.0 {
            return Err(Error::Corrupt);
        }
        let deleted = !matches!(row.get("ZEVICTIONDATE"), None | Some(Cell::Null));
        let mut data = json!({"title":t("ZCUSTOMLABEL"),"date":capture::timestamp(n("ZDATE")+978_307_200.0)?,"duration":n("ZDURATION"),"folder":t("folder_id"),"recently_deleted":deleted,"transcript_state":"unavailable","audio_state":"unavailable"});
        if let Some(relative) = row.get("ZPATH").and_then(Cell::text)
            && let Some(file) = local_audio(root, relative)
            && let Ok((reference, segments)) = self.audio(&file, root)
        {
            let digest = reference["digest"].as_str().ok_or(Error::Corrupt)?;
            handles.insert(
                digest.into(),
                self.assets.handle(digest).ok_or(Error::Incomplete)?,
            );
            data["audio"] = reference;
            data["audio_state"] = json!("present");
            if !segments.is_empty() {
                data["transcript"] = json!(segments);
                data["transcript_state"] = json!("present");
            }
        }
        Ok((t("id").into(), data))
    }
    pub fn scan(
        &mut self,
        path: &Path,
        context: &Context,
        live: bool,
        max_rows: usize,
    ) -> Result<Pending> {
        let reader = Snapshot::open(path, GENERATION)?;
        reader.require(&[(
            "ZCLOUDRECORDING",
            &[
                "Z_PK",
                "ZUNIQUEID",
                "ZCUSTOMLABEL",
                "ZDATE",
                "ZDURATION",
                "ZEVICTIONDATE",
                "ZFOLDER",
            ],
        )])?;
        let columns = reader.query(
            "PRAGMA table_info(ZCLOUDRECORDING)",
            &[],
            100_000,
            Duration::from_secs(5),
        )?;
        let asset_column = if columns.iter().any(|r| capture::text(r, "name") == "ZPATH") {
            ",ZPATH"
        } else {
            ""
        };
        let rows=reader.query(&format!("SELECT COALESCE(NULLIF(ZUNIQUEID,''),CAST(Z_PK AS TEXT)) AS id,CAST(ZFOLDER AS TEXT) AS folder_id,ZUNIQUEID,ZCUSTOMLABEL,ZDATE,ZDURATION,ZEVICTIONDATE,ZFOLDER{asset_column} FROM ZCLOUDRECORDING"),&[],max_rows,Duration::from_secs(5))?;
        let root = path.parent().ok_or(Error::Corrupt)?.canonicalize()?;
        let mut handles = std::collections::BTreeMap::new();
        let mut current = Vec::new();
        for row in rows {
            current.push(self.recording(&row, &root, &mut handles)?);
        }
        current.sort_by(|a, b| a.0.cmp(&b.0));
        let mut observations = Vec::new();
        for (id, data) in &current {
            observations.push(capture::upsert(
                "voice_memos.recording",
                id,
                Some(format!("voice_memos/{}/{id}", reader.generation)),
                data.clone(),
                context,
            )?);
        }
        let tables = reader.query(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name='ZFOLDER'",
            &[],
            2,
            Duration::from_secs(5),
        )?;
        if !tables.is_empty() {
            let rows = reader.query(
                "SELECT Z_PK,ZENCRYPTEDNAME FROM ZFOLDER",
                &[],
                max_rows,
                Duration::from_secs(5),
            )?;
            let mut folders = Vec::new();
            for row in rows {
                let Some(Cell::Integer(key)) = row.get("Z_PK") else {
                    return Err(Error::Corrupt);
                };
                let id = key.to_string();
                let count = current
                    .iter()
                    .filter(|(_, v)| v["folder"] == id && v["recently_deleted"] != true)
                    .count();
                folders.push((
                    id,
                    json!({"name":capture::text(&row,"ZENCRYPTEDNAME"),"count":count}),
                ));
            }
            folders.sort_by(|a, b| a.0.cmp(&b.0));
            for (id, data) in folders {
                observations.push(capture::upsert(
                    "voice_memos.folder",
                    &id,
                    Some(format!("voice_memos/folder/{}/{id}", reader.generation)),
                    data,
                    context,
                )?);
            }
        }
        reader.finish(GENERATION)?;
        let mut pending=self.state.prepare(capture::ScanInput{source:"voice_memos",generation:&reader.generation,records:observations,scope:vec!["voice_memos.recording".into(),"voice_memos.folder".into()],coverage:json!({"fields":"metadata_and_local_assets","transcript":"embedded_if_present","audio":"local_if_present"}),context,live})?;
        pending.batch.blob_handles = handles;
        Ok(pending)
    }
    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        self.state.commit(pending)
    }
}
