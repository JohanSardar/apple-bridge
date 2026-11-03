use crate::{Code, Error};
use serde_json::{Value, value::RawValue};
use std::{collections::HashMap, sync::OnceLock};
struct Field {
    key: String,
    name: String,
    kind: String,
}
type Shapes = HashMap<String, Vec<Field>>;
fn compile(shapes: &Value) -> Result<Shapes, Error> {
    let mut out = Shapes::new();
    for (kind, fields) in shapes.as_object().ok_or(Code::Corrupt)? {
        let Some(fields) = fields.as_array() else {
            continue;
        };
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
        let mut compiled = Vec::with_capacity(ordered.len());
        for field in ordered {
            let name = field[0].as_str().ok_or(Code::Corrupt)?;
            let key = format!(
                "{}:",
                serde_json::to_string(name).map_err(|_| Code::Corrupt)?
            );
            compiled.push(Field {
                key,
                name: name.to_owned(),
                kind: field[1].as_str().ok_or(Code::Corrupt)?.to_owned(),
            });
        }
        out.insert(kind.clone(), compiled);
    }
    Ok(out)
}
fn shapes(replica: bool) -> Result<&'static Shapes, Error> {
    static PLAIN: OnceLock<Result<Shapes, Code>> = OnceLock::new();
    static REPLICA: OnceLock<Result<Shapes, Code>> = OnceLock::new();
    let cell = if replica { &REPLICA } else { &PLAIN };
    cell.get_or_init(|| {
        let mut shapes: Value = serde_json::from_str(include_str!("legacy_shapes.json"))
            .map_err(|_| Code::Corrupt)?;
        if replica && let Some(fields) = shapes["Record"].as_array_mut() {
            for field in fields {
                if field[0] == "data" {
                    field[1] = Value::String("ReplicaStatus".into());
                }
            }
        }
        compile(&shapes).map_err(|e| e.code)
    })
    .as_ref()
    .map_err(|code| Error::from(*code))
}
fn leaf(value: &Value, out: &mut Vec<u8>) -> Result<(), Error> {
    serde_json::to_writer(out, value).map_err(|_| Code::Corrupt.into())
}
fn emit(value: &Value, kind: &str, shapes: &Shapes, out: &mut Vec<u8>) -> Result<(), Error> {
    if let Some(kind) = kind.strip_prefix("map[string]")
        && let Some(object) = value.as_object()
    {
        out.push(b'{');
        for (i, (key, child)) in object.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            serde_json::to_writer(&mut *out, key).map_err(|_| Code::Corrupt)?;
            out.push(b':');
            emit(child, kind, shapes, out)?;
        }
        out.push(b'}');
        return Ok(());
    }
    if let Some(items) = value.as_array() {
        out.push(b'[');
        let kind = kind.trim_start_matches("[]");
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            emit(item, kind, shapes, out)?;
        }
        out.push(b']');
    } else if let (Some(object), Some(fields)) = (value.as_object(), shapes.get(kind)) {
        out.push(b'{');
        let mut first = true;
        for field in fields {
            if let Some(child) = object.get(&field.name) {
                if !first {
                    out.push(b',');
                }
                first = false;
                out.extend_from_slice(field.key.as_bytes());
                emit(child, &field.kind, shapes, out)?;
            }
        }
        out.push(b'}');
    } else {
        leaf(value, out)?;
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
    let shapes = shapes(name == "replica.status")?;
    let mut out = Vec::with_capacity(4096);
    emit(value, kind, shapes, &mut out)?;
    RawValue::from_string(String::from_utf8(out).map_err(|_| Code::Corrupt)?)
        .map_err(|_| Code::Corrupt.into())
}
