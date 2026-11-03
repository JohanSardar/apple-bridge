#![forbid(unsafe_code)]
#[cfg(feature = "replica-feed")]
mod actions;
#[cfg(feature = "replica-feed")]
mod approvals;
pub mod bundle;
#[cfg(feature = "replica-feed")]
mod calendar_actions;
#[cfg(feature = "replica-feed")]
mod capture;
pub mod connector;
mod control;
pub mod duration;
#[cfg(feature = "replica-feed")]
mod forward;
#[cfg(all(feature = "replica-feed", feature = "lab-harness"))]
mod forward_observer;
#[cfg(feature = "replica-feed")]
mod gateway;
#[cfg(feature = "replica-feed")]
mod gateway_status;
pub mod helper;
pub mod helper_blobs;
#[cfg(feature = "replica-feed")]
mod ingress;
pub mod inspection;
#[cfg(feature = "replica-feed")]
mod iphone;
#[cfg(all(feature = "lab-harness", feature = "replica-feed"))]
mod lab_gateway;
#[cfg(all(feature = "lab-harness", feature = "replica-feed"))]
mod lab_relay;
pub mod maintenance;
#[cfg(feature = "replica-feed")]
mod message_actions;
#[cfg(feature = "replica-feed")]
mod query_audit;
#[cfg(feature = "replica-feed")]
mod replica;
mod shape;
#[cfg(feature = "replica-feed")]
pub mod transport;
use ab_operations::{Code, Command, Error as OperationError};
use ab_wire::gojson::{self, CommandHash, EscapeMode, GoTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error(transparent)]
    State(#[from] ab_state::Error),
    #[error(transparent)]
    Trust(#[from] ab_trust::Error),
    #[error(transparent)]
    Remote(#[from] ab_remote::Error),
    #[error("ledger_missing: command ledger missing; run bridge restore --accept-ledger-loss")]
    LedgerMissing,
    #[error("home is already locked")]
    Locked,
    #[error(
        "replica feed integration requires the replica-feed feature and pending State scope/blob interfaces"
    )]
    FeedIntegrationPending,
    #[error("invalid replica configuration")]
    Invalid,
}
pub(crate) fn tolerate_capacity<T>(
    result: Result<T, OperationError>,
) -> Result<(), OperationError> {
    match result {
        Err(error) if error.code == Code::Capacity => Ok(()),
        other => other.map(|_| ()),
    }
}
#[derive(Debug)]
pub struct GoJson;
impl ab_operations::Codec for GoJson {
    fn json(&self, raw: &[u8]) -> Result<Vec<u8>, OperationError> {
        gojson::legacy_raw(raw, EscapeMode::Html).map_err(|_| Code::Corrupt.into())
    }
    fn command_hash(&self, command: &Command) -> Result<String, OperationError> {
        let expires =
            GoTime::parse(&command.expires_at).map_err(|_| OperationError::from(Code::Invalid))?;
        CommandHash {
            op: &command.op,
            args: command.args.get().as_bytes(),
            version: command.expected_version,
            expires,
            era: &command.admission_era,
        }
        .digest()
        .map_err(|_| Code::Invalid.into())
    }
    fn fingerprint(&self, raw: &[u8]) -> String {
        format!("{:x}", Sha256::digest(raw))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub role: String,
    pub home: PathBuf,
    pub peer_address: String,
    pub api_address: String,
    pub socket: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub helper_app: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub helper_lab_commands: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub listen: String,
    #[serde(default)]
    pub retention_rows: u64,
    #[serde(default)]
    pub blob_cap: u64,
    #[serde(default)]
    pub blob_grace_ms: u64,
    #[serde(default)]
    pub retention_age: i64,
}
impl Config {
    pub fn validate(&self) -> Result<SocketAddr, Error> {
        let address: SocketAddr = self.api_address.parse().map_err(|_| Error::Invalid)?;
        if !matches!(self.role.as_str(), "replica" | "gateway") || !address.ip().is_loopback() {
            return Err(Error::Invalid);
        }
        Ok(address)
    }
    pub fn load(home: &Path) -> Result<Self, Error> {
        Ok(serde_json::from_slice(&fs::read(
            home.join("runtime.json"),
        )?)?)
    }
    pub fn save(&self) -> Result<(), Error> {
        ab_trust::atomic_write(&self.home.join("runtime.json"), &serde_json::to_vec(self)?)
            .map_err(|_| Error::Invalid)?;
        Ok(())
    }
}
#[derive(Debug)]
pub struct HomeLock {
    _file: File,
}
impl HomeLock {
    pub fn acquire(home: &Path) -> Result<Self, Error> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(home)?;
        fs::set_permissions(home, fs::Permissions::from_mode(0o700))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(home.join("lock"))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.try_lock().map_err(|_| Error::Locked)?;
        Ok(Self { _file: file })
    }
}
#[derive(Debug)]
pub struct AuditLog(Mutex<File>);
impl AuditLog {
    pub fn open(home: &Path) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(home.join("frontdoor-audit.jsonl"))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(Self(Mutex::new(file)))
    }
}
impl ab_frontdoor::Audit for AuditLog {
    fn record(&self, entry: ab_frontdoor::AuditEntry) -> Result<(), OperationError> {
        let mut file = self
            .0
            .lock()
            .map_err(|_| OperationError::from(Code::Unavailable))?;
        let mut bytes =
            serde_json::to_vec(&entry).map_err(|_| OperationError::from(Code::Corrupt))?;
        bytes.push(b'\n');
        file.write_all(&bytes)
            .map_err(|_| OperationError::from(Code::Unavailable))
    }
}
pub fn default_home() -> Result<PathBuf, Error> {
    let home = std::env::var_os("HOME").ok_or(Error::Invalid)?;
    if cfg!(target_os = "macos") {
        return Ok(PathBuf::from(home).join("Library/Application Support/AppleBridge"));
    }
    let root = std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map_or_else(|| PathBuf::from(home).join(".local/share"), PathBuf::from);
    Ok(root.join("apple-bridge"))
}
pub async fn serve(
    mut config: Config,
    service: std::sync::Arc<ab_operations::Service>,
) -> Result<(), Error> {
    let address = config.validate()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    config.api_address = listener.local_addr()?.to_string();
    let audit = std::sync::Arc::new(AuditLog::open(&config.home)?);
    let frontdoor = ab_frontdoor::Frontdoor::new(
        service,
        ab_frontdoor::Config {
            hosts: vec![
                config.api_address.clone(),
                format!("localhost:{}", listener.local_addr()?.port()),
            ],
            origins: vec![],
            public: false,
            requests_per_second: 200,
        },
        audit,
    );
    config.save()?;
    let (shutdown, mut receiver) = tokio::sync::watch::channel(false);
    let signal = async {
        shutdown_signal().await;
        let _sent = shutdown.send(true);
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    };
    let server = axum::serve(listener, frontdoor.router()).with_graceful_shutdown(async move {
        let _signal = receiver.wait_for(|stopping| *stopping).await;
    });
    tokio::select! { result=server=>result?, ()=signal=>{} }
    Ok(())
}

