use crate::Error;
use ab_replication::{gateway::StateAdapter, server::Server};
use axum::body::Body;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use serde::Deserialize;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::watch;

#[derive(Debug, Deserialize)]
struct Config {
    listen: String,
    cert: String,
    key: String,
}
pub async fn serve(
    home: &Path,
    feed: Arc<Server<StateAdapter>>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), Error> {
    let raw = match std::fs::read(home.join("lab-relay.json")) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = stop.changed().await;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let config: Config = serde_json::from_slice(&raw)?;
    let chain = CertificateDer::pem_file_iter(&config.cert)
        .map_err(|_| Error::Invalid)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::Invalid)?;
    let key = PrivateKeyDer::from_pem_file(&config.key).map_err(|_| Error::Invalid)?;
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| Error::Invalid)?
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .map_err(|_| Error::Invalid)?;
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    let run = home.join("run");
    std::fs::create_dir_all(&run)?;
    std::fs::write(
        run.join("lab-relay-addr"),
        listener.local_addr()?.to_string(),
    )?;
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept(), if connections.len() < 64 => {
                let (socket, _) = accepted?;
                let acceptor = acceptor.clone();
                let feed = feed.clone();
                connections.spawn(async move {
                    let Ok(Ok(stream)) = tokio::time::timeout(Duration::from_secs(5), acceptor.accept(socket)).await else {
                        return;
                    };
                    let binding = stream
                        .get_ref()
                        .1
                        .export_keying_material([0; 32], ab_replication::relay::LABEL, Some(&[]))
                        .ok();
                    let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                        let feed = feed.clone();
                        async move {
                            let request = request.map(Body::new);
                            if request.uri().path() != "/v1/feed" {
                                return Ok::<_, std::convert::Infallible>(
                                    ab_frontdoor::problem(ab_operations::Code::Unauthorized.into()),
                                );
                            }
                            Ok(feed.relay_http(request, binding).map(Body::new))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .with_upgrades()
                        .await;
                });
            }
        }
    }
    connections.abort_all();
    Ok(())
}
