use crate::http::{Client, MCP_HEADERS, accepted, mcp_body, mcp_structured};
use lab_world::{Error, Result};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    fmt::Write as _,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

const LIMIT_SECONDS: u64 = 30;

#[derive(Debug)]
pub struct Fast {
    address: String,
    token: String,
    stream: RefCell<Option<BufReader<TcpStream>>>,
    limited: Cell<usize>,
    wire: Cell<f64>,
}
fn io(error: &std::io::Error) -> Error {
    Error::Invalid(format!("http: {error}"))
}
impl Fast {
    #[must_use]
    pub fn at(address: &str, token: &str) -> Self {
        Self {
            address: address.to_owned(),
            token: token.to_owned(),
            stream: RefCell::new(None),
            limited: Cell::new(0),
            wire: Cell::new(0.0),
        }
    }
    #[must_use]
    pub fn of(client: &Client) -> Self {
        Self::at(client.address(), client.token())
    }
    #[must_use]
    pub fn rate_limited(&self) -> usize {
        self.limited.get()
    }
    #[must_use]
    pub fn wire_milliseconds(&self) -> f64 {
        self.wire.get()
    }
    fn open(&self) -> Result<BufReader<TcpStream>> {
        let stream = TcpStream::connect(&self.address).map_err(|e| io(&e))?;
        let limit = Some(Duration::from_secs(LIMIT_SECONDS));
        stream.set_read_timeout(limit).map_err(|e| io(&e))?;
        stream.set_write_timeout(limit).map_err(|e| io(&e))?;
        stream.set_nodelay(true).map_err(|e| io(&e))?;
        Ok(BufReader::new(stream))
    }
    fn exchange(
        &self,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<(u16, Vec<u8>)> {
        let mut request = format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n",
            self.address,
            self.token,
            body.len()
        );
        for (name, value) in headers {
            let _ = write!(request, "{name}: {value}\r\n");
        }
        request.push_str("\r\n");
        let mut bytes = request.into_bytes();
        bytes.extend_from_slice(body);
        let started = std::time::Instant::now();
        let outcome = self.attempt(&bytes);
        self.wire
            .set(self.wire.get() + started.elapsed().as_secs_f64() * 1000.0);
        outcome
    }
    fn attempt(&self, bytes: &[u8]) -> Result<(u16, Vec<u8>)> {
        for attempt in 0..2 {
            let reused = self.stream.borrow().is_some();
            let mut stream = match self.stream.borrow_mut().take() {
                Some(stream) => stream,
                None => self.open()?,
            };
            let outcome = stream
                .get_mut()
                .write_all(bytes)
                .map_err(|e| io(&e))
                .and_then(|()| read_response(&mut stream));
            match outcome {
                Ok((status, reply, keep)) => {
                    if keep {
                        *self.stream.borrow_mut() = Some(stream);
                    }
                    return Ok((status, reply));
                }
                Err(error) if attempt == 1 || !reused => return Err(error),
                Err(_) => {}
            }
        }
        Err(Error::Invalid("http: exhausted".into()))
    }
    pub fn call(&self, protocol: &str, operation: &str, input: &Value) -> Option<Value> {
        if protocol == "mcp" {
            let body = mcp_body(operation, input).ok()?;
            let (status, reply) = self.exchange("/mcp", &MCP_HEADERS, &body).ok()?;
            self.count(&reply);
            let (status, value) = mcp_structured(status, &reply).ok().flatten()?;
            accepted(status, value)
        } else {
            let body = serde_json::to_vec(input).ok()?;
            let (status, reply) = self
                .exchange(
                    &format!("/v1/op/{operation}"),
                    &[("Content-Type", "application/json")],
                    &body,
                )
                .ok()?;
            self.count(&reply);
            let value = serde_json::from_slice(&reply).ok()?;
            accepted(status, value)
        }
    }
    fn count(&self, reply: &[u8]) {
        if reply.len() <= 32 && String::from_utf8_lossy(reply).trim() == "rate limited" {
            self.limited.set(self.limited.get() + 1);
        }
    }
}
fn read_response(stream: &mut BufReader<TcpStream>) -> Result<(u16, Vec<u8>, bool)> {
    let mut line = String::new();
    stream.read_line(&mut line).map_err(|e| io(&e))?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| Error::Invalid("http: status line".into()))?;
    let mut length = None;
    let mut chunked = false;
    let mut keep = true;
    loop {
        line.clear();
        if stream.read_line(&mut line).map_err(|e| io(&e))? == 0 {
            return Err(Error::Invalid("http: truncated headers".into()));
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let (name, value) = (name.to_ascii_lowercase(), value.trim().to_ascii_lowercase());
            match name.as_str() {
                "content-length" => length = value.parse::<usize>().ok(),
                "transfer-encoding" => chunked = value.contains("chunked"),
                "connection" => keep = !value.contains("close"),
                _ => {}
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            stream.read_line(&mut line).map_err(|e| io(&e))?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or_default(), 16)
                .map_err(|_| Error::Invalid("http: chunk size".into()))?;
            if size == 0 {
                loop {
                    line.clear();
                    stream.read_line(&mut line).map_err(|e| io(&e))?;
                    if line.trim().is_empty() {
                        break;
                    }
                }
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            stream.read_exact(&mut body[start..]).map_err(|e| io(&e))?;
            let mut crlf = [0u8; 2];
            stream.read_exact(&mut crlf).map_err(|e| io(&e))?;
        }
    } else if let Some(length) = length {
        body.resize(length, 0);
        stream.read_exact(&mut body).map_err(|e| io(&e))?;
    } else {
        stream.read_to_end(&mut body).map_err(|e| io(&e))?;
        keep = false;
    }
    Ok((status, body, keep))
}
