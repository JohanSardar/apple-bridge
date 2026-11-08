#![forbid(unsafe_code)]

use rustls::pki_types::pem::PemObject;

use ab_relay::{
    http::Api,
    server::{Capacity, ConnectionDeadlines, Server},
    store::{Limits, Store},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
mod control;
use control::serve_control;

fn listen_addr(addr: &str) -> String {
    if addr.starts_with(':') {
        format!("[::]{addr}")
    } else {
        addr.to_owned()
    }
}

fn options() -> std::result::Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    let mut out = HashMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if !arg.starts_with('-') {
            return Err("unexpected positional argument".into());
        }
        let arg = arg.trim_start_matches('-');
        if let Some((k, v)) = arg.split_once('=') {
            out.insert(k.to_owned(), v.to_owned());
        } else if arg == "control-acme" || arg == "apns-sandbox" {
            out.insert(arg.to_owned(), "true".into());
        } else {
            out.insert(arg.to_owned(), args.next().ok_or("flag value required")?);
        }
    }
    for key in out.keys() {
        if ![
            "dns-challenge-alias",
            "legacy-zones",
            "legacy-dns-zone",
            "extra-hosts",
            "zone",
            "state",
            "addr",
            "control-host",
            "control-addr",
            "control-cert",
            "control-key",
            "owner-keys-file",
            "idle-timeout",
            "connections",
            "bytes-per-second",
            "monthly-bytes",
            "dns-project",
            "dns-zone",
            "control-acme",
            "apns-key",
            "apns-sandbox",
            "app-host",
            "app-upstream",
            "dns-listen",
            "public-ip",
            "unrouted-slots",
            "tenant-slots",
            "control-slots",
            "app-slots",
            "source-slots",
            "first-request-timeout",
            "keep-alive-timeout",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unknown flag: {key}").into());
        }
    }
    Ok(out)
}
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
async fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args = options()?;
    let get =
        |key: &str, default: &str| args.get(key).cloned().unwrap_or_else(|| default.to_owned());
    let idle = humantime::parse_duration(&get("idle-timeout", "2m"))?;
    if idle.is_zero() {
        return Err("idle-timeout must be positive".into());
    }
    let store = configured_store(&args)?;
    let stop = CancellationToken::new();
    let dns = configured_dns(&args).await?;
    let config = control_tls(&args, tls_builder()?, dns.clone(), stop.clone()).await?;
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let control = if get("control-host", "").is_empty() {
        Some(TcpListener::bind(listen_addr(&get("control-addr", "127.0.0.1:8443"))).await?)
    } else {
        None
    };
    let (control_tx, control_rx) = tokio::sync::mpsc::channel(256);
    let mut server = Server::new(store, idle);
    server.capacity = configured_capacity(&args)?;
    let deadlines = ConnectionDeadlines {
        first_request: humantime::parse_duration(&get("first-request-timeout", "10s"))?,
        idle: humantime::parse_duration(&get("keep-alive-timeout", "30s"))?,
        ..ConnectionDeadlines::default()
    };
    let control_slots = Arc::new(Semaphore::new(server.capacity.control));
    if let Some(host) = args.get("control-host").filter(|s| !s.is_empty()) {
        server.control = Some((host.clone(), control_tx));
    }
    let app_host = get("app-host", "");
    let (app_tx, app_rx) = tokio::sync::mpsc::channel(256);
    if !app_host.is_empty() {
        server.app = Some((app_host.clone(), app_tx));
    }
    let extra_hosts = additional_hosts(&args, &mut server)?;
    let server = Arc::new(server);
    let mut api = Api::new(
        server.clone(),
        std::env::var("APPLE_BRIDGE_RELAY_ADMIN").unwrap_or_default(),
    )?;
    api.dns = dns.clone();
    if !app_host.is_empty() {
        api.accounts_url = format!("https://{app_host}");
    }
    if let Some(path) = args.get("apns-key").filter(|p| !p.is_empty()) {
        api.push.provider = Some(Arc::new(ab_relay::push::Apns::load(
            std::path::Path::new(path),
            get("apns-sandbox", "true") == "true",
        )?));
    }
    let api = Arc::new(api);
    let router = api.clone().router();
    let make = router.clone().into_make_service_with_connect_info();
    let public = TcpListener::bind(listen_addr(&get("addr", "0.0.0.0:443"))).await?;
    tokio::spawn(cancel_on_signal(stop.clone()));
    let stop_api = stop.clone();
    tokio::spawn(ab_relay::ct::monitor(api, stop.clone()));
    if !app_host.is_empty() {
        let dns = dns.clone().ok_or("the sign-in host needs a DNS provider")?;
        let state = PathBuf::from(get("state", "relay-state.json"));
        let path = state
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("control-tls/app");
        let upstream = get("app-upstream", "127.0.0.1:8789");
        tokio::spawn(serve_app(
            path,
            app_host,
            upstream,
            dns,
            tls_builder()?,
            app_rx,
            stop.clone(),
            deadlines,
        ));
    }
    let own_listener = control.map(|listener| (listener, control_slots));
    serve_additional_hosts(
        extra_hosts,
        &get("state", "relay-state.json"),
        dns,
        router,
        stop.clone(),
        deadlines,
    )
    .await?;
    let api_task = serve_control(
        own_listener,
        control_rx,
        make,
        acceptor,
        stop_api,
        deadlines,
    );
    tokio::select! {
        result = server.serve(public, stop.clone()) => result?,
        () = api_task => {},
    }
    stop.cancel();
    Ok(())
}

