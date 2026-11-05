#![allow(clippy::unwrap_used, clippy::expect_used)]

use ab_relay::{
    http::Api,
    server::Server,
    store::{Limits, Store},
};
use ab_remote::{
    Gateway, Ingress, certificate::Certificates, relay::Identity, relay::Relay, state,
};
use futures_util::{SinkExt, StreamExt};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tokio_util::sync::CancellationToken;
use tower::Service;

const ZONE: &str = "relay.test";
const SYN: u8 = 0;
const FIN: u8 = 1;
const DATA: u8 = 2;
const KEEPALIVE: u8 = 3;
const WINDOW_UPDATE: u8 = 4;
const WINDOW: u32 = 262_144;

struct Fixture {
    endpoint: String,
    public: SocketAddr,
    root: CertificateDer<'static>,
    stop: CancellationToken,
    _dir: tempfile::TempDir,
}

fn limits(connections: i64) -> Limits {
    Limits {
        connections,
        bytes_per_second: 1 << 30,
        monthly_bytes: 1 << 40,
    }
}

async fn fixture(connections: i64) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("relay.json"), ZONE, limits(connections)).unwrap();
    let server = Arc::new(Server::new(store, Duration::from_secs(120)));
    let api = Api::new(server.clone(), String::new()).unwrap();
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

#[derive(Clone, Default)]
struct Origin {
    open: Arc<AtomicUsize>,
    accepted: Arc<AtomicUsize>,
}

async fn counting_origin(response: Vec<u8>) -> (SocketAddr, Origin) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let origin = Origin::default();
    let counters = origin.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            counters.open.fetch_add(1, Ordering::SeqCst);
            counters.accepted.fetch_add(1, Ordering::SeqCst);
            let counters = counters.clone();
            let response = response.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                if socket.read(&mut buffer).await.is_ok_and(|n| n > 0) && !response.is_empty() {
                    let _ = socket.write_all(&response).await;
                    let _ = socket.shutdown().await;
                }
                while socket.read(&mut buffer).await.is_ok_and(|n| n > 0) {}
                counters.open.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    (address, origin)
}

struct Tunnel {
    host: String,
    site: CertificateDer<'static>,
    stop: watch::Sender<bool>,
    run: tokio::task::JoinHandle<ab_remote::Result<()>>,
    _home: tempfile::TempDir,
}

async fn real_tunnel(fixture: &Fixture, upstream: SocketAddr) -> Tunnel {
    let home = tempfile::tempdir().unwrap();
    let (http, tls) = client(&fixture.root);
    let identity = ab_remote::relay::enroll(home.path(), &fixture.endpoint, ZONE, &http)
        .await
        .unwrap();
    let host = identity.credential.hostname.clone();
    let certified = rcgen::generate_simple_self_signed(vec![host.clone()]).unwrap();
    let remote = home.path().join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("relay-cert.pem"), certified.cert.pem()).unwrap();
    std::fs::write(
        remote.join("relay-key.pem"),
        certified.signing_key.serialize_pem(),
    )
    .unwrap();
    let site = certified.cert.der().clone();
    let relay = Relay {
        home: home.path().to_owned(),
        endpoint: fixture.endpoint.clone(),
        zone: ZONE.into(),
        client: http,
        certificates: Arc::new(Certificates::new(
            home.path().to_owned(),
            "https://acme.invalid/directory".into(),
        )),
        tls,
        domain: String::new(),
        resolver: None,
        domain_poll: Duration::from_millis(100),
        certificate_monitor: false,
    };
    let (statuses, observed) = watch::channel(std::collections::BTreeMap::new());
    let gateway = Gateway::new(upstream).with_status(statuses);
    let (stop, stopped) = watch::channel(false);
    let run = tokio::spawn(async move { relay.run(gateway, stopped).await });
    for _ in 0..200 {
        if observed.borrow().get("relay").is_some_and(|e| e.active) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        observed.borrow().get("relay").is_some_and(|e| e.active),
        "the tunnel never came up"
    );
    Tunnel {
        host,
        site,
        stop,
        run,
        _home: home,
    }
}

