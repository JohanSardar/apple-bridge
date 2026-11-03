use lab_world::{Error, Result};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    crypto::ring::default_provider,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
};
use std::{
    io::{ErrorKind, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

fn failed<E: std::fmt::Display>(error: E) -> Error {
    Error::Invalid(error.to_string())
}

#[derive(Debug)]
pub struct Origin {
    pub address: String,
    pub authority: CertificateDer<'static>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Drop for Origin {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn issue(host: &str) -> Result<(CertificateDer<'static>, CertificateDer<'static>, Vec<u8>)> {
    let authority_key = KeyPair::generate().map_err(failed)?;
    let mut authority_params = CertificateParams::new(Vec::<String>::new()).map_err(failed)?;
    authority_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    authority_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let authority = authority_params
        .self_signed(&authority_key)
        .map_err(failed)?;
    let issuer = Issuer::new(authority_params, authority_key);
    let leaf_key = KeyPair::generate().map_err(failed)?;
    let leaf = CertificateParams::new(vec![host.to_owned()])
        .map_err(failed)?
        .signed_by(&leaf_key, &issuer)
        .map_err(failed)?;
    Ok((
        authority.der().clone(),
        leaf.der().clone(),
        leaf_key.serialize_der(),
    ))
}

fn read_all(stream: &mut impl Read, until: impl Fn(&[u8]) -> Option<usize>) -> Option<Vec<u8>> {
    let mut raw = vec![];
    let mut buffer = [0u8; 8192];
    loop {
        if let Some(total) = until(&raw)
            && raw.len() >= total
        {
            return Some(raw);
        }
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&buffer[..n]);
    }
}
fn length_of(raw: &[u8]) -> Option<usize> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
    let length = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    Some(end + 4 + length)
}

pub fn origin(host: &str, label: &'static str, marker: Vec<u8>) -> Result<Origin> {
    let (authority, leaf, key) = issue(host)?;
    let provider = Arc::new(default_provider());
    let config = Arc::new(
        ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(failed)?
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )
            .map_err(failed)?,
    );
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = thread::spawn(move || {
        let mut workers = vec![];
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((socket, _)) => {
                    let _ = socket.set_nonblocking(false);
                    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
                    let (config, marker) = (config.clone(), marker.clone());
                    workers.push(thread::spawn(move || {
                        let Ok(connection) = ServerConnection::new(config) else {
                            return;
                        };
                        let mut stream = StreamOwned::new(connection, socket);
                        let Some(request) = read_all(&mut stream, length_of) else {
                            return;
                        };
                        let end = request
                            .windows(4)
                            .position(|w| w == b"\r\n\r\n")
                            .map_or(0, |p| p + 4);
                        if request[end..] != marker[..] {
                            let _ = stream.write_all(
                                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            );
                            return;
                        }
                        let mut body = label.as_bytes().to_vec();
                        body.extend(std::iter::repeat_n(b'x', 8192));
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                        stream.conn.send_close_notify();
                        let _ = stream.flush();
                        let _ = stream.sock.shutdown(Shutdown::Both);
                    }));
                }
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        }
        for worker in workers {
            let _ = worker.join();
        }
    });
    Ok(Origin {
        address,
        authority,
        stop,
        thread: Some(thread),
    })
}

pub fn client_config(roots: &[CertificateDer<'static>]) -> Result<Arc<ClientConfig>> {
    let mut store = RootCertStore::empty();
    for root in roots {
        store.add(root.clone()).map_err(failed)?;
    }
    Ok(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(failed)?
            .with_root_certificates(store)
            .with_no_client_auth(),
    ))
}

pub type Session = StreamOwned<ClientConnection, TcpStream>;

pub fn connect(
    config: &Arc<ClientConfig>,
    address: &str,
    host: &str,
    seconds: u64,
) -> Result<Session> {
    let socket = TcpStream::connect(address)?;
    socket.set_read_timeout(Some(Duration::from_secs(seconds)))?;
    socket.set_write_timeout(Some(Duration::from_secs(seconds)))?;
    let name = ServerName::try_from(host.to_owned()).map_err(failed)?;
    let connection = ClientConnection::new(config.clone(), name).map_err(failed)?;
    let mut session = StreamOwned::new(connection, socket);
    while session.conn.is_handshaking() {
        session.conn.complete_io(&mut session.sock)?;
    }
    Ok(session)
}

pub fn request(
    config: &Arc<ClientConfig>,
    address: &str,
    host: &str,
    marker: &[u8],
) -> Result<String> {
    let mut session = connect(config, address, host, 5)?;
    let head = format!(
        "POST / HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        marker.len()
    );
    session.write_all(head.as_bytes())?;
    session.write_all(marker)?;
    session.flush()?;
    let reply = read_all(&mut session, length_of)
        .ok_or_else(|| Error::Invalid("incomplete reply".into()))?;
    let end = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(0, |p| p + 4);
    Ok(String::from_utf8_lossy(&reply[end..]).into_owned())
}

pub fn hello(config: &Arc<ClientConfig>, host: &str) -> Vec<u8> {
    let Ok(name) = ServerName::try_from(host.to_owned()) else {
        return vec![];
    };
    let Ok(mut connection) = ClientConnection::new(config.clone(), name) else {
        return vec![];
    };
    let mut bytes = vec![];
    let _ = connection.write_tls(&mut bytes);
    bytes
}

#[must_use]
pub fn stream_closed(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::UnexpectedEof
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
    )
}

pub fn closed_within(session: &mut Session, seconds: u64) -> bool {
    let _ = session
        .sock
        .set_read_timeout(Some(Duration::from_secs(seconds)));
    let mut byte = [0u8; 1];
    match session.read(&mut byte) {
        Ok(n) => n == 0,
        Err(error) => stream_closed(&error),
    }
}

pub fn drain(socket: &mut TcpStream, seconds: u64) -> (bool, usize) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(seconds)));
    let mut total = 0;
    let mut buffer = [0u8; 8192];
    loop {
        match socket.read(&mut buffer) {
            Ok(0) => return (true, total),
            Ok(n) => total += n,
            Err(_) => return (false, total),
        }
    }
}
