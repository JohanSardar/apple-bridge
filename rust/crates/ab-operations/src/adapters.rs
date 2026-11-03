use crate::{Code, Command, Cursor, Error, Identity, Principal, ReadQuery, ReadStore, Visibility};
use serde::Serialize;
use serde_json::value::RawValue;
use std::sync::{Arc, Mutex};
#[derive(Debug)]
pub struct StateReads<Role = ab_state::Replica>(
    pub Arc<Mutex<ab_state::State<Role>>>,
    pub Arc<tokio::sync::Mutex<ab_blobs::Store>>,
);
fn state_error(error: &ab_state::Error) -> Error {
    if let ab_state::Error::Contract(code) = error
        && let Ok(code) = serde_json::to_value(code).and_then(serde_json::from_value::<Code>)
    {
        return Error::from(code);
    }
    Code::Unavailable.into()
}
fn raw<T: Serialize>(value: &T) -> Result<Box<RawValue>, Error> {
    serde_json::value::to_raw_value(value).map_err(|_| Code::Corrupt.into())
}
pub fn visibility(input: &Visibility) -> ab_state::Visibility {
    let mut result: ab_state::Visibility = input.iter().map(|(s, v)| (s.clone(), *v)).collect();
    if input.get("admin") == Some(&true) {
        for source in [
            "location",
            "audio",
            "calendar",
            "capture",
            "contacts",
            "files",
            "focus",
            "health",
            "iphone",
            "mac",
            "mail",
            "maps",
            "messages",
            "notes",
            "pdf",
            "phone",
            "print",
            "reminders",
            "safari",
            "shortcuts",
            "speech",
            "system",
            "usage",
            "vision",
            "voice_memos",
            "voicemail",
            "commands:all",
        ] {
            result.insert(source.into(), true);
        }
    }
    result
}
impl<Role: std::fmt::Debug + Send + Sync> ReadStore for StateReads<Role> {
    fn record_activity(&self, activity: &ab_trust::activity::AgentActivity) -> Result<(), Error> {
        let call = || {
            self.0
                .lock()
                .map_err(|_| Code::Unavailable)?
                .record_activity(&serde_json::to_value(activity).map_err(|_| Code::Corrupt)?)
                .map_err(|error| state_error(&error))
        };
        if tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
        {
            tokio::task::block_in_place(call)
        } else {
            call()
        }
    }

