mod action;
mod shapes;
mod sources;
pub use action::{ActionContext, NativeFile, PrepareContext, native_action, prepare};
use base64::{
    Engine,
    engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig},
};
use jiff::Timestamp;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid input")]
    Invalid,
    #[error("operation unsupported")]
    Unsupported,
    #[error("file exceeds upload limit")]
    TooLarge,
    #[error("credential expired")]
    Expired,
}
impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
            Self::TooLarge => "too_large",
            Self::Expired => "expired",
        }
    }
}
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Command {
    #[serde(default, rename = "command_id")]
    pub id: String,
    pub op: String,
    pub args: Box<serde_json::value::RawValue>,
    #[serde(default)]
    pub expected_version: u64,
    #[serde(default)]
    pub wait_ms: i64,
}
#[derive(Debug, Clone)]
pub struct Validated {
    pub id: String,
    pub list_id: String,
    pub folder_id: String,
}
#[derive(Debug, Clone, serde::Serialize, thiserror::Error)]
#[error("{field}: {problem}")]
pub struct Violation {
    pub code: &'static str,
    pub field: String,
    pub problem: &'static str,
    pub expected: &'static str,
    pub hint: &'static str,
    pub retryable: bool,
}

impl Violation {
    fn invalid(field: impl Into<String>, problem: &'static str, expected: &'static str) -> Self {
        Self {
            code: "invalid",
            field: field.into(),
            problem,
            expected,
            hint: "Correct the input before retrying.",
            retryable: false,
        }
    }

    fn legacy(&self) -> Error {
        match self.code {
            "unsupported" => Error::Unsupported,
            "too_large" => Error::TooLarge,
            "expired" => Error::Expired,
            _ => Error::Invalid,
        }
    }
}

impl From<Error> for Violation {
    fn from(error: Error) -> Self {
        Self {
            code: error.code(),
            ..Self::invalid(
                "args",
                "arguments violate operation constraints",
                "valid arguments for the operation",
            )
        }
    }
}

#[derive(Debug)]
struct Field {
    name: &'static str,
    kind: &'static str,
    omit: bool,
}
type Args = Map<String, Value>;
fn s<'a>(a: &'a Args, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("")
}
fn n(a: &Args, k: &str) -> f64 {
    a.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}
fn p(a: &Args, k: &str) -> bool {
    a.get(k).is_some_and(|v| !v.is_null())
}
fn b(a: &Args, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn arr<'a>(a: &'a Args, k: &str) -> &'a [Value] {
    a.get(k)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}
fn check(ok: bool) -> Result<(), Error> {
    if ok { Ok(()) } else { Err(Error::Invalid) }
}
struct UniqueJson(Value);

impl<'de> serde::Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON with unique case-insensitive object keys")
            }

            fn visit_bool<E>(self, value: bool) -> Result<UniqueJson, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_i64<E>(self, value: i64) -> Result<UniqueJson, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<UniqueJson, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<UniqueJson, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_str<E>(self, value: &str) -> Result<UniqueJson, E> {
                Ok(UniqueJson(value.into()))
            }

            fn visit_unit<E>(self) -> Result<UniqueJson, E> {
                Ok(UniqueJson(Value::Null))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<UniqueJson, A::Error> {
                let mut items = Vec::new();
                while let Some(UniqueJson(value)) = seq.next_element()? {
                    items.push(value);
                }
                Ok(UniqueJson(Value::Array(items)))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<UniqueJson, A::Error> {
                let mut seen = std::collections::HashSet::new();
                let mut fields = Args::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.to_ascii_lowercase()) {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                    let UniqueJson(value) = map.next_value()?;
                    fields.insert(key, value);
                }
                Ok(UniqueJson(Value::Object(fields)))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

pub fn parse_arguments(raw: &str) -> Result<Value, Violation> {
    serde_json::from_str::<UniqueJson>(raw)
        .map(|UniqueJson(value)| value)
        .map_err(|_| {
            Violation::invalid(
                "args",
                "invalid or duplicate JSON fields",
                "JSON with unique case-insensitive object keys",
            )
        })
}

fn decode(raw: &str, shape: &str) -> Result<Args, Violation> {
    let Value::Object(fields) = parse_arguments(raw)? else {
        return Err(Violation::invalid(
            "args",
            "arguments are not an object",
            "an object",
        ));
    };
    decode_fields(fields, shape, "args")
}

fn decode_fields(
    entries: impl IntoIterator<Item = (String, Value)>,
    shape: &str,
    path: &str,
) -> Result<Args, Violation> {
    let fields = shapes::fields(shape);
    let mut out = Args::new();
    for (key, value) in entries {
        let field_path = format!("{path}.{key}");
        let field = fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case(&key))
            .ok_or_else(|| {
                Violation::invalid(
                    field_path.clone(),
                    "field is unknown",
                    "a field supported by the operation",
                )
            })?;
        if value.is_null() {
            continue;
        }
        let kind = field.kind.trim_start_matches('*');
        let invalid = || Violation::invalid(field_path.clone(), "field has the wrong type", kind);
        let value = match kind {
            "raw" => value,
            "string" if value.is_string() => value,
            "bool" if value.is_boolean() => value,
            "int" if value.as_i64().is_some() => value,
            "float64" if value.is_number() => value,
            _ if kind.starts_with("[]") => {
                let Value::Array(items) = value else {
                    return Err(invalid());
                };
                let element = &kind[2..];
                let mut decoded = Vec::with_capacity(items.len());
                for (index, item) in items.into_iter().enumerate() {
                    let item_path = format!("{field_path}[{index}]");
                    let value = match (element, item) {
                        ("string", Value::Null) => Value::String(String::new()),
                        ("string", item @ Value::String(_)) => item,
                        ("string", _) => {
                            return Err(Violation::invalid(
                                item_path,
                                "item has the wrong type",
                                "a string",
                            ));
                        }
                        (_, Value::Null) => Value::Object(Args::new()),
                        (_, Value::Object(item)) => {
                            Value::Object(decode_fields(item, element, &item_path)?)
                        }
                        _ => {
                            return Err(Violation::invalid(
                                item_path,
                                "item has the wrong type",
                                "an object",
                            ));
                        }
                    };
                    decoded.push(value);
                }
                Value::Array(decoded)
            }
            _ => return Err(invalid()),
        };
        out.insert(field.name.into(), value);
    }
    Ok(out)
}
pub fn valid_command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
}
pub fn command_ttl(op: &str) -> (i64, i64) {
    if op.starts_with("messages.") {
        (60, 300)
    } else {
        (300, 3600)
    }
}
pub fn expiry(op: &str, now: Timestamp, requested: Option<Timestamp>) -> Result<Timestamp, Error> {
    let (ttl, max) = command_ttl(op);
    let end = requested.unwrap_or(
        now.checked_add(jiff::SignedDuration::from_secs(ttl))
            .map_err(|_| Error::Invalid)?,
    );
    if end <= now || end.as_nanosecond() - now.as_nanosecond() > i128::from(max) * 1_000_000_000 {
        return Err(Error::Expired);
    }
    Ok(end)
}
fn timestamp(s: &str) -> bool {
    s.parse::<Timestamp>().is_ok()
}
fn base64(s: &str) -> Result<Vec<u8>, Error> {
    GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
    )
    .decode(s.replace(['\r', '\n'], ""))
    .map_err(|_| Error::Invalid)
}
pub fn validate(c: &Command, now: Timestamp) -> Result<Validated, Error> {
    validate_request(c, now).map_err(|violation| violation.legacy())
}

