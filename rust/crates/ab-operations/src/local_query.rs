use crate::{Code, Error, GatewayStore, Principal, peer::ApplePeer};
use serde_json::{Value, value::RawValue};
use std::sync::Arc;
use tokio::sync::Mutex;

fn blob_error(error: &ab_blobs::Error) -> Error {
    match error {
        ab_blobs::Error::Capacity => Code::Capacity,
        ab_blobs::Error::Corrupt => Code::Corrupt,
        ab_blobs::Error::Unauthorized => Code::Unauthorized,
        ab_blobs::Error::Invalid => Code::Invalid,
        ab_blobs::Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Code::NotFound
        }
        _ => Code::Unavailable,
    }
    .into()
}
#[derive(Debug)]
pub struct QueryBlobContext<'a> {
    pub principal: &'a Principal,
    pub id: &'a str,
    pub scope: &'a str,
    pub max_size: u64,
    pub session: crate::peer::PeerSession,
}
#[derive(Debug)]
pub struct LocalQueryStore {
    pub state: GatewayStore,
    pub blobs: Arc<Mutex<ab_blobs::Store>>,
}
#[derive(Debug)]
struct Pins(Arc<Mutex<ab_blobs::Store>>, Vec<String>);
impl Drop for Pins {
    fn drop(&mut self) {
        let store = self.0.clone();
        let pins = std::mem::take(&mut self.1);
        tokio::spawn(async move {
            let mut store = store.lock_owned().await;
            let _ = tokio::task::spawn_blocking(move || {
                for pin in pins {
                    let _ = store.release_pin(&pin);
                }
            })
            .await;
        });
    }
}
impl LocalQueryStore {
    pub fn open(
        state: GatewayStore,
        home: impl AsRef<std::path::Path>,
        cap: u64,
    ) -> Result<Self, Error> {
        Ok(Self {
            state,
            blobs: Arc::new(Mutex::new(
                ab_blobs::Store::open(home, cap).map_err(|_| Code::Unavailable)?,
            )),
        })
    }

    pub(crate) fn mail_args(&self, p: &Principal, args: &Value) -> Result<Box<RawValue>, Error> {
        if !args
            .as_object()
            .is_some_and(|a| a.keys().all(|k| k == "message_id"))
        {
            return Err(Code::Invalid.into());
        }
        let id = args["message_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Code::Invalid)?;
        let visibility = crate::adapters::visibility(&p.visibility());
        let native = self.state.with(|s| {
            let records = s.read(
                &visibility,
                &ab_state::ReadQuery {
                    collection: "mail.message".into(),
                    id: id.into(),
                    ..Default::default()
                },
            )?;
            if records.records.len() != 1 {
                return Err(ab_trust::domain::ErrorCode::NotFound.into());
            }
            s.native_key("mail.message", id)
        })?;
        serde_json::value::to_raw_value(&serde_json::json!({"message_id":native}))
            .map_err(|_| Code::Corrupt.into())
    }
    pub async fn hydrate(
        &self,
        peer: &dyn ApplePeer,
        context: &QueryBlobContext<'_>,
        blobs: &mut [Value],
    ) -> Result<(), Error> {
        let p = context.principal;
        let id = context.id;
        let scope = context.scope;
        let limit = context.max_size;
        let session = context.session;
        let mut pins = Pins(self.blobs.clone(), Vec::new());
        for blob in blobs.iter_mut() {
            let digest = blob["digest"].as_str().ok_or(Code::Corrupt)?.to_owned();
            let size = blob["size"]
                .as_u64()
                .filter(|n| *n <= limit)
                .ok_or(Code::Corrupt)?;
            let handle = blob["handle"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(Code::Corrupt)?;
            ab_blobs::Digest::try_from(digest.clone()).map_err(|_| Code::Corrupt)?;
            let mut data = Vec::new();
            while (data.len() as u64) < size {
                let offset = i64::try_from(data.len()).map_err(|_| Code::Corrupt)?;
                let length = i64::try_from(size - data.len() as u64)
                    .map_err(|_| Code::Corrupt)?
                    .min(128 << 10);
                let (bytes, done) = peer.read_blob_from(session, handle, offset, length).await?;
                if bytes.is_empty()
                    || data.len() as u64 + bytes.len() as u64 > size
                    || done && data.len() as u64 + bytes.len() as u64 != size
                {
                    return Err(Code::Corrupt.into());
                }
                data.extend(bytes);
            }
            if ab_trust::fingerprint(&data) != digest {
                return Err(Code::Corrupt.into());
            }
            let mut store = self.blobs.clone().lock_owned().await;
            let pin_store = self.blobs.clone();
            let mut staged = tokio::task::spawn_blocking(move || {
                store
                    .stage(&data)
                    .map(|(_, pin)| Pins(pin_store, vec![pin]))
                    .map_err(|error| blob_error(&error))
            })
            .await
            .map_err(|_| Code::Unavailable)??;
            pins.1.append(&mut staged.1);
            blob.as_object_mut().ok_or(Code::Corrupt)?.remove("handle");
        }
        let refs = blobs
            .iter()
            .map(|b| serde_json::json!({"digest":b["digest"],"size":b["size"],"state":"present"}))
            .collect::<Vec<_>>();
        let updates =
            serde_json::from_value(serde_json::json!({id:refs})).map_err(|_| Code::Corrupt)?;
        let mut store = self.blobs.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            store
                .update_protection(&updates, false)
                .map_err(|error| blob_error(&error))
        })
        .await
        .map_err(|_| Code::Unavailable)??;
        self.state
            .with(|s| s.save_query_blobs(&p.id, id, scope, &Value::Array(blobs.to_owned())))
    }
}
