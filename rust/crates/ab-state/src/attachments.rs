use crate::{Collection, Connection, Record, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn availability(blob: &mut Value, entries: &Value) {
    if !blob.is_object() {
        *blob = json!({"state":"missing","reason":"attachment_metadata_unavailable"});
        return;
    }
    let digest = digest(Some(blob));
    let entry = digest.and_then(|digest| entries.get(digest));
    let source_state = state(Some(blob));
    let claims_local = matches!(source_state, "present" | "available");
    let (state, reason) = match entry.and_then(|entry| entry["state"].as_str()) {
        Some("present") => ("present", None),
        _ if source_state == "offloaded" => ("offloaded", None),
        Some("pending") => ("pending", None),
        None if digest.is_some() && claims_local => ("pending", None),
        None if claims_local || source_state == "pending" => {
            ("missing", Some("attachment_digest_unavailable"))
        }
        Some("offloaded") => ("offloaded", None),
        Some(state @ ("failed" | "over_cap" | "disk_full" | "missing")) => {
            let Some(entry) = entry else { return };
            let reason = if entry["corruptions"].as_u64().is_some_and(|n| n > 0) {
                Some("digest_mismatch")
            } else {
                entry["reason"].as_str().filter(|reason| !reason.is_empty())
            };
            (state, reason)
        }
        _ => return,
    };
    if let Some(reason) = reason {
        blob["reason"] = json!(reason);
    } else if let Some(fields) = blob.as_object_mut() {
        fields.remove("reason");
    }
    blob["state"] = json!(state);
}

pub(crate) fn project_record(record: &mut Record, entries: &Value) -> Result<()> {
    if record.collection == Collection::MessagesAttachment && !record.deleted {
        let mut data: Value = serde_json::from_str(record.data.get())?;
        project_data(&mut data, entries);
        record.data = serde_json::value::to_raw_value(&data)?;
    }
    Ok(())
}

fn project_data(data: &mut Value, entries: &Value) {
    if let Some(fields) = data.as_object_mut() {
        availability(fields.entry("blob").or_insert(Value::Null), entries);
    }
}

pub(crate) fn project_items(result: &mut Value, entries: &Value) {
    for item in result["items"].as_array_mut().into_iter().flatten() {
        if item["collection"] == "messages.attachment" {
            project_data(&mut item["data"], entries);
        }
        for attachment in item
            .get_mut("attachments")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            project_data(&mut attachment["data"], entries);
        }
    }
}

pub(crate) fn counts(db: &Connection, entries: &Value) -> Result<Value> {
    let mut counts = [0_u64; 4];
    let mut failures = BTreeMap::<String, u64>::new();
    let mut stmt = db.prepare_cached(
        "SELECT json_extract(CAST(payload AS TEXT),'$.data.blob') FROM records WHERE collection='messages.attachment' AND source='messages' AND deleted=0",
    )?;
    for row in stmt.query_map([], |row| row.get::<_, Option<String>>(0))? {
        let mut blob = row?.map_or(Ok(Value::Null), |raw| serde_json::from_str(&raw))?;
        availability(&mut blob, entries);
        counts[match state(Some(&blob)) {
            "present" => 0,
            "offloaded" => 1,
            "pending" => 2,
            other => {
                let reason = blob["reason"].as_str().unwrap_or(other);
                *failures.entry(reason.to_owned()).or_default() += 1;
                3
            }
        }] += 1;
    }
    Ok(
        json!({"present":counts[0],"offloaded":counts[1],"pending":counts[2],"failed":counts[3],"total":counts.iter().sum::<u64>(),"failure_reasons":failures}),
    )
}

const KEPT_FIELDS: [&str; 2] = ["media", "derivatives"];

fn digest(blob: Option<&Value>) -> Option<&str> {
    blob?
        .get("digest")?
        .as_str()
        .filter(|digest| !digest.is_empty())
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn state(blob: Option<&Value>) -> &str {
    blob.and_then(|blob| blob.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

pub(crate) fn keep_known(old: Option<&Value>, next: &mut Value) {
    let Some(old) = old else {
        return;
    };
    let Some(known) = digest(old.get("blob")) else {
        return;
    };
    let known = known.to_owned();
    let Some(fields) = next.as_object_mut() else {
        return;
    };
    if digest(fields.get("blob")).is_none()
        && matches!(state(fields.get("blob")), "offloaded" | "missing")
        && let Some(blob) = fields.get_mut("blob").and_then(Value::as_object_mut)
    {
        blob.insert("digest".into(), Value::String(known.clone()));
    }
    if digest(fields.get("blob")) != Some(known.as_str()) {
        return;
    }
    for name in KEPT_FIELDS {
        let omitted = fields.get(name).is_none_or(is_empty_object);
        if omitted && let Some(value) = old.get(name).filter(|value| !is_empty_object(value)) {
            fields.insert(name.into(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn old() -> Value {
        json!({
            "blob": {"state":"present","digest":"aa"},
            "media": {"width": 10},
            "derivatives": {"thumb": {"digest":"bb","state":"present"}}
        })
    }

    #[test]
    fn offloaded_keeps_digest_and_derivatives() {
        let mut next = json!({"blob":{"state":"offloaded"}});
        keep_known(Some(&old()), &mut next);
        assert_eq!(next["blob"], json!({"state":"offloaded","digest":"aa"}));
        assert_eq!(next["media"], old()["media"]);
        assert_eq!(next["derivatives"], old()["derivatives"]);
    }

    #[test]
    fn same_digest_keeps_derivatives_the_incoming_record_omits_or_empties() {
        let mut next = json!({"blob":{"state":"present","digest":"aa"}});
        keep_known(Some(&old()), &mut next);
        assert_eq!(next["derivatives"], old()["derivatives"]);
        let mut next =
            json!({"blob":{"state":"present","digest":"aa"},"media":{},"derivatives":{}});
        keep_known(Some(&old()), &mut next);
        assert_eq!(next["media"], old()["media"]);
        assert_eq!(next["derivatives"], old()["derivatives"]);
    }

    #[test]
    fn a_new_non_empty_answer_replaces_what_was_known() {
        let mut next = json!({
            "blob":{"state":"present","digest":"aa"},
            "media":{"width":20},
            "derivatives":{"display":{"digest":"cc","state":"present"}}
        });
        keep_known(Some(&old()), &mut next);
        assert_eq!(next["media"], json!({"width":20}));
        assert_eq!(
            next["derivatives"],
            json!({"display":{"digest":"cc","state":"present"}})
        );
    }

    #[test]
    fn changed_or_pending_blob_keeps_nothing() {
        let mut next = json!({"blob":{"state":"present","digest":"cc"}});
        keep_known(Some(&old()), &mut next);
        assert_eq!(next, json!({"blob":{"state":"present","digest":"cc"}}));
        let mut next = json!({"blob":{"state":"pending"}});
        keep_known(Some(&old()), &mut next);
        assert_eq!(next, json!({"blob":{"state":"pending"}}));
    }

    #[test]
    fn nothing_known_changes_nothing() {
        let mut next = json!({"blob":{"state":"offloaded"}});
        keep_known(None, &mut next);
        assert_eq!(next, json!({"blob":{"state":"offloaded"}}));
        keep_known(Some(&json!({"blob":{"state":"offloaded"}})), &mut next);
        assert_eq!(next, json!({"blob":{"state":"offloaded"}}));
    }
}