type TlsBuilder = rustls::ConfigBuilder<rustls::ServerConfig, rustls::server::WantsServerCert>;
type Dns = Option<Arc<dyn ab_relay::http::DnsProvider>>;

fn tls_builder() -> std::result::Result<TlsBuilder, rustls::Error> {
    Ok(rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth())
}

async fn dns_provider(
    args: &HashMap<String, String>,
) -> std::result::Result<Dns, Box<dyn std::error::Error>> {
    let get =
        |key: &str, default: &str| args.get(key).cloned().unwrap_or_else(|| default.to_owned());
    if let Some(listen) = args.get("dns-listen").filter(|s| !s.is_empty()) {
        let public_ip = get("public-ip", "").parse()?;
        return Ok(Some(
            ab_relay::authoritative::Server::start(listen, &get("zone", ""), public_ip).await?,
        ));
    }
    if let Some(project) = args.get("dns-project").filter(|s| !s.is_empty()) {
        let primary = Arc::new(ab_relay::cloud_dns::CloudDns::new(
            project,
            &get("dns-zone", ""),
            &get("zone", ""),
        )?);
        if let Some(legacy_zone) = args.get("legacy-dns-zone") {
            let suffix = get("legacy-zones", "");
            if suffix.contains(',') || suffix.is_empty() {
                return Err("legacy Cloud DNS needs exactly one legacy zone".into());
            }
            let legacy = Arc::new(ab_relay::cloud_dns::CloudDns::new(
                project,
                legacy_zone,
                &suffix,
            )?);
            return Ok(Some(Arc::new(MigrationDns {
                primary,
                legacy,
                suffix,
            })));
        }
        return Ok(Some(primary));
    }
    Ok(None)
}

async fn control_tls(
    args: &HashMap<String, String>,
    builder: TlsBuilder,
    dns: Dns,
    stop: CancellationToken,
) -> std::result::Result<rustls::ServerConfig, Box<dyn std::error::Error>> {
    let get =
        |key: &str, default: &str| args.get(key).cloned().unwrap_or_else(|| default.to_owned());
    if args.get("control-acme").is_some_and(|v| v == "true") {
        let state = PathBuf::from(get("state", "relay-state.json"));
        let path = state
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("control-tls/remote");
        let dns = dns.ok_or("control ACME requires hostname and DNS provider")?;
        let resolver = ab_relay::acme::start(path, get("control-host", ""), dns, stop).await?;
        return Ok(builder.with_cert_resolver(resolver));
    }
    let cert = std::fs::read(get("control-cert", ""))?;
    let key = std::fs::read(get("control-key", ""))?;
    let certs = rustls::pki_types::CertificateDer::pem_slice_iter(&cert)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(&key)?;
    Ok(builder.with_single_cert(certs, key)?)
}