fn visitor_tls(site: CertificateDer<'static>) -> tokio_rustls::TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(site).unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

async fn wait_until(what: &str, deadline: Duration, condition: impl Fn() -> bool) {
    let end = tokio::time::Instant::now() + deadline;
    while tokio::time::Instant::now() < end {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("{what}");
}

#[tokio::test]
async fn a_visitor_that_drops_without_close_notify_releases_the_mac_connection() {
    let fixture = fixture(4).await;
    let (upstream, origin) = counting_origin(Vec::new()).await;
    let tunnel = real_tunnel(&fixture, upstream).await;
    let connector = visitor_tls(tunnel.site.clone());
    for visit in 1..=8 {
        let socket = TcpStream::connect(fixture.public).await.unwrap();
        let mut secure = connector
            .connect(ServerName::try_from(tunnel.host.clone()).unwrap(), socket)
            .await
            .unwrap();
        secure.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        secure.flush().await.unwrap();
        wait_until(
            "the visit never reached the Mac's gateway",
            Duration::from_secs(5),
            || origin.accepted.load(Ordering::SeqCst) == visit,
        )
        .await;
        let (raw, _) = secure.into_inner();
        raw.set_zero_linger().unwrap();
        drop(raw);
        wait_until(
            "an abruptly closed visitor left the Mac's gateway connection open",
            Duration::from_secs(8),
            || origin.open.load(Ordering::SeqCst) == 0,
        )
        .await;
    }
    tunnel.stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), tunnel.run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    fixture.stop.cancel();
}

#[tokio::test]
async fn a_large_response_reaches_the_visitor_before_the_stream_closes() {
    let fixture = fixture(4).await;
    let body = vec![b'x'; 3 << 20];
    let mut response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    let (upstream, _origin) = counting_origin(response.clone()).await;
    let tunnel = real_tunnel(&fixture, upstream).await;
    let socket = TcpStream::connect(fixture.public).await.unwrap();
    let mut secure = visitor_tls(tunnel.site.clone())
        .connect(ServerName::try_from(tunnel.host.clone()).unwrap(), socket)
        .await
        .unwrap();
    secure.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    secure.flush().await.unwrap();
    let mut received = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(30), secure.read_to_end(&mut received)).await;
    assert_eq!(received.len(), response.len());
    tunnel.stop.send(true).unwrap();
    fixture.stop.cancel();
}

struct FakeMac {
    host: String,
    outbound: mpsc::Sender<Vec<u8>>,
    frames: mpsc::UnboundedReceiver<Frame>,
    closed: CancellationToken,
    _home: tempfile::TempDir,
}

#[derive(Debug)]
struct Frame {
    command: u8,
    id: u32,
    data: Vec<u8>,
}

fn frame(command: u8, id: u32, data: &[u8]) -> Vec<u8> {
    let mut out = vec![2, command];
    out.extend_from_slice(&u16::try_from(data.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(data);
    out
}

async fn fake_mac(fixture: &Fixture, features: Option<&str>) -> FakeMac {
    let home = tempfile::tempdir().unwrap();
    let (http, tls) = client(&fixture.root);
    let identity: Identity = ab_remote::relay::enroll(home.path(), &fixture.endpoint, ZONE, &http)
        .await
        .unwrap();
    let _: Identity = state::read(home.path(), "relay").unwrap();
    let proof = identity.proof(&http, "tunnel").await.unwrap();
    let mut request = format!(
        "{}/tunnel",
        fixture.endpoint.replacen("https://", "wss://", 1)
    )
    .into_client_request()
    .unwrap();
    request.headers_mut().insert(
        "X-Bridge-Proof",
        proof.encode_header().unwrap().parse().unwrap(),
    );
    if let Some(features) = features {
        request
            .headers_mut()
            .insert("X-Bridge-Tunnel-Features", features.parse().unwrap());
    }
    let (ws, _) = tokio_tungstenite::connect_async_tls_with_config(
        request,
        None,
        false,
        Some(tokio_tungstenite::Connector::Rustls(tls)),
    )
    .await
    .unwrap();
    let (mut sink, mut source) = ws.split();
    let (outbound, mut pending) = mpsc::channel::<Vec<u8>>(1024);
    let (delivered, frames) = mpsc::unbounded_channel();
    let closed = CancellationToken::new();
    let writer_closed = closed.clone();
    tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(Duration::from_millis(500));
        loop {
            let bytes = tokio::select! {
                _ = heartbeat.tick() => frame(KEEPALIVE, 0, &[]),
                bytes = pending.recv() => match bytes { Some(bytes) => bytes, None => break },
            };
            if sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
        writer_closed.cancel();
    });
    let reader_closed = closed.clone();
    tokio::spawn(async move {
        let mut buffered = Vec::new();
        while let Some(Ok(message)) = source.next().await {
            let Message::Binary(bytes) = message else {
                continue;
            };
            buffered.extend_from_slice(&bytes);
            while buffered.len() >= 8 {
                let n = usize::from(u16::from_le_bytes([buffered[2], buffered[3]]));
                if buffered.len() < 8 + n {
                    break;
                }
                let command = buffered[1];
                let id = u32::from_le_bytes(buffered[4..8].try_into().unwrap());
                let data = buffered[8..8 + n].to_vec();
                buffered.drain(..8 + n);
                if command != KEEPALIVE {
                    let _ = delivered.send(Frame { command, id, data });
                }
            }
        }
        reader_closed.cancel();
    });
    FakeMac {
        host: identity.credential.hostname,
        outbound,
        frames,
        closed,
        _home: home,
    }
}

fn client_hello(host: &str) -> Vec<u8> {
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(rustls::RootCertStore::empty())
    .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned()).unwrap();
    let mut connection = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
    let mut hello = Vec::new();
    connection.write_tls(&mut hello).unwrap();
    hello
}