    fn cached_query_blob<'a>(
        &'a self,
        principal: &'a Principal,
        id: &'a str,
        digest: &'a str,
    ) -> crate::Task<'a, Vec<u8>> {
        Box::pin(async move {
            self.query_blob_size(principal, id, digest)?;
            let record: ab_trust::domain::Record = serde_json::from_value(serde_json::json!({"collection":"system.command","id":id,"source":"system","version":1,"observed_at":"0001-01-01T00:00:00Z","deleted":false,"coverage":{"fields":"","missing":null},"data":{"digest":digest}})).map_err(|_| Code::Corrupt)?;
            let grant = ab_blobs::RecordGrant::from_record(
                &ab_trust::Visibility {
                    all: true,
                    commands_all: true,
                    ..ab_trust::Visibility::default()
                },
                &record,
            )
            .map_err(|_| Code::Corrupt)?;
            let digest =
                ab_blobs::Digest::try_from(digest.to_owned()).map_err(|_| Code::Invalid)?;
            let file = self
                .1
                .lock()
                .await
                .open_authorized(&grant, &digest)
                .map_err(|_| Code::NotFound)?;
            tokio::task::spawn_blocking(move || {
                use std::io::Read;
                let mut bytes = Vec::new();
                file.take(64 << 20)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Code::Unavailable)?;
                Ok(bytes)
            })
            .await
            .map_err(|_| Code::Unavailable)?
        })
    }
    fn cache_blob<'a>(&'a self, digest: &'a str, bytes: Vec<u8>) -> crate::Task<'a, Vec<u8>> {
        Box::pin(async move {
            let digest =
                ab_blobs::Digest::try_from(digest.to_owned()).map_err(|_| Code::Invalid)?;
            let mut store = self.1.clone().lock_owned().await;
            tokio::task::spawn_blocking(move || {
                if let Some(mut offset) = store
                    .begin(
                        &digest,
                        u64::try_from(bytes.len()).map_err(|_| Code::Invalid)?,
                    )
                    .map_err(|_| Code::Capacity)?
                {
                    let start = usize::try_from(offset).map_err(|_| Code::Corrupt)?;
                    for chunk in bytes.get(start..).ok_or(Code::Corrupt)?.chunks(1 << 20) {
                        store
                            .chunk(&digest, offset, chunk)
                            .map_err(|_| Code::Unavailable)?;
                        offset += u64::try_from(chunk.len()).map_err(|_| Code::Corrupt)?;
                    }
                    store.finish(&digest).map_err(|_| Code::Corrupt)?;
                }
                Ok(bytes)
            })
            .await
            .map_err(|_| Code::Unavailable)?
        })
    }

    fn save_query_blobs(
        &self,
        principal: &str,
        id: &str,
        scope: &str,
        blobs: &serde_json::Value,
    ) -> Result<(), Error> {
        self.0
            .lock()
            .map_err(|_| Code::Unavailable)?
            .save_query_blobs(principal, id, scope, blobs)
            .map_err(|e| state_error(&e))
    }
    fn query_blob_size(&self, principal: &Principal, id: &str, digest: &str) -> Result<u64, Error> {
        let (scope, blobs) = self
            .0
            .lock()
            .map_err(|_| Code::Unavailable)?
            .query_blob_grant(&principal.id, id)
            .map_err(|e| state_error(&e))?;
        principal.authorize(&scope).map_err(|_| Code::Forbidden)?;
        blobs
            .as_array()
            .ok_or(Code::Corrupt)?
            .iter()
            .find(|b| b["digest"] == digest)
            .and_then(|b| b["size"].as_u64())
            .ok_or_else(|| Code::Forbidden.into())
    }
    fn blob<'a>(
        &'a self,
        visible: &'a Visibility,
        record_id: &'a str,
        digest: &'a str,
    ) -> crate::Task<'a, Vec<u8>> {
        Box::pin(async move {
            let grant = {
                let state = self.0.lock().map_err(|_| Code::Unavailable)?;
                let visibility = visibility(visible);
                let result = state
                    .read(
                        &visibility,
                        &ab_state::ReadQuery {
                            id: record_id.into(),
                            ..ab_state::ReadQuery::default()
                        },
                    )
                    .map_err(|e| match state_error(&e) {
                        Error {
                            code: Code::NotFound,
                            ..
                        } => Code::Unauthorized.into(),
                        error => error,
                    })?;
                let record = result
                    .records
                    .iter()
                    .find(|r| r.id.as_str() == record_id)
                    .ok_or(Code::Unauthorized)?;
                let visibility = ab_trust::Visibility {
                    all: visible.get("admin") == Some(&true),
                    sources: visibility
                        .iter()
                        .filter(|(_, v)| **v)
                        .map(|(k, _)| k.clone())
                        .collect(),
                    ..ab_trust::Visibility::default()
                };
                ab_blobs::RecordGrant::from_record(&visibility, record)
                    .map_err(|_| Code::Unauthorized)?
            };
            let digest =
                ab_blobs::Digest::try_from(digest.to_owned()).map_err(|_| Code::Invalid)?;
            let file = self
                .1
                .lock()
                .await
                .open_authorized(&grant, &digest)
                .map_err(|error| match error {
                    ab_blobs::Error::Unauthorized => Code::Unauthorized,
                    _ => Code::NotFound,
                })?;
            tokio::task::spawn_blocking(move || {
                use std::io::Read;
                let mut bytes = Vec::new();
                let mut file = file;
                file.read_to_end(&mut bytes)
                    .map_err(|_| Code::Unavailable)?;
                Ok(bytes)
            })
            .await
            .map_err(|_| Code::Unavailable)?
        })
    }
    fn head(&self) -> Result<Cursor, Error> {
        let state = self.0.lock().map_err(|_| Error::from(Code::Unavailable))?;
        let head = state.head().map_err(|error| state_error(&error))?;
        Ok(Cursor {
            epoch: head.epoch,
            subscription_revision: head.subscription_revision,
            sequence: head.sequence,
        })
    }
    fn read(&self, visible: &Visibility, query: &ReadQuery) -> Result<Box<RawValue>, Error> {
        let state = self.0.lock().map_err(|_| Error::from(Code::Unavailable))?;
        let query = ab_state::ReadQuery {
            collection: query.collection.clone(),
            id: query.id.clone(),
            text: query.text.clone(),
            after: query.after.clone(),
            include_deleted: query.include_deleted,
            include_aliases: query.include_aliases,
            limit: query.limit,
        };
        raw(&state
            .read(&visibility(visible), &query)
            .map_err(|error| state_error(&error))?)
    }
    fn read_model(
        &self,
        visible: &Visibility,
        name: &str,
        query: &serde_json::Value,
    ) -> Result<Box<RawValue>, Error> {
        let state = self.0.lock().map_err(|_| Error::from(Code::Unavailable))?;
        let mut result = state
            .read_model(&visibility(visible), name, query)
            .map_err(|error| state_error(&error))?;
        if result["replica"].is_object() {
            result["replica"]["cursor"] =
                serde_json::to_value(state.head().map_err(|e| state_error(&e))?)
                    .map_err(|_| Code::Corrupt)?;
            if let Some(heartbeat) = result["replica"]["last_heartbeat"]
                .as_str()
                .and_then(|t| t.parse::<jiff::Timestamp>().ok())
            {
                result["replica"]["staleness_ms"] = serde_json::json!(
                    (jiff::Timestamp::now().as_millisecond() - heartbeat.as_millisecond()).max(0)
                );
            }
            if name == "replica.status" {
                result["records"][0]["data"] = result["replica"].clone();
            }
        }
        crate::legacy::read_model(name, &result)
    }
    fn changes(
        &self,
        visible: &Visibility,
        cursor: &Cursor,
    ) -> Result<Option<Box<RawValue>>, Error> {
        let state = self.0.lock().map_err(|_| Error::from(Code::Unavailable))?;
        let cursor = ab_state::Cursor {
            epoch: cursor.epoch.clone(),
            subscription_revision: cursor.subscription_revision,
            sequence: cursor.sequence,
        };
        let mut changes = state
            .changes(visibility(visible), cursor)
            .map_err(|error| state_error(&error))?;
        state
            .next_changes(&mut changes)
            .map_err(|error| state_error(&error))?
            .map(|batch| raw(&batch))
            .transpose()
    }
}
#[derive(Debug)]
pub struct TrustIdentity(pub Arc<ab_trust::Authority>);
fn principal(value: ab_trust::Principal) -> Principal {
    Principal {
        id: value.id,
        kind: value.kind,
        grants: value.grants.iter().map(|g| g.as_str().into()).collect(),
        level: value.level,
        resource: value
            .extra
            .get("resource")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .into(),
        remote: value.remote,
    }
}
fn trust_error(error: &ab_trust::Error) -> Error {
    match error {
        ab_trust::Error::Unauthorized => Code::Unauthorized.into(),
        ab_trust::Error::Invalid => Code::Invalid.into(),
        _ => Code::Unavailable.into(),
    }
}
impl Identity for TrustIdentity {
    fn dial_replicas(&self) -> Result<Option<serde_json::Value>, Error> {
        let credentials = self.0.load().map_err(|e| trust_error(&e))?;
        let rows: Vec<_> = credentials.principals.into_iter()
            .filter(|p| p.kind == "replica" && !p.dial.is_empty())
            .map(|p| serde_json::json!({"replica_id":p.id,"state":if p.active() {p.extra.get("dial_state").and_then(serde_json::Value::as_str).unwrap_or("connecting")} else {"revoked"},"attempts":p.extra.get("dial_attempts").and_then(serde_json::Value::as_u64).unwrap_or(0)}))
            .collect();
        Ok(Some(serde_json::json!(rows)))
    }
    fn approval_principal(
        &self,
        id: &str,
        _command_id: &str,
        _hash: &str,
    ) -> Result<Principal, Error> {
        if id.contains('/') {
            return Err(Code::Unavailable.into());
        }
        let current = self.0.principal(id).map_err(|error| trust_error(&error))?;
        if current.kind != "agent" {
            return Err(Code::Unauthorized.into());
        }
        Ok(principal(current))
    }
    fn iphones(&self) -> Result<Box<RawValue>, Error> {
        let credentials = self.0.load().map_err(|e| trust_error(&e))?;
        raw(&credentials
            .principals
            .iter()
            .filter(|p| p.kind == "iphone")
            .map(|p| serde_json::json!({"id":p.id}))
            .collect::<Vec<_>>())
    }
    fn authenticate(&self, token: &str) -> Result<Principal, Error> {
        Ok(principal(
            self.0
                .authenticate_token(token)
                .map_err(|error| trust_error(&error))?,
        ))
    }
    fn current(&self, original: &Principal) -> Result<Principal, Error> {
        let credentials = self.0.load().map_err(|error| trust_error(&error))?;
        if original.kind == "delegated" {
            let (parent, _) = original.id.split_once('/').ok_or(Code::Unauthorized)?;
            let parent = credentials
                .principals
                .iter()
                .find(|principal| {
                    principal.id == parent && principal.kind == "replica" && principal.active()
                })
                .ok_or(Code::Unauthorized)?;
            let ceiling = original
                .grants
                .iter()
                .map(|grant| ab_trust::Grant::parse(grant))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| trust_error(&error))?;
            let mut current = original.clone();
            current.grants = ab_trust::effective_grants(&ceiling, &parent.grants)
                .iter()
                .map(|grant| grant.as_str().to_owned())
                .collect();
            return Ok(current);
        }
        let mut current = credentials
            .principals
            .into_iter()
            .find(|p| p.id == original.id && p.active())
            .ok_or(Code::Unauthorized)?;
        let ceiling = original
            .grants
            .iter()
            .map(|g| ab_trust::Grant::parse(g))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| trust_error(&error))?;
        current.grants = ab_trust::effective_grants(&current.grants, &ceiling);
        if let Some(enrollment) = credentials.enrollment {
            current.grants = ab_trust::effective_grants(&current.grants, &enrollment.grants);
        }
        Ok(principal(current))
    }
    fn sign(&self, original: &Principal, id: &str, hash: &str) -> Result<String, Error> {
        let current = self.current(original)?;
        let grants = current
            .grants
            .iter()
            .map(|g| ab_trust::Grant::parse(g))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| trust_error(&error))?;
        self.0
            .sign_delegation(
                &ab_trust::Principal {
                    id: current.id,
                    kind: current.kind,
                    level: current.level,
                    grants,
                    ..ab_trust::Principal::default()
                },
                id,
                hash,
                "",
            )
            .map_err(|error| trust_error(&error))
    }
}
pub fn empty_command() -> Result<Command, Error> {
    serde_json::from_str("{\"args\":null}").map_err(|_| Code::Corrupt.into())
}
