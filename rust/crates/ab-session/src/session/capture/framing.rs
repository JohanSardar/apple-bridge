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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chunks_keep_metadata_and_put_absences_after_present_records() -> Result<(), crate::Error> {
        let capture = json!({"source": "messages", "generation": "g1", "observations": [
            {"kind": "absent", "id": "gone"},
            {"kind": "present", "id": "first", "body": "x".repeat(10_000)},
            {"kind": "present", "id": "second", "body": "y".repeat(10_000)}
        ]});
        let chunks = split(&capture)?;
        assert_eq!(chunks.len(), 2);
        let ids: Vec<_> = chunks
            .iter()
            .flat_map(|chunk| chunk["observations"].as_array().into_iter().flatten())
            .map(|row| row["id"].clone())
            .collect();
        assert_eq!(ids, vec![json!("first"), json!("second"), json!("gone")]);
        for chunk in &chunks {
            assert_eq!(chunk["source"], "messages");
            assert_eq!(chunk["generation"], "g1");
        }
        let mut live = capture;
        live["live"] = json!(true);
        assert_eq!(split(&live)?.len(), 1);
        Ok(())
    }

    #[test]
    fn empty_captures_still_emit_a_chunk_and_missing_observations_fail() -> Result<(), crate::Error>
    {
        let empty = json!({"observations": []});
        assert_eq!(split(&empty)?, vec![empty]);
        assert!(split(&json!({})).is_err());
        Ok(())
    }
}
