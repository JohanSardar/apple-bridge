#![forbid(unsafe_code)]
#[cfg(feature = "replica-feed")]
mod replica;
pub mod transport;
use ab_operations::{Code, Command, Error as OperationError};
use ab_wire::gojson::{self, CommandHash, EscapeMode, GoTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    net::{Ipv4Addr, SocketAddr},
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
    #[error("replica home is already locked")]
    Locked,
    #[error(
        "replica feed integration requires the replica-feed feature and pending State scope/blob interfaces"
    )]
    FeedIntegrationPending,
    #[error("invalid replica configuration")]
    Invalid,
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
    pub listen: String,
    #[serde(default)]
    pub retention_rows: u64,
    #[serde(default)]
    pub retention_age: u64,
}
impl Config {
    pub fn validate(&self) -> Result<SocketAddr, Error> {
        let address: SocketAddr = self.api_address.parse().map_err(|_| Error::Invalid)?;
        if self.role != "replica" || address.ip() != Ipv4Addr::LOCALHOST {
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
        let path = self.home.join("runtime.json");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.sync_all()?;
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
    let _lock = HomeLock::acquire(&config.home)?;
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
    axum::serve(listener, frontdoor.router())
        .with_graceful_shutdown(async {
            let _signal = tokio::signal::ctrl_c().await;
        })
        .await?;
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
    config: Config,
    authority: std::sync::Arc<ab_trust::Authority>,
) -> Result<(), Error> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    config.validate()?;
    let enrollment = authority
        .enrollment()
        .map_err(|_| Error::Invalid)?
        .ok_or(Error::Invalid)?;
    let state = Arc::new(Mutex::new(
        ab_state::State::<ab_state::Replica>::open(&config.home).map_err(|_| Error::Invalid)?,
    ));
    let reads = Arc::new(ab_operations::StateReads(state.clone()));
    let blobs = ab_blobs::Store::open(&config.home, 0).map_err(|_| Error::Invalid)?;
    let client = Arc::new(ab_replication::Client::new(
        Arc::new(tokio::sync::Mutex::new(replica::StateAdapter(state))),
        authority.clone(),
        Some(Arc::new(tokio::sync::Mutex::new(blobs))),
    ));
    let identity = Arc::new(ab_operations::TrustIdentity(authority.clone()));
    let connected = Arc::new(AtomicBool::new(false));
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
        transport: Arc::new(transport::DirectTransport {
            authority,
            connected: connected.clone(),
        }),
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
    let mut status = client.status();
    let listen = config.listen.clone();
    let follow = async {
        if listen.is_empty() {
            client.follow(receiver).await
        } else {
            client.listen(&listen, receiver).await
        }
    };
    let track = async {
        while status.changed().await.is_ok() {
            connected.store(
                matches!(
                    status.borrow().state,
                    ab_replication::ConnectionState::Connected
                ),
                Ordering::Release,
            );
        }
    };
    tokio::select! {
        result = serve(config, service) => { let _sent = stop.send(true); result },
        result = follow => { result.map_err(|_| Error::Operation(Code::Unavailable.into())) },
        () = track => Err(Error::Operation(Code::Unavailable.into())),
    }
}
