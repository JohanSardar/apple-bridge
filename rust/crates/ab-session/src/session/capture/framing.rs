use super::{BULK_CHUNK_BYTES, LIVE_CHUNK_BYTES};
use crate::{Error, frame::MAX_FRAME};
use serde_json::Value;

pub(super) fn frame_bytes(frame: &Value) -> Result<Vec<u8>, Error> {
    let body = ab_native::native_json::record_data(frame)
        .map_err(|error| Error::Reader(error.to_string()))?;
    if body.len() > MAX_FRAME {
        return Err(Error::Invalid);
    }
    let mut bytes = u32::try_from(body.len())
        .map_err(|_| Error::Invalid)?
        .to_be_bytes()
        .to_vec();
    bytes.extend(body);
    Ok(bytes)
}

pub(super) fn split(capture: &Value) -> Result<Vec<Value>, Error> {
    let limit = if capture["live"].as_bool().unwrap_or(false) {
        LIVE_CHUNK_BYTES
    } else {
        BULK_CHUNK_BYTES
    };
    let observations = capture["observations"].as_array().ok_or(Error::Invalid)?;
    let absent = |row: &&Value| row["kind"] == "absent";
    let ordered: Vec<&Value> = observations
        .iter()
        .filter(|row| !absent(row))
        .chain(observations.iter().filter(absent))
        .collect();
    let mut groups: Vec<Vec<Value>> = vec![Vec::new()];
    let mut bytes = 0;
    for row in ordered {
        let size = serde_json::to_vec(row)?.len() + 1;
        if bytes + size > limit && groups.last().is_some_and(|group| !group.is_empty()) {
            groups.push(Vec::new());
            bytes = 0;
        }
        if let Some(group) = groups.last_mut() {
            group.push(row.clone());
        }
        bytes += size;
    }
    Ok(groups
        .into_iter()
        .map(|group| {
            let mut chunk = capture.clone();
            chunk["observations"] = Value::Array(group);
            chunk
        })
        .collect())
}
