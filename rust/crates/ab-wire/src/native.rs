use crate::{
    gojson::GoTime,
    messages::{Capture, Checkpoint, IpcFrame, Observation},
};
pub use ab_domain::{ErrorCode, Source};
use serde::{
    Deserialize, Serialize,
    de::{self, IgnoredAny, MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{Read, Write},
};

pub const PROTOCOL: &str = "native/1";
pub const VERSION: u64 = 1;
pub const MAX_FRAME: usize = 262_144;

type Raw = Box<RawValue>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub checkpoints: BTreeMap<String, Checkpoint>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ready {
    #[serde(default)]
    pub coverage: BTreeMap<String, Raw>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, Raw>,
    #[serde(default)]
    pub helper_build: String,
    #[serde(default)]
    pub os_build: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceState {
    pub source: Source,
    pub state: String,
    #[serde(default)]
    pub coverage: BTreeMap<String, Raw>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, Raw>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scan {
    pub id: String,
    pub source: Source,
    pub generation: String,
    #[serde(default)]
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub scan_id: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub scan_started_at: String,
    #[serde(default)]
    pub completion_evidence: String,
    #[serde(default)]
    pub blob_handles: BTreeMap<String, String>,
    #[serde(default)]
    pub live: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanCommitted {
    pub id: String,
    pub source: Source,
    pub checkpoint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanFailed {
    pub id: String,
    pub code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub command_id: String,
    pub principal: String,
    pub request_hash: String,
    #[serde(default)]
    pub origin: String,
    pub expires_at: String,
    pub action: Raw,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_state: Option<Raw>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prepared {
    pub command_id: String,
    pub native_state: Raw,
    #[serde(default)]
    pub route: String,
    #[serde(default)]
    pub candidate_ids: Vec<Raw>,
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rejected {
    pub command_id: String,
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Raw>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dispatch {
    pub command_id: String,
    pub request_hash: String,
    pub action: Raw,
    pub native_state: Raw,
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub command_id: String,
    pub disposition: String,
    pub evidence: Raw,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resolve {
    pub command_id: String,
    pub saved: Raw,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Query {
    pub id: String,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Raw>,
    #[serde(default)]
    pub deadline_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResult {
    pub id: String,
    pub result_json: Raw,
    #[serde(default)]
    pub blobs: Vec<Raw>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryError {
    pub id: String,
    pub code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryCancel {
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobRead {
    pub handle: String,
    pub offset: i64,
    pub length: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobChunk {
    pub handle: String,
    pub offset: i64,
    pub bytes: String,
    pub digest: String,
    #[serde(default)]
    pub eof: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobUnavailable {
    pub handle: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug)]
pub enum Frame {
    Hello { owner_fence: u64 },
    Welcome,
    Session(Session),
    Ready(Ready),
    SourceState(SourceState),
    Scan(Scan),
    ScanCommitted(ScanCommitted),
    ScanFailed(ScanFailed),
    Prepare(Command),
    Prepared(Prepared),
    Rejected(Rejected),
    Dispatch(Dispatch),
    Outcome(Outcome),
    Resolve(Resolve),
    Query(Query),
    QueryResult(QueryResult),
    QueryError(QueryError),
    QueryCancel(QueryCancel),
    BlobRead(BlobRead),
    BlobChunk(BlobChunk),
    BlobUnavailable(BlobUnavailable),
    Close,
}

#[derive(Serialize)]
struct Tagged<'a, T: Serialize> {
    t: &'static str,
    #[serde(flatten)]
    body: &'a T,
}

#[derive(Serialize)]
struct Handshake {
    protocol: &'static str,
    version: u64,
}

#[derive(Serialize)]
struct HelloBody {
    protocol: &'static str,
    version: u64,
    owner_fence: u64,
}

#[derive(Serialize)]
struct Empty {}

#[derive(Deserialize)]
struct HelloIn {
    protocol: String,
    version: u64,
    owner_fence: u64,
}

#[derive(Deserialize)]
struct WelcomeIn {
    protocol: String,
    version: u64,
}

struct Envelope(String);

impl<'de> Deserialize<'de> for Envelope {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Keys;
        impl<'de> Visitor<'de> for Keys {
            type Value = Envelope;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a native/1 object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Envelope, A::Error> {
                let mut seen = BTreeSet::new();
                let mut tag = None;
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom("duplicate key"));
                    }
                    if key == "t" {
                        tag = Some(map.next_value::<String>()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                tag.filter(|tag| !tag.is_empty())
                    .map(Envelope)
                    .ok_or_else(|| de::Error::custom("missing t"))
            }
        }
        deserializer.deserialize_map(Keys)
    }
}

fn body<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ErrorCode> {
    serde_json::from_slice(bytes).map_err(|_| ErrorCode::Invalid)
}

fn raw_text(value: &str) -> Result<Raw, ErrorCode> {
    RawValue::from_string(serde_json::to_string(value).map_err(|_| ErrorCode::Invalid)?)
        .map_err(|_| ErrorCode::Invalid)
}

impl Frame {
    pub fn decode(bytes: &[u8]) -> Result<Self, ErrorCode> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME {
            return Err(ErrorCode::Capacity);
        }
        let Envelope(tag) = serde_json::from_slice(bytes).map_err(|_| ErrorCode::Invalid)?;
        Ok(match tag.as_str() {
            "Hello" => {
                let hello: HelloIn = body(bytes)?;
                if hello.protocol != PROTOCOL || hello.version != VERSION || hello.owner_fence == 0
                {
                    return Err(ErrorCode::Invalid);
                }
                Self::Hello {
                    owner_fence: hello.owner_fence,
                }
            }
            "Welcome" => {
                let welcome: WelcomeIn = body(bytes)?;
                if welcome.protocol != PROTOCOL || welcome.version != VERSION {
                    return Err(ErrorCode::Invalid);
                }
                Self::Welcome
            }
            "Session" => Self::Session(body(bytes)?),
            "Ready" => Self::Ready(body(bytes)?),
            "SourceState" => Self::SourceState(body(bytes)?),
            "Scan" => {
                let scan: Scan = body(bytes)?;
                if scan.id.is_empty() {
                    return Err(ErrorCode::Invalid);
                }
                Self::Scan(scan)
            }
            "ScanCommitted" => Self::ScanCommitted(body(bytes)?),
            "ScanFailed" => Self::ScanFailed(body(bytes)?),
            "Prepare" => Self::Prepare(body(bytes)?),
            "Prepared" => Self::Prepared(body(bytes)?),
            "Rejected" => Self::Rejected(body(bytes)?),
            "Dispatch" => Self::Dispatch(body(bytes)?),
            "Outcome" => Self::Outcome(body(bytes)?),
            "Resolve" => Self::Resolve(body(bytes)?),
            "Query" => Self::Query(body(bytes)?),
            "QueryResult" => Self::QueryResult(body(bytes)?),
            "QueryError" => Self::QueryError(body(bytes)?),
            "QueryCancel" => Self::QueryCancel(body(bytes)?),
            "BlobRead" => Self::BlobRead(body(bytes)?),
            "BlobChunk" => Self::BlobChunk(body(bytes)?),
            "BlobUnavailable" => Self::BlobUnavailable(body(bytes)?),
            "Close" => Self::Close,
            _ => return Err(ErrorCode::Invalid),
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, ErrorCode> {
        let json = match self {
            Self::Hello { owner_fence } => tagged(
                "Hello",
                &HelloBody {
                    protocol: PROTOCOL,
                    version: VERSION,
                    owner_fence: *owner_fence,
                },
            ),
            Self::Welcome => tagged(
                "Welcome",
                &Handshake {
                    protocol: PROTOCOL,
                    version: VERSION,
                },
            ),
            Self::Session(frame) => tagged("Session", frame),
            Self::Ready(frame) => tagged("Ready", frame),
            Self::SourceState(frame) => tagged("SourceState", frame),
            Self::Scan(frame) => tagged("Scan", frame),
            Self::ScanCommitted(frame) => tagged("ScanCommitted", frame),
            Self::ScanFailed(frame) => tagged("ScanFailed", frame),
            Self::Prepare(frame) => tagged("Prepare", frame),
            Self::Prepared(frame) => tagged("Prepared", frame),
            Self::Rejected(frame) => tagged("Rejected", frame),
            Self::Dispatch(frame) => tagged("Dispatch", frame),
            Self::Outcome(frame) => tagged("Outcome", frame),
            Self::Resolve(frame) => tagged("Resolve", frame),
            Self::Query(frame) => tagged("Query", frame),
            Self::QueryResult(frame) => tagged("QueryResult", frame),
            Self::QueryError(frame) => tagged("QueryError", frame),
            Self::QueryCancel(frame) => tagged("QueryCancel", frame),
            Self::BlobRead(frame) => tagged("BlobRead", frame),
            Self::BlobChunk(frame) => tagged("BlobChunk", frame),
            Self::BlobUnavailable(frame) => tagged("BlobUnavailable", frame),
            Self::Close => tagged("Close", &Empty {}),
        }?;
        if json.is_empty() || json.len() > MAX_FRAME {
            return Err(ErrorCode::Capacity);
        }
        let mut out = u32::try_from(json.len())
            .map_err(|_| ErrorCode::Capacity)?
            .to_be_bytes()
            .to_vec();
        out.extend(json);
        Ok(out)
    }
}

fn tagged<T: Serialize>(t: &'static str, body: &T) -> Result<Vec<u8>, ErrorCode> {
    serde_json::to_vec(&Tagged { t, body }).map_err(|_| ErrorCode::Invalid)
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Frame, ErrorCode> {
    let mut header = [0_u8; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| ErrorCode::Unavailable)?;
    let size = usize::try_from(u32::from_be_bytes(header)).map_err(|_| ErrorCode::Capacity)?;
    if size == 0 || size > MAX_FRAME {
        return Err(ErrorCode::Capacity);
    }
    let mut bytes = vec![0_u8; size];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| ErrorCode::Unavailable)?;
    Frame::decode(&bytes)
}

pub fn write_frame<W: Write>(writer: &mut W, frame: &Frame) -> Result<(), ErrorCode> {
    let bytes = frame.encode()?;
    writer.write_all(&bytes).map_err(|_| ErrorCode::Unavailable)
}

impl Ready {
    #[must_use]
    pub fn into_ipc(self) -> IpcFrame {
        IpcFrame {
            t: "Ready".into(),
            protocol: "ipc/1".into(),
            coverage: self.coverage,
            capabilities: self.capabilities,
            helper_build: self.helper_build,
            os_build: self.os_build,
            ..IpcFrame::default()
        }
    }
}

impl SourceState {
    #[must_use]
    pub fn into_ipc(self) -> IpcFrame {
        IpcFrame {
            t: "SourceState".into(),
            source: Some(self.source),
            state: self.state,
            coverage: self.coverage,
            capabilities: self.capabilities,
            reason: self.reason,
            ..IpcFrame::default()
        }
    }
}

fn raw_json(value: &serde_json::Value) -> Result<Raw, ErrorCode> {
    RawValue::from_string(value.to_string()).map_err(|_| ErrorCode::Invalid)
}

impl Command {
    pub fn from_ipc(frame: &IpcFrame) -> Result<Self, ErrorCode> {
        Ok(Self {
            command_id: frame.command_id.clone(),
            principal: frame.principal.clone(),
            request_hash: frame.request_hash.clone(),
            origin: frame.origin.clone(),
            expires_at: frame.expires_at.as_str().to_owned(),
            action: frame.action.clone().ok_or(ErrorCode::Invalid)?,
            expected_state: frame.expected_state.clone(),
        })
    }

    pub fn expires_unix(&self) -> Result<i64, ErrorCode> {
        GoTime::parse(&self.expires_at)
            .map(|time| time.unix_seconds())
            .map_err(|_| ErrorCode::Invalid)
    }

    #[must_use]
    pub fn operation(&self) -> String {
        serde_json::from_str::<serde_json::Value>(self.action.get())
            .ok()
            .and_then(|action| action["op"].as_str().map(str::to_owned))
            .unwrap_or_default()
    }
}

impl Query {
    pub fn from_ipc(frame: &IpcFrame) -> Self {
        Self {
            id: frame.id.clone(),
            op: frame.op.clone(),
            args: frame.args.clone(),
            deadline_ms: frame.deadline_ms,
        }
    }
}

impl BlobRead {
    #[must_use]
    pub fn from_ipc(frame: &IpcFrame) -> Self {
        Self {
            handle: frame.handle.clone(),
            offset: frame.offset,
            length: frame.length,
        }
    }
}

impl Prepared {
    pub fn into_ipc(self, ticket: String, fingerprint: String) -> IpcFrame {
        IpcFrame {
            t: "Prepared".into(),
            command_id: self.command_id,
            ticket,
            native_state: Some(self.native_state),
            route: if self.route.is_empty() {
                "eventkit".into()
            } else {
                self.route
            },
            fingerprint,
            candidate_ids: Some(self.candidate_ids),
            ..IpcFrame::default()
        }
    }
}

impl Rejected {
    pub fn into_ipc(self) -> Result<IpcFrame, ErrorCode> {
        let error = raw_json(&serde_json::json!({"code": self.code, "message": self.message}))?;
        Ok(IpcFrame {
            t: "Rejected".into(),
            command_id: self.command_id,
            disposition: "rejected_before_dispatch".into(),
            error: Some(error),
            evidence: self.evidence,
            ..IpcFrame::default()
        })
    }
}

impl Outcome {
    #[must_use]
    pub fn into_ipc(self) -> IpcFrame {
        IpcFrame {
            t: "Outcome".into(),
            command_id: self.command_id,
            disposition: self.disposition,
            evidence: Some(self.evidence),
            ..IpcFrame::default()
        }
    }
}

impl QueryResult {
    #[must_use]
    pub fn into_ipc(self) -> IpcFrame {
        IpcFrame {
            t: "QueryResult".into(),
            id: self.id,
            result_json: Some(self.result_json),
            blobs: Some(self.blobs),
            ..IpcFrame::default()
        }
    }
}

impl QueryError {
    #[must_use]
    pub fn into_ipc(self) -> IpcFrame {
        IpcFrame {
            t: "QueryError".into(),
            id: self.id,
            code: self.code,
            ..IpcFrame::default()
        }
    }
}

impl BlobChunk {
    #[must_use]
    pub fn into_ipc(self) -> IpcFrame {
        IpcFrame {
            t: "BlobChunk".into(),
            handle: self.handle,
            offset: self.offset,
            bytes: self.bytes,
            digest: self.digest,
            eof: self.eof,
            ..IpcFrame::default()
        }
    }
}

impl BlobUnavailable {
    #[must_use]
    pub fn into_ipc(self) -> IpcFrame {
        IpcFrame {
            t: "BlobUnavailable".into(),
            handle: self.handle,
            reason: self.reason,
            ..IpcFrame::default()
        }
    }
}

impl Session {
    pub fn from_open(open: &IpcFrame) -> Result<Self, ErrorCode> {
        let checkpoints = open
            .checkpoints
            .iter()
            .map(|(source, raw)| {
                serde_json::from_str::<Checkpoint>(raw.get())
                    .map(|checkpoint| (source.clone(), checkpoint))
                    .map_err(|_| ErrorCode::Invalid)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { checkpoints })
    }
}

impl Scan {
    pub fn capture(
        &self,
        observations: Vec<Observation>,
        predecessor: String,
        checkpoint: String,
    ) -> Result<Capture, ErrorCode> {
        let scoped = !self.scan_id.is_empty();
        let mut blob_handles = BTreeMap::new();
        for (digest, handle) in &self.blob_handles {
            blob_handles.insert(digest.clone(), raw_text(handle)?);
        }
        let scope = if scoped {
            Some(
                self.scope
                    .iter()
                    .map(|item| raw_text(item))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else {
            None
        };
        let scan_started_at = if self.scan_started_at.is_empty() {
            GoTime::default()
        } else {
            GoTime::parse(&self.scan_started_at).map_err(|_| ErrorCode::Invalid)?
        };
        Ok(Capture {
            live: self.live,
            blob_handles,
            source: Some(self.source),
            generation: self.generation.clone(),
            predecessor,
            checkpoint,
            observations: Some(observations),
            scan_id: self.scan_id.clone(),
            scope,
            scan_started_at,
            completion_evidence: if scoped {
                self.completion_evidence.clone()
            } else {
                String::new()
            },
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn round(frame: &Frame) -> Frame {
        let bytes = frame.encode().expect("encode");
        Frame::decode(&bytes[4..]).expect("decode")
    }

    #[test]
    fn handshake_is_byte_exact() {
        let hello = Frame::Hello { owner_fence: 7 }.encode().expect("hello");
        assert_eq!(
            &hello[4..],
            br#"{"t":"Hello","protocol":"native/1","version":1,"owner_fence":7}"#
        );
        let welcome = Frame::Welcome.encode().expect("welcome");
        assert_eq!(
            &welcome[4..],
            br#"{"t":"Welcome","protocol":"native/1","version":1}"#
        );
        assert!(matches!(
            round(&Frame::Hello { owner_fence: 7 }),
            Frame::Hello { owner_fence: 7 }
        ));
        assert!(matches!(round(&Frame::Welcome), Frame::Welcome));
    }

    #[test]
    fn rejects_malformed_envelopes() {
        for bytes in [
            br#"{"t":"Hello","t":"Hello","protocol":"native/1","version":1,"owner_fence":1}"#
                .as_slice(),
            br#"{"t":"Hello","protocol":"native/1","version":2,"owner_fence":1}"#,
            br#"{"t":"Hello","protocol":"native/1","version":1,"owner_fence":0}"#,
            br#"{"t":"Welcome","protocol":"native/2","version":1}"#,
            br#"{"t":"Nope"}"#,
            br#"{"id":"x"}"#,
            br"[]",
            b"",
        ] {
            assert!(
                Frame::decode(bytes).is_err(),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
    }

    #[test]
    fn scan_becomes_a_gateway_capture() {
        let bytes = br#"{"t":"Scan","id":"s1","source":"reminders","generation":"g","scan_id":"a","scope":["reminders.reminder"],"scan_started_at":"2026-01-02T03:04:05Z","completion_evidence":"complete","blob_handles":{"d":"h"},"live":true,"observations":[{"kind":"upsert","collection":"reminders.reminder","native_key":"k","data":{"title":"x"},"observed_at":"2026-01-02T03:04:05Z"}]}"#;
        let Frame::Scan(scan) = Frame::decode(bytes).expect("scan") else {
            panic!("not a scan");
        };
        let observations = scan.observations.clone();
        let capture = scan
            .capture(observations, "p".into(), "c".into())
            .expect("capture");
        let frame = IpcFrame {
            t: "Capture".into(),
            capture: Some(capture),
            ..IpcFrame::default()
        };
        let encoded = frame.encode().expect("ipc");
        let json: serde_json::Value = serde_json::from_slice(&encoded[4..]).expect("json");
        assert_eq!(json["t"], "Capture");
        assert_eq!(json["source"], "reminders");
        assert_eq!(json["predecessor"], "p");
        assert_eq!(json["checkpoint"], "c");
        assert_eq!(json["scan_id"], "a");
        assert_eq!(json["scope"][0], "reminders.reminder");
        assert_eq!(json["blob_handles"]["d"], "h");
        assert_eq!(json["live"], true);
        assert_eq!(json["observations"][0]["data"]["title"], "x");
    }

    #[test]
    fn unscoped_scan_omits_scan_fields() {
        let bytes = br#"{"t":"Scan","id":"s1","source":"mail","generation":"g","observations":[]}"#;
        let Frame::Scan(scan) = Frame::decode(bytes).expect("scan") else {
            panic!("not a scan");
        };
        let capture = scan
            .capture(Vec::new(), String::new(), "c".into())
            .expect("capture");
        let frame = IpcFrame {
            t: "Capture".into(),
            capture: Some(capture),
            ..IpcFrame::default()
        };
        let encoded = frame.encode().expect("ipc");
        let json: serde_json::Value = serde_json::from_slice(&encoded[4..]).expect("json");
        assert!(json.get("scan_id").is_none());
        assert!(json.get("scope").is_none());
        assert!(json.get("completion_evidence").is_none());
    }

    #[test]
    fn oversized_frames_are_refused() {
        let scan = Scan {
            id: "s".into(),
            source: Source::Mail,
            generation: "g".into(),
            observations: Vec::new(),
            scan_id: String::new(),
            scope: Vec::new(),
            scan_started_at: String::new(),
            completion_evidence: "x".repeat(MAX_FRAME),
            blob_handles: BTreeMap::new(),
            live: false,
        };
        assert!(Frame::Scan(scan).encode().is_err());
    }
}