impl FakeMac {
    async fn next_syn(&mut self, within: Duration) -> Frame {
        tokio::time::timeout(within, async {
            loop {
                let frame = self.frames.recv().await.expect("tunnel closed");
                if frame.command == SYN {
                    return frame;
                }
            }
        })
        .await
        .expect("the relay never opened the stream")
    }
}

async fn visitor(public: SocketAddr, host: &str) -> TcpStream {
    let mut socket = TcpStream::connect(public).await.unwrap();
    socket.write_all(&client_hello(host)).await.unwrap();
    socket
}

async fn stalled_visitor(public: SocketAddr, host: &str) -> TcpStream {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut stream = socket.connect(public).await.unwrap();
    stream.write_all(&client_hello(host)).await.unwrap();
    stream
}

#[tokio::test]
async fn one_stalled_visitor_does_not_block_other_streams_or_keepalives() {
    let fixture = fixture(8).await;
    let mut mac = fake_mac(&fixture, None).await;
    let stalled = stalled_visitor(fixture.public, &mac.host).await;
    let opened = mac.next_syn(Duration::from_secs(5)).await;

    let mut sent = 0u32;
    let mut consumed = 0u32;
    let chunk = [b'x'; 1024];
    let mut idle = 0;
    while idle < 5 {
        while sent.wrapping_sub(consumed) + 1024 <= WINDOW {
            mac.outbound
                .send(frame(DATA, opened.id, &chunk))
                .await
                .unwrap();
            sent = sent.wrapping_add(1024);
        }
        match tokio::time::timeout(Duration::from_millis(300), mac.frames.recv()).await {
            Ok(Some(update)) if update.command == WINDOW_UPDATE && update.id == opened.id => {
                consumed = u32::from_le_bytes(update.data[..4].try_into().unwrap());
                idle = 0;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("the tunnel closed while one visitor stalled"),
            Err(_) => idle += 1,
        }
    }

    let mut second = visitor(fixture.public, &mac.host).await;
    let other = mac.next_syn(Duration::from_secs(2)).await;
    mac.outbound
        .send(frame(DATA, other.id, b"still flowing"))
        .await
        .unwrap();
    let mut delivered = [0u8; 13];
    tokio::time::timeout(Duration::from_secs(2), second.read_exact(&mut delivered))
        .await
        .expect("an unrelated stream stalled behind the blocked one")
        .unwrap();
    assert_eq!(&delivered, b"still flowing");
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        !mac.closed.is_cancelled(),
        "the relay dropped the tunnel because one stream stalled"
    );
    drop(stalled);
    fixture.stop.cancel();
}

