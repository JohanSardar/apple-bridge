use crate::{Code, Error, LocalQueryStore, Principal};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, value::RawValue};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
};

#[derive(Debug)]
pub struct DocumentInput {
    pub args: Box<RawValue>,
    directory: Option<PathBuf>,
}
impl Drop for DocumentInput {
    fn drop(&mut self) {
        if let Some(directory) = &self.directory {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}
const INLINE_DOCUMENT_BYTES: usize = 128 << 10;
fn raw(value: &Value) -> Result<Box<RawValue>, Error> {
    serde_json::value::to_raw_value(value).map_err(|_| Code::Corrupt.into())
}
fn grant(digest: &str) -> Result<ab_blobs::RecordGrant, Error> {
    let record=serde_json::from_value(serde_json::json!({"collection":"system.command","id":"cmd_query_blob","source":"system","version":1,"observed_at":"0001-01-01T00:00:00Z","deleted":false,"coverage":{"fields":"","missing":null},"data":{"digest":digest}})).map_err(|_| Code::Corrupt)?;
    ab_blobs::RecordGrant::from_record(
        &ab_trust::Visibility {
            all: true,
            commands_all: true,
            ..Default::default()
        },
        &record,
    )
    .map_err(|_| Code::Corrupt.into())
}
impl LocalQueryStore {
    pub async fn query_blob(
        &self,
        p: &Principal,
        id: &str,
        digest: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, Error> {
        let (scope, blobs) = self.state.with(|s| s.query_blob_grant(&p.id, id))?;
        p.authorize(&scope).map_err(|_| Code::Forbidden)?;
        if !blobs
            .as_array()
            .is_some_and(|a| a.iter().any(|b| b["digest"] == digest))
        {
            return Err(Code::Unauthorized.into());
        }
        self.read_blob(digest, offset, length).await
    }
    pub(crate) async fn read_blob(&self, digest: &str, offset: u64, length: u64) -> Result<Vec<u8>, Error> {
        let d = ab_blobs::Digest::try_from(digest.to_owned()).map_err(|_| Code::Invalid)?;
        let file = self
            .blobs
            .lock()
            .await
            .open_authorized(&grant(digest)?, &d)
            .map_err(|_| Code::NotFound)?;
        tokio::task::spawn_blocking(move || {
            let mut file = file;
            file.seek(SeekFrom::Start(offset))
                .map_err(|_| Code::Unavailable)?;
            let mut data = Vec::new();
            file.take(length)
                .read_to_end(&mut data)
                .map_err(|_| Code::Unavailable)?;
            Ok(data)
        })
        .await
        .map_err(|_| Code::Unavailable)?
    }
    pub async fn document_input(
        &self,
        p: &Principal,
        input: &RawValue,
    ) -> Result<DocumentInput, Error> {
        let args = self.resolve_document(p, input).await?;
        Self::stage_document(args, INLINE_DOCUMENT_BYTES).await
    }
    pub async fn document_file(
        &self,
        p: &Principal,
        input: &RawValue,
    ) -> Result<DocumentInput, Error> {
        let args = self.resolve_document(p, input).await?;
        let document = Self::stage_document(args, 0).await?;
        if document.directory.is_none() {
            return Err(Code::Invalid.into());
        }
        Ok(document)
    }
    async fn resolve_document(&self, p: &Principal, input: &RawValue) -> Result<Value, Error> {
        let mut args: Value = serde_json::from_str(input.get()).map_err(|_| Code::Invalid)?;
        let object = args.as_object().ok_or(Code::Invalid)?;
        if object
            .keys()
            .any(|k| k.eq_ignore_ascii_case("file") || k.eq_ignore_ascii_case("allow_file"))
        {
            return Err(Code::Invalid.into());
        }
        if let Some(reference) = object.get("blob") {
            if object.contains_key("data_base64") {
                return Err(Code::Invalid.into());
            }
            let reference = reference.as_object().ok_or(Code::Invalid)?;
            if reference
                .keys()
                .any(|k| !["query_id", "digest", "record_id", "collection"].contains(&k.as_str()))
                || reference.values().any(|v| !v.is_string() && !v.is_null())
            {
                return Err(Code::Invalid.into());
            }
            let text = |k: &str| reference.get(k).and_then(Value::as_str).unwrap_or_default();
            let digest = text("digest");
            ab_blobs::Digest::try_from(digest.to_owned()).map_err(|_| Code::Invalid)?;
            let query = text("query_id");
            let record = text("record_id");
            let collection = text("collection");
            let data = if !query.is_empty() && record.is_empty() && collection.is_empty() {
                self.query_blob(p, query, digest, 0, (8 << 20) + 1).await?
            } else if query.is_empty() && !record.is_empty() && !collection.is_empty() {
                let _: ab_trust::domain::Collection =
                    serde_json::from_value(Value::String(collection.into()))
                        .map_err(|_| Code::Invalid)?;
                let visibility = crate::adapters::visibility(&p.visibility());
                let record = self
                    .state
                    .with(|s| {
                        s.read(
                            &visibility,
                            &ab_state::ReadQuery {
                                collection: collection.into(),
                                id: record.into(),
                                ..Default::default()
                            },
                        )
                    })?
                    .records
                    .into_iter()
                    .next()
                    .ok_or(Code::NotFound)?;
                let refs = ab_blobs::references(record.data.get()).map_err(|_| Code::Corrupt)?;
                if !refs.iter().any(|r| r.digest == digest) {
                    return Err(Code::Unauthorized.into());
                }
                self.read_blob(digest, 0, (8 << 20) + 1).await?
            } else {
                return Err(Code::Invalid.into());
            };
            if data.is_empty() || data.len() > 8 << 20 {
                return Err(Code::Invalid.into());
            }
            let object = args.as_object_mut().ok_or(Code::Invalid)?;
            object.remove("blob");
            object.insert("data_base64".into(), Value::String(STANDARD.encode(data)));
        }
        Ok(args)
    }
    async fn stage_document(mut args: Value, inline_limit: usize) -> Result<DocumentInput, Error> {
        if let Some(encoded) = args["data_base64"].as_str().filter(|s| s.len() > inline_limit) {
            if encoded.len() > (8_usize << 20).div_ceil(3) * 4 {
                return Err(Code::Invalid.into());
            }
            let data = STANDARD
                .decode(encoded.replace(['\r', '\n'], ""))
                .map_err(|_| Code::Invalid)?;
            if data.is_empty() || data.len() > 8 << 20 {
                return Err(Code::Invalid.into());
            }
            let directory = std::env::temp_dir().join(format!("ab-document-{}", ulid::Ulid::new()));
            return tokio::task::spawn_blocking(move || {
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&directory)
                    .map_err(|_| Code::Unavailable)?;
                let mut prepared = DocumentInput {
                    args: raw(&args)?,
                    directory: Some(directory.clone()),
                };
                let path = directory.join("document");
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .map_err(|_| Code::Unavailable)?;
                file.write_all(&data).map_err(|_| Code::Unavailable)?;
                file.sync_all().map_err(|_| Code::Unavailable)?;
                let object = args.as_object_mut().ok_or(Code::Invalid)?;
                object.remove("data_base64");
                object.insert(
                    "file".into(),
                    Value::String(path.to_str().ok_or(Code::Invalid)?.into()),
                );
                prepared.args = raw(&args)?;
                Ok(prepared)
            })
            .await
            .map_err(|_| Code::Unavailable)?;
        }
        Ok(DocumentInput {
            args: raw(&args)?,
            directory: None,
        })
    }
}
