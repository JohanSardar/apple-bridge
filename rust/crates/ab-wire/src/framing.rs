use crate::{
    fields,
    gojson::{self, EscapeMode},
};
use ab_domain::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub const MAX_FRAME: usize = 256 * 1024;
pub const MAX_QUERY_FRAME: usize = (1 << 20) + (64 << 10);
pub const BLOB_CHUNK_SIZE: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Ipc,
    Feed,
}

#[derive(Clone, Debug)]
pub struct Frame {
    protocol: Protocol,
    fields: BTreeMap<String, Box<RawValue>>,
}

impl Frame {
    pub fn from_json(protocol: Protocol, bytes: &[u8]) -> Result<Self, ErrorCode> {
        if bytes.len() > MAX_QUERY_FRAME {
            return Err(ErrorCode::Capacity);
        }
        let fields: BTreeMap<String, Box<RawValue>> =
            serde_json::from_slice(bytes).map_err(|_| ErrorCode::Invalid)?;
        let frame = Self { protocol, fields };
        let kind = frame.kind()?;
        if bytes.len() > MAX_FRAME
            && !matches!(
                (protocol, kind.as_str()),
                (Protocol::Ipc, "QueryResult") | (Protocol::Feed, "CommandResponse")
            )
        {
            return Err(ErrorCode::Capacity);
        }
        frame.order()?;
        Ok(frame)
    }
    pub fn kind(&self) -> Result<String, ErrorCode> {
        let raw = self.fields.get("t").ok_or(ErrorCode::Invalid)?;
        let kind: String = serde_json::from_str(raw.get()).map_err(|_| ErrorCode::Invalid)?;
        if kind.is_empty() {
            return Err(ErrorCode::Invalid);
        }
        Ok(kind)
    }
    pub fn field(&self, name: &str) -> Option<&RawValue> {
        self.fields.get(name).map(AsRef::as_ref)
    }
    fn order(&self) -> Result<&'static [&'static str], ErrorCode> {
        let kind = self.kind()?;
        Ok(match (self.protocol, kind.as_str()) {
            (Protocol::Ipc, "Capture") => CAPTURE_FRAME_FIELDS,
            (Protocol::Ipc, "Query") => &["t", "id", "op", "args", "deadline_ms"],
            (Protocol::Ipc, "QueryResult") => &["t", "id", "result_json", "blobs"],
            (Protocol::Ipc, "QueryError") => &["t", "id", "code"],
            (Protocol::Ipc, "QueryCancel") => &["t", "id"],
            (Protocol::Ipc, _) => fields::I_P_C_FRAME_FIELDS,
            (Protocol::Feed, "Batch") => BATCH_FRAME_FIELDS,
            (Protocol::Feed, _) => fields::FEED_FRAME_FIELDS,
        })
    }
    pub fn encode_json(&self) -> Result<Vec<u8>, ErrorCode> {
        let mut out = Vec::from(b"{".as_slice());
        for key in self.order()? {
            if let Some(value) = self.fields.get(*key) {
                if out.len() > 1 {
                    out.push(b',');
                }
                out.extend(serde_json::to_vec(key).map_err(|_| ErrorCode::Invalid)?);
                out.push(b':');
                out.extend(
                    gojson::legacy_raw(value.get().as_bytes(), EscapeMode::NoHtml)
                        .map_err(|_| ErrorCode::Invalid)?,
                );
            }
        }
        for (key, value) in &self.fields {
            if !self.order()?.contains(&key.as_str()) {
                if out.len() > 1 {
                    out.push(b',');
                }
                out.extend(serde_json::to_vec(key).map_err(|_| ErrorCode::Invalid)?);
                out.push(b':');
                out.extend(
                    gojson::legacy_raw(value.get().as_bytes(), EscapeMode::NoHtml)
                        .map_err(|_| ErrorCode::Invalid)?,
                );
            }
        }
        out.push(b'}');
        if out.len() > MAX_QUERY_FRAME
            || (out.len() > MAX_FRAME
                && !matches!(
                    (self.protocol, self.kind()?.as_str()),
                    (Protocol::Ipc, "QueryResult") | (Protocol::Feed, "CommandResponse")
                ))
        {
            return Err(ErrorCode::Capacity);
        }
        Ok(out)
    }
    pub fn encode_ipc(&self) -> Result<Vec<u8>, ErrorCode> {
        if self.protocol != Protocol::Ipc {
            return Err(ErrorCode::Invalid);
        }
        let raw = self.encode_json()?;
        let mut out = u32::try_from(raw.len())
            .map_err(|_| ErrorCode::Capacity)?
            .to_be_bytes()
            .to_vec();
        out.extend(raw);
        Ok(out)
    }
}

