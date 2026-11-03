use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio_rustls::rustls;
#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("client") {
        let authority = ab_trust::Authority::open(args.get(2).ok_or("home")?)?;
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from_pem_file(args.get(5).ok_or("CA")?)?)?;
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(args.get(4).ok_or("name")?.clone())?;
        let tcp = tokio::net::TcpStream::connect(args.get(3).ok_or("address")?).await?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await?;
        let binding = stream.get_ref().1.export_keying_material(
            [0; 32],
            ab_replication::relay::LABEL,
            Some(&[]),
        )?;
        let (mut ws, _) = tokio_tungstenite::client_async(
            format!("wss://{}/v1/feed", args.get(4).ok_or("name")?),
            stream,
        )
        .await?;
        let result = ab_replication::relay::client(
            &authority,
            &mut ws,
            &binding,
            args.get(6).ok_or("pin")?,
            None,
        )
        .await;
        println!(
            "{}",
            if result.is_ok() {
                "authenticated"
            } else {
                "rejected"
            }
        );
        return Ok(());
    }
    let home = args.get(1).ok_or("home")?;
    let cert = CertificateDer::from_pem_file(args.get(2).ok_or("certificate")?)?;
    let key = PrivateKeyDer::from_pem_file(args.get(3).ok_or("key")?)?;
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_no_client_auth()
    .with_single_cert(vec![cert], key)?;
    let authority = Arc::new(ab_trust::Authority::open(home)?);
    let mut state = ab_state::State::<ab_state::Gateway>::open(format!("{home}/relay2-feed"))?;
    state.commit_capture(&serde_json::from_value(serde_json::json!({"source":"reminders","generation":"r3","checkpoint":"1","observations":[{"kind":"upsert","collection":"reminders.reminder","native_key":"synthetic","observed_at":"2026-01-02T00:00:00Z","data":{"title":"synthetic-splice-feed-marker"}}]}))?)?;
    let server = Arc::new(ab_replication::server::Server {
        state: Arc::new(tokio::sync::Mutex::new(
            ab_replication::gateway::StateAdapter(Arc::new(Mutex::new(state))),
        )),
        authority,
        blobs: None,
        uploads: None,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    println!("{}", listener.local_addr()?);
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    loop {
        let (socket, _) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let server = server.clone();
        tokio::spawn(async move {
            let Ok(mut stream) = acceptor.accept(socket).await else {
                return;
            };
            let Ok(binding) = stream.get_ref().1.export_keying_material(
                [0; 32],
                ab_replication::relay::LABEL,
                Some(&[]),
            ) else {
                let _ = stream.shutdown().await;
                return;
            };
            let service = hyper::service::service_fn(move |request| {
                let server = server.clone();
                async move {
                    Ok::<_, std::convert::Infallible>(server.relay_http(request, Some(binding)))
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}
