use ab_operations::{
    Code, Command, Error, GatewayStore, Principal,
    local::{ActionResolver, PreparedAction},
};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct Actions {
    pub state: GatewayStore,
    pub peer: std::sync::Arc<dyn ab_operations::peer::ApplePeer>,
}
impl ActionResolver for Actions {
    fn resolve(&self, principal: &Principal, command: &Command) -> Result<PreparedAction, Error> {
        if !command.op.starts_with("reminders.") {
            return Err(Code::Unsupported.into());
        }
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let id = args["id"].as_str().unwrap_or_default();
        let mut list_id = args["list_id"].as_str().unwrap_or_default().to_owned();
        let collection = if matches!(
            command.op.as_str(),
            "reminders.rename_list" | "reminders.delete_list" | "reminders.create_list"
        ) {
            "reminders.list"
        } else {
            "reminders.reminder"
        };
        let mut action = json!({"op":command.op,"args":args});
        if !command.origin.is_empty() {
            action["origin"] = json!(command.origin);
        }
        let mut expected_state = None;
        if !id.is_empty() {
            let visibility = principal.visibility().into_iter().collect();
            let record =
                self.state.with(|state| {
                    let result = state.read(
                        &visibility,
                        &ab_state::ReadQuery {
                            collection: collection.into(),
                            id: id.into(),
                            ..ab_state::ReadQuery::default()
                        },
                    )?;
                    result.records.into_iter().next().ok_or_else(|| {
                        ab_state::Error::Contract(ab_trust::domain::ErrorCode::NotFound)
                    })
                })?;
            if record.version != command.expected_version {
                let mut error = Error::from(Code::Conflict);
                error.current =
                    Some(serde_json::value::to_raw_value(&record).map_err(|_| Code::Corrupt)?);
                return Err(error);
            }
            action["native_key"] =
                json!(self.state.with(|state| state.native_key(collection, id))?);
            let data: Value = serde_json::from_str(record.data.get()).map_err(|_| Code::Corrupt)?;
            if collection != "reminders.list" {
                let container = data["list_id"].as_str().ok_or(Code::Corrupt)?;
                action["expected_list_key"] = json!(
                    self.state
                        .with(|state| state.native_key("reminders.list", container))?
                );
                if list_id.is_empty() {
                    list_id = container.into();
                }
            }
            expected_state = Some(record.data);
        }
        if !list_id.is_empty() {
            let visibility = principal.visibility().into_iter().collect();
            self.state.with(|state| {
                let visible = state.read(
                    &visibility,
                    &ab_state::ReadQuery {
                        collection: "reminders.list".into(),
                        id: list_id.clone(),
                        ..ab_state::ReadQuery::default()
                    },
                )?;
                if visible.records.is_empty() {
                    return Err(ab_state::Error::Contract(
                        ab_trust::domain::ErrorCode::NotFound,
                    ));
                }
                Ok(())
            })?;
            action["list_key"] = json!(
                self.state
                    .with(|state| state.native_key("reminders.list", &list_id))?
            );
        }
        Ok(PreparedAction {
            action: serde_json::value::to_raw_value(&action).map_err(|_| Code::Corrupt)?,
            expected_state,
            collection: collection.into(),
            serialization_key: if !id.is_empty() {
                id.into()
            } else if !list_id.is_empty() {
                list_id
            } else {
                "lists".into()
            },
        })
    }
    fn conflict(
        &self,
        principal: &Principal,
        command: &Command,
        frame: &ab_wire::messages::IpcFrame,
    ) -> Result<Option<Error>, Error> {
        let problem: Option<Error> = frame
            .error
            .as_ref()
            .map(|raw| serde_json::from_str(raw.get()).map_err(|_| Code::Corrupt))
            .transpose()?;
        if problem
            .as_ref()
            .is_none_or(|problem| problem.code != Code::Conflict)
        {
            return Ok(problem);
        }
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let id = args["id"].as_str().ok_or(Code::Corrupt)?;
        let collection = if command.op.ends_with("_list") {
            "reminders.list"
        } else {
            "reminders.reminder"
        };
        let native_key = self.state.with(|state| state.native_key(collection, id))?;
        let evidence: Value =
            serde_json::from_str(frame.evidence.as_ref().ok_or(Code::Corrupt)?.get())
                .map_err(|_| Code::Corrupt)?;
        if native_key.is_empty() || evidence["native_key"] != native_key {
            return Err(Code::Corrupt.into());
        }
        let cursor = self
            .peer
            .captured(
                "reminders",
                evidence["capture_checkpoint"]
                    .as_str()
                    .ok_or(Code::Corrupt)?,
                collection,
                &native_key,
            )
            .map_err(|_| Code::Corrupt)?;
        let visibility = principal.visibility().into_iter().collect();
        let result = self.state.with(|state| {
            state.read(
                &visibility,
                &ab_state::ReadQuery {
                    collection: collection.into(),
                    id: id.into(),
                    include_deleted: true,
                    ..ab_state::ReadQuery::default()
                },
            )
        })?;
        if result.cursor.epoch != cursor.epoch
            || result.cursor.sequence < cursor.sequence
            || result.records.len() != 1
        {
            return Err(Code::Corrupt.into());
        }
        let mut error = Error::from(Code::Conflict);
        error.current =
            Some(serde_json::value::to_raw_value(&result.records[0]).map_err(|_| Code::Corrupt)?);
        Ok(Some(error))
    }
}
