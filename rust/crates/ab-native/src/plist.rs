use crate::DecodeError;
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, DecodeError>;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Date(f64),
    Data(Vec<u8>),
    String(String),
    Uid(usize),
    Array(Vec<Self>),
    Dictionary(BTreeMap<String, Self>),
}

impl Value {
    pub fn dictionary(&self) -> Option<&BTreeMap<String, Self>> {
        if let Self::Dictionary(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn string(&self) -> Option<&str> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }
    #[allow(clippy::cast_precision_loss)]
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Integer(v) => Some(*v as f64),
            Self::Real(v) => Some(*v),
            Self::Bool(v) => Some(f64::from(u8::from(*v))),
            _ => None,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offsets: Vec<usize>,
    reference_size: usize,
    remaining: usize,
}

fn integer(bytes: &[u8], start: usize, size: usize) -> Result<u64> {
    if size == 0 || size > 8 {
        return Err(DecodeError::Malformed);
    }
    let end = start.checked_add(size).ok_or(DecodeError::Malformed)?;
    let bytes = bytes.get(start..end).ok_or(DecodeError::Malformed)?;
    Ok(bytes.iter().fold(0, |n, b| n << 8 | u64::from(*b)))
}
fn index(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| DecodeError::Limit)
}

impl Reader<'_> {
    fn integer(&self, start: usize, size: usize) -> Result<u64> {
        if start < 8 {
            return Err(DecodeError::Malformed);
        }
        integer(&self.bytes[..self.bytes.len() - 32], start, size)
    }
    fn child(&mut self, body: usize, offset: usize, depth: usize) -> Result<Value> {
        let reference =
            index(self.integer(body + offset * self.reference_size, self.reference_size)?)?;
        self.object(reference, depth + 1)
    }
    fn object(&mut self, id: usize, depth: usize) -> Result<Value> {
        if depth >= 64 || self.remaining == 0 {
            return Err(DecodeError::Limit);
        }
        self.remaining -= 1;
        let start = *self.offsets.get(id).ok_or(DecodeError::Malformed)?;
        if start < 8 || start >= self.bytes.len() - 32 {
            return Err(DecodeError::Malformed);
        }
        let tag = self.bytes[start];
        let kind = tag >> 4;
        let mut count = usize::from(tag & 15);
        let mut body = start + 1;
        if count == 15 && [4, 5, 6, 10, 13].contains(&kind) {
            let length = *self.bytes.get(body).ok_or(DecodeError::Malformed)?;
            if length >> 4 != 1 || length & 15 > 3 {
                return Err(DecodeError::Malformed);
            }
            let size = 1 << (length & 15);
            count = index(self.integer(body + 1, size)?)?;
            if count > 100_000 {
                return Err(DecodeError::Limit);
            }
            body += 1 + size;
        }
        let slice = |n: usize| self.bytes[..self.bytes.len() - 32].get(body..body.checked_add(n)?);
        match kind {
            0 => Ok(match count {
                8 => Value::Bool(false),
                9 => Value::Bool(true),
                _ => Value::Null,
            }),
            1 if count <= 3 => Ok(Value::Integer(i64::from_ne_bytes(
                self.integer(body, 1 << count)?.to_ne_bytes(),
            ))),
            2 if count == 2 => Ok(Value::Real(f64::from(f32::from_bits(
                u32::try_from(self.integer(body, 4)?).map_err(|_| DecodeError::Malformed)?,
            )))),
            2 if count == 3 => Ok(Value::Real(f64::from_bits(self.integer(body, 8)?))),
            3 => Ok(Value::Date(f64::from_bits(self.integer(body, 8)?))),
            4 => Ok(Value::Data(
                slice(count).ok_or(DecodeError::Malformed)?.to_vec(),
            )),
            5 => {
                let bytes = slice(count).ok_or(DecodeError::Malformed)?;
                if !bytes.is_ascii() {
                    return Err(DecodeError::Malformed);
                }
                Ok(Value::String(
                    String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::Malformed)?,
                ))
            }
            6 => {
                let units = slice(count * 2)
                    .ok_or(DecodeError::Malformed)?
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>();
                Ok(Value::String(
                    String::from_utf16(&units).map_err(|_| DecodeError::Malformed)?,
                ))
            }
            8 => Ok(Value::Uid(index(self.integer(body, count + 1)?)?)),
            10 => {
                let mut values = Vec::new();
                for i in 0..count {
                    values.push(self.child(body, i, depth)?);
                }
                Ok(Value::Array(values))
            }
            13 => {
                let mut values = BTreeMap::new();
                for i in 0..count {
                    let Value::String(key) = self.child(body, i, depth)? else {
                        return Err(DecodeError::Malformed);
                    };
                    values.insert(key, self.child(body, i + count, depth)?);
                }
                Ok(Value::Dictionary(values))
            }
            _ => Err(DecodeError::Unsupported),
        }
    }
}

