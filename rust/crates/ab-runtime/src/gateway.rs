use crate::{AuditLog, Config, Error, GoJson, HomeLock, helper, shutdown_signal};
use ab_operations::Code;
use rand::RngCore;
use std::sync::{Arc, Mutex};

#[allow(clippy::too_many_lines)]
pub async fn start(mut config: Config, authority: Arc<ab_trust::Authority>) -> Result<(), Error> {
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
    let identity = Arc::new(ab_operations::TrustIdentity(authority.clone()));
    let codec = Arc::new(GoJson);
    let commands = ab_applepeer::Commands::default();
    let mut local = ab_operations::local::LocalGateway::new(
        store.clone(),
        identity.clone(),
        codec.clone(),
        Arc::new(commands.clone()),
        Arc::new(crate::actions::Actions {
            state: (*store).clone(),
            peer: Arc::new(commands.clone()),
        }),
    );
    local.query_store = Some(Arc::new(ab_operations::LocalQueryStore {
        state: (*store).clone(),
        blobs: blobs.clone(),
    }));
    let local = Arc::new(local);
    local.recover()?;
    let service = Arc::new(ab_operations::Service {
        store: Arc::new(ab_operations::StateReads(state.clone(), blobs.clone())),
        identity,
        target: ab_operations::CommandTarget::Local(local.clone()),
        codec,
    });
    std::fs::create_dir_all(config.home.join("run"))?;
    let control = crate::control::bind(&config.home)?;
    let secret_path = config.home.join("apple-secret");
    let secret: [u8; 32] = match std::fs::read(&secret_path) {
        Ok(bytes) => bytes.try_into().map_err(|_| Error::Invalid)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut secret = [0; 32];
            rand::rngs::OsRng.fill_bytes(&mut secret);
            ab_trust::atomic_write(&secret_path, &secret)?;
            secret
        }
        Err(error) => return Err(error.into()),
    };
    let (jobs, hydration) = tokio::sync::mpsc::channel(64);
    let blob_source = crate::helper_blobs::HelperBlobs(commands.clone());
    let backend = Arc::new(crate::capture::Backend {
        state: (*store).clone(),
        blobs: blobs.clone(),
        jobs,
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
    let frontdoor = ab_frontdoor::Frontdoor::new(
        service,
        ab_frontdoor::Config {
            hosts: vec![
                config.api_address.clone(),
                format!("localhost:{}", api_listener.local_addr()?.port()),
            ],
            origins: vec![],
            public: false,
            requests_per_second: 200,
        },
        Arc::new(AuditLog::open(&config.home)?),
    );
    let feed = Arc::new(ab_replication::server::Server {
        state: Arc::new(tokio::sync::Mutex::new(
            ab_replication::gateway::StateAdapter(state),
        )),
        blobs: Some(blobs.clone()),
        authority: authority.clone(),
        uploads: Some(Arc::new(crate::forward::Forward {
            local: local.clone(),
            authority: authority.clone(),
            uploads: ab_replication::upstream::Uploads::new(authority.clone(), blobs.clone()),
        })),
    });
    let approvals = Arc::new(crate::approvals::Approvals(local.clone()));
    let web = ab_frontdoor::web::Web::new(
        authority,
        api_listener.local_addr()?,
        String::new(),
        Some(approvals),
    );
    let router = frontdoor.router().fallback_service(web.router());
    config.save()?;
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let mut tasks = tokio::task::JoinSet::new();
    let hydrate_stop = receiver.clone();
    let status_stop = receiver.clone();
    tasks.spawn(crate::gateway_status::run(
        (*store).clone(),
        blobs.clone(),
        config.blob_grace_ms,
        status_stop,
    ));
    tasks.spawn(crate::capture::hydrate(
        blobs,
        blob_source,
        hydration,
        hydrate_stop,
    ));
    let control_stop = receiver.clone();
    tasks.spawn(crate::control::serve(
        control,
        (*store).clone(),
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
    let mut prune_stop = receiver.clone();
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
    tasks.spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = prune_stop.changed() => return Ok(()),
                _ = tick.tick() => store.with(|state| state.prune(retention_rows, retention_age))?,
            }
        }
    });
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
