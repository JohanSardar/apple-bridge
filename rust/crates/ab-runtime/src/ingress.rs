use crate::{AuditLog, Error, approvals::Approvals};
use ab_operations::{Code, Service};
use ab_remote::{Gateway, Ingress};
use axum::{Router, body::Body};
use rand::RngCore;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use subtle::ConstantTimeEq;
use tokio::{io::AsyncReadExt, sync::watch};
use tower::ServiceExt;

#[derive(Debug)]
pub struct App {
    pub home: PathBuf,
    pub address: SocketAddr,
    pub service: Arc<Service>,
    pub authority: Arc<ab_trust::Authority>,
    pub audit: Arc<AuditLog>,
    pub approvals: Arc<Approvals>,
    pub phone: Router,
    pub feed: Arc<ab_replication::server::Server<ab_replication::gateway::StateAdapter>>,
    routers: Mutex<BTreeMap<String, Router>>,
    statuses: watch::Sender<ab_remote::IngressStatuses>,
}
impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        home: PathBuf,
        address: SocketAddr,
        service: Arc<Service>,
        authority: Arc<ab_trust::Authority>,
        audit: Arc<AuditLog>,
        approvals: Arc<Approvals>,
        phone: Router,
        feed: Arc<ab_replication::server::Server<ab_replication::gateway::StateAdapter>>,
    ) -> Self {
        Self {
            home,
            address,
            service,
            authority,
            audit,
            approvals,
            phone,
            feed,
            routers: Mutex::default(),
            statuses: watch::channel(BTreeMap::new()).0,
        }
    }
    fn router(&self, host: &str) -> Result<Router, Error> {
        if !hosts(&selected(&self.home)?, &self.statuses.borrow()).contains(host) {
            return Err(ab_operations::Error::from(Code::Unauthorized).into());
        }
        let mut routers = self.routers.lock().map_err(|_| Error::Invalid)?;
        if let Some(router) = routers.get(host) {
            return Ok(router.clone());
        }
        let frontdoor = ab_frontdoor::Frontdoor::new(
            self.service.clone(),
            ab_frontdoor::Config {
                hosts: vec![host.into()],
                origins: vec![],
                public: host != "localhost:7780",
                requests_per_second: 200,
            },
            self.audit.clone(),
        );
        let web = ab_frontdoor::web::Web::new(
            self.authority.clone(),
            self.address,
            host.into(),
            Some(self.approvals.clone()),
        );
        let router = frontdoor
            .router()
            .merge(self.phone.clone())
            .fallback_service(web.router());
        #[cfg(feature = "lab-harness")]
        let router = if host == "localhost:7780"
            && std::env::var_os("APPLE_BRIDGE_LAB_PRIVATE_FEED").is_some()
        {
            router
                .route(
                    "/",
                    axum::routing::get(|| async {
                        (axum::http::StatusCode::UNAUTHORIZED, "passkey required")
                    }),
                )
                .route(
                    "/v1/feed",
                    axum::routing::get(|| async {
                        (
                            [("content-type", "text/event-stream")],
                            "data: private-feed\n\n",
                        )
                    }),
                )
        } else {
            router
        };
        if routers.len() >= 4 {
            routers.clear();
        }
        routers.insert(host.into(), router.clone());
        Ok(router)
    }
}
fn selected(home: &Path) -> Result<BTreeMap<String, bool>, Error> {
    match ab_remote::state::read(home, "ingresses") {
        Ok(selected) => Ok(selected),
        Err(ab_remote::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            match ab_remote::state::read::<ab_remote::state::Config>(home, "config") {
                Ok(config) => Ok(BTreeMap::from([(config.backend, config.enabled)])),
                Err(ab_remote::Error::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    Ok(BTreeMap::new())
                }
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}
fn hosts(
    selected: &BTreeMap<String, bool>,
    statuses: &ab_remote::IngressStatuses,
) -> BTreeSet<String> {
    statuses
        .iter()
        .filter(|(name, status)| selected.get(*name) == Some(&true) && status.active)
        .map(|(_, status)| status.host.clone())
        .collect()
}
fn publish(
    home: &Path,
    selected: &BTreeMap<String, bool>,
    statuses: &ab_remote::IngressStatuses,
) -> Result<(), Error> {
    let config =
        ab_remote::state::read::<ab_remote::state::Config>(home, "config").unwrap_or_default();
    let active = statuses
        .iter()
        .find(|(name, status)| selected.get(*name) == Some(&true) && status.active);
    let state = if active.is_some() {
        "on"
    } else if selected.values().any(|enabled| *enabled) {
        "starting"
    } else {
        "off"
    };
    let private = active.is_some_and(|(name, _)| name == "private");
    ab_remote::state::write(
        home,
        "status",
        &serde_json::json!({"id":config.id,"state":state,
        "backend":active.map_or(config.backend.as_str(),|(name,_)|name.as_str()),"url":active.filter(|(name,_)| name.as_str() != "private").map_or_else(String::new,|(_,status)|format!("https://{}",status.host)),
        "message": if private { ab_remote::state::read::<String>(home,"invitation").unwrap_or_default() } else { String::new() },"last_error":"","pending_link":"","tls_termination": if private { "Tailscale encrypted private transport" } else { "on this Mac" },"certificate_expiry":"0001-01-01T00:00:00Z"}),
    )?;
    Ok(())
}
pub async fn run(app: Arc<App>, mut stop: watch::Receiver<bool>) -> Result<(), Error> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mut proof = [0; 32];
    rand::rngs::OsRng.fill_bytes(&mut proof);
    let gateway =
        Gateway::authenticated(listener.local_addr()?, proof).with_status(app.statuses.clone());
    let statuses = app.statuses.subscribe();
    let ingress_stop = stop.clone();
    let home = app.home.clone();
    let mut adapters =
        tokio::spawn(async move { reconcile(home, gateway, statuses, ingress_stop).await });
    let mut connections = tokio::task::JoinSet::new();
    let result = loop {
        tokio::select! {
            _ = stop.changed() => break Ok(()),
            result = &mut adapters => break result.map_err(|_| Error::Invalid)?,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept(), if connections.len() < 64 => {
                let (socket, address) = accepted?;
                if !address.ip().is_loopback() { continue; }
                let app = app.clone(); let stop = stop.clone();
                connections.spawn(async move { connection(socket,proof,app,stop).await });
            }
        }
    };
    connections.abort_all();
    if !adapters.is_finished() {
        let _ = adapters.await;
    }
    result
}
async fn connection(
    mut socket: tokio::net::TcpStream,
    proof: [u8; 32],
    app: Arc<App>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let binding = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut magic = vec![0; ab_remote::INGRESS_MAGIC.len()];
        socket.read_exact(&mut magic).await?;
        let mut received = [0; 32];
        socket.read_exact(&mut received).await?;
        if magic != ab_remote::INGRESS_MAGIC || !bool::from(proof.ct_eq(&received)) {
            return Err(Error::Invalid);
        }
        let flag = socket.read_u8().await?;
        let binding = match flag {
            0 => None,
            1 => {
                let mut b = [0; 32];
                socket.read_exact(&mut b).await?;
                Some(b)
            }
            _ => return Err(Error::Invalid),
        };
        Ok(binding)
    })
    .await
    .map_err(|_| Error::Invalid)??;
    let service =
        hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let app = app.clone();
            async move {
                let mut request = request.map(Body::new);
                let Some(host) = request
                    .headers()
                    .get(hyper::header::HOST)
                    .and_then(|host| host.to_str().ok())
                    .map(str::to_owned)
                else {
                    return Ok::<_, std::convert::Infallible>(ab_frontdoor::problem(
                        Code::Unauthorized.into(),
                    ));
                };
                let lab_private_feed = false;
                #[cfg(feature = "lab-harness")]
                let lab_private_feed = host == "localhost:7780" && std::env::var_os("APPLE_BRIDGE_LAB_PRIVATE_FEED").is_some();
                if request.uri().path() == "/v1/feed" && !lab_private_feed {
                    if !hosts(
                        &selected(&app.home).unwrap_or_default(),
                        &app.statuses.borrow(),
                    )
                    .contains(&host)
                    {
                        return Ok(ab_frontdoor::problem(Code::Unauthorized.into()));
                    }
                    return Ok(app.feed.clone().relay_http(request, binding).map(Body::new));
                }
                let router = tokio::task::spawn_blocking(move || app.router(&host)).await;
                let Ok(Ok(router)) = router else {
                    return Ok(ab_frontdoor::problem(Code::Unauthorized.into()));
                };
                request
                    .extensions_mut()
                    .insert(ab_frontdoor::web::SecureTransport);
                router.oneshot(request).await
            }
        });
    let connection = hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
        .with_upgrades();
    tokio::select! {
        _ = stop.changed() => Ok(()),
        result = connection => result.map_err(|_| Error::Invalid),
    }
}
async fn reconcile(
    home: PathBuf,
    gateway: Gateway,
    mut statuses: watch::Receiver<ab_remote::IngressStatuses>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let mut prior = BTreeMap::new();
    let mut cancels = BTreeMap::<&str, watch::Sender<bool>>::new();
    let mut running = BTreeSet::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = statuses.changed() => { publish(&home,&prior,&statuses.borrow())?; },
            _ = stop.changed() => {
                for cancel in cancels.values() { let _ = cancel.send(true); }
                while tasks.join_next().await.is_some() {}
                return Ok(());
            },
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Ok((name,_)) = result { running.remove(name); cancels.remove(name); }
            },
            _ = tick.tick() => {
                let current = selected(&home)?;
                for (name,cancel) in &cancels {
                    if current.get(*name) != Some(&true) { let _ = cancel.send(true); }
                }
                prior = current.clone();
                publish(&home,&current,&statuses.borrow())?;
                for name in ["relay","system","private"] {
                    if current.get(name) != Some(&true) || running.contains(name) { continue; }
                    let Ok(adapter) = adapter(&home,name) else { continue; };
                    let gateway = gateway.clone(); let (cancel, stop) = watch::channel(false);
                    cancels.insert(name,cancel);
                    running.insert(name);
                    tasks.spawn(async move { (name, adapter.run(gateway,stop).await) });
                }
            }
        }
    }
}
fn adapter(home: &Path, name: &str) -> Result<Box<dyn Ingress>, Error> {
    if name == "private" {
        return Ok(Box::new(ab_remote::private::Private {
            home: home.to_owned(),
            tailscale: ab_remote::tailscale::Tailscale::local(
                home.join("private"),
                &ab_remote::private::socket_path(),
            )?,
        }));
    }
    if name == "system" {
        let socket = std::env::var_os("APPLE_BRIDGE_TAILSCALE_SOCKET")
            .map_or_else(|| "/var/run/tailscaled.socket".into(), PathBuf::from);
        return Ok(Box::new(ab_remote::tailscale::Tailscale::local(
            home.to_owned(),
            &socket,
        )?));
    }
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| Error::Invalid)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Error::Invalid)?;
    Ok(Box::new(ab_remote::relay::Relay {
        home: home.to_owned(),
        endpoint: std::env::var("APPLE_BRIDGE_RELAY_URL")
            .unwrap_or_else(|_| "https://ab.lexaai.tech".into()),
        zone: std::env::var("APPLE_BRIDGE_RELAY_ZONE").unwrap_or_else(|_| "ab.lexaai.tech".into()),
        client,
        certificates: Arc::new(ab_remote::certificate::Certificates::new(
            home.to_owned(),
            std::env::var("APPLE_BRIDGE_ACME_DIRECTORY")
                .unwrap_or_else(|_| "https://acme-v02.api.letsencrypt.org/directory".into()),
        )),
        tls: Arc::new(tls),
    }))
}
