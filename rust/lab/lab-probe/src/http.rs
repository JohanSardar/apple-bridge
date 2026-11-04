use crate::{Error, Result};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types::ServerName};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::Arc,
    time::Duration,
};

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Reply {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}
#[derive(Debug, Clone)]
pub struct Client {
    pub host: String,
    pub port: u16,
    pub target: SocketAddr,
    pub config: Arc<ClientConfig>,
    pub timeout: Duration,
}
impl Client {
    pub fn new(host: &str, port: u16, target: SocketAddr, roots: RootCertStore) -> Result<Self> {
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()?
                .with_root_certificates(roots)
                .with_no_client_auth();
        Ok(Self {
            host: host.to_owned(),
            port,
            target,
            config: Arc::new(config),
            timeout: Duration::from_secs(5),
        })
    }
    pub fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Reply> {
        let tcp = TcpStream::connect_timeout(&self.target, self.timeout)?;
        tcp.set_read_timeout(Some(self.timeout))?;
        tcp.set_write_timeout(Some(self.timeout))?;
        let name = ServerName::try_from(self.host.clone()).map_err(|_| Error::Invalid("host"))?;
        let mut stream = StreamOwned::new(ClientConnection::new(self.config.clone(), name)?, tcp);
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.host,
            body.len()
        );
        for (key, value) in headers {
            head.push_str(key);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;
        stream.flush()?;
        let mut raw = Vec::new();
        match stream.read_to_end(&mut raw) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => {}
            Err(error) => return Err(error.into()),
        }
        parse(&raw)
    }
}
pub(crate) fn parse(raw: &[u8]) -> Result<Reply> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(Error::Invalid("response"))?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| Error::Invalid("response"))?;
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or(Error::Invalid("status"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"))
    {
        body = dechunk(&body);
    }
    Ok(Reply {
        status,
        headers,
        body,
    })
}
fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = data.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&data[..line_end])
            .ok()
            .and_then(|text| usize::from_str_radix(text.trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 || data.len() < line_end + 2 + size {
            break;
        }
        out.extend_from_slice(&data[line_end + 2..line_end + 2 + size]);
        data = data.get(line_end + 2 + size + 2..).unwrap_or_default();
    }
    out
}
