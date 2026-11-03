use crate::{Args, Command, Error, Value, decode, s, shapes, sources};
use ab_wire::gojson::{self, EscapeMode};
#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(default)]
pub struct ActionContext {
    pub origin: String,
    pub native_key: String,
    pub list_key: String,
    pub expected_list_key: String,
    pub folder_key: String,
    pub resend_key: String,
    pub files: Vec<NativeFile>,
}
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NativeFile {
    pub name: String,
    pub path: String,
    pub digest: String,
}
fn json(v: &Value) -> Result<String, Error> {
    let raw = serde_json::to_vec(v).map_err(|_| Error::Invalid)?;
    String::from_utf8(gojson::record_data(&raw, EscapeMode::Html).map_err(|_| Error::Invalid)?)
        .map_err(|_| Error::Invalid)
}
fn field(out: &mut Vec<String>, key: &str, value: &str) {
    out.push(format!("\"{key}\":{value}"));
}
fn text(out: &mut Vec<String>, key: &str, value: &str) -> Result<(), Error> {
    if !value.is_empty() {
        field(out, key, &json(&Value::String(value.into()))?);
    }
    Ok(())
}
fn zero(v: &Value) -> bool {
    v.is_null() || v == "" || v == false || v == 0 || v.as_array().is_some_and(Vec::is_empty)
}
fn encode(a: &Args, shape: &str) -> Result<String, Error> {
    let mut out = Vec::new();
    for f in shapes::fields(shape) {
        let val = a.get(f.name);
        let pointer = f.kind.starts_with('*');
        if f.omit && val.is_none_or(|v| if pointer { v.is_null() } else { zero(v) }) {
            continue;
        }
        let default = if f.kind == "string" {
            Value::String(String::new())
        } else if f.kind == "bool" {
            Value::Bool(false)
        } else if f.kind == "int" || f.kind == "float64" {
            Value::from(0)
        } else {
            Value::Null
        };
        let v = val.unwrap_or(&default);
        let kind = f.kind.trim_start_matches('*');
        let encoded = if let Some(elem) = kind.strip_prefix("[]").filter(|s| *s != "string") {
            if let Some(arr) = v.as_array() {
                let entries = arr
                    .iter()
                    .map(|v| encode(v.as_object().ok_or(Error::Invalid)?, elem))
                    .collect::<Result<Vec<_>, _>>()?;
                format!("[{}]", entries.join(","))
            } else {
                "null".into()
            }
        } else {
            json(v)?
        };
        field(&mut out, f.name, &encoded);
    }
    Ok(format!("{{{}}}", out.join(",")))
}
pub fn native_action(c: &Command, ctx: &ActionContext) -> Result<Vec<u8>, Error> {
    let op = c.op.as_str();
    let shape = sources::shape(op);
    let contact = op.starts_with("contacts.");
    let a = if contact {
        serde_json::from_str::<Value>(c.args.get())
            .map_err(|_| Error::Invalid)?
            .as_object()
            .ok_or(Error::Invalid)?
            .clone()
    } else {
        decode(c.args.get(), shape)?
    };
    let mut fields = Vec::new();
    text(&mut fields, "origin", &ctx.origin)?;
    if op.starts_with("notes.") {
        field(&mut fields, "note_args", &encode(&a, "NoteArgs")?);
    }
    text(&mut fields, "folder_key", &ctx.folder_key)?;
    if op.starts_with("calendar.") {
        field(&mut fields, "calendar_args", &encode(&a, "CalendarArgs")?);
    }
    if op.starts_with("messages.") {
        let mut m = a.clone();
        m.remove("files");
        if !ctx.resend_key.is_empty() {
            m.insert("resend_of".into(), Value::String(ctx.resend_key.clone()));
        }
        let encoded = encode(&m, "MessagesArgs")?;
        let mut encoded = encoded.trim_end_matches('}').to_string();
        if !ctx.files.is_empty() {
            let files = ctx
                .files
                .iter()
                .map(|f| {
                    let a = Args::from_iter([
                        ("name".into(), Value::String(f.name.clone())),
                        ("path".into(), Value::String(f.path.clone())),
                        ("digest".into(), Value::String(f.digest.clone())),
                    ]);
                    encode(&a, "NativeMessageFile")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if encoded != "{" {
                encoded.push(',');
            }
            encoded.push_str("\"files\":[");
            encoded.push_str(&files.join(","));
            encoded.push(']');
        }
        encoded.push('}');
        field(&mut fields, "messages", &encoded);
    }
    text(&mut fields, "expected_list_key", &ctx.expected_list_key)?;
    field(&mut fields, "op", &json(&Value::String(c.op.clone()))?);
    let args = if op.starts_with("notes.") {
        "null".into()
    } else if contact {
        raw_json(c.args.get())
    } else if op.starts_with("calendar.") || op.starts_with("messages.") {
        let mut r = Args::new();
        if !s(&a, "id").is_empty() {
            r.insert("id".into(), Value::String(s(&a, "id").into()));
        }
        if op.starts_with("calendar.") && !s(&a, "calendar_id").is_empty() {
            r.insert("list_id".into(), Value::String(s(&a, "calendar_id").into()));
        }
        encode(&r, "ReminderArgs")?
    } else {
        encode(&a, shape)?
    };
    field(&mut fields, "args", &args);
    text(&mut fields, "native_key", &ctx.native_key)?;
    text(&mut fields, "list_key", &ctx.list_key)?;
    Ok(format!("{{{}}}", fields.join(",")).into_bytes())
}

fn raw_json(raw: &str) -> String {
    let mut result = String::new();
    let mut quoted = false;
    let mut escape = false;
    for c in raw.chars() {
        if escape {
            result.push(c);
            escape = false;
            continue;
        }
        if quoted && c == '\\' {
            result.push(c);
            escape = true;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
            result.push(c);
            continue;
        }
        if !quoted && c.is_ascii_whitespace() {
            continue;
        }
        match c {
            '<' => result.push_str("\\u003c"),
            '>' => result.push_str("\\u003e"),
            '&' => result.push_str("\\u0026"),
            '\u{2028}' => result.push_str("\\u2028"),
            '\u{2029}' => result.push_str("\\u2029"),
            _ => result.push(c),
        }
    }
    result
}
#[derive(Debug)]
pub struct PrepareContext<'a> {
    pub action: &'a ActionContext,
    pub principal: &'a str,
    pub request_hash: &'a str,
    pub expected_state: Option<&'a serde_json::value::RawValue>,
    pub expires_at: ab_wire::gojson::GoTime,
    pub owner_fence: u64,
}
pub fn prepare(c: &Command, ctx: &PrepareContext<'_>) -> Result<Vec<u8>, Error> {
    let mut fields = Vec::new();
    text(&mut fields, "origin", &ctx.action.origin)?;
    text(&mut fields, "command_id", &c.id)?;
    text(&mut fields, "principal", ctx.principal)?;
    text(&mut fields, "request_hash", ctx.request_hash)?;
    let action = String::from_utf8(native_action(c, ctx.action)?).map_err(|_| Error::Invalid)?;
    field(&mut fields, "action", &action);
    if let Some(expected) = ctx.expected_state {
        field(&mut fields, "expected_state", &raw_json(expected.get()));
    }
    field(
        &mut fields,
        "expires_at",
        &json(&Value::String(ctx.expires_at.as_str().into()))?,
    );
    field(&mut fields, "t", "\"Prepare\"");
    if ctx.owner_fence != 0 {
        field(&mut fields, "owner_fence", &ctx.owner_fence.to_string());
    }
    Ok(format!("{{{}}}", fields.join(",")).into_bytes())
}
