use crate::{
    DecodeError, body_digest,
    plist::{self, Value},
    typedstream::{self, MessageBody},
};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageEdit {
    pub part_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
    pub body: MessageBody,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub edit_history: Vec<MessageEdit>,
    pub retracted_parts: Vec<i64>,
    pub decode_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_body_digest: Option<String>,
}

fn parse(data: &[u8]) -> Result<MessageSummary, DecodeError> {
    let Value::Dictionary(root) = plist::decode(data)? else {
        return Err(DecodeError::Malformed);
    };
    let mut edits = Vec::new();
    if let Some(field) = root.get("ec") {
        let histories = field.dictionary().ok_or(DecodeError::Malformed)?;
        for (key, history) in histories {
            let part = key.parse::<usize>().map_err(|_| DecodeError::Malformed)?;
            let Value::Array(history) = history else {
                return Err(DecodeError::Malformed);
            };
            for edit in history {
                let edit = edit.dictionary().ok_or(DecodeError::Malformed)?;
                let Some(Value::Data(bytes)) = edit.get("t") else {
                    return Err(DecodeError::Malformed);
                };
                edits.push(MessageEdit {
                    part_index: part,
                    timestamp: edit.get("d").and_then(Value::number),
                    body: typedstream::decode(bytes),
                });
            }
        }
    }
    let mut retracted = match root.get("rp") {
        None => Vec::new(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|v| match v {
                Value::Integer(n) => Ok(*n),
                Value::Bool(b) => Ok(i64::from(*b)),
                _ => Err(DecodeError::Malformed),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::Dictionary(parts)) => parts
            .keys()
            .map(|k| {
                k.parse::<i64>()
                    .ok()
                    .filter(|n| *n >= 0)
                    .ok_or(DecodeError::Malformed)
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(DecodeError::Malformed),
    };
    retracted.sort_unstable();
    let complete = root
        .keys()
        .all(|k| ["ec", "rp", "otr", "ust", "amc", "ams", "ep"].contains(&k.as_str()))
        && edits.iter().all(|e| e.body.decode_status == "decoded");
    Ok(MessageSummary {
        edit_history: edits,
        retracted_parts: retracted,
        decode_status: if complete { "decoded" } else { "partial" }.into(),
        raw_body_digest: if complete {
            None
        } else {
            Some(body_digest(data))
        },
    })
}

pub fn decode(data: &[u8]) -> MessageSummary {
    parse(data).unwrap_or_else(|_| MessageSummary {
        edit_history: Vec::new(),
        retracted_parts: Vec::new(),
        decode_status: "undecoded".into(),
        raw_body_digest: Some(body_digest(data)),
    })
}
