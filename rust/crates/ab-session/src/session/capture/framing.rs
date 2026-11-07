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

fn encoded_size(frame: &Value) -> Result<usize, Error> {
    Ok(ab_native::native_json::record_data(frame)
        .map_err(|error| Error::Reader(error.to_string()))?
        .len())
}

fn referenced_handles(
    value: &Value,
    available: &serde_json::Map<String, Value>,
    selected: &mut serde_json::Map<String, Value>,
) {
    match value {
        Value::String(digest) => {
            if let Some(handle) = available.get(digest) {
                selected.insert(digest.clone(), handle.clone());
            }
        }
        Value::Array(values) => {
            for value in values {
                referenced_handles(value, available, selected);
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                referenced_handles(value, available, selected);
            }
        }
        _ => {}
    }
}

fn append(
    chunk: &mut Value,
    row: &Value,
    handles: &serde_json::Map<String, Value>,
) -> Result<(), Error> {
    chunk["observations"]
        .as_array_mut()
        .ok_or(Error::Invalid)?
        .push(row.clone());
    if let Some(selected) = chunk.get_mut("blob_handles") {
        selected.as_object_mut().ok_or(Error::Invalid)?.extend(
            handles
                .iter()
                .map(|(digest, handle)| (digest.clone(), handle.clone())),
        );
    }
    Ok(())
}

fn additional_size(
    chunk: &Value,
    row_size: usize,
    handles: &serde_json::Map<String, Value>,
) -> Result<usize, Error> {
    let comma = usize::from(
        chunk["observations"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()),
    );
    let mut added = row_size + comma;
    if let Some(current) = chunk.get("blob_handles").and_then(Value::as_object) {
        let extra: serde_json::Map<String, Value> = handles
            .iter()
            .filter(|(digest, _)| !current.contains_key(*digest))
            .map(|(digest, handle)| (digest.clone(), handle.clone()))
            .collect();
        if !extra.is_empty() {
            added += encoded_size(&Value::Object(extra))? - 2 + usize::from(!current.is_empty());
        }
    }
    Ok(added)
}

fn fresh(header: &Value, index: usize) -> Value {
    let mut chunk = header.clone();
    if let Some(checkpoint) = header["checkpoint"].as_str() {
        chunk["checkpoint"] = Value::String(format!("{checkpoint}-{index}"));
        if index != 0 && header.get("predecessor").is_some() {
            chunk["predecessor"] = Value::String(format!("{checkpoint}-{}", index - 1));
        }
    }
    chunk
}

