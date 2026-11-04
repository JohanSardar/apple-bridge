use ab_iphone::{
    Error,
    delivery::{
        CaptureReceipt, CaptureUpload, ConnectorApproval, Gateway, HealthBatch, LocationRecord,
        QueuedCommand,
    },
};
use ab_operations::{
    Code, GatewayStore,
    local::{CommandRecord, GatewayState, LocalGateway},
};
use ab_trust::{
    Authority,
    domain::{CommandState, Disposition, Source},
};
use ab_wire::{
    gojson::{EscapeMode, GoTime},
    messages::{Capture, Observation},
};
use serde::Serialize;
use serde_json::value::RawValue;
use std::sync::{Arc, Weak};

#[derive(Debug)]
pub struct PhoneGateway {
    pub connectors: Arc<ab_frontdoor::web_connector::Connectors>,
    #[cfg(feature = "lab-harness")]
    pub home: std::path::PathBuf,
    pub state: GatewayStore,
    pub local: Weak<LocalGateway>,
    pub authority: Arc<Authority>,
    pub blobs: Arc<tokio::sync::Mutex<ab_blobs::Store>>,
}
fn error(error: &ab_operations::Error) -> Error {
    match error.code {
        Code::Unauthorized | Code::Forbidden => Error::Unauthorized,
        Code::NotFound => Error::NotFound,
        Code::Conflict | Code::IdReused => Error::Conflict,
        Code::Invalid => Error::Invalid,
        _ => Error::Unavailable,
    }
}
fn queued(row: CommandRecord) -> Result<QueuedCommand, Error> {
    Ok(QueuedCommand {
        command: serde_json::value::to_raw_value(&row.command)?,
        principal: row.principal,
        source: row.source,
        request_hash: row.request_hash,
        state: row.state,
        created_at: row.created_at,
        expires_at: row.expires_at,
    })
}
fn id(row: &QueuedCommand) -> Result<String, Error> {
    let command: ab_operations::Command = serde_json::from_str(row.command.get())?;
    Ok(command.id)
}
impl Gateway for PhoneGateway {
    fn connector_pending(&mut self) -> Result<Vec<ConnectorApproval>, Error> {
        let pending = self.connectors.pending().map_err(|_| Error::Unavailable)?;
        Ok(pending
            .into_iter()
            .map(|summary| ConnectorApproval {
                expires: summary
                    .expires_at
                    .parse::<jiff::Timestamp>()
                    .map_or(0, jiff::Timestamp::as_second),
                id: summary.id,
                code: summary.code,
                name: summary.client_name,
                scopes: summary.access,
            })
            .collect())
    }
    fn connector_decide(&mut self, id: &str, hash: &str, approve: bool) -> Result<(), Error> {
        self.connectors
            .decide(id, hash, approve)
            .map_err(|_| Error::Conflict)
    }
    fn commands(&mut self) -> Result<Vec<QueuedCommand>, Error> {
        self.state
            .commands()
            .map_err(|error_value| error(&error_value))?
            .into_iter()
            .map(queued)
            .collect()
    }
    fn command(&mut self, command_id: &str) -> Result<QueuedCommand, Error> {
        queued(
            self.state
                .command(command_id)
                .map_err(|error_value| error(&error_value))?,
        )
    }
    fn transition(
        &mut self,
        prior: &QueuedCommand,
        state: CommandState,
        disposition: Option<Disposition>,
        evidence: Option<&RawValue>,
    ) -> Result<QueuedCommand, Error> {
        let mut row = self
            .state
            .command(&id(prior)?)
            .map_err(|error_value| error(&error_value))?;
        if row.principal != prior.principal
            || row.source != prior.source
            || row.request_hash != prior.request_hash
            || row.state != prior.state
        {
            return Err(Error::Conflict);
        }
        row.state = state;
        row.disposition = disposition;
        if let Some(evidence) = evidence {
            row.evidence = Some(serde_json::from_str(evidence.get())?);
        }
        queued(
            self.state
                .transition(Some(prior.state), row)
                .map_err(|error_value| error(&error_value))?,
        )
    }
    fn authorize(&mut self, row: &QueuedCommand) -> Result<(), Error> {
        #[cfg(feature = "lab-harness")]
        if row.principal.starts_with("replica:") {
            let mode =
                std::fs::read_to_string(self.home.join("phone-delegated-mode")).unwrap_or_default();
            if mode.strip_prefix("allow ") == Some(row.request_hash.as_str()) {
                std::fs::write(self.home.join("phone-delegated-authorized"), "")
                    .map_err(|_| Error::Unavailable)?;
                return Ok(());
            }
            return Err(Error::Unauthorized);
        }
        if row.principal.contains('/') {
            return Err(Error::Unauthorized);
        }
        let principal = self.authority.principal(&row.principal)?;
        let command: ab_operations::Command = serde_json::from_str(row.command.get())?;
        let scope = ab_catalog::operations()?
            .into_iter()
            .find(|descriptor| descriptor.name == command.op)
            .ok_or(Error::Invalid)?
            .scope;
        self.authority
            .authorize(&principal, &ab_trust::Grant::parse(&scope)?)?;
        Ok(())
    }
    fn decide(&mut self, command_id: &str, hash: &str, approve: bool) -> Result<(), Error> {
        let local = self.local.upgrade().ok_or(Error::Unavailable)?;
        let row = tokio::runtime::Handle::current()
            .block_on(local.decide_phone(command_id, hash, approve))
            .map_err(|error_value| error(&error_value))?;
        #[cfg(feature = "lab-harness")]
        let _ =
            ab_trust::atomic_write(&self.home.join("phone-decision.json"), row.get().as_bytes());
        #[cfg(not(feature = "lab-harness"))]
        let _ = row;
        Ok(())
    }
    fn capture(
        &mut self,
        row: &QueuedCommand,
        upload: &CaptureUpload,
        bytes: &[u8],
    ) -> Result<CaptureReceipt, Error> {
        let receipt = CaptureReceipt {
            blob_id: format!(
                "qry_{}",
                ab_trust::fingerprint(
                    format!("{}\0{}", upload.command_id, upload.request_hash).as_bytes()
                )
            ),
            digest: ab_trust::fingerprint(bytes),
            size: u64::try_from(bytes.len()).map_err(|_| Error::Invalid)?,
            content_type: upload.content_type.clone(),
        };
        if let Ok((_, grants)) = self
            .state
            .with(|state| state.query_blob_grant(&row.principal, &receipt.blob_id))
        {
            if !grants
                .as_array()
                .is_some_and(|grants| grants.iter().any(|grant| grant["digest"] == receipt.digest))
            {
                return Err(Error::Invalid);
            }
            return Ok(receipt);
        }
        let (_, pin) = self
            .blobs
            .blocking_lock()
            .stage(bytes)
            .map_err(|_| Error::Unavailable)?;
        let saved = self.state.with(|state| {
            state.save_query_blobs(
                &row.principal,
                &receipt.blob_id,
                "iphone:write",
                &serde_json::json!([{"digest":receipt.digest,"size":receipt.size}]),
            )
        });
        if saved.is_ok() {
            let protection = Arc::new(ab_replication::gateway::ProtectionAdapter(
                self.blobs.clone(),
            ));
            self.state
                .with(|state| state.set_blob_protection(protection))
                .map_err(|error_value| error(&error_value))?;
        }
        self.blobs
            .blocking_lock()
            .release_pin(&pin)
            .map_err(|_| Error::Unavailable)?;
        saved.map_err(|error_value| error(&error_value))?;
        Ok(receipt)
    }
    fn ingest_health(
        &mut self,
        device: &str,
        batch: &HealthBatch,
        digest: &str,
    ) -> Result<(), Error> {
        batch.validate()?;
        let samples = batch
            .samples
            .iter()
            .flatten()
            .map(|sample| Ok((sample.id.clone(), serde_json::to_value(sample)?)))
            .collect::<Result<Vec<_>, Error>>()?;
        self.state
            .with(|state| {
                state.ingest_health(
                    device,
                    &batch.stream,
                    batch.sequence,
                    digest,
                    &samples,
                    batch.deleted.as_deref().unwrap_or(&[]),
                )
            })
            .map_err(|error_value| error(&error_value))
    }
    fn ingest_locations(&mut self, device: &str, records: &[LocationRecord]) -> Result<(), Error> {
        ab_iphone::delivery::validate_locations(records)?;
        let observations = records
            .iter()
            .map(|record| {
                #[derive(Serialize)]
                struct Data<'a> {
                    #[serde(flatten)]
                    record: &'a LocationRecord,
                    device_id: &'a str,
                }
                let millis = format!(
                    "{:.0}",
                    ((record.timestamp + 978_307_200.0) * 1000.0).trunc()
                )
                .parse::<i64>()
                .map_err(|_| Error::Invalid)?;
                let at = jiff::Timestamp::from_millisecond(millis).map_err(|_| Error::Invalid)?;
                let data = ab_wire::gojson::legacy_json(
                    &Data {
                        record,
                        device_id: device,
                    },
                    EscapeMode::Html,
                )
                .map_err(|_| Error::Invalid)?;
                Ok(Observation {
                    kind: "upsert".into(),
                    collection: "iphone.location".into(),
                    native_key: format!("{device}:{}", record.id),
                    data: Some(serde_json::from_slice(&data)?),
                    observed_at: GoTime::parse(&at.to_string()).map_err(|_| Error::Invalid)?,
                    ..Observation::default()
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let mut random = [0; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut random);
        let checkpoint = format!("cp_{}", ab_trust::fingerprint(&random));
        self.state
            .with(|state| {
                let mut capture = Capture {
                    source: Some(Source::Iphone),
                    generation: "iphone-location/1".into(),
                    checkpoint,
                    observations: Some(observations),
                    ..Capture::default()
                };
                if let Some(prior) = state
                    .checkpoints()?
                    .get("iphone")
                    .filter(|prior| prior.generation == capture.generation)
                {
                    capture.predecessor.clone_from(&prior.checkpoint);
                }
                state.commit_capture(&capture)?;
                Ok(())
            })
            .map_err(|error_value| error(&error_value))
    }
}

pub fn router(phone: &Arc<ab_iphone::delivery::Delivery<PhoneGateway>>) -> axum::Router {
    let authority = phone.authority.clone();
    phone
        .router()
        .merge(
            axum::Router::new()
                .route("/v1/iphone/pair", axum::routing::post(pair))
                .route("/v1/iphone/status", axum::routing::post(status))
                .layer(axum::extract::DefaultBodyLimit::max(8192))
                .with_state(authority),
        )
        .layer(axum::middleware::from_fn(secure))
}
async fn secure(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    #[cfg(feature = "lab-harness")]
    let plaintext = std::env::var("APPLE_BRIDGE_LAB_IPHONE_PLAINTEXT").is_ok_and(|v| v == "1");
    #[cfg(not(feature = "lab-harness"))]
    let plaintext = false;
    if (!plaintext
        && request
            .extensions()
            .get::<ab_frontdoor::web::SecureTransport>()
            .is_none())
        || request.headers().contains_key(axum::http::header::ORIGIN)
    {
        return ab_frontdoor::problem(Code::Unauthorized.into());
    }
    next.run(request).await
}
async fn pair(
    axum::extract::State(authority): axum::extract::State<Arc<Authority>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Ok(request) = serde_json::from_slice::<ab_trust::iphone::PairRequest>(&body) else {
        return ab_frontdoor::problem(Code::Invalid.into());
    };
    match tokio::task::spawn_blocking(move || authority.pair_iphone(&request)).await {
        Ok(Ok(result)) => match serde_json::to_vec(&result) {
            Ok(bytes) => ab_frontdoor::raw_response(bytes),
            Err(_) => ab_frontdoor::problem(Code::Corrupt.into()),
        },
        _ => ab_frontdoor::problem(Code::Unauthorized.into()),
    }
}
async fn status(
    axum::extract::State(authority): axum::extract::State<Arc<Authority>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let Ok(request) = serde_json::from_slice::<ab_trust::iphone::Request>(&body) else {
        return ab_frontdoor::problem(Code::Invalid.into());
    };
    match tokio::task::spawn_blocking(move || authority.iphone_status(&request)).await {
        Ok(Ok(result)) => match serde_json::to_vec(&result) {
            Ok(bytes) => ab_frontdoor::raw_response(bytes),
            Err(_) => ab_frontdoor::problem(Code::Corrupt.into()),
        },
        _ => ab_frontdoor::problem(Code::Unauthorized.into()),
    }
}
pub async fn run(
    phone: Arc<ab_iphone::delivery::Delivery<PhoneGateway>>,
    wake: Arc<tokio::sync::Notify>,
    home: std::path::PathBuf,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> Result<(), crate::Error> {
    let mut sweep = tokio::time::interval(std::time::Duration::from_secs(1));
    let mut push = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = sweep.tick() => {
                let phone = phone.clone();
                tokio::task::spawn_blocking(move || phone.sweep(ab_trust::decision_now().as_second())).await.map_err(|_| crate::Error::Invalid)?.map_err(|_| crate::Error::Invalid)?;
            },
            _ = push.tick() => push_once(&phone, &home).await,
            () = wake.notified() => push_once(&phone, &home).await,
            () = phone.notified() => push_once(&phone, &home).await,
        }
    }
}
async fn push_once(
    phone: &Arc<ab_iphone::delivery::Delivery<PhoneGateway>>,
    home: &std::path::Path,
) {
    let Ok(identity) = ab_remote::state::read::<ab_remote::relay::Identity>(home, "relay") else {
        return;
    };
    let builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    #[cfg(feature = "lab-harness")]
    let builder = match std::env::var("APPLE_BRIDGE_LAB_TLS_ROOT")
        .ok()
        .and_then(|path| std::fs::read(path).ok())
    {
        Some(bytes) => match reqwest::Certificate::from_pem_bundle(&bytes) {
            Ok(certs) => certs
                .into_iter()
                .fold(builder, |builder, cert| builder.add_root_certificate(cert)),
            Err(_) => return,
        },
        None => builder,
    };
    let Ok(http) = builder.build() else {
        return;
    };
    let Ok(client) = ab_iphone::PushClient::with_client(
        &identity.endpoint,
        identity.credential.hostname,
        identity.credential.credential,
        http,
    ) else {
        return;
    };
    let _ = phone
        .push_once(&client, ab_trust::decision_now().as_second())
        .await;
}
