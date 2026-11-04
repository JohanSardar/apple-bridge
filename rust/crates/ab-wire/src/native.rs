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