pub(super) fn split(capture: &Value) -> Result<Vec<Value>, Error> {
    let limit = if capture["live"].as_bool().unwrap_or(false) {
        LIVE_CHUNK_BYTES
    } else {
        BULK_CHUNK_BYTES
    };
    let observations = capture["observations"].as_array().ok_or(Error::Invalid)?;
    let empty = serde_json::Map::new();
    let handles = match capture.get("blob_handles") {
        None | Some(Value::Null) => &empty,
        Some(Value::Array(values)) if values.is_empty() => &empty,
        Some(Value::Object(handles)) => handles,
        _ => return Err(Error::Invalid),
    };
    let mut header: serde_json::Map<String, Value> = capture
        .as_object()
        .ok_or(Error::Invalid)?
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "observations" | "blob_handles"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    header.insert("observations".into(), Value::Array(Vec::new()));
    if capture.get("blob_handles").is_some() {
        header.insert("blob_handles".into(), Value::Object(serde_json::Map::new()));
    }
    let header = Value::Object(header);
    let mut groups = Vec::new();
    let mut current = fresh(&header, 0);
    let mut complete_bytes = encoded_size(&current)?;
    let mut row_bytes = 0;
    let absent = |row: &&Value| row["kind"] == "absent";
    for row in observations
        .iter()
        .filter(|row| !absent(row))
        .chain(observations.iter().filter(absent))
    {
        let size = encoded_size(row)?;
        let mut selected = serde_json::Map::new();
        referenced_handles(row, handles, &mut selected);
        let mut added = additional_size(&current, size, &selected)?;
        if (row_bytes + size + 1 > limit || complete_bytes + added > MAX_FRAME)
            && current["observations"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        {
            groups.push(current);
            current = fresh(&header, groups.len());
            complete_bytes = encoded_size(&current)?;
            row_bytes = 0;
            added = additional_size(&current, size, &selected)?;
        }
        if complete_bytes + added > MAX_FRAME {
            return Err(Error::Invalid);
        }
        append(&mut current, row, &selected)?;
        complete_bytes += added;
        row_bytes += size + 1;
    }
    groups.push(current);
    if let Some(checkpoint) = capture.get("checkpoint")
        && let Some(last) = groups.last_mut()
    {
        last["checkpoint"] = checkpoint.clone();
    }
    for chunk in &groups {
        if encoded_size(chunk)? > MAX_FRAME {
            return Err(Error::Invalid);
        }
    }
    Ok(groups)
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
    fn chunks_budget_native_json_escaping_before_grouping() -> Result<(), crate::Error> {
        let capture = json!({"source":"voicemail", "live":true, "observations":[
            {"kind":"upsert", "id":"one", "text":"<".repeat(16_000)},
            {"kind":"upsert", "id":"two", "text":"<".repeat(16_000)},
            {"kind":"upsert", "id":"three", "text":"<".repeat(16_000)}
        ]});
        let chunks = split(&capture)?;
        assert_eq!(chunks.len(), 3);
        for chunk in chunks {
            assert!(frame_bytes(&chunk).is_ok());
        }
        Ok(())
    }

    #[test]
    fn chunks_keep_only_referenced_handles_and_fit_the_complete_wire_frame()
    -> Result<(), crate::Error> {
        for source in ["voicemail", "voice_memos", "notes"] {
            let mut capture = json!({"source":source,"t":"Capture","generation":"g","checkpoint":"scan","predecessor":"previous","live":true,"observations":[],"blob_handles":{}});
            for index in 0..2000 {
                let digest = format!("{index:064x}");
                capture["observations"].as_array_mut().ok_or(Error::Invalid)?.push(json!({"kind":"upsert","id":index.to_string(),"data":{"assets":[{"digest":digest}]}}));
                capture["blob_handles"][&digest] = json!(format!("{source}-blob:{digest}"));
            }
            let chunks = split(&capture)?;
            let mut seen = std::collections::BTreeSet::new();
            for chunk in chunks {
                assert!(frame_bytes(&chunk)?.len() <= MAX_FRAME + 4);
                let rows = chunk["observations"].as_array().ok_or(Error::Invalid)?;
                let handles = chunk["blob_handles"].as_object().ok_or(Error::Invalid)?;
                assert_eq!(handles.len(), rows.len());
                for row in rows {
                    let digest = row["data"]["assets"][0]["digest"]
                        .as_str()
                        .ok_or(Error::Invalid)?;
                    assert!(handles.contains_key(digest));
                    seen.insert(digest.to_owned());
                }
            }
            assert_eq!(seen.len(), 2000);
        }
        Ok(())
    }

    #[test]
    fn chunk_budgets_include_handles_and_capture_metadata() -> Result<(), crate::Error> {
        let digest = "a".repeat(64);
        let capture = json!({"checkpoint":"scan","predecessor":"before","scope":["<".repeat(30_000)],"observations":[
            {"kind":"upsert","data":{"digest":digest,"text":"x".repeat(30_000)}},
            {"kind":"upsert","data":{"digest":digest,"text":"y".repeat(30_000)}}
        ],"live":true,"blob_handles":{digest:"h".repeat(40_000)}});
        let chunks = split(&capture)?;
        assert_eq!(chunks.len(), 2);
        for chunk in chunks {
            assert!(frame_bytes(&chunk).is_ok());
        }
        Ok(())
    }

    #[test]
    fn legacy_empty_handle_maps_do_not_interrupt_other_readers() -> Result<(), crate::Error> {
        for handles in [json!([]), Value::Null, json!({})] {
            let chunks = split(
                &json!({"observations":[{"kind":"upsert","data":{}}],"blob_handles":handles}),
            )?;
            assert_eq!(chunks[0]["blob_handles"], json!({}));
        }
        assert!(split(&json!({"observations":[],"blob_handles":["invalid"]})).is_err());
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
