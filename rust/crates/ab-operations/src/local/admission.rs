use super::{
    Arc, BorrowedUploads, Code, Command, CommandRecord, Cursor, Disposition, Duration, Error,
    LocalGateway, Principal, RawValue, State, descriptor_for, raw, timestamp, valid_command_id,
};

type StagePhotos<'a> =
    Box<dyn FnOnce(Vec<String>) -> crate::Task<'a, StagedPhotoMessage> + Send + 'a>;

#[derive(Debug, serde::Serialize)]
pub struct StagedPhoto {
    pub name: String,
    pub upload_id: String,
}

#[derive(Debug)]
pub struct StagedPhotoMessage {
    pub recipient: serde_json::Map<String, super::Value>,
    pub files: Vec<StagedPhoto>,
}

impl LocalGateway {
    pub async fn submit(
        self: &Arc<Self>,
        principal: &Principal,
        command: Command,
    ) -> Result<Box<RawValue>, Error> {
        self.submit_inner(principal, command, None).await
    }

    pub(crate) async fn submit_photo_message(
        self: &Arc<Self>,
        principal: &Principal,
        command: Command,
        stage: StagePhotos<'_>,
    ) -> Result<Box<RawValue>, Error> {
        self.submit_inner(principal, command, Some(stage)).await
    }

    #[allow(clippy::too_many_lines)]
    async fn submit_inner(
        self: &Arc<Self>,
        principal: &Principal,
        mut command: Command,
        stage: Option<StagePhotos<'_>>,
    ) -> Result<Box<RawValue>, Error> {
        let principal = self.authorize(principal, &command.op)?;
        if principal.is_agent() && principal.level == "read" {
            return Err(Code::Unauthorized.into());
        }
        if !valid_command_id(&command.id) || command.wait_ms > 15000 {
            return Err(Code::Invalid.into());
        }
        let descriptor = descriptor_for(&command.op)?;
        if descriptor.kind != ab_catalog::OperationKind::Command {
            return Err(Code::Unsupported.into());
        }
        command.origin = if principal.kind == "device"
            || principal.kind == "delegated" && principal.id.contains("/device:")
            || principal.is_agent() && matches!(principal.level.as_str(), "full" | "approve")
        {
            "interactive".into()
        } else if command.op == "messages.send"
            && principal.kind == "token"
            && !principal.remote
            && principal.id.starts_with("token:")
        {
            "owner_self".into()
        } else {
            String::new()
        };
        let mut arguments = crate::command_contract::parse_arguments(&command)?;
        let selector = reminder_selector(&command, &mut arguments)?;
        if selector.is_some() {
            principal.require_explicit_scope("reminders:read")?;
        }
        let photos = if stage.is_some() {
            let photos = photo_assets(&command, &arguments)?;
            command.args = raw(&arguments)?;
            Some(photos)
        } else {
            None
        };
        let binds_assets = command.op == "messages.send" && arguments.get("asset_ids").is_some();
        if binds_assets {
            principal.require_explicit_scope("photos:read")?;
        }
        let hash = self.codec.command_hash(&command)?;
        let request_id = command.id.clone();
        let lock = self.lock(format!("admission:{}\0{request_id}", principal.id))?;
        let guard = lock.lock().await;
        match self.state.principal_command(&principal.id, &request_id) {
            Ok(old) => {
                if old.request_hash != hash {
                    return Err(Code::IdReused.into());
                }
                let lock = self.lock(format!("command:{}", old.command.id))?;
                let _guard = lock.lock().await;
                let old = self.expire(self.state.command(&old.command.id)?)?;
                if let Some(error) = old.error {
                    return Err(error);
                }
                return raw(&old);
            }
            Err(error) if error.code == Code::NotFound => {}
            Err(error) => return Err(error),
        }
        command.id = format!("cmd_{}", ulid::Ulid::new());
        let era = self.state.admission_era()?;
        if !command.admission_era.is_empty() && command.admission_era != era {
            return Err(Code::Expired.into());
        }
        let created = self.clock.now();
        if let Some(title) = selector {
            let (id, version) = self
                .state
                .reminder_target(&title, command.op == "reminders.uncomplete")?;
            arguments["id"] = id.into();
            command.args = raw(&arguments)?;
            command.expected_version = version;
        }
        let validated = stage
            .is_none()
            .then(|| crate::command_contract::validate_at(&command, created))
            .transpose()?;
        let requested = if command.expires_at == "0001-01-01T00:00:00Z" {
            None
        } else {
            Some(timestamp(&command.expires_at)?)
        };
        let expiry = crate::command_contract::expiry(&command.op, created, requested)?;
        let deadline = tokio::time::Instant::now()
            + Duration::try_from(expiry.duration_since(created)).map_err(|_| Code::Expired)?;
        let approval = principal.asks_first();
        if !(approval
            || self.peer.available()
            || command.op.starts_with("iphone.") && self.phone_notify.is_some())
        {
            return Err(Code::Unavailable.into());
        }
        if let (Some(stage), Some(photos)) = (stage, photos) {
            let staged = tokio::time::timeout_at(deadline, stage(photos))
                .await
                .map_err(|_| Code::Deadline)??;
            let fields = arguments.as_object_mut().ok_or(Code::Invalid)?;
            for field in ["to", "chat_id", "service"] {
                fields.remove(field);
            }
            fields.extend(staged.recipient);
            fields.insert(
                "files".into(),
                serde_json::to_value(staged.files).map_err(|_| Code::Corrupt)?,
            );
            command.args = raw(&arguments)?;
        }
        let target = match validated {
            Some(target) => target,
            None => crate::command_contract::validate_at(&command, created)?,
        };
        if binds_assets {
            arguments
                .as_object_mut()
                .ok_or(Code::Invalid)?
                .remove("asset_ids");
            command.args = raw(&arguments)?;
        }
        let wait = command.wait_ms;
        let id = command.id.clone();
        let borrowed = if let Some(store) = &self.query_store {
            BorrowedUploads {
                references: store.message_uploads(&principal.id, &command).await?,
                lease: None,
            }
        } else {
            self.actions.borrow_uploads(&principal, &command)?
        };
        let _upload_lease = borrowed.lease;
        let record = CommandRecord {
            parts: Vec::new(),
            upload_references: borrowed.references,
            command,
            principal: principal.id.clone(),
            source: descriptor
                .scope
                .split(':')
                .next()
                .ok_or(Code::Corrupt)?
                .to_owned(),
            request_hash: hash,
            state: if approval {
                State::AwaitingApproval
            } else {
                State::Admitted
            },
            admission_era: era,
            created_at: created.to_string(),
            expires_at: if approval {
                created
                    .checked_add(jiff::SignedDuration::from_hours(24))
                    .map_err(|_| Code::Invalid)?
                    .to_string()
            } else {
                expiry.to_string()
            },
            evidence: None,
            disposition: approval.then_some(Disposition::AwaitingApproval),
            commit_seq: 0,
            observed_record_ids: Vec::new(),
            cursor: Cursor::default(),
            history: Vec::new(),
            error: None,
        };
        let record = self.state.admit(&request_id, record)?;
        drop(guard);
        if approval {
            return raw(&record);
        }
        let gateway = self.clone();
        let mut task =
            tokio::spawn(async move { gateway.execute(record, principal, target, deadline).await });
        match tokio::time::timeout(Duration::from_millis(wait), &mut task).await {
            Ok(result) => {
                let record = result.map_err(|_| Code::Unavailable)??;
                if let Some(error) = record.error {
                    return Err(error);
                }
                raw(&record)
            }
            Err(_) => raw(&self.state.command(&id)?),
        }
    }
}

