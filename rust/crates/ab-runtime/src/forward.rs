use ab_operations::{
    Code, Codec, Command, CommandQuery, Error, NativeQuery, Principal, local::LocalGateway,
};
use ab_replication::{
    CommandResponse,
    upstream::{Task, UploadHandler},
};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use std::sync::Arc;

#[derive(Debug)]
pub struct Forward {
    pub local: Arc<LocalGateway>,
    pub authority: Arc<ab_trust::Authority>,
    pub uploads: ab_replication::upstream::Uploads,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(default)]
    blob_digest: String,
    #[serde(default)]
    blob_offset: i64,
    method: String,
    command: Command,
    #[serde(default)]
    query: CommandQuery,
    #[serde(default)]
    native_query: Option<NativeQuery>,
}
impl Forward {
    async fn command(
        &self,
        certificate: &[u8],
        delegation: &str,
        raw: &RawValue,
    ) -> Result<Box<RawValue>, Error> {
        let request: Request = serde_json::from_str(raw.get()).map_err(|_| Code::Invalid)?;
        if !request.blob_digest.is_empty() || request.blob_offset != 0 {
            return Err(Code::Unsupported.into());
        }
        let id = match request.method.as_str() {
            "submit" => &request.command.id,
            "query" => &request.native_query.as_ref().ok_or(Code::Invalid)?.id,
            "status" | "list" => &request.query.id,
            _ => return Err(Code::Invalid.into()),
        };
        let hash = if request.method == "submit" {
            crate::GoJson.command_hash(&request.command)?
        } else {
            crate::GoJson.fingerprint(raw.get().as_bytes())
        };
        let identity = self
            .authority
            .verify_delegation(certificate, delegation, id, &hash, "")
            .map_err(|_| Code::Unauthorized)?;
        let principal = Principal {
            id: identity.id,
            kind: identity.kind,
            grants: identity
                .grants
                .iter()
                .map(|grant| grant.as_str().to_owned())
                .collect(),
            level: identity.level,
            resource: String::new(),
            remote: identity.remote,
        };
        match request.method.as_str() {
            "submit" => self.local.submit(&principal, request.command).await,
            "status" => self.local.status(&principal, &request.query).await,
            "list" => self.local.list(&principal, request.query.limit),
            "query" => {
                self.local
                    .query(&principal, request.native_query.ok_or(Code::Invalid)?)
                    .await
            }
            _ => Err(Code::Invalid.into()),
        }
    }
}
impl UploadHandler for Forward {
    fn control<'a>(
        &'a self,
        certificate: &'a [u8],
        session: &'a str,
        delegation: &'a str,
        request: &'a RawValue,
    ) -> Task<'a, CommandResponse> {
        Box::pin(async move {
            let value: Value = serde_json::from_str(request.get())?;
            if value.get("method").is_none() {
                return self
                    .uploads
                    .control(certificate, session, delegation, request)
                    .await;
            }
            match self.command(certificate, delegation, request).await {
                Ok(result) => {
                    let value: Value = serde_json::from_str(result.get())?;
                    let era = self
                        .local
                        .state
                        .admission_era()
                        .map_err(|_| ab_replication::Error::Invalid)?;
                    let lookup_only = value["admission_era"]
                        .as_str()
                        .is_some_and(|stored| stored != era);
                    Ok(CommandResponse {
                        result: Some(result),
                        error: None,
                        lookup_only,
                    })
                }
                Err(error) => Ok(CommandResponse {
                    result: None,
                    error: Some(serde_json::from_value(serde_json::to_value(error)?)?),
                    lookup_only: false,
                }),
            }
        })
    }
    fn chunk<'a>(
        &'a self,
        certificate: &'a [u8],
        session: &'a str,
        bytes: &'a [u8],
    ) -> Task<'a, (String, CommandResponse)> {
        self.uploads.chunk(certificate, session, bytes)
    }
    fn closed(&self, session: &str) {
        self.uploads.closed(session);
    }
}
