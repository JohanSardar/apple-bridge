#![allow(clippy::unwrap_used, clippy::expect_used)]

use ab_relay::{
    http::{Api, NameResolver},
    server::Server,
    store::{Limits, Store},
};
use ab_remote::{
    Gateway, Ingress, certificate::Certificates, dns, relay::Identity, relay::Relay, state,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    sync::watch,
};
use tokio_util::sync::CancellationToken;
use tower::Service;

type Records = Arc<Mutex<HashMap<String, String>>>;

#[derive(Debug)]
struct ServerSide(Records);
impl NameResolver for ServerSide {
    fn cname<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = ab_relay::Result<String>> + Send + 'a>> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or(ab_relay::Error::Denied)
        })
    }
}

fn encode_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap());
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

async fn responder(records: Records) -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buffer = [0u8; 512];
        loop {
            let Ok((n, peer)) = socket.recv_from(&mut buffer).await else {
                return;
            };
            let query = &buffer[..n];
            let mut at = 12;
            let mut labels = Vec::new();
            while at < n && query[at] != 0 {
                let length = usize::from(query[at]);
                labels.push(String::from_utf8_lossy(&query[at + 1..at + 1 + length]).to_string());
                at += 1 + length;
            }
            let question_end = at + 5;
            let name = labels.join(".");
            let kind = u16::from_be_bytes([query[at + 1], query[at + 2]]);
            let target = records.lock().unwrap().get(&name).cloned();
            let mut reply = vec![query[0], query[1], 0x84, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
            reply.extend_from_slice(&query[12..question_end]);
            match target {
                Some(target) if kind == 5 => {
                    reply[3] = 0;
                    reply[7] = 1;
                    let rdata = encode_name(&target);
                    reply.extend_from_slice(&[0xC0, 0x0C, 0, 5, 0, 1, 0, 0, 0, 1]);
                    reply.extend_from_slice(&u16::try_from(rdata.len()).unwrap().to_be_bytes());
                    reply.extend_from_slice(&rdata);
                }
                _ => reply[3] = 3,
            }
            let _ = socket.send_to(&reply, peer).await;
        }
    });
    address
}

struct Fixture {
    endpoint: String,
    public: SocketAddr,
    server: Arc<Server>,
    records: Records,
    root: CertificateDer<'static>,
    stop: CancellationToken,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(
        dir.path().join("relay.json"),
        "relay.test",
        Limits {
            connections: 8,
            bytes_per_second: 1 << 24,
            monthly_bytes: 1 << 34,
        },
    )
    .unwrap();
    let server = Arc::new(Server::new(store, Duration::from_secs(5)));
    let records: Records = Arc::default();
    let mut api = Api::new(server.clone(), String::new()).unwrap();
    api.resolver = Arc::new(ServerSide(records.clone()));
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let root = certified.cert.der().clone();
    let key = PrivateKeyDer::try_from(certified.signing_key.serialize_der()).unwrap();
    let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![root.clone()], key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let control = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://localhost:{}", control.local_addr().unwrap().port());
    let mut make = Arc::new(api)
        .router()
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move {
        loop {
            let Ok((socket, peer)) = control.accept().await else {
                return;
            };
            let service = make.call(peer).await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(tls) = acceptor.accept(socket).await {
                    let _ = Builder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(
                            TokioIo::new(tls),
                            TowerToHyperService::new(service),
                        )
                        .await;
                }
            });
        }
    });
    let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let public_address = public.local_addr().unwrap();
    let stop = CancellationToken::new();
    tokio::spawn(server.clone().serve(public, stop.clone()));
    Fixture {
        endpoint,
        public: public_address,
        server,
        records,
        root,
        stop,
        _dir: dir,
    }
}

fn client(root: &CertificateDer<'static>) -> (reqwest::Client, Arc<rustls::ClientConfig>) {
    let http = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_der(root).unwrap())
        .build()
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root.clone()).unwrap();
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    (http, Arc::new(tls))
}

async fn origin() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                let _ = socket.read(&mut buffer).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 12\r\nconnection: close\r\n\r\nalias-origin",
                    )
                    .await;
                let _ = socket.shutdown().await;
            });
        }
    });
    address
}

fn seed_certificate(home: &std::path::Path, domain: &str) -> CertificateDer<'static> {
    let certified = rcgen::generate_simple_self_signed(vec![domain.into()]).unwrap();
    let remote = home.join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("relay-cert.pem"), certified.cert.pem()).unwrap();
    std::fs::write(
        remote.join("relay-key.pem"),
        certified.signing_key.serialize_pem(),
    )
    .unwrap();
    certified.cert.der().clone()
}

async fn wait_for<F: FnMut() -> bool>(mut condition: F) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("condition not reached");
}

