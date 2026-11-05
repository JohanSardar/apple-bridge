use super::*;

impl LocalGateway {
    #[allow(clippy::too_many_lines)]
    pub(super) async fn execute(
        &self,
        mut record: CommandRecord,
        principal: Principal,
        deadline: tokio::time::Instant,
    ) -> Result<CommandRecord, Error> {
        if record.source == "iphone"
            && let Some(notify) = &self.phone_notify
        {
            notify.notify_one();
            return Ok(record);
        }
        if tokio::time::Instant::now() >= deadline {
            return self.transition(
                record,
                State::Expired,
                Some(Disposition::Expired),
                None,
                None,
            );
        }
        let key = match self.actions.serialization_key(&principal, &record.command) {
            Ok(plan) => plan,
            Err(error) => {
                return self.transition(
                    record,
                    State::Rejected,
                    Some(Disposition::RejectedBeforeDispatch),
                    None,
                    Some(error),
                );
            }
        };
        let key = self
            .state
            .serialization_key(&principal, &record.command, key)?;
        let lock = self.lock(format!("object:{key}"))?;
        let Ok(_guard) = tokio::time::timeout_at(deadline, lock.lock()).await else {
            return self.transition(
                record,
                State::Expired,
                Some(Disposition::Expired),
                None,
                None,
            );
        };
        let document = if matches!(
            record.command.op.as_str(),
            "pdf.annotate" | "print.document"
        ) {
            let store = self.query_store.as_ref().ok_or(Code::Unavailable)?;
            let input = if record.command.op == "print.document" {
                store.document_file(&principal, &record.command.args).await
            } else {
                store.document_input(&principal, &record.command.args).await
            };
            match input {
                Ok(document) => Some(document),
                Err(error) => {
                    return self.transition(
                        record,
                        State::Rejected,
                        Some(Disposition::RejectedBeforeDispatch),
                        None,
                        Some(error),
                    );
                }
            }
        } else {
            None
        };
        let mut execution = record.command.clone();
        if let Some(document) = &document {
            execution.args = document.args.clone();
        }
        let (mut plan, _lease) = match self.actions.prepare(&principal, &execution) {
            Ok(plan) => plan,
            Err(error) => {
                return self.transition(
                    record,
                    State::Rejected,
                    Some(Disposition::RejectedBeforeDispatch),
                    None,
                    Some(error),
                );
            }
        };
        let _files = if let Some(store) = &self.query_store {
            match store.message_files(&record, &mut plan).await {
                Ok(files) => files,
                Err(error) => {
                    return self.transition(
                        record,
                        State::Rejected,
                        Some(Disposition::RejectedBeforeDispatch),
                        None,
                        Some(error),
                    );
                }
            }
        } else {
            None
        };
        plan.action = match crate::command_contract::prepared(&record.command, &plan.action) {
            Ok(action) => action,
            Err(error) => {
                return self.transition(
                    record,
                    State::Rejected,
                    Some(Disposition::RejectedBeforeDispatch),
                    None,
                    Some(error),
                );
            }
        };
        let prepared = tokio::time::timeout_at(
            deadline,
            self.peer.exchange(IpcFrame {
                t: "Prepare".into(),
                origin: record.command.origin.clone(),
                command_id: record.command.id.clone(),
                principal: principal.id.clone(),
                request_hash: record.request_hash.clone(),
                action: Some(plan.action),
                expected_state: plan.expected_state,
                expires_at: GoTime::parse(&record.expires_at).map_err(|_| Code::Invalid)?,
                ..IpcFrame::default()
            }),
        )
        .await;
        let Ok(Ok(prepared)) = prepared else {
            if self.stopping() {
                return Ok(record);
            }
            if tokio::time::Instant::now() >= deadline {
                return self.transition(
                    record,
                    State::Expired,
                    Some(Disposition::Expired),
                    None,
                    None,
                );
            }
            return self.transition(
                record,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                None,
                Some(Code::Unavailable.into()),
            );
        };
        if self.stopping() {
            return Ok(record);
        }
        if prepared.t == "Rejected" {
            let error = self
                .actions
                .conflict(&principal, &record.command, &prepared)?;
            return self.transition(
                record,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                prepared.evidence,
                error,
            );
        }
        if prepared.t != "Prepared" || prepared.ticket.is_empty() {
            return self.transition(
                record,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                None,
                Some(Code::Corrupt.into()),
            );
        }
        record = self.transition(record, State::Prepared, None, Some(raw(&prepared)?), None)?;
        if tokio::time::Instant::now() >= deadline {
            return self.transition(
                record,
                State::Expired,
                Some(Disposition::Expired),
                None,
                None,
            );
        }
        if let Err(error) = self.authorize(&principal, &record.command.op) {
            return self.transition(
                record,
                State::Rejected,
                Some(Disposition::RejectedBeforeDispatch),
                None,
                Some(error),
            );
        }
        record = self.transition(record, State::Dispatched, None, None, None)?;
        let result = tokio::time::timeout(
            outcome_timeout(&record.command)?,
            self.peer.exchange(IpcFrame {
                t: "Dispatch".into(),
                origin: record.command.origin.clone(),
                command_id: record.command.id.clone(),
                ticket: prepared.ticket,
                ..IpcFrame::default()
            }),
        )
        .await;
        let Ok(Ok(frame)) = result else {
            return self.transition(
                record,
                State::Indeterminate,
                Some(Disposition::Indeterminate),
                None,
                None,
            );
        };
        let frame = self.await_capture(&record, frame).await;
        self.apply_outcome(&principal, record, frame, &plan.collection)
            .await
    }
}
