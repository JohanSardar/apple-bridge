use ab_remote::{Gateway, Ingress, certificate::Certificates, relay::Relay, tailscale::Tailscale};
use rustls::pki_types::{CertificateDer, pem::PemObject};
use std::{path::PathBuf, sync::Arc};

type Resolvers = Result<Option<Arc<dyn ab_remote::dns::Resolver>>, Box<dyn std::error::Error>>;

fn resolver(address: Option<&str>) -> Resolvers {
    Ok(match address {
        Some(address) => Some(Arc::new(ab_remote::dns::Hickory::server(address.parse()?)?)),
        None => None,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| args.windows(2).find(|v| v[0] == name).map(|v| v[1].clone());
    let home = PathBuf::from(option("--home").ok_or("--home required")?);
    let via = option("--via").unwrap_or_else(|| "relay".into());
    if args.first().is_some_and(|v| v == "remote") {
        let action = args.get(1).ok_or("action required")?;
        println!("{}", ab_remote::state::command(&home, action, &via)?);
        return Ok(());
    }
    if args.first().is_some_and(|v| v == "enroll") {
        let ca = std::fs::read(option("--ca").ok_or("--ca required")?)?;
        let client = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&ca)?)
            .build()?;
        let id = ab_remote::relay::enroll(
            &home,
            &option("--endpoint").ok_or("--endpoint required")?,
            "fixture.test",
            &client,
        )
        .await?;
        println!("{}", serde_json::to_string(&id.credential)?);
        return Ok(());
    }
    if args.first().is_some_and(|v| v == "renew") {
        let id = ab_remote::state::read(&home, "relay")?;
        let ca = std::fs::read(option("--ca").ok_or("--ca required")?)?;
        let client = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&ca)?)
            .build()?;
        {
            let mut certificates =
                Certificates::new(home, option("--directory").ok_or("--directory required")?);
            certificates.root = option("--ca").map(PathBuf::from);
            certificates
        }
        .ensure(
            &id,
            &option("--domain").unwrap_or_else(|| id.credential.hostname.clone()),
            &client,
        )
        .await?;
        return Ok(());
    }
    let gateway = Gateway::new(option("--gateway").ok_or("--gateway required")?.parse()?);
    let mut adapters: Vec<Box<dyn Ingress>> = Vec::new();
    if via == "relay" || via == "both" {
        let mut roots = rustls::RootCertStore::empty();
        let ca = std::fs::read(option("--ca").ok_or("--ca required for isolated driver")?)?;
        for cert in CertificateDer::pem_slice_iter(&ca) {
            roots.add(cert?)?;
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
        let client = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&ca)?)
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        adapters.push(Box::new(Relay {
            home: home.clone(),
            endpoint: option("--endpoint").ok_or("--endpoint required")?,
            zone: "fixture.test".into(),
            client,
            certificates: Arc::new(Certificates::new(
                home.clone(),
                option("--directory").unwrap_or_default(),
            )),
            tls: Arc::new(tls),
            domain: option("--domain").unwrap_or_default(),
            resolver: resolver(option("--dns-server").as_deref())?,
            domain_poll: std::time::Duration::from_millis(200),
            certificate_monitor: false,
        }));
    }
    if via == "tailscale-app" || via == "both" {
        let localapi = option("--localapi").ok_or("--localapi required")?;
        let port = localapi
            .rsplit(':')
            .next()
            .and_then(|port| port.trim_end_matches('/').parse::<u16>().ok())
            .ok_or("--localapi must end with a port")?;
        adapters.push(Box::new(Tailscale::tcp(home, port, "fixture".into())));
    }
    let (stop, receiver) = tokio::sync::watch::channel(false);
    let run = ab_remote::run_all(&adapters, gateway, receiver.clone());
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => result?,
        result = tokio::signal::ctrl_c() => {
            result?;
            stop.send(true)?;
            run.await?;
        }
    }
    Ok(())
}
