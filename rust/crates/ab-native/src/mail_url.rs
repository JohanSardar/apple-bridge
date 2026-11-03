use crate::capture::{Error, Result};

#[derive(Debug, serde::Serialize)]
pub struct MailboxUrl {
    pub account: String,
    pub kind: String,
    pub path: String,
    pub name: String,
}
fn decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Some(byte) = std::str::from_utf8(&bytes[index + 1..index + 3])
                .ok()
                .and_then(|v| u8::from_str_radix(v, 16).ok())
        {
            output.push(byte);
            index += 3;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).map_err(|_| Error::Corrupt)
}
pub fn parse(value: &str) -> Result<MailboxUrl> {
    let (scheme, tail) = if let Some(tail) = value.strip_prefix("//") {
        ("unknown", tail)
    } else {
        let (scheme, tail) = value.split_once("://").ok_or(Error::Corrupt)?;
        if !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            || !scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ['+', '-', '.'].contains(&c))
        {
            return Err(Error::Corrupt);
        }
        (scheme, tail)
    };
    let end = tail.find(['/', '?', '#']).unwrap_or(tail.len());
    let authority = &tail[..end];
    let (user, host) = authority
        .rsplit_once('@')
        .map_or((None, authority), |(user, host)| {
            (Some(user.split(':').next().unwrap_or_default()), host)
        });
    let (host, port) = if let Some(host) = host.strip_prefix('[') {
        let (host, after) = host.split_once(']').ok_or(Error::Corrupt)?;
        let port = if after.is_empty() {
            None
        } else {
            Some(after.strip_prefix(':').ok_or(Error::Corrupt)?)
        };
        (host, port)
    } else {
        host.split_once(':')
            .map_or((host, None), |(host, port)| (host, Some(port)))
    };
    if host.is_empty() || port.is_some_and(|port| !port.bytes().all(|b| b.is_ascii_digit())) {
        return Err(Error::Corrupt);
    }
    let host = decode(host)?;
    let account = if let Some(user) = user {
        format!("{}@{host}", decode(user)?)
    } else {
        host
    };
    let raw = &tail[end..];
    let end = raw.find(['?', '#']).unwrap_or(raw.len());
    let raw = &raw[..end];
    let is_root = raw.starts_with('/') && raw.trim_end_matches('/').is_empty();
    let raw = raw.trim_end_matches('/');
    let name = if is_root {
        "/".into()
    } else {
        decode(raw.rsplit('/').next().unwrap_or_default()).unwrap_or_default()
    };
    let decoded = decode(raw).unwrap_or_default();
    let path = if is_root || (!decoded.is_empty() && decoded.bytes().all(|b| b == b'/')) {
        "/".into()
    } else {
        decoded.trim_end_matches('/').into()
    };
    Ok(MailboxUrl {
        account,
        kind: scheme.to_uppercase(),
        path,
        name,
    })
}
