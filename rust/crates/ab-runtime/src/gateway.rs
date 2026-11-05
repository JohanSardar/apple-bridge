use crate::{AuditLog, Config, Error, GoJson, HomeLock, helper, shutdown_signal};
use ab_operations::Code;
use rand::RngCore;
use std::sync::{Arc, Mutex};

#[allow(clippy::too_many_lines)]
pub(crate) async fn start(
    mut config: Config,
    authority: Arc<ab_trust::Authority>,
) -> Result<(), Error> {
    config.validate()?;
    if config.socket.is_empty() {
        config.socket = config
            .home
            .join("run/peer.sock")
            .to_string_lossy()
            .into_owned();
    }
    if config.peer_address.is_empty() {
        config.peer_address = "127.0.0.1:7443".into();
    }
    let _lock = HomeLock::acquire(&config.home)?;
    let storage = Storage::open(&config)?;
    let Storage {
        state,
        blobs,
        store,
    } = &storage;
    let identity = Arc::new(ab_operations::TrustIdentity(authority.clone()));
    let codec = Arc::new(GoJson);
    let commands = ab_applepeer::Commands::default();
    let delegations = Arc::new(ab_replication::delegation::Delegations::new(
        authority.clone(),
    ));
    let phone_notify = Arc::new(tokio::sync::Notify::new());
    let local = storage.local_gateway(
        &config,
        &identity,
        &codec,
        &commands,
        &delegations,
        &phone_notify,
    )?;
    let connectors = ab_frontdoor::web_connector::Connectors::new(authority.clone());
    let phone = Arc::new(ab_iphone::delivery::Delivery::new(
        authority.clone(),
        crate::iphone::PhoneGateway {
            connectors: connectors.clone(),
            #[cfg(feature = "lab-harness")]
            home: config.home.clone(),
            state: (**store).clone(),
            local: Arc::downgrade(&local),
            authority: authority.clone(),
            blobs: blobs.clone(),
        },
    ));
    let service = Arc::new(ab_operations::Service {
        store: Arc::new(ab_operations::StateReads(state.clone(), blobs.clone())),
        identity,
        target: ab_operations::CommandTarget::Local(local.clone()),
        codec,
    });
    std::fs::create_dir_all(config.home.join("run"))?;
    let control = crate::control::bind(&config.home)?;
    let secret_path = config.home.join("apple-secret");
    let secret = apple_secret(&secret_path)?;
    let (jobs, hydration) = tokio::sync::mpsc::unbounded_channel();
    let blob_source = commands.clone();
    let backend = Arc::new(crate::capture::Backend {
        state: (**store).clone(),
        blobs: blobs.clone(),
        jobs,
        #[cfg(feature = "lab-harness")]
        home: config.home.clone(),
    });
    let peer = Arc::new(ab_applepeer::Server {
        socket: config.socket.clone().into(),
        secret,
        backend,
        commands,
    });
    let feed_listener = tokio::net::TcpListener::bind(&config.peer_address).await?;
    config.peer_address = feed_listener.local_addr()?.to_string();
    let api_listener = tokio::net::TcpListener::bind(config.validate()?).await?;
    config.api_address = api_listener.local_addr()?.to_string();
    let audit = Arc::new(AuditLog::open(&config.home)?);
    let frontdoor = ab_frontdoor::Frontdoor::new(
        service.clone(),
        ab_frontdoor::Config {
            hosts: vec![
                config.api_address.clone(),
                format!("localhost:{}", api_listener.local_addr()?.port()),
            ],
            origins: vec![],
            public: false,
            requests_per_second: 200,
        },
        audit.clone(),
    );
    let feed = Arc::new(ab_replication::server::Server {
        delegations,
        state: Arc::new(tokio::sync::Mutex::new(
            ab_replication::gateway::StateAdapter(state.clone()),
        )),
        blobs: Some(blobs.clone()),
        authority: authority.clone(),
        uploads: Some(Arc::new(crate::forward::Forward {
            #[cfg(feature = "lab-harness")]
            home: config.home.clone(),
            local: local.clone(),
            authority: authority.clone(),
            uploads: ab_replication::upstream::Uploads::new(authority.clone(), blobs.clone()),
        })),
    });
    let approvals = Arc::new(crate::approvals::Approvals(local.clone()));
    let web = ab_frontdoor::web::Web::new(
        authority.clone(),
        api_listener.local_addr()?,
        String::new(),
        Some(approvals.clone()),
        connectors.clone(),
    );
    let window = crate::connector::approval_window(config.helper_app.clone(), config.home.clone());
    let notify_phone = Arc::downgrade(&phone);
    connectors.on_waiting(Arc::new(move || {
        if let Some(phone) = notify_phone.upgrade() {
            phone.wake();
        }
        if let Some(window) = &window {
            window();
        }
    }));
    let phone_router = crate::iphone::router(&phone);
    let router = frontdoor
        .router()
        .merge(phone_router.clone())
        .fallback_service(web.clone().router());
    config.save()?;
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let mut tasks = tokio::task::JoinSet::new();
    let connector_listener = crate::connector::bind(&config.home)?;
    tasks.spawn(crate::connector::serve(
        connector_listener,
        connectors.clone(),
        receiver.clone(),
    ));
    #[cfg(feature = "lab-harness")]
    {
        let home = config.home.clone();
        let state = (**store).clone();
        let blobs = blobs.clone();
        let stop = receiver.clone();
        tasks.spawn(async move { crate::lab_gateway::serve(&home, state, blobs, stop).await });
        let home = config.home.clone();
        let stop = receiver.clone();
        let relay_feed = feed.clone();
        tasks.spawn(async move { crate::lab_relay::serve(&home, relay_feed, stop).await });
    }

    let ingress_stop = receiver.clone();
    let ingress = Arc::new(crate::ingress::App::new(
        config.home.clone(),
        api_listener.local_addr()?,
        service,
        authority.clone(),
        audit,
        approvals,
        connectors,
        phone_router,
        feed.clone(),
    ));
    tasks.spawn(crate::ingress::run(ingress, ingress_stop));
    let phone_stop = receiver.clone();
    tasks.spawn(crate::iphone::run(
        phone,
        phone_notify,
        config.home.clone(),
        phone_stop,
    ));
    let hydrate_stop = receiver.clone();
    let status_stop = receiver.clone();
    tasks.spawn(crate::gateway_status::run(
        (**store).clone(),
        blobs.clone(),
        config.blob_grace_ms,
        status_stop,
    ));
    tasks.spawn(crate::capture::hydrate(
        blobs.clone(),
        blob_source,
        hydration,
        hydrate_stop,
    ));
    let control_stop = receiver.clone();
    tasks.spawn(crate::control::serve(
        control,
        (**store).clone(),
        control_stop,
    ));
    let peer_stop = receiver.clone();
    tasks.spawn(async move { peer.serve(peer_stop).await.map_err(Error::from) });
    let dial_feed = feed.clone();
    let dial_stop = receiver.clone();
    tasks.spawn(async move {
        dial_feed
            .dial(dial_stop)
            .await
            .map_err(|_| Error::Operation(Code::Unavailable.into()))
    });
    let (retention_rows, retention_age) = retention(&config)?;
    tasks.spawn(prune(
        store.clone(),
        retention_rows,
        retention_age,
        receiver.clone(),
    ));
    let feed_stop = receiver.clone();
    tasks.spawn(async move {
        feed.listen(feed_listener, feed_stop)
            .await
            .map_err(|_| Error::Operation(Code::Unavailable.into()))
    });
    let mut api_stop = receiver.clone();
    tasks.spawn(async move {
        axum::serve(
            api_listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = api_stop.wait_for(|value| *value).await;
        })
        .await
        .map_err(Error::from)
    });
    if !config.helper_app.is_empty() {
        tasks.spawn(async move { helper::supervise(&config, &secret_path, receiver).await });
    }
    supervise(&mut tasks, &local, &stop).await
}

