use crate::{
    Code, Cursor, Error,
    local::{CommandRecord, GatewayState},
};
use ab_state::{Gateway, State};
use ab_trust::{
    domain::{CommandState, Coverage, Source},
    wire::messages::{Capture, IpcFrame},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct GatewayStore(pub Arc<Mutex<State<Gateway>>>);
fn convert<A: Serialize, B: DeserializeOwned>(value: &A) -> Result<B, Error> {
    serde_json::from_slice(&serde_json::to_vec(value).map_err(|_| Code::Corrupt)?)
        .map_err(|_| Code::Corrupt.into())
}
fn state_error(error: &ab_state::Error) -> Error {
    if let ab_state::Error::MissingScope(scope) = error {
        return Error::missing_scope(scope);
    }
    if matches!(error, ab_state::Error::IdReused) {
        return Code::IdReused.into();
    }
    if let ab_state::Error::Contract(code) = error
        && let Ok(code) = convert::<_, Code>(code)
    {
        return Error::from(code);
    }
    if ab_state::is_capacity(error) {
        return Code::Capacity.into();
    }
    Code::Unavailable.into()
}
impl GatewayStore {
    pub fn open(home: impl AsRef<Path>) -> Result<Self, Error> {
        Ok(Self(Arc::new(Mutex::new(
            State::<Gateway>::open(home).map_err(|error| state_error(&error))?,
        ))))
    }
    pub fn with<T>(
        &self,
        f: impl FnOnce(&mut State<Gateway>) -> ab_state::Result<T>,
    ) -> Result<T, Error> {
        let call = || {
            let mut state = self.0.lock().map_err(|_| Code::Unavailable)?;
            f(&mut state).map_err(|error| state_error(&error))
        };
        if tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
        {
            tokio::task::block_in_place(call)
        } else {
            call()
        }
    }
    pub fn next_fence(&self) -> Result<u64, Error> {
        self.with(State::<Gateway>::next_fence)
    }
    pub fn checkpoints(&self) -> Result<BTreeMap<String, Box<RawValue>>, Error> {
        self.with(|s| s.checkpoints())?
            .into_iter()
            .map(|(key, value)| {
                Ok((
                    key,
                    serde_json::value::to_raw_value(&value).map_err(|_| Code::Corrupt)?,
                ))
            })
            .collect()
    }
    pub fn commit(&self, capture: &Capture) -> Result<(Cursor, bool), Error> {
        self.with(|s| {
            let result = s.commit_capture(capture)?;
            Ok((s.head()?, result.degraded))
        })
        .and_then(|(cursor, degraded)| Ok((convert(&cursor)?, degraded)))
    }
    pub fn source_state(&self, frame: &IpcFrame) -> Result<(), Error> {
        let source: Source = frame.source.ok_or(Code::Invalid)?;
        let key: String = convert(&source)?;
        let coverage: Coverage = frame
            .coverage
            .get(&key)
            .map(|v| serde_json::from_str(v.get()).map_err(|_| Code::Corrupt))
            .transpose()?
            .unwrap_or(
                serde_json::from_value(serde_json::json!({"fields":"","missing":null}))
                    .map_err(|_| Code::Corrupt)?,
            );
        let capabilities: Option<Vec<String>> = frame
            .capabilities
            .get(&key)
            .map(|v| {
                serde_json::from_str::<Option<Vec<String>>>(v.get()).map_err(|_| Code::Corrupt)
            })
            .transpose()?
            .flatten();
        self.with(|s| {
            s.source_state(
                source,
                &frame.state,
                coverage,
                capabilities.as_deref(),
                &frame.reason,
            )
        })
    }
}
impl GatewayState for GatewayStore {
    fn serialization_key(
        &self,
        p: &crate::Principal,
        c: &crate::Command,
        key: String,
    ) -> Result<String, Error> {
        if !c.op.starts_with("calendar.") {
            return Ok(key);
        }
        self.serialization_key_validated(p, c, &crate::command_contract::validate(c)?, key)
    }
    fn serialization_key_validated(
        &self,
        p: &crate::Principal,
        c: &crate::Command,
        target: &crate::command_contract::Validated,
        key: String,
    ) -> Result<String, Error> {
        if !c.op.starts_with("calendar.") || target.id.is_empty() {
            return Ok(key);
        }
        let visibility = crate::adapters::write_visibility(&p.visibility());
        let Ok(result) = self.with(|s| {
            s.read(
                &visibility,
                &ab_state::ReadQuery {
                    collection: "calendar.event".into(),
                    id: target.id.clone(),
                    ..Default::default()
                },
            )
        }) else {
            return Ok(key);
        };
        let Some(record) = result.records.first() else {
            return Ok(key);
        };
        let Ok(data) = serde_json::from_str::<serde_json::Value>(record.data.get()) else {
            return Ok(key);
        };
        let series = data["series_id"].as_str().unwrap_or_default();
        if series.is_empty() {
            return Ok(key);
        }
        Ok(format!(
            "calendar-series:{}:{series}",
            data["calendar_id"].as_str().unwrap_or_default()
        ))
    }

    fn reminder_target(&self, title: &str, completed: bool) -> Result<(String, u64), Error> {
        let record = self.with(|state| state.reminder_target(title, completed))?;
        Ok((record.id.as_str().into(), record.version))
    }

    fn admission_era(&self) -> Result<String, Error> {
        self.with(|s| s.admission_era())
    }
    fn command(&self, id: &str) -> Result<CommandRecord, Error> {
        convert(&self.with(|s| s.command(id))?)
    }
    fn principal_command(&self, principal: &str, id: &str) -> Result<CommandRecord, Error> {
        convert(&self.with(|s| s.principal_command(principal, id))?)
    }
    fn admit(&self, request_id: &str, record: CommandRecord) -> Result<CommandRecord, Error> {
        let record = convert(&record)?;
        convert(&self.with(|s| s.admit_command(request_id, record))?)
    }
    fn commands(&self) -> Result<Vec<CommandRecord>, Error> {
        convert(&self.with(|s| s.commands())?)
    }
    fn visible_commands(
        &self,
        principal: &crate::Principal,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CommandRecord>, Error> {
        let visibility = crate::adapters::write_visibility(&principal.visibility());
        convert(&self.with(|s| s.visible_commands(&principal.id, &visibility, offset, limit))?)
    }
    fn transition(
        &self,
        from: Option<CommandState>,
        record: CommandRecord,
    ) -> Result<CommandRecord, Error> {
        let record = convert(&record)?;
        convert(&self.with(|s| s.transition_command(from, record))?)
    }
    fn update_parts(&self, record: CommandRecord) -> Result<CommandRecord, Error> {
        let record = convert(&record)?;
        convert(&self.with(|s| s.update_command_parts(record))?)
    }
    fn head(&self) -> Result<Cursor, Error> {
        convert(&self.with(|s| s.head())?)
    }
    fn observed_record_id(
        &self,
        collection: &str,
        key: &str,
        cursor: &Cursor,
    ) -> Result<String, Error> {
        let cursor = convert(cursor)?;
        self.with(|s| s.observed_record_id(collection, key, &cursor))
    }
}
