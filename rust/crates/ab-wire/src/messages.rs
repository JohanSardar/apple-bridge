use crate::{
    framing::{Frame, Protocol},
    gojson::{self, EscapeMode, GoTime},
};
use ab_domain::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Observation {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub collection: String,
    #[serde(default)]
    pub native_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scoped_alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Box<RawValue>>,
    #[serde(default)]
    pub observed_at: GoTime,
    #[serde(default, skip_serializing_if = "is_false")]
    pub revalidated: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Capture {
    #[serde(default, skip_serializing_if = "is_false")]
    pub live: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blob_handles: BTreeMap<String, Box<RawValue>>,
    #[serde(default)]
    pub source: Option<ab_domain::Source>,
    #[serde(default)]
    pub generation: String,
    #[serde(default)]
    pub predecessor: String,
    #[serde(default)]
    pub checkpoint: String,
    #[serde(default)]
    pub observations: Option<Vec<Observation>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scan_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<Box<RawValue>>>,
    #[serde(default)]
    pub scan_started_at: GoTime,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub completion_evidence: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Checkpoint {
    #[serde(default, skip_serializing_if = "is_false")]
    pub refresh_blobs: bool,
    #[serde(default)]
    pub generation: String,
    #[serde(default)]
    pub checkpoint: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IpcFrame {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub deadline_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_json: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blobs: Option<Vec<Box<RawValue>>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub handle: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub offset: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub length: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bytes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub eof: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub principal: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_state: Option<Box<RawValue>>,
    #[serde(default)]
    pub expires_at: GoTime,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ticket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_state: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_ids: Option<Vec<Box<RawValue>>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub disposition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Box<RawValue>>,
    #[serde(default)]
    pub t: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub protocol: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub owner_fence: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub checkpoints: BTreeMap<String, Box<RawValue>>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub byte_credit: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub credit: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<Capture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ab_domain::Source>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checkpoint: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, Box<RawValue>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub coverage: BTreeMap<String, Box<RawValue>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub helper_build: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub os_build: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nonce: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub peer_nonce: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proof: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Batch {
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
    #[serde(default)]
    pub epoch: String,
    #[serde(default)]
    pub subscription_revision: u64,
    #[serde(default)]
    pub after: u64,
    #[serde(default)]
    pub through: u64,
    #[serde(default)]
    pub changes: Option<Vec<Box<RawValue>>>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FeedFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<Box<RawValue>>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delegation_protocol: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub clock_protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delegation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub offset: i64,
    #[serde(default)]
    pub t: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub protocol: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub replica_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gateway_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Box<RawValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_offer: Option<Vec<Box<RawValue>>>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub schema: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub credits: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub epoch: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub subscription_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Vec<Box<RawValue>>>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub head: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub retention_floor: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub s: u64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub count: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
    #[serde(default)]
    pub expires_at: GoTime,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub ordinal: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub records: Option<Vec<Box<RawValue>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<Batch>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default)]
    pub at: GoTime,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(v: &bool) -> bool {
    !v
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}
impl IpcFrame {
    pub fn encode(&self) -> Result<Vec<u8>, ErrorCode> {
        if self
            .result_json
            .as_ref()
            .is_some_and(|v| v.get().len() > 1 << 20)
        {
            return Err(ErrorCode::Capacity);
        }
        let mode = if self.t == "QueryResult" {
            EscapeMode::NoHtml
        } else {
            EscapeMode::Html
        };
        let bytes = gojson::legacy_json(self, mode).map_err(|_| ErrorCode::Invalid)?;
        let mut fields: BTreeMap<String, Box<RawValue>> =
            serde_json::from_slice(&bytes).map_err(|_| ErrorCode::Invalid)?;
        if self.t == "Capture" {
            let capture = fields.remove("capture").ok_or(ErrorCode::Invalid)?;
            fields = serde_json::from_str(capture.get()).map_err(|_| ErrorCode::Invalid)?;
            fields.insert(
                "t".into(),
                RawValue::from_string("\"Capture\"".into()).map_err(|_| ErrorCode::Invalid)?,
            );
        } else if ["Query", "QueryResult", "QueryError", "QueryCancel"].contains(&self.t.as_str()) {
            let allowed: &[&str] = match self.t.as_str() {
                "Query" => &["t", "id", "op", "args", "deadline_ms"],
                "QueryResult" => &["t", "id", "result_json", "blobs"],
                "QueryError" => &["t", "id", "code"],
                _ => &["t", "id"],
            };
            fields.retain(|k, _| allowed.contains(&k.as_str()));
            let required: &[(&str, &str)] = match self.t.as_str() {
                "Query" => &[("args", "null")],
                "QueryResult" => &[("id", "\"\""), ("result_json", "null")],
                "QueryError" => &[("id", "\"\""), ("code", "\"\"")],
                _ => &[("id", "\"\"")],
            };
            for (key, value) in required {
                if !fields.contains_key(*key) {
                    fields.insert(
                        (*key).into(),
                        RawValue::from_string((*value).into()).map_err(|_| ErrorCode::Invalid)?,
                    );
                }
            }

            if self.t == "QueryResult" && !fields.contains_key("blobs") {
                fields.insert(
                    "blobs".into(),
                    RawValue::from_string("[]".into()).map_err(|_| ErrorCode::Invalid)?,
                );
            }
        }
        let raw = serde_json::to_vec(&fields).map_err(|_| ErrorCode::Invalid)?;
        Frame::from_json(Protocol::Ipc, &raw)?.encode_ipc()
    }
}
impl FeedFrame {
    pub fn encode(&self) -> Result<Vec<u8>, ErrorCode> {
        let mode = if self.t == "CommandResponse" {
            EscapeMode::NoHtml
        } else {
            EscapeMode::Html
        };
        let bytes = gojson::legacy_json(self, mode).map_err(|_| ErrorCode::Invalid)?;
        if self.t == "Batch" {
            let batch = self.batch.as_ref().ok_or(ErrorCode::Invalid)?;
            let bytes = gojson::legacy_json(batch, mode).map_err(|_| ErrorCode::Invalid)?;
            let mut fields: BTreeMap<String, Box<RawValue>> =
                serde_json::from_slice(&bytes).map_err(|_| ErrorCode::Invalid)?;
            fields.insert(
                "t".into(),
                RawValue::from_string("\"Batch\"".into()).map_err(|_| ErrorCode::Invalid)?,
            );
            let raw = serde_json::to_vec(&fields).map_err(|_| ErrorCode::Invalid)?;
            return Frame::from_json(Protocol::Feed, &raw)?.encode_json();
        }
        Frame::from_json(Protocol::Feed, &bytes)?.encode_json()
    }
}
