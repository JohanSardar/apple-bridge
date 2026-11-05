use super::*;

impl LocalGateway {
    pub(super) async fn apply_outcome(
        &self,
        principal: &Principal,
        record: CommandRecord,
        mut frame: IpcFrame,
        collection: &str,
    ) -> Result<CommandRecord, Error> {
        if frame.t == "Rejected" {
            frame.error = self
                .actions
                .conflict(principal, &record.command, &frame)?
                .map(|error| raw(&error))
                .transpose()?;
        }
        let mut policy = self
            .actions
            .evidence(&record.command, &record.request_hash, &frame)
            .unwrap_or(OutcomeEvidence::Unproven);
        if let OutcomeEvidence::Artifact(blob) = &policy {
            let mut evidence: Value = frame
                .evidence
                .as_ref()
                .and_then(|v| serde_json::from_str(v.get()).ok())
                .unwrap_or(Value::Null);
            let id = format!("qry_{}", ulid::Ulid::new());
            let mut blobs =
                vec![json!({"digest":blob.digest,"size":blob.size,"handle":blob.handle})];
            let descriptor = descriptor_for(&record.command.op)?;
            let result = async {
                self.query_store
                    .as_ref()
                    .ok_or(Code::Unavailable)?
                    .hydrate(
                        self.peer.as_ref(),
                        &crate::QueryBlobContext {
                            principal: principal,
                            id: &id,
                            scope: &descriptor.scope,
                            max_size: 128 << 20,
                            session: crate::peer::PeerSession(frame.owner_fence),
                        },
                        &mut blobs,
                    )
                    .await
            };
            if matches!(
                tokio::time::timeout(Duration::from_secs(75), result).await,
                Ok(Ok(()))
            ) {
                let object = evidence.as_object_mut().ok_or(Code::Corrupt)?;
                object.remove("handle");
                object.insert("blob_id".into(), Value::String(id));
                frame.evidence = Some(raw(&evidence)?);
                policy = OutcomeEvidence::Standalone;
            } else {
                policy = OutcomeEvidence::Unproven;
            }
        }
        self.outcome(record, frame, collection, &policy)
    }
    pub(super) fn bound(
        &self,
        record: &CommandRecord,
        frame: &IpcFrame,
        collection: &str,
    ) -> Option<(Cursor, String)> {
        let evidence: Value = serde_json::from_str(frame.evidence.as_ref()?.get()).ok()?;
        if evidence["fingerprint"] != record.request_hash
            || evidence["observed"] != true
            || evidence["ambiguous"] == true
            || evidence["candidate_count"].as_u64().unwrap_or(0) > 1
        {
            return None;
        }
        let key = evidence["native_key"].as_str()?.to_string();
        if key.is_empty() {
            return None;
        }
        let cursor = self
            .peer
            .captured(
                &record.source,
                evidence["capture_checkpoint"].as_str()?,
                collection,
                &key,
            )
            .ok()?;
        if cursor.sequence == 0 || cursor.epoch != self.state.head().ok()?.epoch {
            return None;
        }
        Some((cursor, key))
    }
    pub(super) async fn await_capture(
        &self,
        record: &CommandRecord,
        mut frame: IpcFrame,
    ) -> IpcFrame {
        if record.command.op != "messages.send" {
            return frame;
        }
        let mut changes = self.peer.capture_revision();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while matches!(frame.disposition.as_str(), "locally_observed" | "delivered")
            && self.bound(record, &frame, "messages.message").is_none()
        {
            let evidence: Value = frame
                .evidence
                .as_ref()
                .and_then(|v| serde_json::from_str(v.get()).ok())
                .unwrap_or(Value::Null);
            if evidence["fingerprint"] != record.request_hash
                || evidence["observed"] != true
                || evidence["ambiguous"] == true
                || evidence["candidate_count"].as_u64().unwrap_or(0) > 1
            {
                break;
            }
            if !matches!(
                tokio::time::timeout_at(deadline, changes.changed()).await,
                Ok(Ok(()))
            ) {
                break;
            }
            match tokio::time::timeout_at(
                deadline,
                self.peer.exchange(IpcFrame {
                    t: "Resolve".into(),
                    command_id: record.command.id.clone(),
                    ..IpcFrame::default()
                }),
            )
            .await
            {
                Ok(Ok(next)) => frame = next,
                _ => break,
            }
        }
        frame
    }
    #[allow(clippy::too_many_lines)]
    pub(super) fn outcome(
        &self,
        mut record: CommandRecord,
        mut frame: IpcFrame,
        collection: &str,
        policy: &OutcomeEvidence,
    ) -> Result<CommandRecord, Error> {
        let prior = record.clone();
        let (mut state, mut disposition) = match frame.disposition.as_str() {
            "locally_observed" => (State::Observed, Disposition::LocallyObserved),
            "absent_confirmed" => (State::Observed, Disposition::AbsentConfirmed),
            "delivered" => (State::Delivered, Disposition::Delivered),
            "delivery_failed" => (State::DeliveryFailed, Disposition::DeliveryFailed),
            "failed_no_effect" => (State::Failed, Disposition::FailedNoEffect),
            "rejected_before_dispatch" => (State::Rejected, Disposition::RejectedBeforeDispatch),
            _ => (State::Indeterminate, Disposition::Indeterminate),
        };
        if frame.t == "Rejected" {
            state = State::Rejected;
            disposition = Disposition::RejectedBeforeDispatch;
        }
        if matches!(policy, OutcomeEvidence::Unproven)
            && matches!(
                state,
                State::Observed | State::Delivered | State::DeliveryFailed
            )
        {
            state = State::Indeterminate;
            disposition = Disposition::Indeterminate;
        }
        if matches!(policy, OutcomeEvidence::CaptureBound)
            && matches!(
                state,
                State::Observed | State::Delivered | State::DeliveryFailed
            )
        {
            if let Some((cursor, key)) = self.bound(&record, &frame, collection) {
                record.commit_seq = cursor.sequence;
                match self.state.observed_record_id(collection, &key, &cursor) {
                    Ok(id) => record.observed_record_ids = vec![id],
                    Err(error) if error.code == Code::NotFound => {}
                    Err(error) => {
                        let mut evidence: Value = serde_json::from_str(
                            frame.evidence.as_ref().ok_or(Code::Corrupt)?.get(),
                        )
                        .map_err(|_| Code::Corrupt)?;
                        evidence["record_id_resolution_error"] =
                            serde_json::to_value(error.code).map_err(|_| Code::Corrupt)?;
                        frame.evidence = Some(raw(&evidence)?);
                    }
                }
            } else if state != State::DeliveryFailed {
                state = State::Indeterminate;
                disposition = Disposition::Indeterminate;
            }
        }
        if self.message_parts(&mut record, &mut frame)? {
            let failed = record.parts.iter().any(|principal| {
                matches!(
                    principal.disposition.as_str(),
                    "delivery_failed" | "failed_no_effect"
                )
            });
            let delivered = record
                .parts
                .iter()
                .all(|principal| principal.disposition == "delivered");
            let observed = record.parts.iter().all(|principal| {
                matches!(principal.disposition.as_str(), "observed" | "delivered")
            });
            (state, disposition) = if failed {
                (State::DeliveryFailed, Disposition::DeliveryFailed)
            } else if delivered {
                (State::Delivered, Disposition::Delivered)
            } else if observed {
                (State::Observed, Disposition::LocallyObserved)
            } else {
                (State::Indeterminate, Disposition::Indeterminate)
            };
            record.commit_seq = record.commit_seq.max(prior.commit_seq);
            if record.state == state {
                if prior.parts == record.parts {
                    return Ok(prior);
                }
                record.disposition = Some(disposition);
                record.evidence = frame.evidence;
                return self.state.update_parts(record);
            }
        }
        if record.state == State::Observed
            && matches!(state, State::Observed | State::Indeterminate)
        {
            return Ok(record);
        }
        if record.state == state && matches!(state, State::Observed | State::Indeterminate) {
            return Ok(record);
        }
        if record.state == State::Dispatched && state == State::Delivered {
            record = self.transition(
                record,
                State::Observed,
                Some(Disposition::LocallyObserved),
                frame.evidence.clone(),
                None,
            )?;
        }
        let error = frame
            .error
            .map(|v| serde_json::from_str(v.get()).map_err(|_| Code::Corrupt))
            .transpose()?;
        self.transition(record, state, Some(disposition), frame.evidence, error)
    }
}