pub fn validate_request(c: &Command, now: Timestamp) -> Result<Validated, Violation> {
    if !valid_command_id(&c.id) {
        return Err(Violation::invalid(
            "command_id",
            "invalid command ID",
            "1 to 128 letters, digits or ._:- characters",
        ));
    }
    if !(0..=15000).contains(&c.wait_ms) {
        return Err(Violation::invalid(
            "wait_ms",
            "wait is outside the supported range",
            "0 to 15000 milliseconds",
        ));
    }
    let a = sources::validate(c, now)?;
    Ok(Validated {
        id: s(&a, "id").into(),
        folder_id: s(&a, "folder_id").into(),
        list_id: if c.op.starts_with("calendar.") {
            s(&a, "calendar_id")
        } else {
            s(&a, "list_id")
        }
        .into(),
    })
}
pub fn note_html_compatible(raw: &[u8], html: &str) -> bool {
    let Ok(Value::Object(a)) = serde_json::from_slice::<Value>(raw) else {
        return false;
    };
    if s(&a, "html").is_empty()
        || s(&a, "content_status") != "complete"
        || s(&a, "block_status") != "complete"
        || s(&a, "round_trip_status") != "unproven"
        || !arr(&a, "attachments").is_empty()
        || html.len() > 1_000_000
        || html.contains('\0')
    {
        return false;
    }
    if [
        "data-notes-read-only",
        "<table",
        "<ul",
        "<ol",
        "<pre",
        "<blockquote",
    ]
    .iter()
    .any(|m| s(&a, "html").contains(m))
    {
        return false;
    }
    let Ok(re) = regex::Regex::new(r"<[^>]*>|[^<]+") else {
        return false;
    };
    let structure = |input: &str| -> Option<Vec<String>> {
        let tokens = re.find_iter(input).map(|m| m.as_str()).collect::<Vec<_>>();
        if tokens.concat() != input {
            return None;
        }
        Some(
            tokens
                .into_iter()
                .map(|t| {
                    if t.starts_with('<') {
                        t.into()
                    } else {
                        "text".into()
                    }
                })
                .collect(),
        )
    };
    structure(s(&a, "html")).is_some_and(|v| Some(v) == structure(html))
}
pub fn clock_estimate(t: [Timestamp; 4]) -> Result<(i128, i128), Error> {
    let [t0, t1, t2, t3] = t.map(Timestamp::as_nanosecond);
    check(t3 >= t0 && t2 >= t1 && t.iter().all(|t| t.to_string() != "0001-01-01T00:00:00Z"))?;
    let rtt = (t3 - t0) - (t2 - t1);
    check(rtt >= 0)?;
    Ok((i128::midpoint(t1 - t0, t2 - t3), rtt / 2))
}
