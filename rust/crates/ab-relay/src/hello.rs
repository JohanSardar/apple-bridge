use crate::{Error, Result};
use tokio::io::{AsyncRead, AsyncReadExt};

pub fn valid_host(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
}
fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    if n > b.len() {
        return Err(Error::Denied);
    }
    let (out, rest) = b.split_at(n);
    *b = rest;
    Ok(out)
}
fn byte(b: &mut &[u8]) -> Result<usize> {
    Ok(usize::from(take(b, 1)?[0]))
}
fn word(b: &mut &[u8]) -> Result<usize> {
    let raw = take(b, 2)?;
    Ok(usize::from(u16::from_be_bytes([raw[0], raw[1]])))
}
pub fn hello_name(mut b: &[u8]) -> Result<String> {
    take(&mut b, 34)?;
    let n = byte(&mut b)?;
    take(&mut b, n)?;
    let n = word(&mut b)?;
    if n == 0 || n % 2 != 0 {
        return Err(Error::Denied);
    }
    take(&mut b, n)?;
    let n = byte(&mut b)?;
    if n == 0 {
        return Err(Error::Denied);
    }
    take(&mut b, n)?;
    let n = word(&mut b)?;
    if n != b.len() {
        return Err(Error::Denied);
    }
    let mut host = String::new();
    while !b.is_empty() {
        let kind = word(&mut b)?;
        let n = word(&mut b)?;
        let mut ext = take(&mut b, n)?;
        if kind != 0 {
            continue;
        }
        if !host.is_empty() {
            return Err(Error::Denied);
        }
        let n = word(&mut ext)?;
        if n != ext.len() {
            return Err(Error::Denied);
        }
        while !ext.is_empty() {
            let kind = byte(&mut ext)?;
            let n = word(&mut ext)?;
            let name = take(&mut ext, n)?;
            if kind == 0 {
                if !host.is_empty() {
                    return Err(Error::Denied);
                }
                host = String::from_utf8_lossy(name).to_lowercase();
            }
        }
    }
    if !valid_host(&host) {
        return Err(Error::Denied);
    }
    Ok(host)
}
pub async fn client_hello<R: AsyncRead + Unpin>(r: &mut R) -> Result<(String, Vec<u8>)> {
    let mut raw = Vec::new();
    let mut hello = Vec::new();
    while raw.len() < 65536 {
        let mut header = [0; 5];
        r.read_exact(&mut header).await?;
        let n = usize::from(u16::from_be_bytes([header[3], header[4]]));
        if header[0] != 22 || header[1] != 3 || n == 0 || n > 16384 || raw.len() + 5 + n > 65536 {
            return Err(Error::Denied);
        }
        let mut body = vec![0; n];
        r.read_exact(&mut body).await?;
        raw.extend_from_slice(&header);
        raw.extend_from_slice(&body);
        hello.extend_from_slice(&body);
        if hello.len() < 4 {
            continue;
        }
        let size =
            usize::from(hello[1]) * 65536 + usize::from(hello[2]) * 256 + usize::from(hello[3]);
        if hello[0] != 1 || size > 65532 {
            return Err(Error::Denied);
        }
        if hello.len() < size + 4 {
            continue;
        }
        return Ok((hello_name(&hello[4..size + 4])?, raw));
    }
    Err(Error::Denied)
}
