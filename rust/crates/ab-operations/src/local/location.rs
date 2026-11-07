use super::{
    Code, Command, CommandRecord, Cursor, Disposition, Error, LocalGateway, NativeQuery, Principal,
    RawValue, State, json, raw, timestamp,
};

impl LocalGateway {
    pub(super) async fn location_approval(
        &self,
        principal: &Principal,
        query: &NativeQuery,
    ) -> Result<Option<Box<RawValue>>, Error> {
        let photos = super::photos::mutation(&query.op);
        if !photos && (query.op != "location.current" || !principal.is_agent()) {
            return Ok(None);
        }
        let args = if photos {
            let canonical = self.codec.json(query.args.get().as_bytes())?;
            raw(&json!({"request_digest":self.codec.fingerprint(&canonical)}))?
        } else {
            raw(query)?
        };
        let command = Command {
            id: query.id.clone(),
            op: query.op.clone(),
            args,
            origin: String::new(),
            expected_version: 0,
            expires_at: "0001-01-01T00:00:00Z".into(),
            admission_era: String::new(),
            wait_ms: 0,
        };
        let hash = self.codec.command_hash(&command)?;
        let lock = self.lock(format!("command:{}", query.id))?;
        let _guard = lock.lock().await;
        match self.state.command(&query.id) {
            Ok(record) => {
                if record.principal != principal.id
                    || record.command.op != query.op
                    || record.request_hash != hash
                    || record.command.args.get() != command.args.get()
                {
                    return Err(Code::IdReused.into());
                }
                let record = self.expire(record)?;
                if record.admission_era != self.state.admission_era()?
                    || timestamp(&record.expires_at)? <= self.clock.now()
                {
                    return Err(Code::Expired.into());
                }
                match record.state {
                    State::AwaitingApproval => {
                        if photos {
                            self.photos_previews.remember(&query.id, &query.args)?;
                        }
                        return Ok(Some(raw(
                            &json!({"id":query.id,"state":"awaiting_approval"}),
                        )?));
                    }
                    State::Admitted => {
                        let record = self.transition(record, State::Prepared, None, None, None)?;
                        self.transition(record, State::Dispatched, None, None, None)?;
                        return Ok(None);
                    }
                    _ => return Err(Code::Forbidden.into()),
                }
            }
            Err(error) if error.code == Code::NotFound => {}
            Err(error) => return Err(error),
        }
        let now = self.clock.now();
        if photos && principal.is_agent() {
            self.photos_previews.remember(&query.id, &query.args)?;
        }
        let record = self.state.transition(
            None,
            CommandRecord {
                command,
                principal: principal.id.clone(),
                source: if photos { "photos" } else { "location" }.into(),
                request_hash: hash,
                state: State::AwaitingApproval,
                admission_era: self.state.admission_era()?,
                created_at: now.to_string(),
                expires_at: now
                    .checked_add(jiff::SignedDuration::from_mins(10))
                    .map_err(|_| Code::Invalid)?
                    .to_string(),
                disposition: Some(Disposition::AwaitingApproval),
                evidence: None,
                error: None,
                parts: Vec::new(),
                upload_references: std::collections::BTreeMap::new(),
                commit_seq: 0,
                observed_record_ids: Vec::new(),
                cursor: Cursor::default(),
                history: Vec::new(),
            },
        )?;
        if photos && !principal.is_agent() {
            let record = self.transition(record, State::Admitted, None, None, None)?;
            let record = self.transition(record, State::Prepared, None, None, None)?;
            self.transition(record, State::Dispatched, None, None, None)?;
            return Ok(None);
        }
        Ok(Some(raw(
            &json!({"id":query.id,"state":"awaiting_approval"}),
        )?))
    }

    pub(super) fn finish_location(
        &self,
        principal: &Principal,
        id: &str,
        operation: &str,
        succeeded: bool,
    ) -> Result<(), Error> {
        let photos = super::photos::mutation(operation);
        if photos || operation == "location.current" && principal.is_agent() {
            let record = self.state.command(id)?;
            let evidence = raw(
                &json!({"approval_consumed":true,"query_succeeded":succeeded,"request_hash":record.request_hash}),
            )?;
            self.transition(
                record,
                if succeeded {
                    State::Observed
                } else if photos {
                    State::Indeterminate
                } else {
                    State::Failed
                },
                None,
                Some(evidence),
                None,
            )?;
        }
        Ok(())
    }
}
