#![allow(clippy::unwrap_used, clippy::expect_used)]

use reqwest::StatusCode;
use rustls::pki_types::ServerName;
use std::{
    fs::{self, File},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    process::{Child, Command},
};

const CONTROL_HOST: &str = "relay.test";
const APP_HOST: &str = "app.relay.test";
const RESETS: usize = 200;
const OPEN_FILE_LIMIT: u32 = 64;
const HELD_CONNECTIONS: usize = 96;

#[derive(Clone, Copy)]
enum Control {
    SharedPort,
    OwnListener,
}

struct Relay {
    child: Child,
    public: SocketAddr,
    control: SocketAddr,
    log: PathBuf,
    certificate: rustls::pki_types::CertificateDer<'static>,
    _dir: TempDir,
}

fn unused_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

impl Relay {
    async fn start(control: Control, open_file_limit: Option<u32>) -> Self {
        Self::start_with(control, open_file_limit, &["--source-slots", "4096"]).await
    }

    async fn start_with(control: Control, open_file_limit: Option<u32>, extra: &[&str]) -> Self {
        let dir = TempDir::new().unwrap();
        let identity =
            rcgen::generate_simple_self_signed(vec![CONTROL_HOST.into(), APP_HOST.into()]).unwrap();
        let certificate = identity.cert.pem();
        let key = identity.signing_key.serialize_pem();
        let cert_path = dir.path().join("control-cert.pem");
        let key_path = dir.path().join("control-key.pem");
        fs::write(&cert_path, &certificate).unwrap();
        fs::write(&key_path, &key).unwrap();
        let app_tls = dir.path().join("control-tls/app");
        fs::create_dir_all(&app_tls).unwrap();
        fs::write(app_tls.join("relay-cert.pem"), &certificate).unwrap();
        fs::write(app_tls.join("relay-key.pem"), &key).unwrap();

        let public = unused_address();
        let mut own_control = public;
        let mut args: Vec<String> = [
            "--zone",
            CONTROL_HOST,
            "--state",
            dir.path().join("state.json").to_str().unwrap(),
            "--addr",
            &public.to_string(),
            "--control-cert",
            cert_path.to_str().unwrap(),
            "--control-key",
            key_path.to_str().unwrap(),
        ]
        .map(str::to_owned)
        .to_vec();
        match control {
            Control::SharedPort => args.extend(
                [
                    "--control-host",
                    CONTROL_HOST,
                    "--app-host",
                    APP_HOST,
                    "--app-upstream",
                    "127.0.0.1:1",
                    "--dns-listen",
                    "127.0.0.1:0",
                    "--public-ip",
                    "127.0.0.1",
                ]
                .map(str::to_owned),
            ),
            Control::OwnListener => {
                own_control = unused_address();
                args.extend(["--control-addr".to_owned(), own_control.to_string()]);
            }
        }

        args.extend(extra.iter().map(|arg| (*arg).to_owned()));

        let binary = env!("CARGO_BIN_EXE_ab-relay");
        let mut command = if let Some(limit) = open_file_limit {
            let mut shell = Command::new("sh");
            shell
                .args(["-c", r#"ulimit -n "$1" && shift && exec "$@""#, "sh"])
                .arg(limit.to_string())
                .arg(binary);
            shell
        } else {
            Command::new(binary)
        };
        let log = dir.path().join("relay.log");
        let child = command
            .args(&args)
            .stdout(Stdio::null())
            .stderr(File::create(&log).unwrap())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut relay = Self {
            child,
            public,
            control: own_control,
            log,
            certificate: identity.cert.der().clone(),
            _dir: dir,
        };
        relay.wait_until_listening().await;
        relay
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn assert_running(&mut self) {
        if let Some(status) = self.child.try_wait().unwrap() {
            panic!("the relay exited with {status}: {}", self.log());
        }
    }

    async fn wait_until_listening(&mut self) {
        for _ in 0..400 {
            self.assert_running();
            let public = TcpStream::connect(self.public).await.is_ok();
            if public && TcpStream::connect(self.control).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the relay never started listening: {}", self.log());
    }

    async fn wait_for_log(&mut self, text: &str) {
        for _ in 0..400 {
            self.assert_running();
            if self.log().contains(text) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the relay never logged {text:?}: {}", self.log());
    }

    async fn request(&self, host: &str, address: SocketAddr) -> StatusCode {
        reqwest::Client::builder()
            .no_proxy()
            .danger_accept_invalid_certs(true)
            .resolve(host, address)
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap()
            .get(format!("https://{host}:{}/info", address.port()))
            .send()
            .await
            .unwrap_or_else(|e| panic!("{host} stopped serving: {e}: {}", self.log()))
            .status()
    }

    async fn connect(&mut self, address: SocketAddr) -> TcpStream {
        match TcpStream::connect(address).await {
            Ok(socket) => socket,
            Err(e) => {
                let exit = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
                panic!(
                    "the relay stopped listening: {e}; exit: {exit:?}; log: {}",
                    self.log()
                );
            }
        }
    }

    async fn silent_tls(&self, host: &str) -> Option<tokio_rustls::client::TlsStream<TcpStream>> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.certificate.clone()).unwrap();
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        let socket = TcpStream::connect(self.public).await.ok()?;
        let name = ServerName::try_from(host.to_owned()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, socket),
        )
        .await
        .ok()?
        .ok()
    }

    #[cfg(target_os = "linux")]
    async fn request_from(&self, source: &str, host: &str) -> reqwest::Result<StatusCode> {
        reqwest::Client::builder()
            .no_proxy()
            .danger_accept_invalid_certs(true)
            .resolve(host, self.public)
            .local_address(source.parse::<std::net::IpAddr>().unwrap())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
            .get(format!("https://{host}:{}/info", self.public.port()))
            .send()
            .await
            .map(|response| response.status())
    }

    async fn reset_after_client_hello(&mut self, host: &str) {
        let hello = client_hello(host);
        for _ in 0..RESETS {
            let mut socket = self.connect(self.public).await;
            let _ = socket.write_all(&hello).await;
            socket.set_zero_linger().unwrap();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    async fn hold_connections_until_accept_fails(&mut self, address: SocketAddr) {
        let mut held = Vec::new();
        for _ in 0..HELD_CONNECTIONS {
            held.push(self.connect(address).await);
        }
        self.wait_for_log("accept failed").await;
        drop(held);
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

#[tokio::test]
async fn a_control_host_reset_right_after_client_hello_leaves_the_relay_serving() {
    let mut relay = Relay::start(Control::SharedPort, None).await;
    relay.reset_after_client_hello(CONTROL_HOST).await;
    relay.assert_running();
    assert_eq!(
        relay.request(CONTROL_HOST, relay.public).await,
        StatusCode::OK
    );
    assert_eq!(
        relay.request(APP_HOST, relay.public).await,
        StatusCode::BAD_GATEWAY,
        "the sign-in host answers even with nothing behind it"
    );
}

#[tokio::test]
async fn an_app_host_reset_right_after_client_hello_leaves_the_sign_in_host_serving() {
    let mut relay = Relay::start(Control::SharedPort, None).await;
    relay.reset_after_client_hello(APP_HOST).await;
    relay.assert_running();
    assert_eq!(
        relay.request(APP_HOST, relay.public).await,
        StatusCode::BAD_GATEWAY,
        "the sign-in host answers even with nothing behind it"
    );
    assert_eq!(
        relay.request(CONTROL_HOST, relay.public).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_failed_accept_on_the_public_listener_does_not_stop_the_relay() {
    let mut relay = Relay::start(Control::SharedPort, Some(OPEN_FILE_LIMIT)).await;
    relay
        .hold_connections_until_accept_fails(relay.public)
        .await;
    assert_eq!(
        relay.request(CONTROL_HOST, relay.public).await,
        StatusCode::OK
    );
    relay.assert_running();
}

#[tokio::test]
async fn a_failed_accept_on_the_control_listener_does_not_stop_the_relay() {
    let mut relay = Relay::start(Control::OwnListener, Some(OPEN_FILE_LIMIT)).await;
    relay
        .hold_connections_until_accept_fails(relay.control)
        .await;
    assert_eq!(
        relay.request(CONTROL_HOST, relay.control).await,
        StatusCode::OK
    );
    relay.assert_running();
}

async fn closes_within(
    mut connection: tokio_rustls::client::TlsStream<TcpStream>,
    within: Duration,
) -> bool {
    let mut byte = [0];
    tokio::time::timeout(within, connection.read(&mut byte))
        .await
        .is_ok()
}

#[tokio::test]
async fn a_tls_connection_that_never_sends_a_request_loses_its_slot() {
    let relay = Relay::start(Control::SharedPort, None).await;
    let silent = relay
        .silent_tls(CONTROL_HOST)
        .await
        .expect("the handshake should finish");
    assert!(
        closes_within(silent, Duration::from_secs(13)).await,
        "a silent TLS connection held its slot past the first-request deadline"
    );
}

#[tokio::test]
async fn a_tunnel_or_api_request_is_not_cut_off_by_the_first_request_deadline() {
    let relay = Relay::start_with(
        Control::SharedPort,
        None,
        &["--source-slots", "4096", "--first-request-timeout", "1s"],
    )
    .await;
    for _ in 0..3 {
        assert_eq!(
            relay.request(CONTROL_HOST, relay.public).await,
            StatusCode::OK
        );
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn silent_connections_cannot_take_the_control_hosts_capacity() {
    let relay = Relay::start(Control::SharedPort, None).await;
    let mut held = Vec::new();
    for _ in 0..300 {
        if let Some(connection) = relay.silent_tls(APP_HOST).await {
            held.push(connection);
        }
    }
    assert_eq!(
        relay
            .request_from("127.0.0.2", CONTROL_HOST)
            .await
            .expect("the control host refused a fresh caller"),
        StatusCode::OK
    );
    drop(held);
}

#[tokio::test]
async fn each_host_has_its_own_slots() {
    let relay = Relay::start_with(
        Control::SharedPort,
        None,
        &["--source-slots", "4096", "--app-slots", "2"],
    )
    .await;
    let mut held = Vec::new();
    for _ in 0..2 {
        held.push(relay.silent_tls(APP_HOST).await.expect("a free app slot"));
    }
    assert!(
        relay.silent_tls(APP_HOST).await.is_none(),
        "the sign-in host took more than its slots"
    );
    assert_eq!(
        relay.request(CONTROL_HOST, relay.public).await,
        StatusCode::OK
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn one_address_cannot_hold_more_than_its_share_of_anonymous_connections() {
    let relay = Relay::start_with(Control::SharedPort, None, &["--source-slots", "2"]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut held = Vec::new();
    for _ in 0..2 {
        held.push(relay.silent_tls(CONTROL_HOST).await.expect("a free slot"));
    }
    assert!(
        relay.silent_tls(CONTROL_HOST).await.is_none(),
        "one address held more than its share"
    );
    assert_eq!(
        relay
            .request_from("127.0.0.2", CONTROL_HOST)
            .await
            .expect("another address was refused"),
        StatusCode::OK
    );
}
