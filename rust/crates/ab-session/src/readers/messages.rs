use super::{CaptureReader, ScanContext, reader_error};
use crate::Error;
use ab_native::messages;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const SCOPE: [&str; 5] = [
    "messages.chat",
    "messages.handle",
    "messages.message",
    "messages.reaction",
    "messages.attachment",
];
const RECONCILE_EVERY: Duration = Duration::from_mins(15);
const BLOB_CHUNK: usize = 128 * 1024;
const MAX_IDLE_STEPS: u32 = 8;

#[derive(Debug)]
struct Known {
    digest: String,
    size: u64,
    declared: i64,
}

#[derive(Debug)]
pub struct MessagesReader {
    root: PathBuf,
    database: PathBuf,
    home: PathBuf,
    reader: Option<messages::Reader>,
    blobs: BTreeMap<String, Known>,
    reconciled_at: Option<Instant>,
    pending: bool,
}

impl MessagesReader {
    #[must_use]
    pub fn new(root: PathBuf, home: PathBuf) -> Self {
        Self {
            database: root.join("chat.db"),
            root,
            home,
            reader: None,
            blobs: BTreeMap::new(),
            reconciled_at: None,
            pending: false,
        }
    }

    fn digest(reader: &messages::Reader, guid: &str) -> Result<(String, u64), Error> {
        let mut hash = Sha256::new();
        let mut offset = 0_u64;
        loop {
            let bytes = reader
                .read_blob(guid, offset, BLOB_CHUNK)
                .map_err(reader_error)?;
            hash.update(&bytes);
            offset += u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
            if bytes.len() < BLOB_CHUNK {
                return Ok((format!("{:x}", hash.finalize()), offset));
            }
        }
    }

    fn attach(&mut self, data: &mut Value, guid: &str, handles: &mut BTreeMap<String, String>) {
        let Some(reader) = self.reader.as_ref() else {
            return;
        };
        if data.pointer("/blob/state").and_then(Value::as_str) != Some("present") {
            return;
        }
        let declared = data["size"].as_i64().unwrap_or(-1);
        let known = match self.blobs.get(guid) {
            Some(known) if known.declared == declared => Some((known.digest.clone(), known.size)),
            _ => None,
        };
        let found = match known {
            Some(known) => Ok(known),
            None => Self::digest(reader, guid),
        };
        let Ok((digest, size)) = found else {
            data["blob"] = json!({"state":"missing"});
            return;
        };
        self.blobs.insert(
            guid.to_owned(),
            Known {
                digest: digest.clone(),
                size,
                declared,
            },
        );
        data["blob"] = json!({"state":"present","digest":digest});
        handles.insert(digest, guid.to_owned());
    }

    fn frame(
        &mut self,
        context: &ScanContext,
        batches: &[messages::Batch],
    ) -> Result<Option<Value>, Error> {
        let at = context.observed_at()?;
        let generation = self
            .reader
            .as_ref()
            .map(|reader| reader.generation.clone())
            .ok_or(Error::Conflict)?;
        let mut rows: BTreeMap<(String, String), Value> = BTreeMap::new();
        let mut order: Vec<(String, String)> = Vec::new();
        for batch in batches {
            for observation in &batch.observations {
                let mut row: Value =
                    serde_json::from_slice(&observation.bytes().map_err(reader_error)?)?;
                let key = (
                    observation.collection.clone(),
                    observation.native_key.clone(),
                );
                row["observed_at"] = Value::String(at.clone());
                row["revalidated"] = Value::Bool(true);
                row["scoped_alias"] = Value::String(format!(
                    "{}/{}",
                    observation.collection, observation.native_key
                ));
                if rows.insert(key.clone(), row).is_none() {
                    order.push(key);
                } else if let Some(position) = order.iter().position(|k| *k == key) {
                    let moved = order.remove(position);
                    order.push(moved);
                }
            }
        }
        let mut handles = BTreeMap::new();
        for (collection, key) in &order {
            if collection != "messages.attachment" {
                continue;
            }
            if let Some(row) = rows.get_mut(&(collection.clone(), key.clone())) {
                let mut data = row["data"].take();
                self.attach(&mut data, key, &mut handles);
                row["data"] = data;
            }
        }
        let mut aliases: BTreeMap<String, Value> = BTreeMap::new();
        for row in rows.values() {
            collect(&row["data"], &at, &mut aliases);
        }
        let mut observations: Vec<Value> = aliases.into_values().collect();
        observations.extend(order.iter().filter_map(|key| rows.remove(key)));
        let complete = batches.iter().any(|batch| batch.complete);
        if observations.is_empty() && !complete {
            return Ok(None);
        }
        let live = !batches.is_empty() && batches.iter().all(|batch| batch.live);
        Ok(Some(json!({
            "source": "messages",
            "generation": generation,
            "checkpoint": context.checkpoint,
            "scan_id": context.scan_id,
            "scan_started_at": at,
            "scope": SCOPE,
            "completion_evidence": if complete { "complete" } else { "" },
            "observations": observations,
            "blob_handles": handles,
            "live": live && !complete,
        })))
    }
}

