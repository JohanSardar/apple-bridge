use ab_remote::{Gateway, Ingress, certificate::Certificates, relay::Relay};
use rustls::pki_types::pem::PemObject;
use std::{path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: remote_client home endpoint root.pem gateway-address".into());
    }
    let home = PathBuf::from(&args[0]);
    let pem = std::fs::read(&args[2])?;
    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&pem)?)
        .build()?;
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls::pki_types::CertificateDer::pem_slice_iter(&pem) {
        roots.add(cert?)?;
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let relay = Relay {
        home: home.clone(),
        endpoint: args[1].clone(),
        zone: "relay.test".into(),
        client,
        certificates: Arc::new(Certificates::new(
            home,
            "https://acme.invalid/directory".into(),
        )),
        tls: Arc::new(config),
    };
    let (stop, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = stop.send(true);
    });
    relay.run(Gateway::new(args[3].parse()?), rx).await?;
    Ok(())
}
