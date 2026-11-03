use lab_world::{Error, Result};
use std::{
    fmt::Write as _,
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Response {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    #[must_use]
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

fn unchunk(mut rest: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
        let Ok(size) = usize::from_str_radix(String::from_utf8_lossy(&rest[..end]).trim(), 16)
        else {
            break;
        };
        rest = &rest[end + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
    out
}

pub fn parse(raw: &[u8]) -> Result<Response> {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| Error::Invalid("malformed HTTP reply".into()))?;
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::Invalid("malformed status".into()))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let body = raw[end + 4..].to_vec();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    Ok(Response {
        status,
        headers,
        body: if chunked { unchunk(&body) } else { body },
    })
}

pub fn request(
    address: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    seconds: u64,
) -> Result<Response> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(seconds)))?;
    stream.set_write_timeout(Some(Duration::from_secs(seconds)))?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        let _ = write!(head, "Host: {address}\r\n");
    }
    for (name, value) in headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    if let Some(data) = body {
        let _ = write!(head, "Content-Length: {}\r\n", data.len());
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if let Some(data) = body {
        stream.write_all(data)?;
    }
    let mut raw = vec![];
    let _ = stream.read_to_end(&mut raw);
    parse(&raw)
}

#[must_use]
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[must_use]
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[must_use]
pub fn query_value(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| decode(v))
    })
}

#[must_use]
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = vec![];
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let value = bytes
                    .get(index + 1..index + 3)
                    .and_then(|pair| std::str::from_utf8(pair).ok())
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok());
                if let Some(value) = value {
                    out.push(value);
                    index += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            other => out.push(other),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
