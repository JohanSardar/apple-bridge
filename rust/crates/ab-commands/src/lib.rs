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
#[derive(Debug)]
struct Ordered(Vec<(String, Value)>);
impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Ordered, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}
fn decode(raw: &str, shape: &str) -> Result<Args, Error> {
    let Ordered(entries) = serde_json::from_str::<Ordered>(raw).map_err(|_| Error::Invalid)?;
    let mut out = Args::new();
    for (key, value) in entries {
        let f = shapes::fields(shape)
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(&key))
            .ok_or(Error::Invalid)?;
        if value.is_null() {
            if f.kind.starts_with('*') || f.kind.starts_with("[]") {
                out.remove(f.name);
            }
            continue;
        }
        let single = Value::Object(Args::from_iter([(key, value)]));
        out.extend(normalize(&single, shape)?);
    }
    Ok(out)
}
fn normalize(v: &Value, shape: &str) -> Result<Args, Error> {
    let input = v.as_object().ok_or(Error::Invalid)?;
    let fields = shapes::fields(shape);
    let mut out = Args::new();
    for (key, value) in input {
        let f = fields
            .iter()
            .find(|f| f.name == key)
            .or_else(|| fields.iter().find(|f| f.name.eq_ignore_ascii_case(key)))
            .ok_or(Error::Invalid)?;
        if value.is_null() {
            continue;
        }
        let kind = f.kind.trim_start_matches('*');
        let val = if kind == "string" {
            check(value.is_string())?;
            value.clone()
        } else if kind == "bool" {
            check(value.is_boolean())?;
            value.clone()
        } else if kind == "int" {
            check(value.as_i64().is_some())?;
            value.clone()
        } else if kind == "float64" {
            check(value.is_number())?;
            value.clone()
        } else if let Some(elem) = kind.strip_prefix("[]") {
            let list = value.as_array().ok_or(Error::Invalid)?;
            let mut result = Vec::new();
            for v in list {
                if elem == "string" {
                    check(v.is_string() || v.is_null())?;
                    result.push(if v.is_null() {
                        Value::String(String::new())
                    } else {
                        v.clone()
                    });
                } else {
                    result.push(Value::Object(if v.is_null() {
                        Args::new()
                    } else {
                        normalize(v, elem)?
                    }));
                }
            }
            Value::Array(result)
        } else {
            return Err(Error::Invalid);
        };
        out.insert(f.name.into(), val);
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
pub fn origin(op: &str, kind: &str, id: &str, remote: bool, agent_level: &str) -> &'static str {
    if op == "messages.send" && kind == "token" && !remote && id.starts_with("token:") {
        "owner_self"
    } else if kind == "device"
        || kind == "agent" && matches!(agent_level, "full" | "approve")
        || kind == "delegated" && id.contains("/device:")
    {
        "interactive"
    } else {
        ""
    }
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
    check(valid_command_id(&c.id) && (0..=15000).contains(&c.wait_ms))?;
    let a = sources::validate(c, now)?;
    Ok(Validated {
        id: s(&a, "id").into(),
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
