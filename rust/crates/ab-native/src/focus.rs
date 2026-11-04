use crate::capture::{self, Context, Pending, Result, ScanInput, State};
use serde_json::{Value, json};

pub const SOURCE: &str = "focus";
pub const COLLECTION: &str = "focus.status";
pub const GENERATION: &str = "native/1";
pub const KEY: &str = "current";
pub const ALIAS: &str = "focus/status/current";
const MAX_ASSERTIONS: usize = 2 * 1024 * 1024;
const ASSERTIONS_VERSION: i64 = 8;
const STORE_KEYS: [&str; 3] = [
    "storeAssertionRecords",
    "storeInvalidationRecords",
    "storeInvalidationRequestRecords",
];

#[must_use]
pub fn asserted(bytes: &[u8]) -> Option<bool> {
    if bytes.len() > MAX_ASSERTIONS {
        return None;
    }
    let root: Value = serde_json::from_slice(bytes).ok()?;
    if root["header"]["version"].as_i64() != Some(ASSERTIONS_VERSION) {
        return None;
    }
    let [store] = root["data"].as_array()?.as_slice() else {
        return None;
    };
    let store = store.as_object()?;
    if store.keys().any(|key| !STORE_KEYS.contains(&key.as_str())) {
        return None;
    }
    match store.get(STORE_KEYS[0]) {
        None => Some(false),
        Some(records) => Some(!records.as_array()?.is_empty()),
    }
}

#[must_use]
pub fn status(fact: &Value, assertions: Option<&[u8]>) -> Value {
    let authorization = fact["authorization"].as_str().unwrap_or("not_determined");
    let focused = if authorization == "authorized" {
        assertions
            .and_then(asserted)
            .map_or_else(|| fact["center_focused"].clone(), Value::Bool)
    } else {
        Value::Null
    };
    json!({"focused": focused, "name": null, "authorization": authorization})
}

pub fn prepare(state: &State, data: Value, context: &Context, live: bool) -> Result<Pending> {
    state.prepare(ScanInput {
        source: SOURCE,
        generation: GENERATION,
        records: vec![capture::upsert(
            COLLECTION,
            KEY,
            Some(ALIAS.into()),
            data,
            context,
        )?],
        scope: vec![COLLECTION.into()],
        coverage: Value::Null,
        context,
        live,
    })
}

#[cfg(test)]
mod tests {
    use super::{asserted, status};
    use serde_json::json;

    fn file(store: &str, version: i64) -> Vec<u8> {
        format!("{{\"header\":{{\"version\":{version}}},\"data\":[{store}]}}").into_bytes()
    }

    #[test]
    fn the_assertions_file_decides_only_in_a_known_shape() {
        assert_eq!(asserted(&file("{}", 8)), Some(false));
        assert_eq!(
            asserted(&file("{\"storeAssertionRecords\":[{\"a\":1}]}", 8)),
            Some(true)
        );
        assert_eq!(
            asserted(&file("{\"storeAssertionRecords\":[]}", 8)),
            Some(false)
        );
        assert_eq!(
            asserted(&file("{\"storeInvalidationRecords\":[]}", 8)),
            Some(false)
        );
        assert_eq!(asserted(&file("{\"foreign\":[]}", 8)), None);
        assert_eq!(asserted(&file("{}", 9)), None);
        assert_eq!(asserted(b"{}"), None);
        assert_eq!(asserted(b"not json"), None);
    }

    #[test]
    fn authorization_gates_the_answer() {
        let on = file("{\"storeAssertionRecords\":[{}]}", 8);
        let authorized = json!({"authorization":"authorized","center_focused":false});
        assert_eq!(status(&authorized, Some(&on))["focused"], json!(true));
        assert_eq!(status(&authorized, None)["focused"], json!(false));
        assert_eq!(status(&authorized, Some(b"junk"))["focused"], json!(false));
        let denied = json!({"authorization":"denied"});
        assert_eq!(status(&denied, Some(&on))["focused"], json!(null));
        assert_eq!(status(&denied, Some(&on))["authorization"], json!("denied"));
    }
}
