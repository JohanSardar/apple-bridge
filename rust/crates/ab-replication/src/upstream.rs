use crate::{CommandResponse, Error};
use serde_json::value::RawValue;
use std::{future::Future, pin::Pin};

pub type Task<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;
pub trait UploadHandler: std::fmt::Debug + Send + Sync {
    fn control<'a>(
        &'a self,
        certificate: &'a [u8],
        session: &'a str,
        delegation: &'a str,
        request: &'a RawValue,
    ) -> Task<'a, CommandResponse>;
    fn chunk<'a>(
        &'a self,
        certificate: &'a [u8],
        session: &'a str,
        bytes: &'a [u8],
    ) -> Task<'a, (String, CommandResponse)>;
    fn closed(&self, session: &str);
}
#[derive(Debug)]
pub(crate) struct Session<'a> {
    pub clock: bool,
    pub delegation: Option<crate::delegation::Registration>,
    pub outgoing: tokio::sync::mpsc::Sender<Result<ab_trust::wire::messages::FeedFrame, Error>>,
    pub incoming: tokio::sync::Mutex<
        tokio::sync::mpsc::Receiver<Result<ab_trust::wire::messages::FeedFrame, Error>>,
    >,
    pub commands: std::sync::Mutex<std::collections::BTreeMap<String, tokio::task::JoinHandle<()>>>,

    pub principal: &'a ab_trust::Principal,
    pub certificate: &'a [u8],
    pub id: String,
    pub uploads: Option<&'a dyn UploadHandler>,
}
impl Drop for Session<'_> {
    fn drop(&mut self) {
        if let Ok(commands) = self.commands.lock() {
            for task in commands.values() {
                task.abort();
            }
        }
        if let Some(uploads) = self.uploads {
            uploads.closed(&self.id);
        }
    }
}

#[derive(Debug)]
struct Binding {
    fingerprint: String,
    principal: String,
    request: ab_blobs::uploads::UploadRequest,
    expires: std::time::Instant,
}
#[derive(Debug)]
pub struct Uploads {
    pub authority: std::sync::Arc<ab_trust::Authority>,
    pub store: std::sync::Arc<tokio::sync::Mutex<ab_blobs::Store>>,
    bindings: std::sync::Mutex<std::collections::HashMap<(String, String, i64), Binding>>,
}
impl Uploads {
    pub fn new(
        authority: std::sync::Arc<ab_trust::Authority>,
        store: std::sync::Arc<tokio::sync::Mutex<ab_blobs::Store>>,
    ) -> Self {
        Self {
            authority,
            store,
            bindings: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
    async fn control_inner(
        &self,
        certificate: &[u8],
        session: &str,
        delegation: &str,
        raw: &RawValue,
    ) -> Result<ab_blobs::uploads::UploadStatus, Error> {
        let request: ab_blobs::uploads::UploadRequest = serde_json::from_str(raw.get())?;
        let principal = self.authority.verify_delegation(
            certificate,
            delegation,
            &request.upload_id,
            &ab_trust::fingerprint(raw.get().as_bytes()),
            "",
        )?;
        self.authority
            .authorize(&principal, &ab_trust::Grant::parse("messages:write")?)?;
        if principal.level == "read" {
            return Err(ab_trust::Error::Unauthorized.into());
        }
        let status = self.store.lock().await.upload(&principal.id, &request)?;
        if request.action == "prepare" {
            if request.chunk_id.is_empty()
                || request.chunk_id.len() > 128
                || !request
                    .chunk_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            {
                return Err(Error::Invalid);
            }
            let mut bindings = self.bindings.lock().map_err(|_| Error::Disconnected)?;
            bindings.retain(|_, b| b.expires > std::time::Instant::now());
            let fingerprint = ab_trust::fingerprint(certificate);
            let key = (session.into(), request.digest.clone(), request.offset);
            if let Some(old) = bindings.get(&key) {
                if old.principal != principal.id || old.request.upload_id != request.upload_id {
                    return Err(Error::Invalid);
                }
            } else if bindings
                .values()
                .filter(|b| b.fingerprint == fingerprint)
                .count()
                >= 16
            {
                return Err(Error::Invalid);
            }
            bindings.insert(
                key,
                Binding {
                    fingerprint,
                    principal: principal.id,
                    request,
                    expires: std::time::Instant::now() + std::time::Duration::from_secs(30),
                },
            );
        }
        Ok(status)
    }
}
fn reply(result: Result<ab_blobs::uploads::UploadStatus, Error>) -> Result<CommandResponse, Error> {
    match result {
        Ok(status) => Ok(CommandResponse {
            result: Some(crate::raw(&status)?),
            error: None,
            lookup_only: false,
        }),
        Err(error) => {
            let code = match error {
                Error::Trust(ab_trust::Error::Unauthorized) => {
                    ab_trust::domain::ErrorCode::Unauthorized
                }
                Error::Blob(ab_blobs::Error::Corrupt) => ab_trust::domain::ErrorCode::Corrupt,
                _ => ab_trust::domain::ErrorCode::Invalid,
            };
            Ok(CommandResponse {
                result: None,
                error: Some(code.into()),
                lookup_only: false,
            })
        }
    }
}
impl UploadHandler for Uploads {
    fn control<'a>(
        &'a self,
        certificate: &'a [u8],
        session: &'a str,
        delegation: &'a str,
        request: &'a RawValue,
    ) -> Task<'a, CommandResponse> {
        Box::pin(async move {
            reply(
                self.control_inner(certificate, session, delegation, request)
                    .await,
            )
        })
    }
    fn chunk<'a>(
        &'a self,
        certificate: &'a [u8],
        session: &'a str,
        bytes: &'a [u8],
    ) -> Task<'a, (String, CommandResponse)> {
        Box::pin(async move {
            let chunk = ab_trust::wire::framing::BlobChunk::decode(bytes)?;
            let key = (session.into(), hex::encode(chunk.digest), chunk.offset);
            let binding = self
                .bindings
                .lock()
                .map_err(|_| Error::Disconnected)?
                .remove(&key)
                .ok_or(Error::Invalid)?;
            let result = async {
                let parent = self.authority.authenticate_certificate(certificate)?;
                self.authority
                    .authorize(&parent, &ab_trust::Grant::parse("messages:write")?)?;
                if parent.fingerprint != binding.fingerprint
                    || binding.expires <= std::time::Instant::now()
                {
                    return Err(Error::Invalid);
                }
                Ok(self.store.lock().await.append_upload(
                    &binding.principal,
                    &binding.request.upload_id,
                    chunk.offset,
                    &chunk.bytes,
                )?)
            }
            .await;
            Ok((binding.request.chunk_id, reply(result)?))
        })
    }
    fn closed(&self, session: &str) {
        if let Ok(mut bindings) = self.bindings.lock() {
            bindings.retain(|key, _| key.0 != session);
        }
    }
}