#[allow(clippy::too_many_arguments)]
async fn serve_app(
    path: PathBuf,
    host: String,
    upstream: String,
    dns: Arc<dyn ab_relay::http::DnsProvider>,
    builder: TlsBuilder,
    incoming: tokio::sync::mpsc::Receiver<ab_relay::server::ControlConnection>,
    stop: CancellationToken,
    deadlines: ConnectionDeadlines,
) {
    let certificates = loop {
        if let Ok(certificates) =
            ab_relay::acme::start(path.clone(), host.clone(), dns.clone(), stop.clone()).await
        {
            break certificates;
        }
        eprintln!("sign-in host certificate unavailable; retrying");
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(Duration::from_mins(10)) => {}
        }
    };
    let acceptor = TlsAcceptor::from(Arc::new(builder.with_cert_resolver(certificates)));
    let Ok(router) = ab_relay::app::router(&upstream) else {
        eprintln!("sign-in host proxy unavailable");
        return;
    };
    let make = router.into_make_service_with_connect_info();
    serve_control(None, incoming, make, acceptor, stop, deadlines).await;
}

fn configured_capacity(
    args: &HashMap<String, String>,
) -> std::result::Result<Capacity, Box<dyn std::error::Error>> {
    let defaults = Capacity::default();
    let slots =
        |key: &str, default: usize| -> std::result::Result<usize, Box<dyn std::error::Error>> {
            let value = args.get(key).map_or(Ok(default), |value| value.parse())?;
            if value == 0 {
                return Err(format!("{key} must be positive").into());
            }
            Ok(value)
        };
    Ok(Capacity {
        unrouted: slots("unrouted-slots", defaults.unrouted)?,
        tenant: slots("tenant-slots", defaults.tenant)?,
        control: slots("control-slots", defaults.control)?,
        app: slots("app-slots", defaults.app)?,
        per_source: slots("source-slots", defaults.per_source)?,
    })
}

fn configured_store(
    args: &HashMap<String, String>,
) -> std::result::Result<Store, Box<dyn std::error::Error>> {
    let get =
        |key: &str, default: &str| args.get(key).cloned().unwrap_or_else(|| default.to_owned());
    let limits = Limits {
        connections: get("connections", "32").parse()?,
        bytes_per_second: get("bytes-per-second", "1048576").parse()?,
        monthly_bytes: get("monthly-bytes", "10737418240").parse()?,
    };
    let mut store = Store::open_with_legacy_zones(
        PathBuf::from(get("state", "relay-state.json")),
        &get("zone", ""),
        &get("legacy-zones", "")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        limits,
    )?;
    if let Some(path) = args.get("owner-keys-file").filter(|s| !s.is_empty()) {
        store.require_owner_keys(path.into());
        if args.get("app-host").is_none_or(String::is_empty) {
            store.validate_owner_keys()?;
        }
    }
    Ok(store)
}

async fn cancel_on_signal(quit: CancellationToken) {
    let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    else {
        quit.cancel();
        return;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term.recv() => {},
    }
    quit.cancel();
}

#[cfg(test)]
mod listen_addr_tests {
    use super::listen_addr;

    #[tokio::test]
    async fn bare_port_binds_all_interfaces_like_go() -> std::io::Result<()> {
        assert_eq!(listen_addr(":443"), "[::]:443");
        assert_eq!(listen_addr("127.0.0.1:443"), "127.0.0.1:443");
        let listener = tokio::net::TcpListener::bind(listen_addr(":0")).await?;
        assert!(listener.local_addr()?.ip().is_unspecified());
        Ok(())
    }
}

