use crate::{Code, Error};
use serde_json::{Value, value::RawValue};
fn emit(value: &Value, kind: &str, shapes: &Value, out: &mut String) -> Result<(), Error> {
    if let Some(kind) = kind.strip_prefix("map[string]")
        && let Some(object) = value.as_object()
    {
        out.push('{');
        for (i, (key, child)) in object.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&serde_json::to_string(key).map_err(|_| Code::Corrupt)?);
            out.push(':');
            emit(child, kind, shapes, out)?;
        }
        out.push('}');
        return Ok(());
    }
    if let Some(items) = value.as_array() {
        out.push('[');
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            emit(item, kind.trim_start_matches("[]"), shapes, out)?;
        }
        out.push(']');
    } else if let (Some(object), Some(fields)) = (value.as_object(), shapes[kind].as_array()) {
        out.push('{');
        let mut first = true;
        let mut ordered = Vec::new();
        for field in fields {
            if field[0] == "@" {
                if let Some(embedded) = shapes[field[1].as_str().unwrap_or_default()].as_array() {
                    ordered.extend(embedded.iter());
                }
            } else {
                ordered.push(field);
            }
        }
        for field in ordered {
            let key = field[0].as_str().ok_or(Code::Corrupt)?;
            if let Some(child) = object.get(key) {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(&serde_json::to_string(key).map_err(|_| Code::Corrupt)?);
                out.push(':');
                emit(child, field[1].as_str().ok_or(Code::Corrupt)?, shapes, out)?;
            }
        }
        out.push('}');
    } else {
        out.push_str(&serde_json::to_string(value).map_err(|_| Code::Corrupt)?);
    }
    Ok(())
}
pub fn read_model(name: &str, value: &Value) -> Result<Box<RawValue>, Error> {
    let kind = match name {
        "health.summary" => "HealthSummary",
        "messages.chats" => "ChatsResult",
        "messages.thread" => "ThreadResult",
        "messages.handles" => "HandlesResult",
        "messages.search" => "MessageSearchResult",
        "messages.collection" => "CollectionResult",
        "contacts.list" => "ContactsListResult",
        "notes.list" => "NotesListResult",
        "reminders.lists" => "ReminderListsResult",
        "reminders.items" => "ReminderItemsResult",
        "calendar.range" => "CalendarRangeResult",
        _ => "ReadResult",
    };
    let mut shapes: Value =
        serde_json::from_str(include_str!("legacy_shapes.json")).map_err(|_| Code::Corrupt)?;
    if name == "replica.status"
        && let Some(fields) = shapes["Record"].as_array_mut()
    {
        for field in fields {
            if field[0] == "data" {
                field[1] = Value::String("ReplicaStatus".into());
            }
        }
    }
    let mut out = String::new();
    emit(value, kind, &shapes, &mut out)?;
    RawValue::from_string(out).map_err(|_| Code::Corrupt.into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn replica_status_carries_the_last_feed_error() {
        let shaped = super::read_model(
            "sources.status",
            &json!({"records": [], "replica": {"role": "replica", "last_error": "the gateway refused the feed"}}),
        )
        .map_err(|_| "shape")
        .unwrap_or_else(|error| panic!("{error}"));
        let shaped: serde_json::Value =
            serde_json::from_str(shaped.get()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            shaped["replica"]["last_error"],
            "the gateway refused the feed"
        );
    }
}