#[cfg(not(feature = "replica-feed"))]
pub fn start(
    _config: Config,
    _authority: std::sync::Arc<ab_trust::Authority>,
) -> std::future::Ready<Result<(), Error>> {
    std::future::ready(Err(Error::FeedIntegrationPending))
}
#[cfg(feature = "replica-feed")]
pub async fn start(
    mut config: Config,
    authority: std::sync::Arc<ab_trust::Authority>,
) -> Result<(), Error> {
    use std::sync::Arc;
    if config.role == "gateway" {
        return gateway::start(config, authority).await;
    }
    config.validate()?;
    let _lock = HomeLock::acquire(&config.home)?;
    let enrollment = authority
        .enrollment()
        .map_err(|_| Error::Invalid)?
        .ok_or(Error::Invalid)?;
    let state = Arc::new(Mutex::new(
        ab_state::State::<ab_state::Replica>::open(&config.home).map_err(|_| Error::Invalid)?,
    ));
    let blobs = ab_blobs::Store::open(&config.home, config.blob_cap).map_err(|_| Error::Invalid)?;
    let blobs = Arc::new(tokio::sync::Mutex::new(blobs));
    let reads = Arc::new(ab_operations::StateReads(state.clone(), blobs.clone()));
    let client = ab_replication::Client::new(
        Arc::new(tokio::sync::Mutex::new(replica::StateAdapter(
            state.clone(),
        ))),
        authority.clone(),
        Some(blobs.clone()),
    );
    #[cfg(feature = "lab-harness")]
    let client = client.with_apply_delay(config.home.join("command-apply-delay"));
    let client = Arc::new(client);
    let identity = Arc::new(ab_operations::TrustIdentity(authority.clone()));
    let codec = Arc::new(GoJson);
    let route = if enrollment.remote || !enrollment.dial.is_empty() {
        ab_operations::GatewayRoute::Feed
    } else {
        ab_operations::GatewayRoute::Direct {
            address: enrollment.address,
            fingerprint: enrollment.fingerprint,
        }
    };
    let remote = Arc::new(ab_operations::RemoteGateway {
        route,
        transport: Arc::new(transport::ReplicaGatewayTransport(
            client.gateway_transport(),
        )),
        identity: identity.clone(),
        store: reads.clone(),
        codec: codec.clone(),
    });
    let service = Arc::new(ab_operations::Service {
        store: reads,
        identity,
        target: ab_operations::CommandTarget::Remote(remote),
        codec,
    });
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let listener = if config.listen.is_empty() {
        None
    } else {
        let listener = tokio::net::TcpListener::bind(&config.listen).await?;
        config.listen = listener.local_addr()?.to_string();
        Some(listener)
    };
    let follow = async {
        if let Some(listener) = listener {
            client.listen_bound(listener, receiver).await
        } else {
            client.follow(receiver).await
        }
    };
    let clock_home = config.home.clone();
    let track = track_status(
        client.status(),
        blobs,
        state,
        clock_home,
        gateway_status::grace_seconds(config.blob_grace_ms)?,
    );
    tokio::select! {
        result = serve(config, service) => { let _sent = stop.send(true); result },
        result = follow => { result.map_err(|_| Error::Operation(Code::Unavailable.into())) },
        result = track => result,
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        return;
    }
    let _signal = tokio::signal::ctrl_c().await;
}

