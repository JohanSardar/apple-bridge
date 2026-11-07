use super::{
    Arc, Code, Disposition, Duration, Error, LocalGateway, Principal, RawValue, State, raw,
    timestamp,
};

impl LocalGateway {
    pub async fn decide(
        self: &Arc<Self>,
        owner: &Principal,
        id: &str,
        approve: bool,
        review: &crate::approval_review::OwnerReview<'_>,
    ) -> Result<Box<RawValue>, Error> {
        if owner.kind != "device" {
            return Err(Code::Unauthorized.into());
        }
        self.identity.current(owner)?.authorize("admin")?;
        self.decide_inner(id, approve, crate::approval_review::Decider::Owner(review))
            .await
    }
    pub async fn decide_phone(
        self: &Arc<Self>,
        id: &str,
        hash: &str,
        approve: bool,
    ) -> Result<Box<RawValue>, Error> {
        if hash.is_empty() {
            return Err(Code::Invalid.into());
        }
        self.decide_inner(id, approve, crate::approval_review::Decider::Phone(hash))
            .await
    }
    pub(super) async fn decide_inner(
        self: &Arc<Self>,
        id: &str,
        approve: bool,
        decider: crate::approval_review::Decider<'_>,
    ) -> Result<Box<RawValue>, Error> {
        let lock = self.lock(format!("command:{id}"))?;
        let _guard = lock.lock().await;
        let row = self.state.command(id)?;
        self.verify_photos_preview(&row, approve, &decider)?;
        self.verify_decider(&row, approve, decider).await?;
        let mut record = self.expire(row)?;
        if record.state != State::AwaitingApproval {
            return raw(&record);
        }
        if !approve {
            self.forget_photos_preview(&record)?;
            return raw(&self.transition(
                record,
                State::RejectedByOwner,
                Some(Disposition::RejectedByOwner),
                None,
                None,
            )?);
        }
        if record.admission_era != self.state.admission_era()? {
            return raw(&self.transition(
                record,
                State::Expired,
                Some(Disposition::Expired),
                None,
                None,
            )?);
        }
        if record.command.op == "location.current" || super::photos::mutation(&record.command.op) {
            self.forget_photos_preview(&record)?;
            return raw(&self.transition(record, State::Admitted, None, None, None)?);
        }
        let principal = if record.principal.contains("/agent:") {
            let reauthorize = self.reauthorize.as_ref().ok_or(Code::Unavailable)?;
            reauthorize
                .reauthorize(&record.principal, &record.command.id, &record.request_hash)
                .await
                .map(|principal| Principal {
                    id: principal.id,
                    kind: principal.kind,
                    level: principal.level,
                    grants: principal
                        .grants
                        .iter()
                        .map(|g| g.as_str().to_owned())
                        .collect(),
                    resource: String::new(),
                    remote: principal.remote,
                    expires_at: None,
                })
                .map_err(|error| {
                    Error::from(if matches!(error, ab_trust::Error::Unauthorized) {
                        Code::Unauthorized
                    } else {
                        Code::Unavailable
                    })
                })
        } else {
            self.identity.approval_principal(
                &record.principal,
                &record.command.id,
                &record.request_hash,
            )
        };
        let principal =
            match principal.and_then(|principal| self.authorize(&principal, &record.command.op)) {
                Ok(principal) => principal,
                Err(error) if error.code == Code::Unavailable => return Err(error),
                Err(error) => {
                    return raw(&self.transition(
                        record,
                        State::Rejected,
                        Some(Disposition::RejectedBeforeDispatch),
                        None,
                        Some(error),
                    )?);
                }
            };
        if !(self.peer.available() || record.source == "iphone" && self.phone_notify.is_some()) {
            return Err(Code::Unavailable.into());
        }
        let ttl = approval_ttl(&record)?;
        let deadline =
            tokio::time::Instant::now() + Duration::try_from(ttl).map_err(|_| Code::Expired)?;
        record.expires_at = self
            .clock
            .now()
            .checked_add(ttl)
            .map_err(|_| Code::Invalid)?
            .to_string();
        record = self.transition(record, State::Admitted, None, None, None)?;
        let result = raw(&record)?;
        let gateway = self.clone();
        tokio::spawn(async move {
            let _ = gateway.execute(record, principal, deadline).await;
        });
        Ok(result)
    }

    fn verify_photos_preview(
        &self,
        record: &super::CommandRecord,
        approve: bool,
        decider: &crate::approval_review::Decider<'_>,
    ) -> Result<(), Error> {
        if approve
            && record.state == State::AwaitingApproval
            && super::photos::mutation(&record.command.op)
        {
            self.photos_preview(record.clone())?;
            if matches!(decider, crate::approval_review::Decider::Phone(_)) {
                return Err(Code::Unsupported.into());
            }
        }
        Ok(())
    }

    fn forget_photos_preview(&self, record: &super::CommandRecord) -> Result<(), Error> {
        if super::photos::mutation(&record.command.op) {
            self.photos_previews.remove(&record.command.id)?;
        }
        Ok(())
    }
}

fn approval_ttl(record: &super::CommandRecord) -> Result<jiff::SignedDuration, Error> {
    if record.command.expires_at == "0001-01-01T00:00:00Z" {
        Ok(jiff::SignedDuration::from_secs(
            if record.source == "messages" { 60 } else { 300 },
        ))
    } else {
        Ok(timestamp(&record.command.expires_at)?.duration_since(timestamp(&record.created_at)?))
    }
}
