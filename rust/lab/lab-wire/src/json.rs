use crate::Error;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub enum Json {
    Null,
    Bool(bool),
    String(String),
    Number(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}
impl Json {
    pub fn decode(input: &str) -> Result<Self, Error> {
        let raw: Box<RawValue> = serde_json::from_str(input)?;
        let text = raw.get();
        match text.as_bytes().first() {
            Some(b'{') => {
                let fields: BTreeMap<String, Box<RawValue>> = serde_json::from_str(text)?;
                Ok(Self::Object(
                    fields
                        .into_iter()
                        .map(|(key, value)| Ok((key, Self::decode(value.get())?)))
                        .collect::<Result<_, Error>>()?,
                ))
            }
            Some(b'[') => {
                let values: Vec<Box<RawValue>> = serde_json::from_str(text)?;
                Ok(Self::Array(
                    values
                        .into_iter()
                        .map(|value| Self::decode(value.get()))
                        .collect::<Result<_, _>>()?,
                ))
            }
            Some(b'"') => Ok(Self::String(serde_json::from_str(text)?)),
            Some(b'n') => Ok(Self::Null),
            Some(b't') => Ok(Self::Bool(true)),
            Some(b'f') => Ok(Self::Bool(false)),
            Some(b'-' | b'0'..=b'9') => Ok(Self::Number(text.to_owned())),
            _ => Err(Error::Invalid),
        }
    }
    pub fn encode(&self, html: bool) -> Result<String, Error> {
        match self {
            Self::Null => Ok("null".into()),
            Self::Bool(value) => Ok(value.to_string()),
            Self::Number(value) => {
                let raw: Box<RawValue> = serde_json::from_str(value)?;
                if !matches!(raw.get().as_bytes().first(), Some(b'-' | b'0'..=b'9')) {
                    return Err(Error::Invalid);
                }
                Ok(value.clone())
            }
            Self::String(value) => quoted(value, html),
            Self::Array(values) => Ok(format!(
                "[{}]",
                values
                    .iter()
                    .map(|v| v.encode(html))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(",")
            )),
            Self::Object(fields) => Ok(format!(
                "{{{}}}",
                fields
                    .iter()
                    .map(|(key, value)| Ok(format!(
                        "{}:{}",
                        quoted(key, html)?,
                        value.encode(html)?
                    )))
                    .collect::<Result<Vec<_>, Error>>()?
                    .join(",")
            )),
        }
    }
}
fn quoted(value: &str, html: bool) -> Result<String, Error> {
    let mut encoded = serde_json::to_string(value)?
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    if html {
        encoded = encoded
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026");
    }
    Ok(encoded)
}
