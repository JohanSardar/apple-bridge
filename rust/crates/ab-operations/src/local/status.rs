use super::{
    Code, CommandQuery, CommandRecord, Disposition, Error, IpcFrame, LocalGateway, Principal,
    RawValue, State, raw, resolvable, timestamp,
};

impl LocalGateway {
    pub async fn status(
        &self,
        principal: &Principal,
        query: &CommandQuery,
    ) -> Result<Box<RawValue>, Error> {
        let record = self.state.command(&query.id)?;
        if record.principal != principal.id
            || !principal
                .visibility()
                .get(&record.source)
                .copied()
                .unwrap_or(false)
                && !principal.visibility().contains_key("admin")
        {
            return Err(Code::NotFound.into());
        }
        let mut record = self.expire(record)?;
        if record.source == "iphone" {
            return raw(&record);
        }
        if query.resolve
            && record.source != "photos"
            && record.admission_era == self.state.admission_era()?
            && resolvable(&record)
        {
            let lock = self.lock(format!("command:{}", query.id))?;
            let _guard = lock.lock().await;
            record = self.state.command(&query.id)?;
            if resolvable(&record) {
                let collection = self.actions.collection(&record.command)?;
                let frame = self
                    .peer
                    .exchange(IpcFrame {
                        t: "Resolve".into(),
                        command_id: query.id.clone(),
                        ..IpcFrame::default()
                    })
                    .await?;
                record = self
                    .apply_outcome(principal, record, frame, &collection)
                    .await?;
            }
        }
        raw(&record)
    }
    pub fn list(&self, principal: &Principal, limit: u64) -> Result<Box<RawValue>, Error> {
        let mut rows = Vec::new();
        for record in self.state.commands()? {
            if record.principal == principal.id
                && (principal.visibility().contains_key("admin")
                    || principal.visibility().get(&record.source) == Some(&true))
            {
                rows.push(self.expire(record)?);
            }
        }
        if limit > 0 && rows.len() as u64 > limit {
            rows.drain(..rows.len() - usize::try_from(limit).map_err(|_| Code::Invalid)?);
        }
        raw(&rows)
    }
    pub fn approvals(&self) -> Result<Vec<CommandRecord>, Error> {
        self.state
            .commands()?
            .into_iter()
            .filter(|record| record.state == State::AwaitingApproval)
            .map(|record| self.expire(record))
            .filter(|record| {
                record
                    .as_ref()
                    .map_or(true, |record| record.state == State::AwaitingApproval)
            })
            .map(|record| {
                record.and_then(|record| {
                    if super::photos::mutation(&record.command.op) {
                        match self.photos_preview(record.clone()) {
                            Ok(preview) => Ok(preview),
                            Err(error) if error.code == Code::Unavailable => Ok(record),
                            Err(error) => Err(error),
                        }
                    } else {
                        Ok(record)
                    }
                })
            })
            .collect()
    }
    pub fn recover(&self) -> Result<(), Error> {
        for record in self.state.commands()? {
            if record.source == "iphone" {
                continue;
            }
            match record.state {
                State::AwaitingApproval => {
                    self.expire(record)?;
                }
                State::Dispatched => {
                    self.transition(
                        record,
                        State::Indeterminate,
                        Some(Disposition::Indeterminate),
                        None,
                        None,
                    )?;
                }
                State::Admitted | State::Prepared => {
                    if timestamp(&record.expires_at)? <= self.clock.now() {
                        self.transition(
                            record,
                            State::Expired,
                            Some(Disposition::Expired),
                            None,
                            None,
                        )?;
                    } else {
                        self.transition(
                            record,
                            State::Rejected,
                            Some(Disposition::RejectedBeforeDispatch),
                            None,
                            Some(Code::Unavailable.into()),
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
