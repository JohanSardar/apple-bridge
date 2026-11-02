use crate::{Code, Command, Cursor, Error, Identity, Principal, ReadQuery, ReadStore, Visibility};
use serde::Serialize;
use serde_json::value::RawValue;
use std::sync::{Arc, Mutex};
#[derive(Debug)]
pub struct StateReads(pub Arc<Mutex<ab_state::State<ab_state::Replica>>>);
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
fn visibility(input: &Visibility) -> ab_state::Visibility {
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
impl ReadStore for StateReads {
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
        raw(&state
            .read_model(&visibility(visible), name, query)
            .map_err(|error| state_error(&error))?)
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
    fn authenticate(&self, token: &str) -> Result<Principal, Error> {
        Ok(principal(
            self.0
                .authenticate_token(token)
                .map_err(|error| trust_error(&error))?,
        ))
    }
    fn current(&self, original: &Principal) -> Result<Principal, Error> {
        let credentials = self.0.load().map_err(|error| trust_error(&error))?;
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
