use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use sha2::Sha256;
use std::io::{Read, Write};

pub const MAX_FRAME: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("wire IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("wire JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame capacity exceeded")]
    Capacity,
    #[error("invalid protocol envelope")]
    Invalid,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Frame {
    pub fields: std::collections::BTreeMap<String, Box<RawValue>>,
}

impl Frame {
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME {
            return Err(Error::Capacity);
        }
        let frame: Self = serde_json::from_slice(bytes)?;
        let tag = frame.fields.get("t").ok_or(Error::Invalid)?;
        if serde_json::from_str::<String>(tag.get())?.is_empty() {
            return Err(Error::Invalid);
        }
        Ok(frame)
    }
}

pub fn encode_json(value: &Value) -> Result<Vec<u8>, Error> {
    let text = serde_json::to_string(value)?;
    Ok(text
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .into_bytes())
}

pub fn frame_bytes(json: &[u8]) -> Result<Vec<u8>, Error> {
    Frame::decode(json)?;
    let length = u32::try_from(json.len()).map_err(|_| Error::Capacity)?;
    let mut bytes = length.to_be_bytes().to_vec();
    bytes.extend_from_slice(json);
    Ok(bytes)
}

pub fn receive(reader: &mut impl Read) -> Result<(Value, usize), Error> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(Error::Capacity);
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Frame::decode(&bytes)?;
    Ok((serde_json::from_slice(&bytes)?, length + 4))
}

pub fn send(writer: &mut impl Write, frame: &Value) -> Result<usize, Error> {
    let bytes = frame_bytes(&encode_json(frame)?)?;
    writer.write_all(&bytes)?;
    Ok(bytes.len())
}

pub fn ipc_proof(secret: &[u8], role: &str, a: &str, b: &str) -> Result<String, Error> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| Error::Invalid)?;
    mac.update(format!("apple-bridge-ipc/1\0{role}\0{a}\0{b}").as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub fn verify_ipc_proof(
    secret: &[u8],
    role: &str,
    a: &str,
    b: &str,
    proof: &str,
) -> Result<(), Error> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| Error::Invalid)?;
    mac.update(format!("apple-bridge-ipc/1\0{role}\0{a}\0{b}").as_bytes());
    mac.verify_slice(&hex::decode(proof).map_err(|_| Error::Invalid)?)
        .map_err(|_| Error::Invalid)
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayProof {
    pub hostname: String,
    pub credential: String,
    pub nonce: String,
    pub signature: String,
}
impl RelayProof {
    pub fn message(&self, role: &str) -> String {
        format!(
            "apple-bridge-relay/1\n{role}\n{}\n{}\n{}",
            self.hostname, self.credential, self.nonce
        )
    }
    pub fn header(&self) -> Result<String, Error> {
        Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcMessage {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Box<RawValue>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RestResponse {
    #[serde(default)]
    pub result: Option<Box<RawValue>>,
    #[serde(default)]
    pub error: Option<Box<RawValue>>,
}

pub fn sse_messages(text: &str) -> Result<Vec<RpcMessage>, Error> {
    text.replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|event| {
            let data = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                None
            } else {
                Some(serde_json::from_str(&data).map_err(Error::from))
            }
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub struct BlobChunk {
    pub digest: [u8; 32],
    pub offset: u64,
    pub payload: Vec<u8>,
}
impl BlobChunk {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.payload.len() > 1024 * 1024 {
            return Err(Error::Capacity);
        }
        let length = u32::try_from(self.payload.len()).map_err(|_| Error::Capacity)?;
        let mut bytes = vec![0; 64];
        bytes[..32].copy_from_slice(&self.digest);
        bytes[32..40].copy_from_slice(&self.offset.to_be_bytes());
        bytes[40..44].copy_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 64 || bytes.len() > 64 + 1024 * 1024 {
            return Err(Error::Capacity);
        }
        if bytes[44..64].iter().any(|v| *v != 0) {
            return Err(Error::Invalid);
        }
        let digest = bytes[..32].try_into().map_err(|_| Error::Invalid)?;
        let offset = u64::from_be_bytes(bytes[32..40].try_into().map_err(|_| Error::Invalid)?);
        let length =
            u32::from_be_bytes(bytes[40..44].try_into().map_err(|_| Error::Invalid)?) as usize;
        if length != bytes.len() - 64 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            digest,
            offset,
            payload: bytes[64..].to_vec(),
        })
    }
}
