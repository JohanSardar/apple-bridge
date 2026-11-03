use crate::actions::Actions;
use ab_operations::{Code, Command, Error, Principal, local::PreparedAction};
use serde_json::{Value, json};

impl Actions {
    fn message_record(
        &self,
        principal: &Principal,
        collection: &str,
        id: &str,
    ) -> Result<ab_trust::domain::Record, Error> {
        let visibility = principal.visibility().into_iter().collect();
        self.state.with(|state| {
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
        })
    }

    pub(crate) fn messages(
        &self,
        principal: &Principal,
        command: &Command,
    ) -> Result<PreparedAction, Error> {
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let id = args["id"].as_str().unwrap_or_default();
        let chat = args["chat_id"].as_str().unwrap_or_default();
        let collection = match command.op.as_str() {
            "messages.mark_read" | "messages.mark_unread" | "messages.delete_chat" => {
                "messages.chat"
            }
            _ => "messages.message",
        };
        let mut action =
            json!({"op": command.op, "args": {}, "messages": args, "origin": command.origin});
        let mut expected_state = None;
        if !id.is_empty() {
            let record = self.message_record(principal, collection, id)?;
            if command.expected_version != 0 && record.version != command.expected_version {
                let mut error = Error::from(Code::Conflict);
                error.current =
                    Some(serde_json::value::to_raw_value(&record).map_err(|_| Code::Corrupt)?);
                return Err(error);
            }
            action["native_key"] =
                json!(self.state.with(|state| state.native_key(collection, id))?);
            expected_state = Some(record.data);
        }
        if !chat.is_empty() {
            self.message_record(principal, "messages.chat", chat)?;
            action["list_key"] = json!(
                self.state
                    .with(|state| state.native_key("messages.chat", chat))?
            );
        }
        if let Some(resend) = args["resend_of"].as_str().filter(|s| !s.is_empty()) {
            let previous = self.message_record(principal, "messages.message", resend)?;
            let data: Value =
                serde_json::from_str(previous.data.get()).map_err(|_| Code::Corrupt)?;
            if data["is_from_me"] != true
                || data["error_code"].as_i64().unwrap_or(0) == 0
                || data["chat_id"] != chat
            {
                return Err(Code::Conflict.into());
            }
            action["messages"]["resend_of"] = json!(
                self.state
                    .with(|state| state.native_key("messages.message", resend))?
            );
        }
        Ok(PreparedAction {
            action: serde_json::value::to_raw_value(&action).map_err(|_| Code::Corrupt)?,
            expected_state,
            collection: if command.op == "messages.react" {
                "messages.reaction"
            } else {
                collection
            }
            .into(),
            serialization_key: if !chat.is_empty() {
                chat
            } else if !id.is_empty() {
                id
            } else {
                "messages"
            }
            .into(),
        })
    }
}
