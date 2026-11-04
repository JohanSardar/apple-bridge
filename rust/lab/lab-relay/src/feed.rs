use crate::{Error, Identity, Result, pinned_config};
use rustls::{ClientConnection, StreamOwned, pki_types::ServerName};
use std::{
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
use tungstenite::{Message, WebSocket, client::IntoClientRequest};

type Stream = StreamOwned<ClientConnection, TcpStream>;

pub struct FeedLink {
    socket: WebSocket<Stream>,
}
impl std::fmt::Debug for FeedLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedLink").finish_non_exhaustive()
    }
}
impl FeedLink {
    pub fn dial(address: &str, identity: &Identity, pin: &str, timeout: Duration) -> Result<Self> {
        let config = pinned_config(identity, pin)?;
        let target = address.to_socket_addrs()?.next().ok_or(Error::Invalid)?;
        let mut tcp = TcpStream::connect_timeout(&target, timeout)?;
        tcp.set_read_timeout(Some(timeout))?;
        tcp.set_write_timeout(Some(timeout))?;
        let mut connection = ClientConnection::new(
            config,
            ServerName::try_from("localhost").map_err(|_| Error::Invalid)?,
        )?;
        while connection.is_handshaking() {
            connection.complete_io(&mut tcp)?;
        }
        let request = format!("wss://{address}/v1/feed").into_client_request()?;
        let (socket, _) = tungstenite::client::client(request, StreamOwned::new(connection, tcp))
            .map_err(|error| match error {
            tungstenite::HandshakeError::Failure(error) => Error::from(error),
            tungstenite::HandshakeError::Interrupted(_) => Error::Timeout,
        })?;
        Ok(Self { socket })
    }
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        self.socket.send(Message::text(text.to_owned()))?;
        Ok(())
    }
    pub fn try_recv_text(&mut self, timeout: Duration) -> Result<Option<String>> {
        match self.recv_text(timeout) {
            Ok(text) => Ok(Some(text)),
            Err(Error::Timeout) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub fn recv_text(&mut self, timeout: Duration) -> Result<String> {
        self.socket.get_mut().sock.set_read_timeout(Some(timeout))?;
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => return Ok(text.as_str().to_owned()),
                Ok(Message::Binary(_)) => return Err(Error::Invalid),
                Ok(Message::Close(_)) => return Err(Error::Unauthorized),
                Ok(_) => {}
                Err(tungstenite::Error::Io(io))
                    if matches!(
                        io.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(Error::Timeout);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}