type GatewayState = Arc<Mutex<ab_state::State<ab_state::Gateway>>>;
type Blobs = Arc<tokio::sync::Mutex<ab_blobs::Store>>;
type Store = Arc<ab_operations::GatewayStore>;

struct Storage {
    state: GatewayState,
    blobs: Blobs,
    store: Store,
}
impl Storage {
    fn open(config: &Config) -> Result<Self, Error> {
        let state = Arc::new(Mutex::new(
            ab_state::State::<ab_state::Gateway>::open(&config.home).map_err(|error| {
                if matches!(error, ab_state::Error::LedgerMissing) {
                    Error::LedgerMissing
                } else {
                    Error::State(error)
                }
            })?,
        ));
        let blobs = Arc::new(tokio::sync::Mutex::new(
            ab_blobs::Store::open(&config.home, config.blob_cap)
                .map_err(|_| ab_operations::Error::from(Code::Unavailable))?,
        ));
        let protection = Arc::new(ab_replication::gateway::ProtectionAdapter(blobs.clone()));
        let store = Arc::new(ab_operations::GatewayStore(state.clone()));
        store.with(|state| state.set_blob_protection(protection))?;
        Ok(Self {
            state,
            blobs,
            store,
        })
    }

    fn local_gateway(
        &self,
        config: &Config,
        identity: &Arc<ab_operations::TrustIdentity>,
        codec: &Arc<GoJson>,
        commands: &ab_applepeer::Commands,
        delegations: &Arc<ab_replication::delegation::Delegations>,
        phone_notify: &Arc<tokio::sync::Notify>,
    ) -> Result<Arc<ab_operations::local::LocalGateway>, Error> {
        let Self { store, blobs, .. } = self;
        let mut local = ab_operations::local::LocalGateway::new(
            store.clone(),
            identity.clone(),
            codec.clone(),
            Arc::new(commands.clone()),
            Arc::new(crate::actions::Actions {
                state: (**store).clone(),
                peer: Arc::new(commands.clone()),
            }),
        );
        local.reauthorize = Some(delegations.clone());
        local.query_audit = Some(Arc::new(crate::query_audit::Audit::open(&config.home)?));
        #[cfg(feature = "lab-harness")]
        {
            local.clock = Arc::new(ab_operations::clock::LabClock(config.home.clone()));
        }
        local.phone_notify = Some(phone_notify.clone());
        local.query_store = Some(Arc::new(ab_operations::LocalQueryStore {
            state: (**store).clone(),
            blobs: blobs.clone(),
        }));
        let local = Arc::new(local);
        local.recover()?;
        Ok(local)
    }
}