fn collect(value: &Value, at: &str, aliases: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(fields) => {
            if let Some(reference) = fields.get("$ref") {
                if let (Some(collection), Some(key)) = (
                    reference["collection"].as_str(),
                    reference["native_key"].as_str(),
                ) {
                    aliases.insert(
                        format!("{collection}/{key}"),
                        json!({"kind":"alias","collection":collection,"native_key":key,
                               "scoped_alias":format!("{collection}/{key}"),"observed_at":at}),
                    );
                }
            } else {
                for field in fields.values() {
                    collect(field, at, aliases);
                }
            }
        }
        Value::Array(values) => {
            for field in values {
                collect(field, at, aliases);
            }
        }
        _ => {}
    }
}

impl CaptureReader for MessagesReader {
    fn source(&self) -> &'static str {
        "messages"
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let name = self.database.display().to_string();
        vec![
            self.database.clone(),
            PathBuf::from(format!("{name}-wal")),
            PathBuf::from(format!("{name}-shm")),
        ]
    }

    fn token(&self) -> Option<String> {
        None
    }

    fn resume(&mut self, _token: &str) -> bool {
        false
    }

    fn ingest(&mut self, capture: &Value) -> Result<bool, Error> {
        let Some(reader) = self.reader.as_mut() else {
            return Ok(false);
        };
        let identities: Vec<(&str, &str)> = capture["observations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some((
                    row["collection"].as_str().filter(|c| SCOPE.contains(c))?,
                    row["native_key"].as_str().filter(|k| !k.is_empty())?,
                ))
            })
            .collect();
        reader.invalidate(identities);
        Ok(false)
    }

    fn owns(&self, handle: &str) -> bool {
        self.blobs.contains_key(handle)
    }

    fn blob(&mut self, handle: &str, offset: u64, length: usize) -> Result<Value, Error> {
        let known = self.blobs.get(handle).ok_or(Error::Invalid)?;
        let reader = self.reader.as_ref().ok_or(Error::Invalid)?;
        let bytes = reader
            .read_blob(handle, offset, length)
            .map_err(reader_error)?;
        let end = offset + u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?;
        Ok(json!({
            "t": "BlobChunk",
            "handle": handle,
            "offset": offset,
            "bytes": STANDARD.encode(&bytes),
            "digest": known.digest,
            "eof": end >= known.size,
        }))
    }

    fn next(&mut self, context: &ScanContext) -> Result<Option<Value>, Error> {
        if self.pending {
            return Err(Error::Conflict);
        }
        let mut request = false;
        if self.reader.is_none() {
            let Ok(reader) = messages::Reader::open(&self.database, &self.home) else {
                return Ok(None);
            };
            self.reader = Some(reader);
            request = true;
        }
        if self
            .reconciled_at
            .is_none_or(|at| at.elapsed() >= RECONCILE_EVERY)
        {
            request = true;
        }
        let mut collected: Vec<messages::Batch> = Vec::new();
        let mut idle = 0;
        let mut failure = None;
        loop {
            let reader = self.reader.as_mut().ok_or(Error::Conflict)?;
            match reader.step(request) {
                Ok(batches) => {
                    request = false;
                    idle = if batches.is_empty() { idle + 1 } else { 0 };
                    collected.extend(batches);
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
            if !reader.reconciliation_pending() || idle >= MAX_IDLE_STEPS {
                break;
            }
        }
        if let Some(error) = failure {
            if collected.is_empty() {
                self.reader = None;
                return Err(reader_error(error));
            }
        } else if collected.iter().any(|batch| batch.complete) {
            self.reconciled_at = Some(Instant::now());
        }
        let frame = self.frame(context, &collected)?;
        self.pending = frame.is_some();
        Ok(frame)
    }

    fn commit(&mut self) -> Result<(), Error> {
        if !self.pending {
            return Err(Error::Conflict);
        }
        self.pending = false;
        Ok(())
    }
}
