use serde_json::Value;

const KEPT_FIELDS: [&str; 2] = ["media", "derivatives"];

fn digest(blob: Option<&Value>) -> Option<&str> {
    blob?
        .get("digest")?
        .as_str()
        .filter(|digest| !digest.is_empty())
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
        if !fields.contains_key(name)
            && let Some(value) = old.get(name)
        {
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
    fn same_digest_keeps_derivatives_only_when_absent() {
        let mut next = json!({"blob":{"state":"present","digest":"aa"}});
        keep_known(Some(&old()), &mut next);
        assert_eq!(next["derivatives"], old()["derivatives"]);
        let mut next =
            json!({"blob":{"state":"present","digest":"aa"},"media":{},"derivatives":{}});
        keep_known(Some(&old()), &mut next);
        assert_eq!(next["media"], json!({}));
        assert_eq!(next["derivatives"], json!({}));
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