#[derive(Debug)]
struct MigrationDns {
    primary: Arc<dyn ab_relay::http::DnsProvider>,
    legacy: Arc<dyn ab_relay::http::DnsProvider>,
    suffix: String,
}
impl ab_relay::http::DnsProvider for MigrationDns {
    fn txt<'a>(
        &'a self,
        name: &'a str,
        value: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ab_relay::Result<()>> + Send + 'a>>
    {
        if name
            .strip_prefix("_acme-challenge.")
            .is_some_and(|host| host == self.suffix || host.ends_with(&format!(".{}", self.suffix)))
        {
            self.legacy.txt(name, value)
        } else {
            self.primary.txt(name, value)
        }
    }
}

#[derive(Debug)]
struct ChallengeAlias {
    provider: Arc<dyn ab_relay::http::DnsProvider>,
    source: String,
    target: String,
}
impl ab_relay::http::DnsProvider for ChallengeAlias {
    fn txt<'a>(
        &'a self,
        name: &'a str,
        value: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ab_relay::Result<()>> + Send + 'a>>
    {
        self.provider.txt(
            if name == self.source {
                &self.target
            } else {
                name
            },
            value,
        )
    }
}

type ExtraHosts = Vec<(
    String,
    String,
    tokio::sync::mpsc::Receiver<ab_relay::server::ControlConnection>,
)>;
fn additional_hosts(
    args: &HashMap<String, String>,
    server: &mut Server,
) -> Result<ExtraHosts, Box<dyn std::error::Error>> {
    let mut extra_hosts = Vec::new();
    for entry in args
        .get("extra-hosts")
        .map_or("", String::as_str)
        .split(',')
        .filter(|s| !s.is_empty())
    {
        let (host, upstream) = entry
            .split_once('=')
            .ok_or("extra host needs host=control or host=upstream")?;
        if !ab_relay::hello::valid_host(host)
            || args.get("app-host").is_some_and(|app| app == host)
            || args
                .get("control-host")
                .is_some_and(|control| control == host)
            || extra_hosts.iter().any(|(name, _, _)| name == host)
        {
            return Err("duplicate or invalid extra host".into());
        }
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        server.additional_hosts.push((host.to_owned(), tx));
        extra_hosts.push((host.to_owned(), upstream.to_owned(), rx));
    }
    Ok(extra_hosts)
}

async fn serve_additional_hosts(
    extra_hosts: ExtraHosts,
    state: &str,
    dns: Dns,
    router: axum::Router,
    stop: CancellationToken,
    deadlines: ConnectionDeadlines,
) -> Result<(), Box<dyn std::error::Error>> {
    for (host, upstream, incoming) in extra_hosts {
        let dns = dns.clone().ok_or("extra hosts need DNS")?;
        let path = PathBuf::from(state)
            .parent()
            .ok_or("state needs parent")?
            .join("control-tls")
            .join(&host);
        let certificates = ab_relay::acme::start(path, host, dns, stop.clone()).await?;
        let acceptor = TlsAcceptor::from(Arc::new(tls_builder()?.with_cert_resolver(certificates)));
        let extra_router = if upstream == "control" {
            router.clone()
        } else {
            ab_relay::app::router(&upstream)?
        };
        tokio::spawn(serve_control(
            None,
            incoming,
            extra_router.into_make_service_with_connect_info(),
            acceptor,
            stop.clone(),
            deadlines,
        ));
    }
    Ok(())
}

async fn configured_dns(args: &HashMap<String, String>) -> Result<Dns, Box<dyn std::error::Error>> {
    let mut dns = dns_provider(args).await?;
    if let Some(alias) = args.get("dns-challenge-alias") {
        let (source, target) = alias
            .split_once('=')
            .ok_or("DNS alias needs source=target")?;
        if ![source, target].iter().all(|name| {
            name.strip_prefix("_acme-challenge.")
                .is_some_and(ab_relay::hello::valid_host)
        }) {
            return Err("invalid DNS challenge alias".into());
        }
        dns = Some(Arc::new(ChallengeAlias {
            provider: dns.ok_or("DNS alias needs provider")?,
            source: source.into(),
            target: target.into(),
        }));
    }
    Ok(dns)
}