fn portable(
    event: ::plist::stream::OwnedEvent,
    events: &mut impl Iterator<Item = std::result::Result<::plist::stream::OwnedEvent, ::plist::Error>>,
    depth: usize,
    remaining: &mut usize,
) -> Result<Value> {
    use ::plist::stream::Event;
    if depth >= 64 || *remaining == 0 {
        return Err(DecodeError::Limit);
    }
    *remaining -= 1;
    Ok(match event {
        Event::StartArray(_) => {
            let mut values = Vec::new();
            loop {
                let event = events
                    .next()
                    .ok_or(DecodeError::Malformed)?
                    .map_err(|_| DecodeError::Malformed)?;
                if matches!(event, Event::EndCollection) {
                    break;
                }
                values.push(portable(event, events, depth + 1, remaining)?);
            }
            Value::Array(values)
        }
        Event::StartDictionary(_) => {
            let mut values = BTreeMap::new();
            loop {
                let event = events
                    .next()
                    .ok_or(DecodeError::Malformed)?
                    .map_err(|_| DecodeError::Malformed)?;
                if matches!(event, Event::EndCollection) {
                    break;
                }
                let Event::String(key) = event else {
                    return Err(DecodeError::Malformed);
                };
                let event = events
                    .next()
                    .ok_or(DecodeError::Malformed)?
                    .map_err(|_| DecodeError::Malformed)?;
                let value = portable(event, events, depth + 1, remaining)?;
                values.insert(key.into_owned(), value);
            }
            Value::Dictionary(values)
        }
        Event::Boolean(v) => Value::Bool(v),
        Event::Data(v) => Value::Data(v.into_owned()),
        Event::String(v) => Value::String(v.into_owned()),
        Event::Real(v) => Value::Real(v),
        Event::Integer(v) => Value::Integer(v.as_signed().ok_or(DecodeError::Limit)?),
        Event::Date(v) => {
            let t: std::time::SystemTime = v.into();
            let epoch = std::time::UNIX_EPOCH + std::time::Duration::from_hours(271_752);
            Value::Date(match t.duration_since(epoch) {
                Ok(d) => d.as_secs_f64(),
                Err(e) => -e.duration().as_secs_f64(),
            })
        }
        Event::Uid(v) => Value::Uid(usize::try_from(v.get()).map_err(|_| DecodeError::Limit)?),
        _ => return Err(DecodeError::Unsupported),
    })
}

