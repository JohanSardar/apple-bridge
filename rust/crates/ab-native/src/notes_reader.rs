pub use crate::notes_store::{DerivativeFacts, Facts, Variant};
use crate::{
    assets::Store,
    capture::{self, Batch, Error, Observation, Result},
    native_json,
    notes_changes::{self, Checkpoint},
    notes_projection as projection,
    notes_store::{self, Database},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Context {
    #[serde(flatten)]
    pub capture: capture::Context,
    pub coverage: Value,
    #[serde(default)]
    pub facts: Facts,
    #[serde(default)]
    pub asset_paths: Vec<PathBuf>,
}
#[derive(Debug, Clone)]
struct Job {
    key: String,
    fingerprint: Value,
    scan_id: String,
}
#[derive(Debug)]
pub struct Reader {
    database: PathBuf,
    assets: Store,
    checkpoint: Option<Checkpoint>,
    previous: BTreeMap<String, Observation>,
    generation: String,
    revision: u64,
    order: VecDeque<String>,
    worker_ran: bool,
    jobs: BTreeMap<String, Job>,
    paths: BTreeMap<String, BTreeSet<PathBuf>>,
    pub changed_notes: usize,
    pub bootstrap_scans: usize,
    pub reconciliation_pending: bool,
}
#[derive(Debug, serde::Serialize)]
pub struct Pending {
    #[serde(flatten)]
    pub batch: Batch,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    checkpoint: Checkpoint,
    #[serde(skip)]
    records: BTreeMap<String, Observation>,
    #[serde(skip)]
    jobs: Vec<Job>,
    #[serde(skip)]
    removed: BTreeSet<String>,
    #[serde(skip)]
    changed: usize,
    #[serde(skip)]
    bootstrap: bool,
    #[serde(skip)]
    asset_paths: Vec<PathBuf>,
}
#[derive(Debug, serde::Serialize)]
pub struct Hydration {
    #[serde(flatten)]
    pub batch: Batch,
    #[serde(skip)]
    job: Job,
    #[serde(skip)]
    record: Observation,
    #[serde(skip)]
    paths: BTreeSet<PathBuf>,
}
fn data(row: &Observation) -> Result<Value> {
    row.data.as_ref().map_or(Ok(Value::Null), |raw| {
        serde_json::from_str(raw.get()).map_err(Error::from)
    })
}
fn replace_data(row: &mut Observation, value: &Value) -> Result<()> {
    row.data = Some(serde_json::from_slice(&native_json::record_data(value)?)?);
    Ok(())
}
fn equal(left: &Observation, right: &Observation) -> bool {
    match (&left.data, &right.data) {
        (Some(left), Some(right)) => capture::data_equal(left.get(), right.get()),
        (None, None) => true,
        _ => false,
    }
}
impl Reader {
    pub fn open(database: &Path, staging: &Path) -> Result<Self> {
        Ok(Self {
            database: database.into(),
            assets: Store::open(staging, "notes-blob:")?,
            checkpoint: None,
            previous: BTreeMap::new(),
            generation: String::new(),
            revision: 0,
            order: VecDeque::new(),
            worker_ran: false,
            jobs: BTreeMap::new(),
            paths: BTreeMap::new(),
            changed_notes: 0,
            bootstrap_scans: 0,
            reconciliation_pending: false,
        })
    }
    pub fn resume_identities(
        &mut self,
        generation: &str,
        identities: &[(String, String)],
    ) -> Result<()> {
        if self.revision != 0 || generation.is_empty() {
            return Err(Error::Conflict);
        }
        let mut previous = BTreeMap::new();
        for (collection, key) in identities {
            if !["notes.folder", "notes.note"].contains(&collection.as_str()) || key.is_empty() {
                return Err(Error::Corrupt);
            }
            let identity = format!("{collection}/{key}");
            if previous.contains_key(&identity) {
                return Err(Error::Corrupt);
            }
            previous.insert(
                identity,
                Observation {
                    kind: "upsert",
                    collection: collection.clone(),
                    native_key: key.clone(),
                    scoped_alias: None,
                    observed_at: String::new(),
                    data: None,
                    revalidated: None,
                },
            );
        }
        self.previous = previous;
        self.generation = generation.into();
        Ok(())
    }
    pub fn identities(&self) -> Vec<(String, String)> {
        self.previous
            .values()
            .map(|r| (r.collection.clone(), r.native_key.clone()))
            .collect()
    }
    pub fn generation(&self) -> &str {
        &self.generation
    }
    pub fn reconcile(&self, context: &Context) -> Result<Pending> {
        let scan = notes_changes::scan(
            &self.database,
            self.checkpoint.as_ref(),
            &context.facts,
            &context.capture,
        )?;
        let previous = if self.generation == scan.snapshot.generation {
            self.previous.clone()
        } else {
            BTreeMap::new()
        };
        let mut records = previous.clone();
        let mut jobs = Vec::new();
        for mut row in scan.snapshot.observations.clone() {
            let key = format!("{}/{}", row.collection, row.native_key);
            if row.collection == "notes.note" {
                let fresh = data(&row)?;
                let value = if let Some(old) = previous.get(&key) {
                    let old = data(old)?;
                    if old["native_fingerprint"] == fresh["native_fingerprint"] {
                        old
                    } else {
                        projection::pending(&fresh)?
                    }
                } else {
                    projection::pending(&fresh)?
                };
                if value["locked"] != true && value["body_status"] == "pending" {
                    jobs.push(Job {
                        key: row.native_key.clone(),
                        fingerprint: fresh["native_fingerprint"].clone(),
                        scan_id: context.capture.scan_id.clone(),
                    });
                }
                replace_data(&mut row, &value)?;
            }
            records.insert(key, row);
        }
        let mut removed = scan.removed;
        if scan.bootstrap {
            let current = scan
                .snapshot
                .observations
                .iter()
                .map(|row| format!("{}/{}", row.collection, row.native_key))
                .collect::<BTreeSet<_>>();
            removed.extend(
                previous
                    .keys()
                    .filter(|key| !current.contains(*key))
                    .cloned(),
            );
        }
        for key in &removed {
            records.remove(key);
        }
        let mut observations = Vec::new();
        for item in &scan.snapshot.observations {
            let key = format!("{}/{}", item.collection, item.native_key);
            if let Some(row) = records.get(&key)
                && previous.get(&key).is_none_or(|old| !equal(old, row))
            {
                observations.push(row.clone());
            }
        }
        for key in &removed {
            if let Some(row) = previous.get(key) {
                observations.push(capture::absent(
                    &row.collection,
                    &row.native_key,
                    None,
                    &context.capture,
                ));
            }
        }
        let batch = Batch {
            source: "notes".into(),
            generation: scan.snapshot.generation,
            checkpoint: context.capture.checkpoint.clone(),
            scan_id: context.capture.scan_id.clone(),
            scan_started_at: context.capture.observed_at.clone(),
            scope: vec!["notes.folder".into(), "notes.note".into()],
            completion_evidence: "complete",
            coverage: context.coverage.clone(),
            observations,
            blob_handles: BTreeMap::new(),
            live: true,
        };
        Ok(Pending {
            batch,
            revision: self.revision,
            checkpoint: scan.checkpoint,
            records,
            jobs,
            removed,
            changed: scan.changed_notes,
            bootstrap: scan.bootstrap,
            asset_paths: context.asset_paths.clone(),
        })
    }
    fn enqueue(&mut self, job: Job, priority: bool, force: bool) {
        if !force
            && self
                .jobs
                .get(&job.key)
                .is_some_and(|old| old.fingerprint == job.fingerprint)
        {
            return;
        }
        let key = job.key.clone();
        self.jobs.insert(key.clone(), job);
        if priority {
            self.order.retain(|old| old != &key);
            self.order.push_front(key);
        } else if !self.order.contains(&key) {
            self.order.push_back(key);
        }
    }
    pub fn commit(&mut self, pending: Pending) -> Result<()> {
        let scan_id = pending.batch.scan_id.clone();
        let asset_paths = pending.asset_paths;
        if pending.revision != self.revision {
            return Err(Error::Conflict);
        }
        self.revision = self.revision.checked_add(1).ok_or(Error::Conflict)?;
        self.generation = pending.batch.generation;
        self.previous = pending.records;
        self.checkpoint = Some(pending.checkpoint);
        self.changed_notes = pending.changed;
        self.bootstrap_scans += usize::from(pending.bootstrap);
        self.reconciliation_pending = false;
        for key in pending.removed {
            let note = key.strip_prefix("notes.note/").unwrap_or(&key);
            self.paths.remove(note);
            self.jobs.remove(note);
        }
        for job in pending.jobs {
            self.enqueue(job, !pending.bootstrap, false);
        }
        let mut retry = Vec::new();
        for row in self.previous.values() {
            if row.collection != "notes.note" {
                continue;
            }
            let value = data(row)?;
            if value["locked"] == true {
                continue;
            }
            let unavailable = value["attachments"].as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item["state"] == "unavailable"
                        && item["reason"] != "attachment_metadata_unavailable"
                })
            });
            if value["body_status"] == "pending"
                || value["body_status"] == "unavailable"
                || unavailable
            {
                retry.push(Job {
                    key: row.native_key.clone(),
                    fingerprint: value["native_fingerprint"].clone(),
                    scan_id: scan_id.clone(),
                });
            }
        }
        for job in retry {
            self.enqueue(job, false, false);
        }
        let forced = self
            .paths
            .iter()
            .filter(|(_, paths)| {
                paths
                    .iter()
                    .any(|path| asset_paths.iter().any(|changed| path.starts_with(changed)))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in forced {
            if let Some(row) = self.previous.get(&format!("notes.note/{key}")) {
                let value = data(row)?;
                if value["locked"] != true {
                    self.enqueue(
                        Job {
                            key,
                            fingerprint: value["native_fingerprint"].clone(),
                            scan_id: scan_id.clone(),
                        },
                        false,
                        true,
                    );
                }
            }
        }
        Ok(())
    }
    pub fn has_hydration(&self) -> bool {
        !self.order.is_empty()
    }
    fn variants(
        &mut self,
        item: &mut Value,
        asset: &notes_store::Attachment,
        digest: &str,
        context: &Context,
    ) -> Result<()> {
        let extension = asset
            .file
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let key = format!("{}|{extension}", asset.uti);
        let supports = context
            .facts
            .supports
            .get(&key)
            .ok_or_else(|| Error::FactsRequired(format!("supports:{key}")))?;
        if !supports {
            return Ok(());
        }
        let facts = context
            .facts
            .derivatives
            .get(digest)
            .ok_or_else(|| Error::FactsRequired(format!("derivatives:{digest}")))?;
        if facts.status == "unavailable" {
            item["variant_status"] = json!("unavailable");
            return Ok(());
        }
        if !["complete", "partial"].contains(&facts.status.as_str()) {
            return Err(Error::Corrupt);
        }
        let mut variants = BTreeMap::new();
        for (name, variant) in &facts.variants {
            let bytes = STANDARD
                .decode(&variant.bytes_base64)
                .map_err(|_| Error::Corrupt)?;
            let mut reference = serde_json::to_value(self.assets.bytes(&bytes)?)?;
            reference["variant"] = json!(name);
            reference["mime"] = json!(variant.mime);
            if let Some(width) = variant.width {
                reference["width"] = json!(width);
            }
            if let Some(height) = variant.height {
                reference["height"] = json!(height);
            }
            variants.insert(name, reference);
        }
        item["derivatives"] = json!(variants);
        item["variant_status"] = json!(facts.status);
        Ok(())
    }
    fn hydrated(
        &mut self,
        job: &Job,
        context: &Context,
    ) -> Result<Option<(Observation, BTreeSet<PathBuf>)>> {
        let db = Database::open(&self.database)?;
        let snapshot = notes_store::read(
            &db,
            &self.database,
            Some(&BTreeSet::from([job.key.clone()])),
            false,
            &context.facts,
            &context.capture,
        )?;
        db.finish()?;
        let Some(mut row) = snapshot
            .observations
            .into_iter()
            .find(|row| row.collection == "notes.note" && row.native_key == job.key)
        else {
            self.reconciliation_pending = true;
            return Ok(None);
        };
        let mut value = data(&row)?;
        if snapshot.generation != self.generation
            || value["native_fingerprint"] != job.fingerprint
            || value["locked"] == true
        {
            self.reconciliation_pending = true;
            return Ok(None);
        }
        let root = self.database.parent().ok_or(Error::Corrupt)?;
        let root = root.to_path_buf();
        let mut attachments = Vec::new();
        for metadata in value["attachments"].as_array().cloned().unwrap_or_default() {
            let mut item = metadata;
            let id = item["id"].as_str().unwrap_or_default();
            if let Some(asset) = snapshot
                .attachments
                .get(&format!("{}:{id}", row.native_key))
            {
                if let Some(file) = &asset.file {
                    if let Ok(reference) = self.assets.stage(file, &root) {
                        let digest = reference.digest.clone();
                        item["blob"] = serde_json::to_value(reference)?;
                        item["state"] = json!("present");
                        self.variants(&mut item, asset, &digest, context)?;
                    } else {
                        item["state"] = json!("unavailable");
                        item["reason"] = json!("attachment_staging_unavailable");
                    }
                } else if let Some(bytes) = &asset.payload {
                    if let Ok(reference) = self.assets.bytes(bytes) {
                        item["blob"] = serde_json::to_value(reference)?;
                        item["state"] = json!("present");
                    } else {
                        item["state"] = json!("unavailable");
                        item["reason"] = json!("attachment_staging_unavailable");
                    }
                }
            }
            attachments.push(item);
        }
        value["attachments"] = json!(attachments);
        if let Some(bytes) = snapshot.raw_bodies.get(&row.native_key) {
            if let Ok(reference) = self.assets.bytes(bytes) {
                value["raw_body"] = serde_json::to_value(reference)?;
            } else {
                value["raw_body"] = json!({"digest":value["raw_body_digest"],"size":bytes.len(),"state":"missing","reason":"native_staging_unavailable"});
            }
        }
        value = projection::fit(&value, &mut self.assets)?;
        replace_data(&mut row, &value)?;
        let paths = snapshot
            .attachments
            .iter()
            .filter(|(key, _)| key.starts_with(&format!("{}:", row.native_key)))
            .filter_map(|(_, asset)| asset.file.as_ref())
            .cloned()
            .collect();
        Ok(Some((row, paths)))
    }
    fn hydration_batch(
        &self,
        row: Observation,
        job: &Job,
        context: &Context,
        unavailable: bool,
    ) -> Result<Hydration> {
        let mut handles = BTreeMap::new();
        for reference in projection::references(&data(&row)?) {
            if let Some(digest) = reference["digest"].as_str()
                && let Some(handle) = self.assets.handle(digest)
            {
                handles.insert(digest.into(), handle);
            }
        }
        let batch = Batch {
            source: "notes".into(),
            generation: self.generation.clone(),
            checkpoint: context.capture.checkpoint.clone(),
            scan_id: format!(
                "{}-{}",
                job.scan_id,
                if unavailable {
                    "unavailable"
                } else {
                    "hydrate"
                }
            ),
            scan_started_at: context.capture.observed_at.clone(),
            scope: Vec::new(),
            completion_evidence: "",
            coverage: context.coverage.clone(),
            observations: vec![row.clone()],
            blob_handles: handles,
            live: false,
        };
        Ok(Hydration {
            batch,
            job: job.clone(),
            record: row,
            paths: BTreeSet::new(),
        })
    }
    pub fn hydrate_next(&mut self, context: &Context) -> Result<Option<Hydration>> {
        self.worker_ran = true;
        let Some(key) = self.order.pop_front() else {
            return Ok(None);
        };
        let Some(job) = self.jobs.remove(&key) else {
            return Ok(None);
        };
        let result = self.hydrated(&job, context);
        match result {
            Ok(Some((row, paths))) => {
                let Some(prior) = self.previous.get(&format!("notes.note/{}", job.key)) else {
                    return Ok(None);
                };
                if data(prior)?["native_fingerprint"] != job.fingerprint {
                    return Ok(None);
                }
                self.paths.insert(job.key.clone(), paths.clone());
                if equal(prior, &row) {
                    return Ok(None);
                }
                let mut pending = self.hydration_batch(row, &job, context, false)?;
                pending.paths = paths;
                Ok(Some(pending))
            }
            Ok(None) => Ok(None),
            Err(error @ Error::FactsRequired(_)) => {
                self.enqueue(job, true, true);
                Err(error)
            }
            Err(_) => {
                let Some(mut row) = self
                    .previous
                    .get(&format!("notes.note/{}", job.key))
                    .cloned()
                else {
                    return Ok(None);
                };
                let mut value = data(&row)?;
                if value["native_fingerprint"] != job.fingerprint {
                    return Ok(None);
                }
                value["body_status"] = json!("unavailable");
                value["body_reason"] = json!("native_hydration_unavailable");
                replace_data(&mut row, &value)?;
                self.hydration_batch(row, &job, context, true).map(Some)
            }
        }
    }
    pub fn hydration_current(&self, pending: &Hydration) -> Result<bool> {
        Ok(self.generation == pending.batch.generation
            && self
                .previous
                .get(&format!("notes.note/{}", pending.job.key))
                .map(data)
                .transpose()?
                .is_some_and(|value| value["native_fingerprint"] == pending.job.fingerprint))
    }
    pub fn commit_hydration(&mut self, pending: Hydration) -> Result<bool> {
        if !self.hydration_current(&pending)? {
            return Ok(false);
        }
        self.paths.insert(pending.job.key.clone(), pending.paths);
        self.previous
            .insert(format!("notes.note/{}", pending.job.key), pending.record);
        Ok(true)
    }
    pub fn finish_hydration(&mut self, now: std::time::SystemTime) -> Result<()> {
        if !self.order.is_empty() || !self.worker_ran {
            return Ok(());
        }
        let mut active = BTreeSet::new();
        for row in self.previous.values() {
            for reference in projection::references(&data(row)?) {
                active.insert(reference["digest"].as_str().unwrap_or_default().to_owned());
            }
        }
        self.assets.prune(&active, now)?;
        self.worker_ran = false;
        Ok(())
    }
    pub fn read_blob(&self, handle: &str, offset: u64, length: usize) -> Result<Value> {
        let _ = std::fs::File::open(&self.database)?;
        self.assets.read(handle, offset, length)
    }
}