#[tokio::test]
async fn a_mac_that_overruns_the_window_loses_only_that_stream() {
    let fixture = fixture(8).await;
    let mut mac = fake_mac(&fixture, None).await;
    let _stalled = stalled_visitor(fixture.public, &mac.host).await;
    let opened = mac.next_syn(Duration::from_secs(5)).await;
    let chunk = [b'x'; 16384];
    let mut reset = false;
    for _ in 0..4096 {
        mac.outbound
            .send(frame(DATA, opened.id, &chunk))
            .await
            .unwrap();
        while let Ok(frame) = mac.frames.try_recv() {
            if frame.command == FIN && frame.id == opened.id {
                reset = true;
            }
        }
        if reset {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(reset, "the overrun stream was never reset");
    let mut second = visitor(fixture.public, &mac.host).await;
    let other = mac.next_syn(Duration::from_secs(2)).await;
    mac.outbound
        .send(frame(DATA, other.id, b"ok"))
        .await
        .unwrap();
    let mut delivered = [0u8; 2];
    tokio::time::timeout(Duration::from_secs(2), second.read_exact(&mut delivered))
        .await
        .expect("the tunnel did not survive the overrun")
        .unwrap();
    fixture.stop.cancel();
}

#[tokio::test]
async fn a_mac_from_before_this_release_gets_only_the_frames_it_understands() {
    let fixture = fixture(8).await;
    let mut mac = fake_mac(&fixture, None).await;
    let _visitor = visitor(fixture.public, &mac.host).await;
    let opened = mac.next_syn(Duration::from_secs(5)).await;
    assert!(
        opened.data.is_empty(),
        "an old Mac tears the whole tunnel down on a stream-open frame with a payload"
    );
    let mut seen = HashMap::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(Duration::from_millis(300), mac.frames.recv()).await
    {
        *seen.entry(frame.command).or_insert(0) += 1;
    }
    assert!(
        seen.keys()
            .all(|command| matches!(*command, SYN | FIN | DATA | WINDOW_UPDATE)),
        "unknown frames reached an old Mac: {seen:?}"
    );
    assert!(seen.get(&DATA).copied().unwrap_or(0) > 0);
    assert!(!mac.closed.is_cancelled());
    fixture.stop.cancel();
}

#[tokio::test]
async fn a_mac_that_asks_for_it_learns_the_address_the_relay_accepted() {
    let fixture = fixture(8).await;
    let mut mac = fake_mac(&fixture, Some("source")).await;
    let _visitor = visitor(fixture.public, &mac.host).await;
    let opened = mac.next_syn(Duration::from_secs(5)).await;
    assert_eq!(
        opened.data,
        vec![127, 0, 0, 1],
        "the source is the accepted peer, not anything a client can send"
    );
    fixture.stop.cancel();
}

fn stalling_origin() -> (SocketAddr, Origin) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = socket.local_addr().unwrap();
    let listener = socket.listen(64).unwrap();
    let origin = Origin::default();
    let counters = origin.clone();
    tokio::spawn(async move {
        let mut parked = Vec::new();
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            if counters.accepted.fetch_add(1, Ordering::SeqCst) == 0 {
                parked.push(stream);
                continue;
            }
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                let _ = stream.read(&mut buffer).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (address, origin)
}

#[tokio::test]
async fn a_gateway_that_stops_reading_one_stream_does_not_stall_the_mac_tunnel() {
    let fixture = fixture(8).await;
    let (upstream, _origin) = stalling_origin();
    let tunnel = real_tunnel(&fixture, upstream).await;
    let connector = visitor_tls(tunnel.site.clone());
    let socket = TcpStream::connect(fixture.public).await.unwrap();
    let mut uploading = connector
        .connect(ServerName::try_from(tunnel.host.clone()).unwrap(), socket)
        .await
        .unwrap();
    let chunk = vec![b'x'; 1 << 16];
    loop {
        let wrote = tokio::time::timeout(Duration::from_secs(2), async {
            uploading.write_all(&chunk).await?;
            uploading.flush().await
        })
        .await;
        if wrote.is_err() {
            break;
        }
    }
    let socket = TcpStream::connect(fixture.public).await.unwrap();
    let mut second = tokio::time::timeout(
        Duration::from_secs(3),
        connector.connect(ServerName::try_from(tunnel.host.clone()).unwrap(), socket),
    )
    .await
    .expect("a second visitor stalled behind the blocked stream")
    .unwrap();
    second.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    second.flush().await.unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), second.read_to_end(&mut reply))
        .await
        .expect("no reply through the tunnel while one stream was stalled")
        .ok();
    assert!(reply.ends_with(b"ok"));
    assert!(!tunnel.run.is_finished());
    tunnel.stop.send(true).unwrap();
    fixture.stop.cancel();
}

#[tokio::test]
async fn a_visitor_that_stalls_the_tls_handshake_is_closed_by_the_mac() {
    let fixture = fixture(8).await;
    let (upstream, _origin) = counting_origin(Vec::new()).await;
    let tunnel = real_tunnel(&fixture, upstream).await;
    let mut stalled = visitor(fixture.public, &tunnel.host).await;
    let mut drained = Vec::new();
    let closed =
        tokio::time::timeout(Duration::from_secs(14), stalled.read_to_end(&mut drained)).await;
    assert!(
        closed.is_ok(),
        "the Mac never gave up on a handshake that stopped"
    );
    tunnel.stop.send(true).unwrap();
    fixture.stop.cancel();
}

async fn closes_within(stream: &mut TcpStream, within: Duration) -> bool {
    let mut buffer = [0u8; 4096];
    tokio::time::timeout(within, async {
        while stream.read(&mut buffer).await.is_ok_and(|n| n > 0) {}
    })
    .await
    .is_ok()
}

#[tokio::test]
async fn visitors_past_the_mac_stream_limit_cost_only_those_visitors() {
    let fixture = fixture(64).await;
    let (upstream, _origin) = counting_origin(Vec::new()).await;
    let tunnel = real_tunnel(&fixture, upstream).await;
    let mut first = visitor(fixture.public, &tunnel.host).await;
    let mut rest = Vec::new();
    for _ in 0..40 {
        rest.push(visitor(fixture.public, &tunnel.host).await);
    }
    let last = rest.last_mut().unwrap();
    assert!(
        closes_within(last, Duration::from_secs(5)).await,
        "a stream past the Mac's limit was never refused"
    );
    assert!(
        !closes_within(&mut first, Duration::from_secs(2)).await,
        "a stream under the limit died with the tunnel"
    );
    tunnel.stop.send(true).unwrap();
    fixture.stop.cancel();
}

#[tokio::test]
async fn the_gateway_learns_the_address_the_relay_accepted() {
    let fixture = fixture(8).await;
    let proof = [7u8; 32];
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let (found, mut learned) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let found = found.clone();
            tokio::spawn(async move {
                let preamble = ab_remote::read_ingress_preamble(&mut socket, &proof).await;
                let _ = found.send(preamble);
                let mut buffer = [0u8; 1024];
                while socket.read(&mut buffer).await.is_ok_and(|n| n > 0) {}
            });
        }
    });
    let home = tempfile::tempdir().unwrap();
    let (http, tls) = client(&fixture.root);
    let identity = ab_remote::relay::enroll(home.path(), &fixture.endpoint, ZONE, &http)
        .await
        .unwrap();
    let host = identity.credential.hostname.clone();
    let certified = rcgen::generate_simple_self_signed(vec![host.clone()]).unwrap();
    let remote = home.path().join("remote");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::write(remote.join("relay-cert.pem"), certified.cert.pem()).unwrap();
    std::fs::write(
        remote.join("relay-key.pem"),
        certified.signing_key.serialize_pem(),
    )
    .unwrap();
    let relay = Relay {
        home: home.path().to_owned(),
        endpoint: fixture.endpoint.clone(),
        zone: ZONE.into(),
        client: http,
        certificates: Arc::new(Certificates::new(
            home.path().to_owned(),
            "https://acme.invalid/directory".into(),
        )),
        tls,
        domain: String::new(),
        resolver: None,
        domain_poll: Duration::from_millis(100),
        certificate_monitor: false,
    };
    let (statuses, observed) = watch::channel(std::collections::BTreeMap::new());
    let gateway = Gateway::authenticated(upstream, proof).with_status(statuses);
    let (stop, stopped) = watch::channel(false);
    tokio::spawn(async move { relay.run(gateway, stopped).await });
    wait_until("the tunnel never came up", Duration::from_secs(5), || {
        observed.borrow().get("relay").is_some_and(|e| e.active)
    })
    .await;
    let socket = TcpStream::connect(fixture.public).await.unwrap();
    let _secure = visitor_tls(certified.cert.der().clone())
        .connect(ServerName::try_from(host).unwrap(), socket)
        .await
        .unwrap();
    let preamble = tokio::time::timeout(Duration::from_secs(5), learned.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(preamble.source, Some("127.0.0.1".parse().unwrap()));
    assert!(preamble.binding.is_some());
    stop.send(true).unwrap();
    fixture.stop.cancel();
}