fn apple_secret(secret_path: &std::path::Path) -> Result<[u8; 32], Error> {
    Ok(match std::fs::read(secret_path) {
        Ok(bytes) => bytes.try_into().map_err(|_| Error::Invalid)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut secret = [0; 32];
            rand::rngs::OsRng.fill_bytes(&mut secret);
            ab_trust::atomic_write(secret_path, &secret)?;
            secret
        }
        Err(error) => return Err(error.into()),
    })
}

fn retention(config: &Config) -> Result<(u64, std::time::Duration), Error> {
    let retention_rows = if config.retention_rows == 0 {
        1_000_000
    } else {
        config.retention_rows
    };
    let retention_age = if config.retention_age == 0 {
        std::time::Duration::from_hours(168)
    } else {
        std::time::Duration::from_nanos(
            u64::try_from(config.retention_age).map_err(|_| Error::Invalid)?,
        )
    };
    Ok((retention_rows, retention_age))
}

async fn prune(
    store: Store,
    retention_rows: u64,
    retention_age: std::time::Duration,
    mut prune_stop: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Error> {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = prune_stop.changed() => return Ok(()),
            _ = tick.tick() => crate::tolerate_capacity(store.with(|state| state.prune(retention_rows, retention_age)))?,
        }
    }
}

async fn supervise(
    tasks: &mut tokio::task::JoinSet<Result<(), Error>>,
    local: &Arc<ab_operations::local::LocalGateway>,
    stop: &tokio::sync::watch::Sender<bool>,
) -> Result<(), Error> {
    let result = tokio::select! {
        () = shutdown_signal() => Ok(()),
        result = tasks.join_next() => result.ok_or(Error::Invalid)?.map_err(|_| Error::Invalid)?,
    };
    local.stop();
    let _ = stop.send(true);
    let drain = async { while tasks.join_next().await.is_some() {} };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), drain).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_preserves_defaults_explicit_limits_and_invalid_age()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut config: Config = serde_json::from_value(
            serde_json::json!({"role":"gateway", "home":"", "peer_address":"", "api_address":"", "socket":""}),
        )?;
        assert_eq!(
            retention(&config)?,
            (1_000_000, std::time::Duration::from_hours(168))
        );
        config.retention_rows = 17;
        config.retention_age = 23;
        assert_eq!(
            retention(&config)?,
            (17, std::time::Duration::from_nanos(23))
        );
        config.retention_age = -1;
        assert!(matches!(retention(&config), Err(Error::Invalid)));
        Ok(())
    }
    #[test]
    fn apple_secret_is_reused_and_malformed_bytes_are_preserved()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let path = home.path().join("apple-secret");
        let created = apple_secret(&path)?;
        assert_eq!(apple_secret(&path)?, created);
        assert_eq!(std::fs::read(&path)?, created);
        std::fs::write(&path, b"invalid")?;
        assert!(matches!(apple_secret(&path), Err(Error::Invalid)));
        assert_eq!(std::fs::read(&path)?, b"invalid");
        Ok(())
    }
}
