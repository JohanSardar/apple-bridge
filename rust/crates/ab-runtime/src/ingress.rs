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
use tokio::sync::watch;
use tower::ServiceExt;

#[derive(Debug)]
pub struct App {
    pub home: PathBuf,
    pub address: SocketAddr,
    pub service: Arc<Service>,
    pub authority: Arc<ab_trust::Authority>,
    pub audit: Arc<AuditLog>,
    pub approvals: Arc<Approvals>,
    pub connectors: Arc<ab_frontdoor::web_connector::Connectors>,
    pub phone: Router,
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
        connectors: Arc<ab_frontdoor::web_connector::Connectors>,
        phone: Router,
    ) -> Self {
        Self {
            home,
            address,
            service,
            authority,
            audit,
            approvals,
            connectors,
            phone,
            routers: Mutex::default(),
            statuses: watch::channel(BTreeMap::new()).0,
        }
    }
    fn router(&self, host: &str) -> Result<Router, Error> {
        if !hosts(&self.home, &self.statuses.borrow()).contains(host) {
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
                public: true,
                requests_per_second: 200,
            },
            self.audit.clone(),
        );
        let web = ab_frontdoor::web::Web::new(
            self.authority.clone(),
            self.address,
            host.into(),
            Some(self.approvals.clone()),
            self.connectors.clone(),
        );
        web.set_route(ab_remote::state::ROUTE);
        let router = frontdoor
            .router()
            .merge(self.phone.clone())
            .fallback_service(web.router());
        if routers.len() >= 4 {
            routers.clear();
        }
        routers.insert(host.to_owned(), router.clone());
        Ok(router)
    }
}
fn hosts(home: &Path, statuses: &ab_remote::IngressStatuses) -> BTreeSet<String> {
    if !ab_remote::state::selected(home) {
        return BTreeSet::new();
    }
    statuses
        .get(ab_remote::state::ROUTE)
        .filter(|status| status.active)
        .map(|status| status.host.clone())
        .into_iter()
        .collect()
}
fn publish(home: &Path, statuses: &ab_remote::IngressStatuses) -> Result<(), Error> {
    let config =
        ab_remote::state::read::<ab_remote::state::Config>(home, "config").unwrap_or_default();
    let selection = ab_remote::state::selection(home);
    let enabled = selection == Some(true);
    let status = statuses.get(ab_remote::state::ROUTE);
    let running = status.is_some_and(|status| status.active);
    let active = enabled && running;
    let progress = status
        .and_then(|status| status.progress.as_ref())
        .filter(|_| enabled && !active);
    let state = if active {
        "on"
    } else if running {
        "stopping"
    } else if let Some(progress) = progress {
        progress.state
    } else if enabled {
        "starting"
    } else {
        "off"
    };
    let url = status
        .filter(|_| active)
        .map_or_else(String::new, |status| format!("https://{}", status.host));
    let route = ab_remote::state::Route {
        state: state.into(),
        url: url.clone(),
        message: progress.map_or_else(String::new, |p| p.message.clone()),
        pending_link: progress.map_or_else(String::new, |p| p.pending_link.clone()),
        link: progress.map_or_else(String::new, |p| p.link.clone()),
        last_error: progress.map_or_else(String::new, |p| p.last_error.clone()),
    };
    let document = ab_remote::state::Status {
        id: config.id,
        state: state.into(),
        url,
        last_error: route.last_error.clone(),
        message: route.message.clone(),
        pending_link: route.pending_link.clone(),
        certificate_expiry: status
            .filter(|_| active)
            .and_then(|status| status.certificate_expiry.clone())
            .unwrap_or_else(|| ab_remote::state::ZERO_TIME.into()),
        routes: if selection.is_some() {
            BTreeMap::from([(ab_remote::state::ROUTE.to_owned(), route)])
        } else {
            BTreeMap::new()
        },
    };
    let written = ab_remote::state::write(home, "status", &document);
    match written {
        Err(ab_remote::Error::Io(error)) if error.raw_os_error() == Some(28) => Ok(()),
        other => other.map_err(Into::into),
    }
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
    let preamble = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ab_remote::read_ingress_preamble(&mut socket, &proof),
    )
    .await
    .map_err(|_| Error::Invalid)?
    .map_err(|_| Error::Invalid)?;
    let source = preamble.source;
    let service =
        hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let app = app.clone();
            async move {
                let mut request = request.map(Body::new);
                if let Some(source) = source {
                    request
                        .extensions_mut()
                        .insert(axum::extract::ConnectInfo(SocketAddr::new(source, 0)));
                }
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
    crate::http_deadline::serve(
        socket,
        service,
        crate::http_deadline::Deadlines::INGRESS,
        &mut stop,
    )
    .await
}
fn config_id(home: &Path) -> String {
    ab_remote::state::read::<ab_remote::state::Config>(home, "config")
        .map(|config| config.id)
        .unwrap_or_default()
}
async fn reconcile(
    home: PathBuf,
    gateway: Gateway,
    mut statuses: watch::Receiver<ab_remote::IngressStatuses>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    const ROUTE: &str = ab_remote::state::ROUTE;
    let mut cancel: Option<watch::Sender<bool>> = None;
    let mut task: tokio::task::JoinSet<ab_remote::Result<()>> = tokio::task::JoinSet::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    let mut failed: Option<String> = None;
    let mut forwarded = (String::new(), String::new());
    loop {
        tokio::select! {
            _ = statuses.changed() => {
                publish(&home, &statuses.borrow())?;
                let id = config_id(&home);
                let link = ab_remote::state::selected(&home)
                    .then(|| statuses.borrow().get(ROUTE).and_then(|status| status.progress.as_ref()).filter(|progress| !progress.link.is_empty()).map(|progress| progress.link.clone()))
                    .flatten();
                if let Some(link) = link && !id.is_empty() && forwarded != (id.clone(), link.clone()) {
                    forwarded = (id.clone(), link.clone());
                    let _ = ab_remote::onboarding::send_link(&home, &id, &link).await;
                }
            },
            _ = stop.changed() => {
                if let Some(cancel) = &cancel { let _ = cancel.send(true); }
                while task.join_next().await.is_some() {}
                return Ok(());
            },
            Some(result) = task.join_next(), if !task.is_empty() => {
                let cancelled = cancel.take().is_none_or(|cancel| *cancel.borrow());
                if let Ok(Err(error)) = result && !cancelled && !*stop.borrow() {
                    eprintln!("Tailscale setup failed; run bridge tailscale --off, then try again: {error}");
                    failed = Some(config_id(&home));
                    gateway.progress(ROUTE, ab_remote::Progress::failed_because(&error.to_string()));
                }
            },
            _ = tick.tick() => {
                let enabled = ab_remote::state::selected(&home);
                if !enabled && let Some(cancel) = &cancel {
                    let _ = cancel.send(true);
                }
                publish(&home, &statuses.borrow())?;
                if !enabled {
                    failed = None;
                    if task.is_empty() {
                        gateway.clear_progress(ROUTE);
                    }
                    continue;
                }
                let id = config_id(&home);
                if !task.is_empty() || failed.as_ref() == Some(&id) {
                    continue;
                }
                failed = None;
                gateway.clear_progress(ROUTE);
                let adapter = ab_remote::tailscale::Tailscale::tailnet(home.join("tailnet"));
                let gateway = gateway.clone();
                let (sender, receiver) = watch::channel(false);
                cancel = Some(sender);
                task.spawn(async move { adapter.run(gateway, receiver).await });
            }
        }
    }
}
