use crate::Error;
use serde_json::{Value, json};
pub fn violations(db: &rusqlite::Connection) -> Result<Value, Error> {
    let shapes: Value = serde_json::from_str(include_str!("record_shapes.json"))?;
    let mut out = serde_json::Map::new();
    let mut stmt=db.prepare("SELECT payload FROM records WHERE deleted=0 AND ((collection>='messages.' AND collection<'messages/') OR collection='phone.call' OR (collection>='mac.' AND collection<'mac/') OR (collection>='safari.' AND collection<'safari/'))").map_err(|_| Error::Invalid)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(|_| Error::Invalid)?;
    for row in rows {
        let record: Value = serde_json::from_slice(&row.map_err(|_| Error::Invalid)?)?;
        let collection = record["collection"].as_str().ok_or(Error::Invalid)?;
        let Some(fields) = shapes[collection].as_object() else {
            continue;
        };
        let Some(data) = record["data"].as_object() else {
            increment(&mut out, format!("{collection}.data: not_object"));
            continue;
        };
        for (name, value) in data {
            let problem = match fields.get(name) {
                None => Some("unknown_field"),
                Some(field) if value.is_null() && field[2] != true => Some("unexpected_null"),
                Some(field)
                    if !value.is_null()
                        && !accepts(field[0].as_str().unwrap_or_default(), value) =>
                {
                    Some("wrong_type")
                }
                _ => None,
            };
            if let Some(problem) = problem {
                increment(&mut out, format!("{collection}.{name}: {problem}"));
            }
        }
        for (name, field) in fields {
            if field[1] == true && !data.contains_key(name) {
                increment(&mut out, format!("{collection}.{name}: missing_field"));
            }
        }
    }
    Ok(Value::Object(out))
}
fn increment(out: &mut serde_json::Map<String, Value>, key: String) {
    let count = out.get(&key).and_then(Value::as_u64).unwrap_or(0) + 1;
    out.insert(key, json!(count));
}
fn accepts(kind: &str, value: &Value) -> bool {
    match kind {
        "String" => value.is_string(),
        "Bool" => value.is_boolean(),
        "Number" => value.is_number(),
        "Array" => value.is_array(),
        "Object" => value.is_object(),
        "Time" => value
            .as_str()
            .is_some_and(|s| s.contains('T') && s.parse::<jiff::Timestamp>().is_ok()),
        _ => false,
    }
}