pub fn decode(data: &[u8]) -> Result<Value> {
    if !data.starts_with(b"bplist00") {
        if data.len() > 16 * 1024 * 1024 {
            return Err(DecodeError::Limit);
        }
        let mut events = ::plist::stream::Reader::new(std::io::Cursor::new(data));
        let event = events
            .next()
            .ok_or(DecodeError::Malformed)?
            .map_err(|_| DecodeError::Malformed)?;
        let value = portable(event, &mut events, 0, &mut 200_000)?;
        if events.next().is_some() {
            return Err(DecodeError::Malformed);
        }
        return Ok(value);
    }
    if data.len() < 40 || data.len() > 16 * 1024 * 1024 || !data.starts_with(b"bplist00") {
        return Err(DecodeError::Malformed);
    }
    let trailer = data.len() - 32;
    let size = usize::from(data[trailer + 6]);
    let reference_size = usize::from(data[trailer + 7]);
    let count = index(integer(data, trailer + 8, 8)?)?;
    let top = index(integer(data, trailer + 16, 8)?)?;
    let start = index(integer(data, trailer + 24, 8)?)?;
    if count == 0
        || count > 100_000
        || top >= count
        || !(1..=8).contains(&size)
        || !(1..=8).contains(&reference_size)
        || start < 8
        || start > trailer
        || count > (trailer - start) / size
    {
        return Err(DecodeError::Limit);
    }
    let offsets = (0..count)
        .map(|i| index(integer(data, start + i * size, size)?))
        .collect::<Result<Vec<_>>>()?;
    Reader {
        bytes: data,
        offsets,
        reference_size,
        remaining: 200_000,
    }
    .object(top, 0)
}

fn resolve(
    value: &Value,
    objects: &[Value],
    visited: &[usize],
    depth: usize,
    remaining: &mut usize,
) -> Result<Value> {
    if depth >= 64 || *remaining == 0 {
        return Err(DecodeError::Limit);
    }
    *remaining -= 1;
    match value {
        Value::Uid(0) => Ok(Value::Null),
        Value::Uid(id) => {
            if visited.contains(id) {
                return Err(DecodeError::Malformed);
            }
            let mut path = visited.to_vec();
            path.push(*id);
            resolve(
                objects.get(*id).ok_or(DecodeError::Malformed)?,
                objects,
                &path,
                depth + 1,
                remaining,
            )
        }
        Value::Array(values) => Ok(Value::Array(
            values
                .iter()
                .map(|v| resolve(v, objects, visited, depth + 1, remaining))
                .collect::<Result<_>>()?,
        )),
        Value::Dictionary(values) => {
            let mut result = BTreeMap::new();
            for (k, v) in values {
                if k != "$class" {
                    result.insert(
                        k.clone(),
                        resolve(v, objects, visited, depth + 1, remaining)?,
                    );
                }
            }
            if let Some(Value::Data(bytes)) = result.get("NS.uuidbytes")
                && bytes.len() == 16
            {
                let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
                return Ok(Value::String(format!(
                    "{}-{}-{}-{}-{}",
                    &hex[0..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..32]
                )));
            }
            if let (Some(Value::Array(keys)), Some(Value::Array(values))) =
                (result.get("NS.keys"), result.get("NS.objects"))
                && keys.len() == values.len()
                && keys.iter().all(|k| k.string().is_some())
            {
                return Ok(Value::Dictionary(
                    keys.iter()
                        .zip(values)
                        .map(|(k, v)| (k.string().unwrap_or_default().to_owned(), v.clone()))
                        .collect(),
                ));
            }
            if let Some(v @ Value::Array(_)) = result.get("NS.objects") {
                return Ok(v.clone());
            }
            for key in ["NS.relative", "NS.string"] {
                if let Some(v @ Value::String(_)) = result.get(key) {
                    return Ok(v.clone());
                }
            }
            Ok(Value::Dictionary(result))
        }
        _ => Ok(value.clone()),
    }
}
pub fn archive_root(data: &[u8]) -> Result<Value> {
    let Value::Dictionary(raw) = decode(data)? else {
        return Err(DecodeError::Malformed);
    };
    let (Some(Value::Array(objects)), Some(Value::Dictionary(top))) =
        (raw.get("$objects"), raw.get("$top"))
    else {
        return Ok(Value::Dictionary(raw));
    };
    let value = top
        .get("root")
        .or_else(|| top.get("NSKeyedArchiveRootObjectKey"))
        .unwrap_or(&Value::Null);
    resolve(value, objects, &[], 0, &mut 100_000)
}

pub fn archive(data: &[u8]) -> Result<BTreeMap<String, Value>> {
    match archive_root(data)? {
        Value::Dictionary(result) => Ok(result),
        _ => Err(DecodeError::Malformed),
    }
}