async fn visit(public: SocketAddr, site: CertificateDer<'static>, domain: &str) -> String {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(site).unwrap();
    let visitor = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(visitor));
    let socket = tokio::net::TcpStream::connect(public).await.unwrap();
    let mut secure = connector
        .connect(
            rustls::pki_types::ServerName::try_from(domain.to_owned()).unwrap(),
            socket,
        )
        .await
        .unwrap();
    secure
        .write_all(format!("GET / HTTP/1.1\r\nhost: {domain}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut body = Vec::new();
    let _ = secure.read_to_end(&mut body).await;
    String::from_utf8_lossy(&body).into_owned()
}

async fn device_domain(fixture: &Fixture, host: &str) -> String {
    fixture
        .server
        .store
        .lock()
        .await
        .devices
        .get(host)
        .unwrap()
        .domain
        .clone()
}

#[tokio::test]
async fn custom_domain_waits_for_records_registers_and_routes_by_sni() {
    let fixture = fixture().await;
    let home = tempfile::tempdir().unwrap();
    let domain = "mac.example.test";
    let site = seed_certificate(home.path(), domain);
    let dns_records: Records = Arc::default();
    let dns_address = responder(dns_records.clone()).await;
    let (http, tls) = client(&fixture.root);
    let relay = Relay {
        home: home.path().to_owned(),
        endpoint: fixture.endpoint.clone(),
        zone: "relay.test".into(),
        client: http,
        certificates: Arc::new(Certificates::new(
            home.path().to_owned(),
            "https://acme.invalid/directory".into(),
        )),
        tls,
        domain: domain.into(),
        resolver: Some(Arc::new(dns::Hickory::server(dns_address).unwrap())),
        domain_poll: Duration::from_millis(100),
    };
    let upstream = origin().await;
    let (statuses, observed) = watch::channel(std::collections::BTreeMap::new());
    let gateway = Gateway::new(upstream).with_status(statuses);
    let (stop, stopped) = watch::channel(false);
    let run = tokio::spawn(async move { relay.run(gateway, stopped).await });
    let observed_progress = observed.clone();
    wait_for(|| {
        observed_progress
            .borrow()
            .get("relay")
            .is_some_and(|entry| entry.progress.is_some())
    })
    .await;
    let identity: Identity = state::read(home.path(), "relay").unwrap();
    let host = identity.credential.hostname.clone();
    let expected = format!(
        "Missing DNS records: {domain} CNAME {host}.; _acme-challenge.{domain} CNAME _acme-challenge.{host}."
    );
    let progress = observed
        .borrow()
        .get("relay")
        .and_then(|entry| entry.progress.clone())
        .unwrap();
    assert_eq!(progress.state, "pending");
    assert_eq!(progress.message, expected);
    assert_eq!(device_domain(&fixture, &host).await, "");
    for records in [&dns_records, &fixture.records] {
        let mut records = records.lock().unwrap();
        records.insert(domain.into(), host.clone());
        records.insert(
            format!("_acme-challenge.{domain}"),
            format!("_acme-challenge.{host}"),
        );
    }
    let ready = observed.clone();
    wait_for(|| {
        ready
            .borrow()
            .get("relay")
            .is_some_and(|entry| entry.active)
    })
    .await;
    let entry = observed.borrow().get("relay").cloned().unwrap();
    assert_eq!(entry.host, domain);
    assert!(entry.progress.is_none());
    assert_eq!(device_domain(&fixture, &host).await, domain);
    assert!(
        visit(fixture.public, site, domain)
            .await
            .ends_with("alias-origin")
    );
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fixture.stop.cancel();
}

#[tokio::test]
async fn custom_domain_owned_by_another_device_is_refused() {
    let fixture = fixture().await;
    let domain = "mac.example.test";
    let (http, tls) = client(&fixture.root);
    let other = tempfile::tempdir().unwrap();
    let first = ab_remote::relay::enroll(other.path(), &fixture.endpoint, "relay.test", &http)
        .await
        .unwrap();
    let dns_records: Records = Arc::default();
    let dns_address = responder(dns_records.clone()).await;
    let home = tempfile::tempdir().unwrap();
    let second = Relay {
        home: home.path().to_owned(),
        endpoint: fixture.endpoint.clone(),
        zone: "relay.test".into(),
        client: http.clone(),
        certificates: Arc::new(Certificates::new(
            home.path().to_owned(),
            "https://acme.invalid/directory".into(),
        )),
        tls,
        domain: domain.into(),
        resolver: Some(Arc::new(dns::Hickory::server(dns_address).unwrap())),
        domain_poll: Duration::from_millis(100),
    };
    let claimed = ab_remote::relay::enroll(home.path(), &fixture.endpoint, "relay.test", &http)
        .await
        .unwrap();
    for records in [&dns_records, &fixture.records] {
        let mut records = records.lock().unwrap();
        records.insert(domain.into(), claimed.credential.hostname.clone());
        records.insert(
            format!("_acme-challenge.{domain}"),
            format!("_acme-challenge.{}", claimed.credential.hostname),
        );
    }
    fixture
        .server
        .store
        .lock()
        .await
        .set_domain(&first.credential.hostname, domain)
        .unwrap();
    let (_stop, stopped) = watch::channel(false);
    let outcome = second
        .run(Gateway::new("127.0.0.1:9".parse().unwrap()), stopped)
        .await;
    assert!(outcome.is_err());
    fixture.stop.cancel();
}
