use crate::{
    Code, Command, Error, LocalQueryStore,
    local::{CommandRecord, PreparedAction},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;

pub(crate) struct MessageFiles {
    blobs: Arc<Mutex<ab_blobs::Store>>,
    command: String,
    owner: String,
    uploads: Vec<String>,
}
impl Drop for MessageFiles {
    fn drop(&mut self) {
        let blobs = self.blobs.clone();
        let command = self.command.clone();
        let owner = self.owner.clone();
        let uploads = self.uploads.clone();
        tokio::spawn(async move {
            let mut store = blobs.lock_owned().await;
            let _ = tokio::task::spawn_blocking(move || {
                for upload_id in uploads {
                    let _ = store.upload(
                        &owner,
                        &ab_blobs::uploads::UploadRequest {
                            action: "abandon".into(),
                            upload_id,
                            ..Default::default()
                        },
                    );
                }
                store.clear_message_files(&command)
            })
            .await;
        });
    }
}
impl LocalQueryStore {
    pub(crate) async fn message_uploads(
        &self,
        owner: &str,
        command: &Command,
    ) -> Result<BTreeMap<String, ab_state::BlobReference>, Error> {
        let mut refs = BTreeMap::new();
        if !command.op.starts_with("messages.") {
            return Ok(refs);
        }
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let mut store = self.blobs.lock().await;
        for file in args["files"].as_array().into_iter().flatten() {
            if let Some(id) = file["upload_id"].as_str() {
                let (digest, size) = store
                    .message_upload(owner, id)
                    .map_err(|_| Code::NotFound)?;
                refs.insert(
                    id.into(),
                    ab_state::BlobReference {
                        digest,
                        size: i64::try_from(size).map_err(|_| Code::Invalid)?,
                        state: "present".into(),
                        variant: false,
                        awaiting_variants: false,
                    },
                );
            }
        }
        Ok(refs)
    }
    pub(crate) async fn message_files(
        &self,
        record: &CommandRecord,
        plan: &mut PreparedAction,
    ) -> Result<Option<MessageFiles>, Error> {
        if !record.command.op.starts_with("messages.") {
            return Ok(None);
        }
        let args: Value =
            serde_json::from_str(record.command.args.get()).map_err(|_| Code::Corrupt)?;
        let Some(files) = args["files"].as_array().filter(|files| !files.is_empty()) else {
            return Ok(None);
        };
        let guard = MessageFiles {
            blobs: self.blobs.clone(),
            command: record.command.id.clone(),
            owner: record.principal.clone(),
            uploads: record.upload_references.keys().cloned().collect(),
        };
        let store = self.blobs.lock().await;
        let mut native = Vec::new();
        for (index, file) in files.iter().enumerate() {
            let bytes = if let Some(id) = file["upload_id"].as_str() {
                let reference = record.upload_references.get(id).ok_or(Code::NotFound)?;
                store
                    .message_bytes(
                        &reference.digest,
                        u64::try_from(reference.size).map_err(|_| Code::Corrupt)?,
                    )
                    .map_err(|_| Code::NotFound)?
            } else {
                STANDARD
                    .decode(file["data"].as_str().ok_or(Code::Invalid)?)
                    .map_err(|_| Code::Invalid)?
            };
            let name = file["name"].as_str().ok_or(Code::Invalid)?;
            let path = store
                .message_file(&record.command.id, index, name, &bytes)
                .map_err(|_| Code::Unavailable)?;
            native
                .push(json!({"name": name, "path": path, "digest": ab_trust::fingerprint(&bytes)}));
        }
        let mut action: Value =
            serde_json::from_str(plan.action.get()).map_err(|_| Code::Corrupt)?;
        action["messages"]["files"] = json!(native);
        plan.action = serde_json::value::to_raw_value(&action).map_err(|_| Code::Corrupt)?;
        Ok(Some(guard))
    }
}