fn reminder_selector(command: &Command, args: &mut super::Value) -> Result<Option<String>, Error> {
    let Some(fields) = args.as_object_mut() else {
        return Ok(None);
    };
    let Some(selector) = fields.remove("reminder") else {
        return Ok(None);
    };
    if !matches!(
        command.op.as_str(),
        "reminders.update" | "reminders.complete" | "reminders.uncomplete" | "reminders.delete"
    ) || fields.keys().any(|key| key.eq_ignore_ascii_case("id"))
        || command.expected_version != 0
    {
        return Err(Code::Invalid.into());
    }
    selector
        .as_str()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(|title| Some(title.to_owned()))
        .ok_or_else(|| Code::Invalid.into())
}

fn photo_assets(command: &Command, args: &super::Value) -> Result<Vec<String>, Error> {
    if command.op != "messages.send" || args.get("files").is_some() {
        return Err(Code::Invalid.into());
    }
    let ids = args["asset_ids"]
        .as_array()
        .filter(|ids| (1..=10).contains(&ids.len()))
        .ok_or(Code::Invalid)?;
    ids.iter()
        .map(|id| {
            id.as_str()
                .filter(|id| !id.trim().is_empty() && id.len() <= 256)
                .map(str::to_owned)
                .ok_or_else(|| Code::Invalid.into())
        })
        .collect()
}
