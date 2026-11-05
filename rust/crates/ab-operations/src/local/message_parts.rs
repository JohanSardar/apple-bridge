use super::*;

impl LocalGateway {
    #[allow(clippy::too_many_lines)]
    pub(super) fn message_parts(
        &self,
        record: &mut CommandRecord,
        frame: &mut IpcFrame,
    ) -> Result<bool, Error> {
        if record.command.op != "messages.send" {
            return Ok(false);
        }
        let Some(raw_proof) = &frame.evidence else {
            return Ok(false);
        };
        let mut proof: Value = serde_json::from_str(raw_proof.get()).map_err(|_| Code::Corrupt)?;
        if proof["fingerprint"] != record.request_hash
            || proof["parts"].as_array().is_none_or(Vec::is_empty) && record.parts.is_empty()
        {
            return Ok(false);
        }
        let args: Value =
            serde_json::from_str(record.command.args.get()).map_err(|_| Code::Corrupt)?;
        let mut parts = Vec::new();
        if args["text"].as_str().is_some_and(|s| !s.is_empty()) {
            parts.push(CommandPart {
                index: 0,
                kind: "text".into(),
                upload_id: String::new(),
                record_id: String::new(),
                disposition: "indeterminate".into(),
            });
        }
        for file in args["files"].as_array().into_iter().flatten() {
            parts.push(CommandPart {
                index: parts.len(),
                kind: "attachment".into(),
                upload_id: file["upload_id"].as_str().unwrap_or_default().into(),
                record_id: String::new(),
                disposition: "indeterminate".into(),
            });
        }
        if !record.parts.is_empty() {
            if record.parts.len() != parts.len() {
                return Ok(false);
            }
            parts.clone_from(&record.parts);
        }
        let previous: Value = record
            .evidence
            .as_ref()
            .and_then(|v| serde_json::from_str(v.get()).ok())
            .unwrap_or(Value::Null);
        let mut proofs = BTreeMap::new();
        for principal in previous["parts"].as_array().into_iter().flatten() {
            if let Some(index) = principal["index"].as_u64() {
                proofs.insert(index, principal.clone());
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut problems = serde_json::Map::new();
        let mut commit = record.commit_seq;
        for principal in proof["parts"].as_array().into_iter().flatten() {
            let Some(index) = principal["index"]
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
            else {
                return Ok(false);
            };
            if index >= parts.len() || !seen.insert(index) || principal["kind"] != parts[index].kind
            {
                return Ok(false);
            }
            let prior = parts[index].clone();
            let mut next = prior.clone();
            next.record_id.clear();
            next.disposition = match principal["disposition"].as_str().unwrap_or_default() {
                "delivery_failed" => "delivery_failed",
                "failed_no_effect" | "not_dispatched" | "rejected_before_dispatch" => {
                    "failed_no_effect"
                }
                _ => "indeterminate",
            }
            .into();
            let mut bound = false;
            let mut candidate = principal.clone();
            candidate["fingerprint"] = proof["fingerprint"].clone();
            candidate["capture_checkpoint"] = proof["capture_checkpoint"].clone();
            if proof["ambiguous"] != true
                && proof["candidate_count"].as_u64().unwrap_or(0) <= 1
                && matches!(
                    principal["disposition"].as_str(),
                    Some("locally_observed" | "delivered" | "delivery_failed")
                )
            {
                let part_frame = IpcFrame {
                    evidence: Some(raw(&candidate)?),
                    ..IpcFrame::default()
                };
                if let Some((cursor, key)) = self.bound(record, &part_frame, "messages.message") {
                    bound = true;
                    next.disposition = if principal["disposition"] == "locally_observed" {
                        "observed".into()
                    } else {
                        principal["disposition"]
                            .as_str()
                            .ok_or(Code::Corrupt)?
                            .into()
                    };
                    match self
                        .state
                        .observed_record_id("messages.message", &key, &cursor)
                    {
                        Ok(id) => next.record_id = id,
                        Err(error) if error.code == Code::NotFound => {}
                        Err(error) => {
                            problems.insert(
                                index.to_string(),
                                serde_json::to_value(error.code).map_err(|_| Code::Corrupt)?,
                            );
                        }
                    }
                    commit = commit.max(cursor.sequence);
                }
            }
            let prior_proof = proofs.get(&(index as u64));
            let same_key = prior_proof.is_some_and(|old| {
                old["native_key"].as_str().is_some_and(|v| !v.is_empty())
                    && old["native_key"] == principal["native_key"]
            });
            let bound_failure = prior.disposition == "observed"
                && next.disposition == "delivery_failed"
                && bound
                && same_key
                && (prior.record_id.is_empty() || prior.record_id == next.record_id);
            if prior.disposition != "indeterminate"
                && prior.disposition != next.disposition
                && !(prior.disposition == "observed" && next.disposition == "delivered")
                && !bound_failure
            {
                continue;
            }
            if prior.disposition != "indeterminate"
                && prior_proof.is_some_and(|old| {
                    old["native_key"].as_str().is_some_and(|v| !v.is_empty())
                        && principal["native_key"]
                            .as_str()
                            .is_some_and(|v| !v.is_empty())
                        && old["native_key"] != principal["native_key"]
                })
            {
                continue;
            }
            if !prior.record_id.is_empty() {
                if !next.record_id.is_empty() && next.record_id != prior.record_id {
                    continue;
                }
                next.record_id = prior.record_id;
            }
            if next == parts[index] && prior_proof.is_some() {
                continue;
            }
            parts[index] = next;
            proofs.insert(index as u64, principal.clone());
        }
        proof["parts"] = Value::Array(proofs.into_values().collect());
        if !problems.is_empty() {
            proof["part_record_id_resolution_error"] = Value::Object(problems);
        }
        frame.evidence = Some(raw(&proof)?);
        record.parts = parts;
        record.commit_seq = commit;
        Ok(true)
    }
}
pub(super) fn resolvable(record: &CommandRecord) -> bool {
    record.state == State::Indeterminate
        || record.source == "messages"
            && (record.state == State::Observed
                || record.state == State::DeliveryFailed
                    && record.parts.iter().any(|principal| {
                        principal.disposition == "indeterminate"
                            || principal.disposition == "observed"
                            || principal.disposition == "delivery_failed"
                                && principal.record_id.is_empty()
                    }))
}
