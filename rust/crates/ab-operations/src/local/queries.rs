use super::*;

impl LocalGateway {
    pub async fn query(
        &self,
        principal: &Principal,
        mut query: NativeQuery,
    ) -> Result<Box<RawValue>, Error> {
        let descriptor = descriptor_for(&query.op)?;
        if descriptor.kind != ab_catalog::OperationKind::Query {
            return Err(Code::Unsupported.into());
        }
        self.authorize(principal, &query.op).map_err(|error| {
            if error.code == Code::Unauthorized {
                Code::Forbidden.into()
            } else {
                error
            }
        })?;
        {
            let mut rates = self.query_rates.lock().map_err(|_| Code::Unavailable)?;
            rates.retain(|_, (at, _)| at.elapsed() <= Duration::from_secs(1));
            let (_, count) = rates
                .entry(principal.id.clone())
                .or_insert((std::time::Instant::now(), 0));
            *count += 1;
            if *count > 100 {
                return Err(Code::Capacity.into());
            }
        }
        let args: Value = serde_json::from_str(query.args.get()).map_err(|_| Code::Invalid)?;
        if matches!(
            query.op.as_str(),
            "helper.version"
                | "mac.clipboard_read"
                | "mac.network_check"
                | "mac.bluetooth_devices"
                | "mac.time_machine_status"
        ) && !args.as_object().is_some_and(serde_json::Map::is_empty)
        {
            return Err(Code::Invalid.into());
        }
        if query.op == "mac.recognize_song" {
            use base64::Engine;
            if !args
                .as_object()
                .is_some_and(|a| a.keys().all(|k| k == "audio_blob"))
            {
                return Err(Code::Invalid.into());
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(
                    args["audio_blob"]
                        .as_str()
                        .unwrap_or_default()
                        .replace(['\r', '\n'], ""),
                )
                .map_err(|_| Code::Invalid)?;
            if data.is_empty() || data.len() > 8 << 20 {
                return Err(Code::Invalid.into());
            }
        }
        if query.id.is_empty() {
            query.id = format!("qry_{}", ulid::Ulid::new());
        }
        if !valid_command_id(&query.id)
            || !query.id.starts_with("qry_")
            || query.id.len() <= 4
            || query.deadline_ms > 60000
        {
            return Err(Code::Invalid.into());
        }
        if query.deadline_ms == 0 {
            query.deadline_ms = 1000;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(query.deadline_ms);
        let mut audit = crate::query_audit::AuditGuard::new(
            self.query_audit.clone(),
            principal,
            &query.id,
            &query.op,
            query.args.get().len(),
            deadline,
        );
        let result = self
            .route_query(
                principal,
                query,
                &descriptor.scope,
                deadline,
                &mut audit.stats,
            )
            .await;
        let recorded = audit.finish();
        match (result, recorded) {
            (Ok(_), Err(error)) => Err(error),
            (result, _) => result,
        }
    }
    pub(super) async fn route_query(
        &self,
        principal: &Principal,
        mut query: NativeQuery,
        scope: &str,
        deadline: tokio::time::Instant,
        audit: &mut crate::QueryAudit,
    ) -> Result<Box<RawValue>, Error> {
        let args: Value = serde_json::from_str(query.args.get()).map_err(|_| Code::Invalid)?;
        let document = if matches!(
            query.op.as_str(),
            "vision.ocr" | "pdf.text" | "pdf.search" | "audio.transcribe"
        ) {
            audit.argument_bytes = 0;
            let store = self.query_store.as_ref().ok_or(Code::Unavailable)?;
            Some(
                tokio::time::timeout_at(deadline, store.document_input(principal, &query.args))
                    .await
                    .map_err(|_| Code::Deadline)??,
            )
        } else {
            None
        };
        if let Some(document) = &document {
            query.args = document.args.clone();
            audit.argument_bytes = query.args.get().len();
        }
        if query.op == "mail.body" {
            query.args = self
                .query_store
                .as_ref()
                .ok_or(Code::Unavailable)?
                .mail_args(principal, &args)?;
            audit.argument_bytes = query.args.get().len();
        }
        let frame = tokio::time::timeout_at(
            deadline,
            self.peer.exchange(IpcFrame {
                t: "Query".into(),
                id: query.id.clone(),
                op: query.op,
                args: Some(query.args),
                deadline_ms: i64::try_from(query.deadline_ms).map_err(|_| Code::Invalid)?,
                ..IpcFrame::default()
            }),
        )
        .await
        .map_err(|_| Code::Deadline)??;
        if frame.t == "QueryError" {
            return Err(serde_json::from_value(
                json!({"code": frame.code, "message": "query failed", "retryable": false}),
            )
            .map_err(|_| Code::Corrupt)?);
        }
        if frame.t != "QueryResult"
            || frame.result_json.is_none()
            || frame
                .result_json
                .as_ref()
                .is_some_and(|v| v.get().len() > 1 << 20)
            || frame.blobs.as_ref().is_some_and(|v| v.len() > 16)
        {
            return Err(Code::Corrupt.into());
        }
        audit.result_bytes = frame.result_json.as_ref().map_or(0, |v| v.get().len());
        audit.blob_bytes = frame
            .blobs
            .as_ref()
            .into_iter()
            .flatten()
            .filter_map(|b| serde_json::from_str::<Value>(b.get()).ok())
            .filter_map(|b| b["size"].as_i64())
            .sum();
        let mut blobs: Vec<Value> = frame
            .blobs
            .unwrap_or_default()
            .into_iter()
            .map(|b| serde_json::from_str(b.get()).map_err(|_| Code::Corrupt))
            .collect::<Result<_, _>>()?;
        if !blobs.is_empty() {
            let store = self.query_store.as_ref().ok_or(Code::Corrupt)?;
            tokio::time::timeout_at(
                deadline,
                store.hydrate(
                    self.peer.as_ref(),
                    &crate::QueryBlobContext {
                        principal: principal,
                        id: &query.id,
                        scope,
                        max_size: 64 << 20,
                        session: crate::peer::PeerSession(frame.owner_fence),
                    },
                    &mut blobs,
                ),
            )
            .await
            .map_err(|_| Code::Deadline)??;
        }
        raw(&json!({"id": query.id, "result_json": frame.result_json, "blobs": blobs}))
    }
}