const CAPTURE_FRAME_FIELDS: &[&str] = &[
    "t",
    "live",
    "blob_handles",
    "source",
    "generation",
    "predecessor",
    "checkpoint",
    "observations",
    "scan_id",
    "scope",
    "scan_started_at",
    "completion_evidence",
];
const BATCH_FRAME_FIELDS: &[&str] = &[
    "t",
    "partial",
    "epoch",
    "subscription_revision",
    "after",
    "through",
    "changes",
];

pub fn decode_ipc(input: &[u8]) -> Result<(Frame, usize), ErrorCode> {
    let header: [u8; 4] = input
        .get(..4)
        .ok_or(ErrorCode::Invalid)?
        .try_into()
        .map_err(|_| ErrorCode::Invalid)?;
    let n = usize::try_from(u32::from_be_bytes(header)).map_err(|_| ErrorCode::Capacity)?;
    if n == 0 || n > MAX_QUERY_FRAME {
        return Err(ErrorCode::Capacity);
    }
    let bytes = input.get(4..4 + n).ok_or(ErrorCode::Invalid)?;
    Ok((Frame::from_json(Protocol::Ipc, bytes)?, 4 + n))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobChunk {
    pub digest: [u8; 32],
    pub offset: i64,
    pub bytes: Vec<u8>,
}
impl BlobChunk {
    pub fn encode(&self) -> Result<Vec<u8>, ErrorCode> {
        if self.offset < 0 || self.bytes.len() > BLOB_CHUNK_SIZE {
            return Err(ErrorCode::Invalid);
        }
        let mut out = vec![0; 64];
        out[..32].copy_from_slice(&self.digest);
        out[32..40].copy_from_slice(&self.offset.to_be_bytes());
        out[40..44].copy_from_slice(
            &u32::try_from(self.bytes.len())
                .map_err(|_| ErrorCode::Invalid)?
                .to_be_bytes(),
        );
        out.extend_from_slice(&self.bytes);
        Ok(out)
    }
    pub fn decode(input: &[u8]) -> Result<Self, ErrorCode> {
        if !(64..=64 + BLOB_CHUNK_SIZE).contains(&input.len()) {
            return Err(ErrorCode::Corrupt);
        }
        let length = u32::from_be_bytes(input[40..44].try_into().map_err(|_| ErrorCode::Corrupt)?);
        if usize::try_from(length).map_err(|_| ErrorCode::Corrupt)? != input.len() - 64 {
            return Err(ErrorCode::Corrupt);
        }
        let offset = i64::from_be_bytes(input[32..40].try_into().map_err(|_| ErrorCode::Corrupt)?);
        if offset < 0 || input[44..64].iter().any(|b| *b != 0) {
            return Err(ErrorCode::Invalid);
        }
        Ok(Self {
            digest: input[..32].try_into().map_err(|_| ErrorCode::Corrupt)?,
            offset,
            bytes: input[64..].to_vec(),
        })
    }
}

pub fn legacy_json<T: Serialize>(value: &T, mode: EscapeMode) -> Result<Vec<u8>, ErrorCode> {
    gojson::legacy_json(value, mode).map_err(|_| ErrorCode::Invalid)
}

pub fn canonical_data(bytes: &[u8]) -> Result<Vec<u8>, ErrorCode> {
    gojson::record_data(bytes, EscapeMode::Html).map_err(|_| ErrorCode::Invalid)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RelayProof {
    pub hostname: String,
    pub credential: String,
    pub nonce: String,
    pub signature: String,
}
impl RelayProof {
    pub fn signature_input(&self, action: &str) -> Vec<u8> {
        format!(
            "apple-bridge-relay/1\n{action}\n{}\n{}\n{}",
            self.hostname, self.credential, self.nonce
        )
        .into_bytes()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelayChallenge {
    pub nonce: String,
}
impl RelayProof {
    pub fn encode_header(&self) -> Result<String, ErrorCode> {
        use base64::Engine;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(legacy_json(self, EscapeMode::Html)?))
    }
    pub fn decode_header(header: &str) -> Result<Self, ErrorCode> {
        use base64::Engine;
        if header.len() > MAX_FRAME {
            return Err(ErrorCode::Capacity);
        }
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(header)
            .map_err(|_| ErrorCode::Invalid)?;
        serde_json::from_slice(&raw).map_err(|_| ErrorCode::Invalid)
    }
}

pub fn device_link_message(
    relay: &str,
    transaction: &str,
    nonce: &str,
    public_key: &str,
) -> Vec<u8> {
    format!("apple-bridge-device-link/1\n{relay}\n{transaction}\n{nonce}\n{public_key}")
        .into_bytes()
}

pub fn device_key_fingerprint(public_key: &[u8; 32]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(public_key);
    digest[..8]
        .chunks(2)
        .map(|pair| format!("{:02X}{:02X}", pair[0], pair[1]))
        .collect::<Vec<_>>()
        .join("-")
}
