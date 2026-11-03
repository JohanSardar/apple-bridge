#![forbid(unsafe_code)]

use rustls::pki_types::pem::PemObject;

use ab_relay::{
    http::Api,
    server::Server,
    store::{Limits, Store},
};
use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use tower::Service;

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
    let dns: Option<Arc<dyn ab_relay::http::DnsProvider>> =
        if let Some(project) = args.get("dns-project").filter(|s| !s.is_empty()) {
            Some(Arc::new(ab_relay::cloud_dns::CloudDns::new(
                project,
                &get("dns-zone", ""),
                &get("zone", ""),
            )?))
        } else {
            None
        };
    let builder = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth();
    let config = if args.get("control-acme").is_some_and(|v| v == "true") {
        let state = PathBuf::from(get("state", "relay-state.json"));
        let path = state
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("control-tls/remote");
        builder.with_cert_resolver(
            ab_relay::acme::start(
                path,
                get("control-host", ""),
                dns.clone()
                    .ok_or("control ACME requires hostname and DNS provider")?,
                stop.clone(),
            )
            .await?,
        )
    } else {
        let cert = std::fs::read(get("control-cert", ""))?;
        let key = std::fs::read(get("control-key", ""))?;
        let certs = rustls::pki_types::CertificateDer::pem_slice_iter(&cert)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(&key)?;
        builder.with_single_cert(certs, key)?
    };
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let control = if get("control-host", "").is_empty() {
        Some(TcpListener::bind(get("control-addr", "127.0.0.1:8443")).await?)
    } else {
        None
    };
    let (control_tx, control_rx) = tokio::sync::mpsc::channel(256);
    let mut server = Server::new(store, idle);
    if let Some(host) = args.get("control-host").filter(|s| !s.is_empty()) {
        server.control = Some((host.clone(), control_tx));
    }
    let server = Arc::new(server);
    let mut api = Api::new(
        server.clone(),
        std::env::var("APPLE_BRIDGE_RELAY_ADMIN").unwrap_or_default(),
    )?;
    api.dns = dns;
    if let Some(path) = args.get("apns-key").filter(|p| !p.is_empty()) {
        api.push.provider = Some(Arc::new(ab_relay::push::Apns::load(
            std::path::Path::new(path),
            get("apns-sandbox", "true") == "true",
        )?));
    }
    let api = Arc::new(api);
    let router = api.clone().router();
    let make = router.into_make_service_with_connect_info();
    let public = TcpListener::bind(get("addr", "0.0.0.0:443")).await?;
    let quit = stop.clone();
    tokio::spawn(async move {
        let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        else {
            quit.cancel();
            return;
        };
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        quit.cancel();
    });
    let stop_api = stop.clone();
    tokio::spawn(ab_relay::ct::monitor(api, stop.clone()));
    let api_task = serve_control(control, control_rx, make, acceptor, stop_api);
    tokio::select! { result = server.serve(public,stop.clone()) => result?, result = api_task => result? }
    stop.cancel();
    Ok(())
}

async fn serve_control(
    control: Option<TcpListener>,
    mut incoming: tokio::sync::mpsc::Receiver<ab_relay::server::ControlConnection>,
    mut make: IntoMakeServiceWithConnectInfo<axum::Router, SocketAddr>,
    acceptor: TlsAcceptor,
    stop_api: CancellationToken,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut workers = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            () = stop_api.cancelled() => break,
            Some(_) = workers.join_next(), if !workers.is_empty() => {},
            accepted = async {
                tokio::select! {
                    result = async { match &control { Some(listener)=>listener.accept().await, None=>std::future::pending().await } } => result.map(|(socket,_)| ab_relay::server::ControlConnection { socket,prefix:Vec::new(),completed:None }),
                    result = incoming.recv() => result.ok_or_else(|| std::io::Error::other("control listener closed")),
                }
            } => {
                let ab_relay::server::ControlConnection { socket,prefix,completed } = accepted?;
                let peer = socket.peer_addr()?;
                let socket = ab_relay::replay::Replay::new(socket,prefix);
                let service = make.call(peer).await?; let acceptor = acceptor.clone();
                workers.spawn(async move {
                    let _completed = completed;
                    if let Ok(Ok(tls)) = tokio::time::timeout(Duration::from_secs(5),acceptor.accept(socket)).await {
                        let mut builder=Builder::new(TokioExecutor::new());
                        builder.http1().timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(Duration::from_secs(5)).max_buf_size(12288);
                        let _ = builder.serve_connection_with_upgrades(TokioIo::new(tls),TowerToHyperService::new(service)).await;
                    }
                });
            }
        }
    }
    workers.abort_all();
    while workers.join_next().await.is_some() {}
    Ok::<_, Box<dyn std::error::Error>>(())
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
    let mut store = Store::open(
        PathBuf::from(get("state", "relay-state.json")),
        &get("zone", ""),
        limits,
    )?;
    if let Some(path) = args.get("owner-keys-file").filter(|s| !s.is_empty()) {
        store.require_owner_keys(path.into());
        store.validate_owner_keys()?;
    }
    Ok(store)
}
