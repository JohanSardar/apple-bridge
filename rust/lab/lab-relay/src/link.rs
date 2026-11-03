use crate::{Error, Result, ca::HOST};
use rustls::{ClientConfig, ClientConnection, StreamOwned, pki_types::ServerName};
use std::{
    net::{TcpStream, ToSocketAddrs},
    sync::Arc,
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, client::IntoClientRequest};

pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-Channel-Binding";
pub type Stream = StreamOwned<ClientConnection, TcpStream>;

pub fn client_config(roots: rustls::RootCertStore) -> Result<Arc<ClientConfig>> {
    Ok(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}
pub struct Link {
    socket: WebSocket<Stream>,
    pub binding: [u8; 32],
}
impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}
impl Link {
    pub fn dial(address: &str, config: &Arc<ClientConfig>, origin: Option<&str>) -> Result<Self> {
        Self::dial_as(address, config, HOST, origin)
    }
    pub fn dial_as(
        address: &str,
        config: &Arc<ClientConfig>,
        name: &str,
        origin: Option<&str>,
    ) -> Result<Self> {
        let target = address.to_socket_addrs()?.next().ok_or(Error::Invalid)?;
        let mut tcp = TcpStream::connect_timeout(&target, Duration::from_secs(5))?;
        tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut connection = ClientConnection::new(
            config.clone(),
            ServerName::try_from(name.to_owned()).map_err(|_| Error::Invalid)?,
        )?;
        while connection.is_handshaking() {
            connection.complete_io(&mut tcp)?;
        }
        let mut binding = [0; 32];
        connection.export_keying_material(&mut binding, EXPORTER_LABEL, Some(&[]))?;
        let port = target.port();
        let mut request = format!("wss://{name}:{port}/v1/feed").into_client_request()?;
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("Origin", origin.parse().map_err(|_| Error::Invalid)?);
        }
        let (socket, _) = tungstenite::client::client(request, StreamOwned::new(connection, tcp))
            .map_err(|error| match error {
            tungstenite::HandshakeError::Failure(error) => Error::from(error),
            tungstenite::HandshakeError::Interrupted(_) => Error::Timeout,
        })?;
        Ok(Self { socket, binding })
    }
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.socket.send(Message::text(text.to_owned()))?;
        Ok(())
    }
    pub fn send_binary(&mut self, bytes: &[u8]) -> Result<()> {
        self.socket.send(Message::binary(bytes.to_vec()))?;
        Ok(())
    }
    pub fn recv_text(&mut self, timeout: Duration) -> Result<String> {
        self.socket.get_mut().sock.set_read_timeout(Some(timeout))?;
        loop {
            match self.socket.read()? {
                Message::Text(text) => {
                    if text.len() > 8192 {
                        return Err(Error::Invalid);
                    }
                    return Ok(text.as_str().to_owned());
                }
                Message::Binary(_) => return Err(Error::Invalid),
                Message::Close(_) => return Err(Error::Unauthorized),
                _ => {}
            }
        }
    }
    pub fn try_recv_text(&mut self, timeout: Duration) -> Result<Option<String>> {
        self.socket.get_mut().sock.set_read_timeout(Some(timeout))?;
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => return Ok(Some(text.as_str().to_owned())),
                Ok(Message::Binary(_)) => return Err(Error::Invalid),
                Ok(Message::Close(_)) => return Err(Error::Unauthorized),
                Ok(_) => {}
                Err(tungstenite::Error::Io(io))
                    if matches!(
                        io.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    pub fn closed_within(&mut self, timeout: Duration) -> bool {
        let started = Instant::now();
        match self.recv_text(timeout) {
            Ok(_) => false,
            Err(Error::Socket(error)) => {
                !matches!(&*error, tungstenite::Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
                    && started.elapsed() < timeout + Duration::from_millis(200)
            }
            Err(Error::Io(io)) => !matches!(
                io.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            Err(_) => true,
        }
    }
}
