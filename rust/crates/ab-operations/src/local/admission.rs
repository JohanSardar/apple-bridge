use super::{
    Arc, BorrowedUploads, Code, Command, CommandRecord, Cursor, Disposition, Duration, Error,
    LocalGateway, Principal, RawValue, State, descriptor_for, raw, timestamp, valid_command_id,
};

impl LocalGateway {
    #[allow(clippy::too_many_lines)]
    pub async fn submit(
        self: &Arc<Self>,
        principal: &Principal,
        mut command: Command,
    ) -> Result<Box<RawValue>, Error> {
        let principal = self.authorize(principal, &command.op)?;
        if principal.is_agent() && principal.level == "read" {
            return Err(Code::Unauthorized.into());
        }
        if !valid_command_id(&command.id) || command.wait_ms > 15000 {
            return Err(Code::Invalid.into());
        }
        if matches!(
            command.op.as_str(),
            "mac.lock_screen" | "mac.sleep_display" | "mac.restart" | "mac.shutdown"
        ) && (!principal.is_agent() || principal.level != "approve")
        {
            return Err(Code::Forbidden.into());
        }
        if command.op == "mac.clipboard_write"
            && principal.is_agent()
            && principal.level == "approve"
        {
            return Err(Code::Unsupported.into());
        }
        let descriptor = descriptor_for(&command.op)?;
        if descriptor.kind != ab_catalog::OperationKind::Command {
            return Err(Code::Unsupported.into());
        }
        self.actions.validate(&command)?;
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
        let hash = self.codec.command_hash(&command)?;
        let lock = self.lock(format!("command:{}", command.id))?;
        let guard = lock.lock().await;
        match self.state.command(&command.id) {
            Ok(old) => {
                if old.principal != principal.id || old.request_hash != hash {
                    return Err(Code::IdReused.into());
                }
                let old = self.expire(old)?;
                if let Some(error) = old.error {
                    return Err(error);
                }
                return raw(&old);
            }
            Err(error) if error.code == Code::NotFound => {}
            Err(error) => return Err(error),
        }
        let era = self.state.admission_era()?;
        if !command.admission_era.is_empty() && command.admission_era != era {
            return Err(Code::Expired.into());
        }
        let created = self.clock.now();
        let requested = if command.expires_at == "0001-01-01T00:00:00Z" {
            None
        } else {
            Some(timestamp(&command.expires_at)?)
        };
        let expiry = crate::command_contract::expiry(&command.op, created, requested)?;
        let deadline = tokio::time::Instant::now()
            + Duration::try_from(expiry.duration_since(created)).map_err(|_| Code::Expired)?;
        let approval = principal.is_agent()
            && (principal.level == "approve"
                || matches!(
                    command.op.as_str(),
                    "focus.set"
                        | "capture.screenshot"
                        | "mac.screen_record"
                        | "capture.camera_photo"
                        | "capture.record_audio"
                ));
        if !(approval
            || self.peer.available()
            || command.op.starts_with("iphone.") && self.phone_notify.is_some())
        {
            return Err(Code::Unavailable.into());
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
        let record = self.state.transition(None, record)?;
        drop(guard);
        if approval {
            return raw(&record);
        }
        let gateway = self.clone();
        let mut task =
            tokio::spawn(async move { gateway.execute(record, principal, deadline).await });
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