#[cfg(feature = "replica-feed")]
async fn track_status(
    mut status: tokio::sync::watch::Receiver<ab_replication::FollowStatus>,
    blobs: std::sync::Arc<tokio::sync::Mutex<ab_blobs::Store>>,
    state: std::sync::Arc<Mutex<ab_state::State<ab_state::Replica>>>,
    clock_home: PathBuf,
    grace: i64,
) -> Result<(), Error> {
    let mut last_clock = None;
    let mut last_collect = std::time::Instant::now();
    loop {
        tokio::select! {
            changed = status.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
        }
        let current = status.borrow().clone();
        if let Some(sample) = &current.clock_sample
            && last_clock.as_ref() != Some(sample)
        {
            last_clock = Some(sample.clone());
            ab_trust::atomic_write(
                &clock_home.join("gateway-clock.json"),
                &serde_json::to_vec(sample)?,
            )
            .map_err(|_| Error::Invalid)?;
        }
        if last_collect.elapsed() >= std::time::Duration::from_secs(1) {
            last_collect = std::time::Instant::now();
            let mut owned = blobs.clone().lock_owned().await;
            if let Err(error) = tokio::task::spawn_blocking(move || {
                owned.gc(jiff::Timestamp::now(), grace)
            })
            .await
            .map_err(|_| Error::Invalid)?
            {
                eprintln!(
                    "{}",
                    serde_json::json!({"stage":"blob_gc_deferred","error":error.to_string()})
                );
            }
        }
        let progress = blobs.lock().await.progress();
        let staleness_ms = current
            .last_heartbeat
            .parse::<jiff::Timestamp>()
            .map_or(0, |t| {
                jiff::Timestamp::now()
                    .as_millisecond()
                    .saturating_sub(t.as_millisecond())
            });
        let connection = if staleness_ms > 15_000 {
            serde_json::json!("disconnected")
        } else {
            serde_json::to_value(&current.state)?
        };
        let mut state = state.lock().map_err(|_| Error::Invalid)?;
        let cursor = state.head().map_err(|_| Error::Invalid)?;
        state.set_replica_status(serde_json::json!({
                "state": connection, "mode": current.mode,
                "gateway_head": current.gateway_head, "cursor": cursor,
                "role": "replica", "schema": current.schema, "snapshot_sequence": current.snapshot_sequence,
                "lag": current.gateway_head.saturating_sub(cursor.sequence), "blob_backlog": 0,
                "staleness_ms": staleness_ms,
                "blobs": progress,
                "snapshots": current.snapshots, "resumes": current.resumes,
                "snapshot_pages": current.snapshot_pages,
                "last_heartbeat": current.last_heartbeat
            })).map_err(|_| Error::Invalid)?;
    }
    Ok::<(), Error>(())
}
