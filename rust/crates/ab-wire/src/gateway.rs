use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRef {
    pub generation: String,
    pub checkpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub owner_fence: u64,
    pub byte_credit: u64,
    pub checkpoints: BTreeMap<String, CheckpointRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    pub source: String,
    pub checkpoint: String,
    pub credit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRead {
    pub handle: String,
    pub offset: Option<u64>,
    pub length: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    pub command_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub id: String,
    pub op: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayFrame {
    Open(Open),
    Committed(Committed),
    BlobRead(BlobRead),
    Dispatch(Dispatch),
    Query(Query),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureHeader {
    pub source: String,
    pub checkpoint: String,
    pub generation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperFrame {
    Capture(CaptureHeader),
    Outcome,
    Rejected,
    Ready,
    Other,
}

impl HelperFrame {
    pub fn from_value(value: &Value) -> Result<Self, Malformed> {
        Ok(match value["t"].as_str() {
            Some("Capture") => Self::Capture(CaptureHeader {
                source: text(value, "source").ok_or(Malformed)?,
                checkpoint: owned(value, "checkpoint"),
                generation: owned(value, "generation"),
            }),
            Some("Outcome") => Self::Outcome,
            Some("Rejected") => Self::Rejected,
            Some("Ready") => Self::Ready,
            _ => Self::Other,
        })
    }
}

fn text(value: &Value, field: &str) -> Option<String> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn owned(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap_or_default().to_owned()
}

impl GatewayFrame {
    pub fn from_value(value: &Value) -> Result<Self, Malformed> {
        Ok(match value["t"].as_str() {
            Some("Open") => Self::Open(Open {
                owner_fence: value["owner_fence"].as_u64().unwrap_or(0),
                byte_credit: value["byte_credit"].as_u64().unwrap_or(0),
                checkpoints: value["checkpoints"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(source, checkpoint)| {
                        (
                            source.clone(),
                            CheckpointRef {
                                generation: owned(checkpoint, "generation"),
                                checkpoint: owned(checkpoint, "checkpoint"),
                            },
                        )
                    })
                    .collect(),
            }),
            Some("Committed") => Self::Committed(Committed {
                source: text(value, "source").ok_or(Malformed)?,
                checkpoint: owned(value, "checkpoint"),
                credit: value["credit"].as_u64(),
            }),
            Some("BlobRead") => Self::BlobRead(BlobRead {
                handle: text(value, "handle").ok_or(Malformed)?,
                offset: match &value["offset"] {
                    serde_json::Value::Null => Some(0),
                    other => other.as_u64(),
                },
                length: value["length"].as_u64(),
            }),
            Some("Dispatch") => Self::Dispatch(Dispatch {
                command_id: value["command_id"].as_str().map(str::to_owned),
            }),
            Some("Query") => Self::Query(Query {
                id: owned(value, "id"),
                op: owned(value, "op"),
            }),
            _ => Self::Other,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn open_carries_fence_credit_and_checkpoints() {
        let frame = GatewayFrame::from_value(&json!({
            "t":"Open","owner_fence":7,"byte_credit":4096,
            "checkpoints":{"mail":{"generation":"g","checkpoint":"c"},"notes":{}}
        }));
        let Ok(GatewayFrame::Open(open)) = frame else {
            panic!("not an Open: {frame:?}")
        };
        assert_eq!((open.owner_fence, open.byte_credit), (7, 4096));
        assert_eq!(
            open.checkpoints["mail"],
            CheckpointRef {
                generation: "g".into(),
                checkpoint: "c".into()
            }
        );
        assert_eq!(open.checkpoints["notes"].checkpoint, "");
    }

    #[test]
    fn committed_and_blob_read_need_their_keys() {
        assert_eq!(
            GatewayFrame::from_value(&json!({"t":"Committed","checkpoint":"c"})),
            Err(Malformed)
        );
        assert_eq!(
            GatewayFrame::from_value(&json!({"t":"BlobRead","handle":""})),
            Err(Malformed)
        );
        assert_eq!(
            GatewayFrame::from_value(
                &json!({"t":"Committed","source":"mail","checkpoint":"c","credit":9})
            ),
            Ok(GatewayFrame::Committed(Committed {
                source: "mail".into(),
                checkpoint: "c".into(),
                credit: Some(9)
            }))
        );
        assert_eq!(
            GatewayFrame::from_value(&json!({"t":"BlobRead","handle":"h","length":16})),
            Ok(GatewayFrame::BlobRead(BlobRead {
                handle: "h".into(),
                offset: Some(0),
                length: Some(16)
            }))
        );
        assert_eq!(
            GatewayFrame::from_value(&json!({"t":"BlobRead","handle":"h","offset":-1,"length":16})),
            Ok(GatewayFrame::BlobRead(BlobRead {
                handle: "h".into(),
                offset: None,
                length: Some(16)
            }))
        );
    }

    #[test]
    fn helper_frames_are_classified_and_captures_need_a_source() {
        assert_eq!(
            HelperFrame::from_value(
                &json!({"t":"Capture","source":"mail","checkpoint":"c","generation":"g"})
            ),
            Ok(HelperFrame::Capture(CaptureHeader {
                source: "mail".into(),
                checkpoint: "c".into(),
                generation: "g".into()
            }))
        );
        assert_eq!(
            HelperFrame::from_value(&json!({"t":"Capture"})),
            Err(Malformed)
        );
        assert_eq!(
            HelperFrame::from_value(&json!({"t":"Outcome"})),
            Ok(HelperFrame::Outcome)
        );
        assert_eq!(
            HelperFrame::from_value(&json!({"t":"Rejected"})),
            Ok(HelperFrame::Rejected)
        );
        assert_eq!(
            HelperFrame::from_value(&json!({"t":"Ready"})),
            Ok(HelperFrame::Ready)
        );
        assert_eq!(
            HelperFrame::from_value(&json!({"t":"QueryResult"})),
            Ok(HelperFrame::Other)
        );
    }

    #[test]
    fn query_dispatch_and_unknown_frames() {
        assert_eq!(
            GatewayFrame::from_value(&json!({"t":"Query","id":"q","op":"mail.body"})),
            Ok(GatewayFrame::Query(Query {
                id: "q".into(),
                op: "mail.body".into()
            }))
        );
        assert_eq!(
            GatewayFrame::from_value(&json!({"t":"Dispatch"})),
            Ok(GatewayFrame::Dispatch(Dispatch { command_id: None }))
        );
        assert_eq!(
            GatewayFrame::from_value(&json!({"t":"Prepare"})),
            Ok(GatewayFrame::Other)
        );
        assert_eq!(
            GatewayFrame::from_value(&json!({})),
            Ok(GatewayFrame::Other)
        );
    }
}
