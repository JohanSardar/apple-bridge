use crate::{
    body_digest,
    capture::{self, Context, Error, Observation, Result, Snapshot},
    native_json, notes,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};
pub(crate) type Row = BTreeMap<String, Value>;
pub(crate) fn text<'a>(row: &'a Row, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}
pub(crate) fn number(row: &Row, key: &str) -> f64 {
    row.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}
pub(crate) fn integer(row: &Row, key: &str) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let value = number(row, key) as i64;
    value
}
#[derive(Debug)]
pub(crate) struct Database {
    snapshot: Snapshot,
}
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            snapshot: Snapshot::open_with_busy_timeout(
                path,
                "SELECT Z_UUID FROM Z_METADATA",
                Duration::from_secs(1),
            )?,
        })
    }
    pub fn query(&self, sql: &str, values: &[String]) -> Result<Vec<Row>> {
        let mut result = Vec::new();
        for row in self
            .snapshot
            .query_notes(sql, values, 1_000_000, Duration::from_secs(30))?
        {
            let mut current = Row::new();
            for (key, value) in row {
                let value = match value {
                    capture::Cell::Integer(_) | capture::Cell::Real(_) => json!(value.number()),
                    capture::Cell::Text(value) => json!(value),
                    capture::Cell::Blob(value) if !value.is_empty() => {
                        json!(STANDARD.encode(value))
                    }
                    _ => continue,
                };
                current.insert(key, value);
            }
            result.push(current);
        }
        Ok(result)
    }
    pub fn columns(&self, table: &str) -> Result<BTreeSet<String>> {
        Ok(self
            .query(&format!("PRAGMA table_info({table})"), &[])?
            .iter()
            .map(|row| text(row, "name").into())
            .collect())
    }
    pub fn finish(&self) -> Result<()> {
        self.snapshot.finish("SELECT Z_UUID FROM Z_METADATA")
    }
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Facts {
    #[serde(default)]
    pub mime: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub supports: BTreeMap<String, bool>,
    #[serde(default)]
    pub derivatives: BTreeMap<String, DerivativeFacts>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DerivativeFacts {
    pub status: String,
    #[serde(default)]
    pub variants: BTreeMap<String, Variant>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Variant {
    pub bytes_base64: String,
    pub mime: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}
#[derive(Debug, Clone)]
pub(crate) struct Attachment {
    pub metadata: Value,
    pub file: Option<PathBuf>,
    pub payload: Option<Vec<u8>>,
    pub uti: String,
}
#[derive(Debug)]
pub(crate) struct StoreSnapshot {
    pub generation: String,
    pub observations: Vec<Observation>,
    pub attachments: BTreeMap<String, Attachment>,
    pub raw_bodies: BTreeMap<String, Vec<u8>>,
}
fn component(value: &str) -> bool {
    !value.is_empty() && !matches!(value, "." | "..") && !value.contains(['/', '\0'])
}
fn belongs(db: &Database, row: &Row, note: i64) -> bool {
    let mut current = row.clone();
    let mut seen = BTreeSet::new();
    for _ in 0..16 {
        if ["ZNOTE", "ZNOTE1"]
            .iter()
            .any(|key| integer(&current, key) == note)
        {
            return true;
        }
        let parent = integer(&current, "ZPARENTATTACHMENT");
        if parent <= 0 || !seen.insert(parent) {
            return false;
        }
        let Ok(rows) = db.query(
            "SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=?",
            &[parent.to_string()],
        ) else {
            return false;
        };
        let Some(row) = rows.into_iter().next() else {
            return false;
        };
        current = row;
    }
    false
}
fn attachment_candidates(row: &Row, media: &Row, account: &str, root: &Path) -> Vec<PathBuf> {
    let id = text(row, "ZIDENTIFIER");
    let filename = media
        .get("ZFILENAME")
        .or_else(|| row.get("ZFILENAME"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let bases = if component(account) {
        vec![root.join("Accounts").join(account), root.into()]
    } else {
        vec![root.into()]
    };
    let mut candidates = Vec::new();
    let identifier = text(media, "ZIDENTIFIER");
    if component(identifier) && component(filename) {
        let generation = [
            "ZGENERATION",
            "ZGENERATION1",
            "ZFALLBACKIMAGEGENERATION",
            "ZFALLBACKPDFGENERATION",
            "ZPAPERBUNDLEGENERATION",
        ]
        .iter()
        .map(|key| text(media, key))
        .find(|v| component(v));
        for base in &bases {
            let directory = base.join("Media").join(identifier);
            if let Some(generation) = generation {
                candidates.push(directory.join(generation).join(filename));
            }
            candidates.push(directory.join(filename));
        }
    }
    if component(id) {
        for base in &bases {
            let directory = base.join("FallbackImages");
            let generation = text(row, "ZFALLBACKIMAGEGENERATION");
            if component(generation) {
                for extension in ["png", "jpeg", "jpg"] {
                    candidates.push(
                        directory
                            .join(id)
                            .join(generation)
                            .join(format!("FallbackImage.{extension}")),
                    );
                }
            }
            for extension in ["png", "jpeg", "jpg"] {
                candidates.push(directory.join(format!("{id}.{extension}")));
            }
        }
    }
    candidates
}
fn attachment(
    db: &Database,
    row: &Row,
    account: &str,
    root: &Path,
    facts: &Facts,
) -> Result<Attachment> {
    let id = text(row, "ZIDENTIFIER");
    let uti = row
        .get("ZTYPEUTI1")
        .or_else(|| row.get("ZTYPEUTI"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload = if uti == "com.apple.notes.table" {
        ["ZMERGEABLEDATA1", "ZMERGEABLEDATA"]
            .iter()
            .filter_map(|key| row.get(*key).and_then(Value::as_str))
            .find_map(|v| STANDARD.decode(v).ok())
    } else {
        None
    };
    let media = integer(row, "ZMEDIA");
    let media = if media > 0 {
        db.query(
            "SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=?",
            &[media.to_string()],
        )?
        .into_iter()
        .next()
        .unwrap_or_default()
    } else {
        Row::new()
    };
    let filename = media
        .get("ZFILENAME")
        .or_else(|| row.get("ZFILENAME"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = if component(filename) {
        filename
    } else if component(id) {
        id
    } else {
        "Attachment"
    };
    let candidates = attachment_candidates(row, &media, account, root);
    let canonical = root.canonicalize()?;
    let file = candidates.into_iter().find(|path| {
        path.canonicalize()
            .is_ok_and(|file| file.starts_with(&canonical) && file != canonical)
            && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
    });
    let inline = uti == "com.apple.notes.inlinetextattachment.hashtag";
    let local = file.is_some() || payload.is_some();
    let mut metadata = json!({"id":id,"uti":uti,"name":name,"state":if inline{"inline"}else if local{"local"}else{"unavailable"}});
    if !local && !inline {
        metadata["reason"] = json!("attachment_bytes_unavailable");
    }
    let alternate = text(row, "ZALTTEXT");
    if !alternate.is_empty() {
        metadata["alternate_text"] = json!(alternate);
    }
    let mime = facts
        .mime
        .get(uti)
        .ok_or_else(|| Error::FactsRequired(format!("mime:{uti}")))?;
    if let Some(mime) = mime {
        metadata["mime"] = json!(mime);
    }
    Ok(Attachment {
        metadata,
        file,
        payload,
        uti: uti.into(),
    })
}
#[allow(clippy::float_cmp)]
fn flags(row: &Row, folders: &BTreeMap<i64, Row>) -> (bool, bool) {
    let mut current = row;
    let mut seen = BTreeSet::new();
    let mut recent = false;
    let mut read_only = false;
    for _ in 0..128 {
        if !seen.insert(integer(current, "Z_PK")) {
            return (recent, true);
        }
        recent |= number(current, "ZFOLDERTYPE") == 1.0;
        read_only |= recent || current.contains_key("ZSMARTFOLDERQUERYJSON");
        let parent = integer(current, "ZPARENT");
        if parent <= 0 {
            return (recent, read_only);
        }
        let Some(next) = folders.get(&parent) else {
            return (recent, true);
        };
        current = next;
    }
    (recent, true)
}
fn account(row: &Row, accounts: &BTreeMap<i64, Row>) -> Row {
    if let Some(owner) = row
        .get("ZACCOUNT8")
        .and_then(Value::as_f64)
        .and_then(|_| accounts.get(&integer(row, "ZACCOUNT8")))
    {
        return owner.clone();
    }
    let candidates = row
        .keys()
        .filter(|k| k.starts_with("ZACCOUNT"))
        .filter_map(|key| accounts.get(&integer(row, key)))
        .collect::<Vec<_>>();
    if candidates.len() == 1 {
        candidates[0].clone()
    } else {
        Row::new()
    }
}
fn reference(key: &str) -> Value {
    json!({"$ref":{"collection":"notes.folder","native_key":key}})
}
fn observation(
    collection: &str,
    row: &Row,
    data: Value,
    uuid: &str,
    context: &Context,
) -> Result<Observation> {
    let key = text(row, "ZIDENTIFIER");
    if key.is_empty() {
        return Err(Error::Corrupt);
    }
    let kind = if collection == "notes.folder" {
        "ICFolder"
    } else {
        "ICNote"
    };
    capture::upsert(
        collection,
        key,
        Some(format!(
            "x-coredata://{uuid}/{kind}/p{}",
            integer(row, "Z_PK")
        )),
        data,
        context,
    )
}
fn unsupported(block: &Value) -> bool {
    block.get("unsupported").is_some()
        || block["children"]
            .as_array()
            .is_some_and(|v| v.iter().any(unsupported))
        || block["rows"].as_array().is_some_and(|rows| {
            rows.iter()
                .filter_map(Value::as_array)
                .flatten()
                .any(|cell| {
                    cell["blocks"]
                        .as_array()
                        .is_some_and(|blocks| blocks.iter().any(unsupported))
                })
        })
}
fn full_body(
    db: &Database,
    row: &Row,
    bytes: &[u8],
    owner: &Row,
    root: &Path,
    facts: &Facts,
) -> Result<(notes::NotesBody, Vec<Attachment>)> {
    let mut decoded = notes::decode(bytes, &BTreeMap::new())?;
    let mut known = BTreeMap::new();
    let mut tables = BTreeMap::new();
    for _ in 0..8 {
        let missing = decoded
            .attachments
            .iter()
            .filter(|id| !known.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            break;
        }
        for ids in missing.chunks(128) {
            let placeholders = vec!["?"; ids.len()].join(",");
            let rows = db.query(
                &format!(
                    "SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE ZIDENTIFIER IN ({placeholders})"
                ),
                ids,
            )?;
            for value in rows {
                if !belongs(db, &value, integer(row, "Z_PK")) {
                    continue;
                }
                let id = text(&value, "ZIDENTIFIER").to_owned();
                for field in ["ZMERGEABLEDATA1", "ZMERGEABLEDATA"] {
                    if let Some(data) = value
                        .get(field)
                        .and_then(Value::as_str)
                        .and_then(|v| STANDARD.decode(v).ok())
                    {
                        tables.insert(id.clone(), data);
                        break;
                    }
                }
                known.insert(id, value);
            }
        }
        decoded = notes::decode(bytes, &tables)?;
        if missing.iter().all(|id| !known.contains_key(id)) {
            break;
        }
    }
    let mut attachments = Vec::new();
    for id in &decoded.attachments {
        if let Some(row) = known.get(id) {
            attachments.push(attachment(
                db,
                row,
                text(owner, "ZIDENTIFIER"),
                root,
                facts,
            )?);
        } else {
            attachments.push(Attachment{metadata:json!({"id":id,"state":"unavailable","reason":"attachment_metadata_unavailable"}),file:None,payload:None,uti:String::new()});
        }
    }
    Ok((decoded, attachments))
}
fn data_value(row: &Row, key: &str, default: Value) -> Value {
    row.get(key).cloned().unwrap_or(default)
}
pub(crate) fn generation(path: &Path) -> Result<String> {
    let db = Database::open(path)?;
    let uuid = db
        .query("SELECT Z_UUID FROM Z_METADATA", &[])?
        .first()
        .map(|r| text(r, "Z_UUID").to_owned())
        .filter(|v| !v.is_empty())
        .ok_or(Error::Corrupt)?;
    Ok(format!("notes-{uuid}"))
}

pub(crate) fn read(
    db: &Database,
    path: &Path,
    keys: Option<&BTreeSet<String>>,
    metadata_only: bool,
    facts: &Facts,
    context: &Context,
) -> Result<StoreSnapshot> {
    let uuid = db
        .query("SELECT Z_UUID FROM Z_METADATA", &[])?
        .first()
        .map(|r| text(r, "Z_UUID").to_owned())
        .filter(|v| !v.is_empty())
        .ok_or(Error::Corrupt)?;
    let entities = db.query("SELECT Z_ENT,Z_NAME FROM Z_PRIMARYKEY", &[])?;
    let entity = |name: &str| {
        entities
            .iter()
            .find(|r| text(r, "Z_NAME") == name)
            .map(|r| integer(r, "Z_ENT"))
            .ok_or(Error::Unsupported)
    };
    let note_entity = entity("ICNote")?;
    let folder_entity = entity("ICFolder")?;
    let columns = db.columns("ZICCLOUDSYNCINGOBJECT")?;
    if !columns.contains("ZISPASSWORDPROTECTED") {
        return Err(Error::Unsupported);
    }
    let selected = keys
        .map(|keys| keys.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let filter = if keys.is_none() {
        String::new()
    } else {
        format!(
            " AND o.ZIDENTIFIER IN ({})",
            if selected.is_empty() {
                "NULL".into()
            } else {
                vec!["?"; selected.len()].join(",")
            }
        )
    };
    let excluded = format!(
        "WITH RECURSIVE excluded(pk) AS (SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT={folder_entity} AND (ZFOLDERTYPE=1 OR COALESCE(ZMARKEDFORDELETION,0)<>0) UNION SELECT f.Z_PK FROM ZICCLOUDSYNCINGOBJECT f JOIN excluded p ON f.ZPARENT=p.pk WHERE f.Z_ENT={folder_entity}) "
    );
    let notes=db.query(&format!("{excluded}SELECT o.*,d.ZDATA AS BODY FROM ZICCLOUDSYNCINGOBJECT o LEFT JOIN ZICNOTEDATA d ON d.Z_PK=o.ZNOTEDATA WHERE o.Z_ENT={note_entity} AND COALESCE(o.ZMARKEDFORDELETION,0)=0 AND o.ZFOLDER NOT IN (SELECT pk FROM excluded){filter} ORDER BY o.Z_PK"),&selected)?;
    let folder_rows=db.query(&format!("SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT={folder_entity} AND COALESCE(ZMARKEDFORDELETION,0)=0 ORDER BY Z_PK"),&[])?;
    let accounts=db.query("SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE Z_ENT=(SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME='ICAccount') AND COALESCE(ZMARKEDFORDELETION,0)=0",&[])?.into_iter().map(|row|(integer(&row,"Z_PK"),row)).collect::<BTreeMap<_,_>>();
    let folders = folder_rows
        .iter()
        .map(|row| (integer(row, "Z_PK"), row.clone()))
        .collect::<BTreeMap<_, _>>();
    let projection = Projection {
        db,
        root: path.parent().ok_or(Error::Corrupt)?,
        uuid: &uuid,
        columns: &columns,
        accounts: &accounts,
        folders: &folders,
        facts,
        context,
    };
    let mut snapshot = StoreSnapshot {
        generation: format!("notes-{uuid}"),
        observations: Vec::new(),
        attachments: BTreeMap::new(),
        raw_bodies: BTreeMap::new(),
    };
    for row in &folder_rows {
        snapshot.observations.push(projection.folder(row)?);
    }
    for row in notes {
        if let Some(observation) = projection.note(&row, metadata_only, &mut snapshot)? {
            snapshot.observations.push(observation);
        }
    }
    Ok(snapshot)
}
#[derive(Debug)]
struct Projection<'a> {
    db: &'a Database,
    root: &'a Path,
    uuid: &'a str,
    columns: &'a BTreeSet<String>,
    accounts: &'a BTreeMap<i64, Row>,
    folders: &'a BTreeMap<i64, Row>,
    facts: &'a Facts,
    context: &'a Context,
}
impl Projection<'_> {
    fn folder(&self, row: &Row) -> Result<Observation> {
        let owner = account(row, self.accounts);
        let (recent, read_only) = flags(row, self.folders);
        let mut data = json!({"title":data_value(row,"ZTITLE2",json!("")),"account":data_value(&owner,"ZIDENTIFIER",json!("")),"recently_deleted":recent,"read_only":read_only});
        if let Some(name) = owner.get("ZNAME") {
            data["account_name"] = name.clone();
        }
        if row.contains_key("ZSMARTFOLDERQUERYJSON") {
            data["smart"] = json!(true);
            data["smart_filter_status"] = json!("unsupported");
        }
        if let Some(parent) = self
            .folders
            .get(&integer(row, "ZPARENT"))
            .and_then(|r| r.get("ZIDENTIFIER"))
            .and_then(Value::as_str)
        {
            data["parent_id"] = reference(parent);
        }
        observation("notes.folder", row, data, self.uuid, self.context)
    }
    fn note(
        &self,
        row: &Row,
        metadata_only: bool,
        snapshot: &mut StoreSnapshot,
    ) -> Result<Option<Observation>> {
        let Some(folder) = self.folders.get(&integer(row, "ZFOLDER")) else {
            return Ok(None);
        };
        let folder_key = text(folder, "ZIDENTIFIER");
        let key = text(row, "ZIDENTIFIER");
        if folder_key.is_empty() || key.is_empty() {
            return Ok(None);
        }
        let locked = number(row, "ZISPASSWORDPROTECTED") != 0.0;
        let (recent, read_only) = flags(folder, self.folders);
        let mut data = json!({"folder_id":reference(folder_key),"title":data_value(row,"ZTITLE1",json!("")),"locked":locked,"plain_text":"","attachments":[],"content_status":if locked{"locked"}else{"offloaded"},"editable":"none","recently_deleted":recent,"read_only":read_only});
        for (native, field) in [
            ("ZCREATIONDATE1", "created_at"),
            ("ZMODIFICATIONDATE1", "modified_at"),
        ] {
            if row.contains_key(native) {
                data[field] = json!(capture::timestamp(number(row, native) + 978_307_200.0)?);
            }
        }
        if let Some(pinned) = row.get("ZISPINNED").and_then(Value::as_f64)
            && [0.0, 1.0].contains(&pinned)
        {
            data["pinned"] = json!(pinned != 0.0);
        }
        let digest = self.body(row, &mut data, metadata_only, snapshot)?;
        let folder_state =
            json!({"key":folder_key,"read_only":read_only,"recently_deleted":recent});
        data["native_fingerprint"] = json!(self.fingerprint(row, &data, &digest, &folder_state)?);
        observation("notes.note", row, data, self.uuid, self.context).map(Some)
    }
    fn body(
        &self,
        row: &Row,
        data: &mut Value,
        metadata_only: bool,
        snapshot: &mut StoreSnapshot,
    ) -> Result<String> {
        if data["locked"] == true {
            return Ok(String::new());
        }
        let Some(bytes) = row
            .get("BODY")
            .and_then(Value::as_str)
            .and_then(|b| STANDARD.decode(b).ok())
            .filter(|b| !b.is_empty())
        else {
            return Ok(String::new());
        };
        let digest = body_digest(&bytes);
        if metadata_only {
            if let Ok(text) = notes::plain_text(&bytes) {
                data["plain_text"] = json!(text);
                data["content_status"] = json!("partial");
            }
            data["body_status"] = json!("pending");
        } else {
            let folder = self
                .folders
                .get(&integer(row, "ZFOLDER"))
                .ok_or(Error::Corrupt)?;
            let owner = account(folder, self.accounts);
            match full_body(self.db, row, &bytes, &owner, self.root, self.facts) {
                Ok((decoded, assets)) => Self::decoded(row, data, snapshot, &decoded, assets),
                Err(error @ Error::FactsRequired(_)) => return Err(error),
                Err(_) => {
                    data["plain_text"] = json!("");
                    data["attachments"] = json!([]);
                    data["editable"] = json!("none");
                    data["content_status"] = json!("undecoded");
                    data["raw_body_digest"] = json!(digest);
                    snapshot
                        .raw_bodies
                        .insert(text(row, "ZIDENTIFIER").into(), bytes);
                }
            }
        }
        Ok(digest)
    }
    fn decoded(
        row: &Row,
        data: &mut Value,
        snapshot: &mut StoreSnapshot,
        decoded: &notes::NotesBody,
        assets: Vec<Attachment>,
    ) {
        data["plain_text"] = json!(decoded.text);
        data["blocks"] = json!(decoded.blocks);
        data["html"] = json!(decoded.html);
        data["content_status"] = json!("complete");
        data["block_status"] = json!(if decoded.blocks.iter().any(unsupported) {
            "partial"
        } else {
            "complete"
        });
        data["editable"] = json!(
            if data["read_only"] != true && !decoded.rich && decoded.lossless {
                "plain"
            } else {
                "none"
            }
        );
        data["round_trip_status"] = json!(if decoded.lossless {
            "unproven"
        } else {
            "unsupported"
        });
        let mut tags = BTreeSet::new();
        let mut values = Vec::new();
        let key = text(row, "ZIDENTIFIER");
        for asset in assets {
            if asset.uti == "com.apple.notes.inlinetextattachment.hashtag"
                && let Some(text) = asset.metadata["alternate_text"]
                    .as_str()
                    .filter(|s| s.starts_with('#'))
            {
                tags.insert(text.to_owned());
            }
            let id = asset.metadata["id"].as_str().unwrap_or_default();
            snapshot
                .attachments
                .insert(format!("{key}:{id}"), asset.clone());
            values.push(asset.metadata);
        }
        data["attachments"] = json!(values);
        data["tags"] = json!(tags);
    }
    fn fingerprint(
        &self,
        row: &Row,
        data: &Value,
        digest: &str,
        folder_state: &Value,
    ) -> Result<String> {
        let mut value = json!({"key":text(row,"ZIDENTIFIER"),"title":data["title"],"locked":data["locked"],"pinned":number(row,"ZISPINNED"),"body_digest":digest,"generation":self.uuid,"folder_state":folder_state});
        let owners = ["ZNOTE", "ZNOTE1"]
            .iter()
            .filter(|v| self.columns.contains(**v))
            .map(|v| format!("{v}=?"))
            .collect::<Vec<_>>();
        if !owners.is_empty() && self.columns.contains("Z_OPT") {
            let params = vec![integer(row, "Z_PK").to_string(); owners.len()];
            let dependencies=self.db.query(&format!("WITH RECURSIVE owned(pk) AS (SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE {} UNION SELECT a.Z_PK FROM ZICCLOUDSYNCINGOBJECT a JOIN owned p ON a.ZPARENTATTACHMENT=p.pk) SELECT Z_PK,Z_OPT FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK IN (SELECT pk FROM owned) OR Z_PK IN (SELECT ZMEDIA FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK IN (SELECT pk FROM owned)) ORDER BY Z_PK",owners.join(" OR ")),&params)?;
            value["dependencies"] = json!(dependencies);
        }
        Ok(body_digest(&native_json::foundation_data(&value)?))
    }
}
