use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub enum EscapeMode {
    Html,
    NoHtml,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid JSON")]
    Json(#[from] serde_json::Error),
    #[error("invalid time")]
    Time,
}

#[derive(Debug, Clone)]
pub struct GoTime(String);

impl GoTime {
    pub fn parse(input: &str) -> Result<Self, Error> {
        let t: jiff::Timestamp = input.parse().map_err(|_| Error::Time)?;
        let text = t.to_string();
        let year = text.split('-').next().ok_or(Error::Time)?;
        if year.len() != 4 {
            return Err(Error::Time);
        }
        Ok(Self(text))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub struct CommandHash<'a> {
    pub op: &'a str,
    pub args: &'a [u8],
    pub version: u64,
    pub expires: GoTime,
    pub era: &'a str,
}

impl CommandHash<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let args = record_data(self.args, EscapeMode::Html)?;
        let mut out = String::from("{\"Op\":");
        out.push_str(&quote(self.op, EscapeMode::Html)?);
        out.push_str(",\"Args\":");
        out.push_str(std::str::from_utf8(&args).map_err(|_| Error::Time)?);
        out.push_str(",\"Version\":");
        out.push_str(&self.version.to_string());
        out.push_str(",\"Expires\":");
        out.push_str(&quote(self.expires.as_str(), EscapeMode::Html)?);
        out.push_str(",\"Era\":");
        out.push_str(&quote(self.era, EscapeMode::Html)?);
        out.push('}');
        Ok(out.into_bytes())
    }
    pub fn digest(&self) -> Result<String, Error> {
        Ok(format!("{:x}", Sha256::digest(self.encode()?)))
    }
}

pub fn record_data(input: &[u8], mode: EscapeMode) -> Result<Vec<u8>, Error> {
    let text = go_utf8(input);
    let normalized = normalize_surrogates(&text);
    let raw: Box<RawValue> = serde_json::from_slice(&normalized)?;
    Ok(encode(&raw, mode)?.into_bytes())
}

fn quote(input: &str, mode: EscapeMode) -> Result<String, Error> {
    let mut out = serde_json::to_string(input)?;
    out = out
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    if matches!(mode, EscapeMode::Html) {
        out = out
            .replace('&', "\\u0026")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
    }
    Ok(out)
}

fn encode(raw: &RawValue, mode: EscapeMode) -> Result<String, Error> {
    let text = raw.get();
    match text.as_bytes().first() {
        Some(b'{') => {
            let map: BTreeMap<String, Box<RawValue>> = serde_json::from_str(text)?;
            let entries = map
                .iter()
                .map(|(k, v)| Ok(format!("{}:{}", quote(k, mode)?, encode(v, mode)?)))
                .collect::<Result<Vec<_>, Error>>()?;
            Ok(format!("{{{}}}", entries.join(",")))
        }
        Some(b'[') => {
            let array: Vec<Box<RawValue>> = serde_json::from_str(text)?;
            let entries = array
                .iter()
                .map(|v| encode(v, mode))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("[{}]", entries.join(",")))
        }
        Some(b'"') => quote(&serde_json::from_str::<String>(text)?, mode),
        _ => Ok(text.to_owned()),
    }
}

fn normalize_surrogates(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'\\') {
            if bytes.get(i + 1) == Some(&b'u') {
                let hex = text
                    .get(i + 2..i + 6)
                    .and_then(|v| u16::from_str_radix(v, 16).ok());
                if let Some(code) = hex.filter(|code| (0xD800..=0xDFFF).contains(code)) {
                    let next = if bytes.get(i + 6..i + 8) == Some(b"\\u") {
                        text.get(i + 8..i + 12)
                            .and_then(|v| u16::from_str_radix(v, 16).ok())
                    } else {
                        None
                    };
                    if (0xD800..=0xDBFF).contains(&code)
                        && next.is_some_and(|v| (0xDC00..=0xDFFF).contains(&v))
                    {
                        normalized.extend_from_slice(&bytes[i..i + 12]);
                        i += 12;
                        continue;
                    }
                    normalized.extend_from_slice(b"\\ufffd");
                    i += 6;
                    continue;
                }
            }
            normalized.push(bytes[i]);
            i += 1;
            if let Some(b) = bytes.get(i) {
                normalized.push(*b);
                i += 1;
            }
        } else {
            normalized.push(bytes[i]);
            i += 1;
        }
    }
    normalized
}

fn go_utf8(mut input: &[u8]) -> String {
    let mut out = String::new();
    while !input.is_empty() {
        match std::str::from_utf8(input) {
            Ok(valid) => {
                out.push_str(valid);
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                out.push_str(&String::from_utf8_lossy(&input[..valid]));
                out.push('\u{fffd}');
                input = &input[valid + 1..];
            }
        }
    }
    out
}
