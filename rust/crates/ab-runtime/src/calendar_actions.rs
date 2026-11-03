use crate::actions::Actions;
use ab_operations::{Code, Command, Error, Principal, local::PreparedAction};
use serde_json::{Value, json};

impl Actions {
    pub(crate) fn calendar(
        &self,
        principal: &Principal,
        command: &Command,
    ) -> Result<PreparedAction, Error> {
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let id = args["id"].as_str().unwrap_or_default();
        let mut calendar_id = args["calendar_id"].as_str().unwrap_or_default().to_owned();
        let collection = if command.op == "calendar.create_calendar" {
            "calendar.calendar"
        } else {
            "calendar.event"
        };
        let visibility = principal.visibility().into_iter().collect();
        let mut action = json!({"op":command.op,"args":args,"calendar_args":args});
        if !command.origin.is_empty() {
            action["origin"] = json!(command.origin);
        }
        let mut expected_state = None;
        let mut serialization_key = id.to_owned();
        if id.is_empty() {
            serialization_key = format!("calendar:{calendar_id}");
        } else {
            let record = self.state.with(|state| {
                state
                    .read(
                        &visibility,
                        &ab_state::ReadQuery {
                            collection: collection.into(),
                            id: id.into(),
                            ..ab_state::ReadQuery::default()
                        },
                    )?
                    .records
                    .into_iter()
                    .next()
                    .ok_or(ab_state::Error::Contract(
                        ab_trust::domain::ErrorCode::NotFound,
                    ))
            })?;
            if record.version != command.expected_version {
                let mut error = Error::from(Code::Conflict);
                error.current =
                    Some(serde_json::value::to_raw_value(&record).map_err(|_| Code::Corrupt)?);
                return Err(error);
            }
            let data: Value = serde_json::from_str(record.data.get()).map_err(|_| Code::Corrupt)?;
            let container = data["calendar_id"].as_str().ok_or(Code::Corrupt)?;
            action["native_key"] =
                json!(self.state.with(|state| state.native_key(collection, id))?);
            action["expected_list_key"] = json!(
                self.state
                    .with(|state| state.native_key("calendar.calendar", container))?
            );
            if let Some(series) = data["series_id"].as_str().filter(|s| !s.is_empty()) {
                serialization_key = format!("calendar-series:{container}:{series}");
            }
            if calendar_id.is_empty() {
                calendar_id = container.into();
            }
            expected_state = Some(record.data);
        }
        if !calendar_id.is_empty() {
            self.state.with(|state| {
                let result = state.read(
                    &visibility,
                    &ab_state::ReadQuery {
                        collection: "calendar.calendar".into(),
                        id: calendar_id.clone(),
                        ..ab_state::ReadQuery::default()
                    },
                )?;
                if result.records.is_empty() {
                    return Err(ab_state::Error::Contract(
                        ab_trust::domain::ErrorCode::NotFound,
                    ));
                }
                Ok(())
            })?;
            action["list_key"] = json!(
                self.state
                    .with(|state| state.native_key("calendar.calendar", &calendar_id))?
            );
        }
        Ok(PreparedAction {
            action: serde_json::value::to_raw_value(&action).map_err(|_| Code::Corrupt)?,
            expected_state,
            collection: collection.into(),
            serialization_key,
        })
    }
}
