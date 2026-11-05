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
        if command.op.starts_with("calendar.") {
            return self.calendar(principal, command);
        }
        if command.op.starts_with("messages.") {
            return self.messages(principal, command);
        }
        if command.op.starts_with("mac.") && command.op != "mac.screen_record" {
            return mac_action(command);
        }
        if native_command(&command.op) {
            return native_action(command);
        }
        self.prepare(principal, command).map_err(|error| {
            if command.op.starts_with("notes.") {
                note_failure(error)
            } else {
                error
            }
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
        if command.op.starts_with("messages.") {
            return Ok(problem);
        }
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let id = args["id"].as_str().ok_or(Code::Corrupt)?;
        let collection = collection(&command.op);
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
                command.op.split('.').next().ok_or(Code::Invalid)?,
                evidence["capture_checkpoint"]
                    .as_str()
                    .ok_or(Code::Corrupt)?,
                collection,
                &native_key,
            )
            .map_err(|_| Code::Corrupt)?;
        let visibility = ab_operations::state_visibility(&principal.visibility());
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

fn collection(op: &str) -> &str {
    if op.starts_with("calendar.") {
        "calendar.event"
    } else if op.starts_with("contacts.") {
        if op.contains("group") || op.ends_with("_member") {
            "contacts.group"
        } else {
            "contacts.contact"
        }
    } else if op.starts_with("notes.") {
        if op.ends_with("_folder") {
            "notes.folder"
        } else {
            "notes.note"
        }
    } else if op.ends_with("_list") {
        "reminders.list"
    } else {
        "reminders.reminder"
    }
}
impl Actions {
    fn visible_key(
        &self,
        principal: &Principal,
        collection: &str,
        id: &str,
    ) -> Result<String, Error> {
        self.state.with(|state| {
            let result = state.read(
                &ab_operations::state_visibility(&principal.visibility()),
                &ab_state::ReadQuery {
                    collection: collection.into(),
                    id: id.into(),
                    ..ab_state::ReadQuery::default()
                },
            )?;
            if result.records.is_empty() {
                return Err(ab_state::Error::Contract(
                    ab_trust::domain::ErrorCode::NotFound,
                ));
            }
            state.native_key(collection, id)
        })
    }
    fn writable_folder(&self, principal: &Principal, id: &str) -> Result<(), Error> {
        let mut id = id.to_owned();
        let mut seen = std::collections::BTreeSet::new();
        while !id.is_empty() {
            if !seen.insert(id.clone()) {
                return Err(Code::Corrupt.into());
            }
            let result = self.state.with(|state| {
                state.read(
                    &ab_operations::state_visibility(&principal.visibility()),
                    &ab_state::ReadQuery {
                        collection: "notes.folder".into(),
                        id: id.clone(),
                        ..ab_state::ReadQuery::default()
                    },
                )
            })?;
            let record = result.records.first().ok_or(Code::NotFound)?;
            let data: Value = serde_json::from_str(record.data.get()).map_err(|_| Code::Corrupt)?;
            if data["read_only"] == true || data["recently_deleted"] == true {
                return Err(Code::Unsupported.into());
            }
            id = data["parent_id"].as_str().unwrap_or_default().into();
        }
        Ok(())
    }
}

fn command_error(error: ab_commands::Error) -> Error {
    match error {
        ab_commands::Error::Invalid => Code::Invalid,
        ab_commands::Error::Unsupported => Code::Unsupported,
        ab_commands::Error::TooLarge => Code::TooLarge,
        ab_commands::Error::Expired => Code::Expired,
    }
    .into()
}

impl Actions {
    #[allow(clippy::too_many_lines)]
    fn prepare(&self, principal: &Principal, command: &Command) -> Result<PreparedAction, Error> {
        if !["reminders.", "contacts.", "notes."]
            .iter()
            .any(|prefix| command.op.starts_with(prefix))
        {
            return Err(Code::Unsupported.into());
        }
        let validated_command = ab_commands::Command {
            id: command.id.clone(),
            op: command.op.clone(),
            args: command.args.clone(),
            expected_version: command.expected_version,
            wait_ms: i64::try_from(command.wait_ms).map_err(|_| Code::Invalid)?,
        };
        ab_commands::validate(&validated_command, jiff::Timestamp::now()).map_err(command_error)?;
        let args: Value = serde_json::from_str(command.args.get()).map_err(|_| Code::Invalid)?;
        let id = args["id"].as_str().unwrap_or_default();
        let mut list_id = args["list_id"].as_str().unwrap_or_default().to_owned();
        let collection = collection(&command.op);
        let mut action = json!({"op":command.op,"args":args});
        if !command.origin.is_empty() {
            action["origin"] = json!(command.origin);
        }
        if command.op.starts_with("notes.") {
            action["note_args"] = args.clone();
        }
        if let Some(contact) = args["contact_id"].as_str() {
            action["list_key"] = json!(self.visible_key(principal, "contacts.contact", contact)?);
        }
        let mut expected_state = None;
        if !id.is_empty() {
            let visibility = ab_operations::state_visibility(&principal.visibility());
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
            if !command.op.starts_with("notes.") && record.version != command.expected_version {
                let mut error = Error::from(Code::Conflict);
                error.current =
                    Some(serde_json::value::to_raw_value(&record).map_err(|_| Code::Corrupt)?);
                return Err(error);
            }
            action["native_key"] =
                json!(self.state.with(|state| state.native_key(collection, id))?);
            let data: Value = serde_json::from_str(record.data.get()).map_err(|_| Code::Corrupt)?;
            if collection == "reminders.reminder" {
                let container = data["list_id"].as_str().ok_or(Code::Corrupt)?;
                action["expected_list_key"] = json!(
                    self.state
                        .with(|state| state.native_key("reminders.list", container))?
                );
                if list_id.is_empty() {
                    list_id = container.into();
                }
            }
            if command.op.starts_with("notes.") {
                if command.op == "notes.rename_folder" {
                    self.writable_folder(principal, id)?;
                    if !data["title"]
                        .as_str()
                        .unwrap_or_default()
                        .starts_with("Bridge Test [ab-test")
                    {
                        return Err(Code::Unsupported.into());
                    }
                }
                let content_version = data["content_version"]
                    .as_u64()
                    .filter(|version| *version > 0)
                    .unwrap_or(record.version)
                    .min(record.version);
                if command.expected_version < content_version
                    || command.expected_version > record.version
                {
                    let mut error = Error::from(Code::Conflict);
                    error.current =
                        Some(serde_json::value::to_raw_value(&record).map_err(|_| Code::Corrupt)?);
                    return Err(error);
                }
                if command.op == "notes.update"
                    && args["html"].as_str().is_some_and(|html| {
                        !ab_commands::note_html_compatible(record.data.get().as_bytes(), html)
                    })
                {
                    return Err(Code::Unsupported.into());
                }
                if data["recently_deleted"] == true
                    || command.op == "notes.update" && note_update_refused(&data, &args)
                {
                    return Err(Code::Unsupported.into());
                }
                if let Some(folder) = data["folder_id"].as_str() {
                    self.writable_folder(principal, folder)?;
                }
            }
            let mut expected = data;
            if collection == "contacts.group"
                && let Some(members) = expected["member_ids"].as_array_mut()
            {
                for member in members {
                    let key = self.state.with(|state| {
                        state.native_key("contacts.contact", member.as_str().unwrap_or_default())
                    })?;
                    *member = json!({"$ref":{"collection":"contacts.contact","native_key":key}});
                }
            }
            expected_state =
                Some(serde_json::value::to_raw_value(&expected).map_err(|_| Code::Corrupt)?);
        }
        if !list_id.is_empty() {
            let visibility = ab_operations::state_visibility(&principal.visibility());
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
        if let Some(folder) = args["folder_id"].as_str() {
            self.writable_folder(principal, folder)?;
            action["folder_key"] = json!(self.visible_key(principal, "notes.folder", folder)?);
        }
        Ok(PreparedAction {
            action: {
                let context = serde_json::from_value(action).map_err(|_| Code::Corrupt)?;
                let bytes = ab_commands::native_action(&validated_command, &context)
                    .map_err(command_error)?;
                serde_json::from_slice(&bytes).map_err(|_| Code::Corrupt)?
            },
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
}

fn note_failure(mut error: Error) -> Error {
    error.message = match error.code {
        Code::Unsupported => "Notes can't apply this change. Use a writable plain-text note, or preserve the existing formatting in an HTML update. Edit locked or unsupported content in Notes.",
        Code::Conflict => "This note or folder changed. Refresh it and try again.",
        Code::Unavailable => "Notes didn't respond. Open Notes, check its Automation access, and try again.",
        Code::NotFound => "This note or folder is no longer available. Refresh Notes and try again.",
        Code::Unauthorized => "This connection can't change Notes. Reconnect with Notes write access.",
        Code::Invalid => "The Notes request is incomplete or invalid. Check the note or folder details and try again.",
        _ => return error,
    }.into();
    error
}

fn mac_action(command: &Command) -> Result<PreparedAction, Error> {
    let command_shape = ab_commands::Command {
        id: command.id.clone(),
        op: command.op.clone(),
        args: command.args.clone(),
        expected_version: command.expected_version,
        wait_ms: i64::try_from(command.wait_ms).map_err(|_| Code::Invalid)?,
    };
    let bytes = ab_commands::native_action(
        &command_shape,
        &ab_commands::ActionContext {
            origin: command.origin.clone(),
            ..ab_commands::ActionContext::default()
        },
    )
    .map_err(command_error)?;
    Ok(PreparedAction {
        action: serde_json::from_slice(&bytes).map_err(|_| Code::Corrupt)?,
        expected_state: None,
        collection: String::new(),
        serialization_key: "lists".into(),
    })
}

fn native_action(command: &Command) -> Result<PreparedAction, Error> {
    Ok(PreparedAction {
        action: serde_json::value::to_raw_value(&json!({
            "op": command.op, "args": command.args, "origin": command.origin
        }))
        .map_err(|_| Code::Corrupt)?,
        expected_state: None,
        collection: if command.op == "pdf.annotate" {
            "pdf.artifact"
        } else if command.op.starts_with("files.") {
            "files.item"
        } else {
            ""
        }
        .into(),
        serialization_key: if command.op.starts_with("files.") {
            "files"
        } else {
            &command.op
        }
        .into(),
    })
}

fn native_command(op: &str) -> bool {
    op.starts_with("files.")
        || matches!(
            op,
            "pdf.annotate"
                | "maps.open"
                | "speech.say"
                | "capture.play_sound"
                | "capture.screenshot"
                | "capture.camera_photo"
                | "capture.record_audio"
                | "mac.screen_record"
                | "print.document"
                | "shortcuts.run"
                | "focus.set"
        )
}

fn note_update_refused(data: &Value, args: &Value) -> bool {
    let finished = data["content_status"] == "complete";
    data["locked"] == true || finished && args["html"].is_null() && data["editable"] != "plain"
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_note_whose_body_has_not_arrived_is_not_refused_yet() {
        use super::note_update_refused;
        use serde_json::json;
        let args = json!({"plain_text": "x"});
        let pending = json!({"content_status": "partial", "editable": "none"});
        assert!(!note_update_refused(&pending, &args));
        let finished = json!({"content_status": "complete", "editable": "none"});
        assert!(note_update_refused(&finished, &args));
        let plain = json!({"content_status": "complete", "editable": "plain"});
        assert!(!note_update_refused(&plain, &args));
        let locked = json!({"content_status": "locked", "editable": "none", "locked": true});
        assert!(note_update_refused(&locked, &args));
    }

    use super::*;

    #[test]
    fn admission_resolves_admin_and_scoped_targets_without_exposing_hidden_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        let home =
            std::env::temp_dir().join(format!("admission-visibility-{}", std::process::id()));
        let store = GatewayStore::open(&home)?;
        let capture = serde_json::from_value(json!({
            "source":"reminders", "generation":"synthetic", "checkpoint":"one",
            "observations":[{"collection":"reminders.list", "native_key":"native-list",
                "kind":"upsert", "observed_at":"2026-01-01T00:00:00Z",
                "coverage":{"fields":"full","missing":null}, "data":{"title":"Synthetic"}}]
        }))?;
        store.with(|state| state.commit_capture(&capture))?;
        let id = store
            .with(|state| {
                state.read(
                    &ab_state::Visibility::from([("reminders".into(), true)]),
                    &ab_state::ReadQuery {
                        collection: "reminders.list".into(),
                        ..Default::default()
                    },
                )
            })?
            .records[0]
            .id
            .as_str()
            .to_owned();
        let actions = Actions {
            state: store,
            peer: std::sync::Arc::new(ab_applepeer::Commands::default()),
        };
        let command: Command = serde_json::from_value(json!({
            "command_id":"cmd_visibility_regression", "op":"reminders.create",
            "args":{"list_id":id,"title":"Synthetic"}
        }))?;
        let mut principal = Principal {
            id: "agent:synthetic".into(),
            kind: "agent".into(),
            grants: vec!["admin".into()],
            level: "act".into(),
            resource: String::new(),
            remote: false,
        };
        for grant in ["admin", "reminders:write"] {
            principal.grants = vec![grant.into()];
            let prepared = actions.resolve(&principal, &command)?;
            let action: Value = serde_json::from_str(prepared.action.get())?;
            assert_eq!(action["list_key"], "native-list");
        }
        principal.grants = vec!["contacts:write".into()];
        assert_eq!(
            actions
                .resolve(&principal, &command)
                .err()
                .ok_or("hidden target exposed")?
                .code,
            Code::NotFound
        );
        drop(actions);
        std::fs::remove_dir_all(home)?;
        Ok(())
    }
}
