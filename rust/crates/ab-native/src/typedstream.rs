use crate::{DecodeError, body_digest};
use std::collections::{BTreeMap, VecDeque};

type Result<T> = std::result::Result<T, DecodeError>;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePart {
    pub index: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "attachmentGUID", skip_serializing_if = "Option::is_none")]
    pub attachment_guid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention: Option<String>,
    pub utf16_location: usize,
    pub utf16_length: usize,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageFormatting {
    pub utf16_location: usize,
    pub utf16_length: usize,
    pub styles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageBody {
    pub parts: Vec<MessagePart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub decode_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_body_digest: Option<String>,
    pub formatting: Vec<MessageFormatting>,
}

#[derive(Debug, Clone)]
enum Value {
    Number(i64),
    Bytes(Vec<u8>),
    Object(usize),
    Classes(Vec<String>),
    Null,
    Values,
}

impl Value {
    fn number(&self) -> Option<i64> {
        if let Self::Number(n) = self {
            Some(*n)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Object {
    classes: Vec<String>,
    groups: Vec<Vec<Value>>,
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    strings: Vec<Vec<u8>>,
    references: Vec<Value>,
    objects: Vec<Object>,
    depth: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(DecodeError::Malformed)?;
        let value = self
            .data
            .get(self.position..end)
            .ok_or(DecodeError::Malformed)?;
        self.position = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn integer(&mut self, initial: Option<u8>) -> Result<i64> {
        let tag = match initial {
            Some(v) => v,
            None => self.byte()?,
        };
        match tag {
            0x81 => {
                let b = self.take(2)?;
                Ok(i64::from(i16::from_le_bytes([b[0], b[1]])))
            }
            0x82 => {
                let b = self.take(4)?;
                Ok(i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])))
            }
            0x80..=0x91 => Err(DecodeError::Unsupported),
            _ => Ok(i64::from(i8::from_ne_bytes([tag]))),
        }
    }
    fn raw_string(&mut self) -> Result<Vec<u8>> {
        let n = usize::try_from(self.integer(None)?).map_err(|_| DecodeError::Malformed)?;
        Ok(self.take(n)?.to_vec())
    }
    fn reference_index(&mut self, tag: u8) -> Result<usize> {
        usize::try_from(self.integer(Some(tag))? + 110).map_err(|_| DecodeError::Malformed)
    }
    fn shared_string(&mut self) -> Result<Option<Vec<u8>>> {
        let tag = self.byte()?;
        if tag == 0x85 {
            return Ok(None);
        }
        if tag == 0x84 {
            let v = self.raw_string()?;
            self.strings.push(v.clone());
            return Ok(Some(v));
        }
        let i = self.reference_index(tag)?;
        Ok(Some(
            self.strings.get(i).ok_or(DecodeError::Malformed)?.clone(),
        ))
    }
    fn enter(&mut self) -> Result<()> {
        if self.depth >= 64 {
            return Err(DecodeError::Limit);
        }
        self.depth += 1;
        Ok(())
    }
    fn class_chain(&mut self) -> Result<Vec<String>> {
        let tag = self.byte()?;
        if tag == 0x85 {
            return Ok(Vec::new());
        }
        if tag != 0x84 {
            let i = self.reference_index(tag)?;
            return match self.references.get(i) {
                Some(Value::Classes(v)) => Ok(v.clone()),
                _ => Err(DecodeError::Malformed),
            };
        }
        self.enter()?;
        let name = String::from_utf8(self.shared_string()?.ok_or(DecodeError::Malformed)?)
            .map_err(|_| DecodeError::Malformed)?;
        self.integer(None)?;
        let slot = self.references.len();
        self.references.push(Value::Null);
        let mut names = vec![name];
        names.extend(self.class_chain()?);
        self.references[slot] = Value::Classes(names.clone());
        self.depth -= 1;
        Ok(names)
    }
    fn object(&mut self) -> Result<Value> {
        let tag = self.byte()?;
        if tag == 0x85 {
            return Ok(Value::Null);
        }
        if tag != 0x84 {
            let i = self.reference_index(tag)?;
            return match self.references.get(i) {
                Some(v @ Value::Object(_)) => Ok(v.clone()),
                _ => Err(DecodeError::Malformed),
            };
        }
        self.enter()?;
        if self.references.len() >= 100_000 {
            return Err(DecodeError::Limit);
        }
        let id = self.objects.len();
        self.objects.push(Object::default());
        self.references.push(Value::Object(id));
        let classes = self.class_chain()?;
        let mut groups = Vec::new();
        while self.data.get(self.position).is_some_and(|b| *b != 0x86) {
            groups.push(self.group()?);
        }
        if self.byte()? != 0x86 {
            return Err(DecodeError::Malformed);
        }
        self.objects[id] = Object { classes, groups };
        self.depth -= 1;
        Ok(Value::Object(id))
    }
    fn group(&mut self) -> Result<Vec<Value>> {
        let encoding = self.shared_string()?.ok_or(DecodeError::Malformed)?;
        if encoding.is_empty() || !encoding.is_ascii() {
            return Err(DecodeError::Malformed);
        }
        self.values(&mut encoding.into(), None)
    }
    fn values(&mut self, types: &mut VecDeque<u8>, closing: Option<u8>) -> Result<Vec<Value>> {
        self.enter()?;
        if types.len() > 4096 {
            return Err(DecodeError::Limit);
        }
        let mut result = Vec::new();
        while let Some(kind) = types.pop_front() {
            if Some(kind) == closing {
                self.depth -= 1;
                return Ok(result);
            }
            result.push(match kind {
                b'@' => self.object()?,
                b'+' => Value::Bytes(self.raw_string()?),
                b'c' | b'C' | b'B' => Value::Number(i64::from(self.byte()?)),
                b'i' | b'I' | b's' | b'S' | b'l' | b'L' | b'q' | b'Q' => {
                    Value::Number(self.integer(None)?)
                }
                b'f' | b'd' => {
                    if self.data.get(self.position) == Some(&0x83) {
                        self.position += 1;
                        Value::Bytes(self.take(if kind == b'd' { 8 } else { 4 })?.to_vec())
                    } else {
                        Value::Number(self.integer(None)?)
                    }
                }
                b'*' => {
                    let tag = self.byte()?;
                    if tag == 0x85 {
                        Value::Null
                    } else if tag == 0x84 {
                        let value = Value::Bytes(self.shared_string()?.unwrap_or_default());
                        self.references.push(value.clone());
                        value
                    } else {
                        let i = self.reference_index(tag)?;
                        match self.references.get(i) {
                            Some(v @ Value::Bytes(_)) => v.clone(),
                            _ => return Err(DecodeError::Malformed),
                        }
                    }
                }
                b'%' | b':' => Value::Bytes(self.shared_string()?.unwrap_or_default()),
                b'{' => {
                    while types.front().is_some_and(|b| *b != b'=') {
                        types.pop_front();
                    }
                    if types.pop_front() != Some(b'=') {
                        return Err(DecodeError::Unsupported);
                    }
                    self.values(types, Some(b'}'))?;
                    Value::Values
                }
                b'[' => {
                    let mut n = 0usize;
                    let mut digits = 0;
                    while types.front().is_some_and(u8::is_ascii_digit) {
                        n = n
                            .checked_mul(10)
                            .and_then(|v| {
                                v.checked_add(usize::from(types.pop_front().unwrap_or(b'0') - b'0'))
                            })
                            .ok_or(DecodeError::Limit)?;
                        digits += 1;
                    }
                    if digits == 0 || n > 1_000_000 {
                        return Err(DecodeError::Limit);
                    }
                    let mut element = VecDeque::new();
                    loop {
                        match types.pop_front() {
                            Some(b']') => break,
                            Some(v) => element.push_back(v),
                            None => return Err(DecodeError::Malformed),
                        }
                    }
                    if element == *b"c" || element == *b"C" {
                        Value::Bytes(self.take(n)?.to_vec())
                    } else {
                        for _ in 0..n {
                            self.values(&mut element.clone(), None)?;
                        }
                        Value::Values
                    }
                }
                _ => return Err(DecodeError::Unsupported),
            });
        }
        self.depth -= 1;
        if closing.is_some() {
            return Err(DecodeError::Malformed);
        }
        Ok(result)
    }
    fn object_value(&self, v: &Value) -> Option<&Object> {
        if let Value::Object(i) = v {
            self.objects.get(*i)
        } else {
            None
        }
    }
    fn string(&self, v: &Value) -> Option<String> {
        let object = self.object_value(v)?;
        if !object.classes.iter().any(|v| v == "NSString") {
            return None;
        }
        object
            .groups
            .iter()
            .flatten()
            .find_map(|v| {
                if let Value::Bytes(b) = v {
                    Some(String::from_utf8(b.clone()).ok())
                } else {
                    None
                }
            })
            .flatten()
    }
    fn scalar(&self, v: &Value) -> i64 {
        v.number()
            .or_else(|| {
                self.object_value(v).and_then(|o| {
                    o.groups
                        .iter()
                        .flatten()
                        .filter_map(Value::number)
                        .next_back()
                })
            })
            .unwrap_or(0)
    }
    fn dictionary_values(&self, dictionary: &Object) -> Result<BTreeMap<String, Value>> {
        let count = dictionary
            .groups
            .first()
            .and_then(|g| g.first())
            .and_then(Value::number)
            .ok_or(DecodeError::Malformed)?;
        if !(0..=100_000).contains(&count) {
            return Err(DecodeError::Malformed);
        }
        let values: Vec<_> = dictionary.groups.iter().skip(1).flatten().collect();
        if i64::try_from(values.len()).ok() != Some(count * 2) {
            return Err(DecodeError::Malformed);
        }
        let mut attrs = BTreeMap::new();
        for pair in values.as_chunks::<2>().0 {
            attrs.insert(
                self.string(pair[0]).ok_or(DecodeError::Malformed)?,
                pair[1].clone(),
            );
        }
        Ok(attrs)
    }
    fn attributes(
        &self,
        root: &Object,
        cursor: &mut usize,
        attribute: i64,
        cache: &mut BTreeMap<i64, BTreeMap<String, Value>>,
    ) -> Result<BTreeMap<String, Value>> {
        let dictionary = root
            .groups
            .get(*cursor)
            .and_then(|g| g.first())
            .and_then(|v| self.object_value(v))
            .filter(|o| o.classes.iter().any(|c| c == "NSDictionary"));
        if let Some(dictionary) = dictionary {
            let attrs = self.dictionary_values(dictionary)?;
            *cursor += 1;
            cache.insert(attribute, attrs.clone());
            Ok(attrs)
        } else {
            cache.get(&attribute).cloned().ok_or(DecodeError::Malformed)
        }
    }
    fn formatting(
        &self,
        attrs: &BTreeMap<String, Value>,
        location: usize,
        length: usize,
    ) -> Option<MessageFormatting> {
        let mut styles = Vec::new();
        for (key, style) in [
            ("Bold", "bold"),
            ("Italic", "italic"),
            ("Underline", "underline"),
            ("Strikethrough", "strikethrough"),
        ] {
            if attrs
                .get(&format!("__kIMText{key}AttributeName"))
                .is_some_and(|v| self.scalar(v) != 0)
            {
                styles.push(style.into());
            }
        }
        let effect = attrs
            .get("__kIMTextEffectAttributeName")
            .and_then(|v| match self.scalar(v) {
                5 => Some("big"),
                11 => Some("small"),
                9 => Some("shake"),
                8 => Some("nod"),
                12 => Some("explode"),
                4 => Some("ripple"),
                6 => Some("bloom"),
                10 => Some("jitter"),
                _ => None,
            })
            .map(str::to_owned);
        if !styles.is_empty() || effect.is_some() {
            return Some(MessageFormatting {
                utf16_location: location,
                utf16_length: length,
                styles,
                effect,
            });
        }
        None
    }
    fn read_root(&mut self) -> Result<usize> {
        if self.data.len() > 16 * 1024 * 1024
            || self.byte()? != 4
            || self.byte()? != 11
            || self.take(11)? != b"streamtyped"
            || self.integer(None)? != 1000
        {
            return Err(DecodeError::Unsupported);
        }
        let group = self.group()?;
        if self.position != self.data.len() || group.len() != 1 {
            return Err(DecodeError::Malformed);
        }
        let root = self.object_value(&group[0]).ok_or(DecodeError::Malformed)?;
        if !root.classes.iter().any(|v| v == "NSAttributedString") {
            return Err(DecodeError::Unsupported);
        }
        match group[0] {
            Value::Object(id) => Ok(id),
            _ => Err(DecodeError::Malformed),
        }
    }
    fn decode(&mut self) -> Result<MessageBody> {
        let id = self.read_root()?;
        let root = self.objects.get(id).ok_or(DecodeError::Malformed)?;
        let string = root
            .groups
            .first()
            .and_then(|v| v.first())
            .and_then(|v| self.string(v))
            .ok_or(DecodeError::Malformed)?;
        let units: Vec<u16> = string.encode_utf16().collect();
        let mut location = 0;
        let mut index = 0;
        let mut cursor = 1;
        let mut cache = BTreeMap::new();
        let mut parts = Vec::new();
        let mut formatting = Vec::new();
        while cursor < root.groups.len() {
            let range = &root.groups[cursor];
            if range.len() != 2 {
                return Err(DecodeError::Malformed);
            }
            let attribute = range[0].number().ok_or(DecodeError::Malformed)?;
            let length = usize::try_from(range[1].number().ok_or(DecodeError::Malformed)?)
                .map_err(|_| DecodeError::Malformed)?;
            if length == 0 || length > units.len() - location {
                return Err(DecodeError::Malformed);
            }
            let end = location + length;
            let run =
                String::from_utf16(&units[location..end]).map_err(|_| DecodeError::Malformed)?;
            cursor += 1;
            let attrs = self.attributes(root, &mut cursor, attribute, &mut cache)?;
            if let Some(explicit) = attrs
                .get("__kIMMessagePartAttributeName")
                .and_then(|v| self.object_value(v))
                .and_then(|o| o.groups.iter().flatten().find_map(Value::number))
            {
                index = explicit;
            }
            let attachment = attrs
                .get("__kIMFileTransferGUIDAttributeName")
                .and_then(|v| self.string(v));
            let mention = attrs
                .get("__kIMMentionConfirmedMention")
                .and_then(|v| self.string(v))
                .or_else(|| {
                    attrs
                        .get("__kIMMentionConfirmedMentionAttributeName")
                        .and_then(|v| self.string(v))
                });
            if let Some(style) = self.formatting(&attrs, location, length) {
                formatting.push(style);
            }
            let genmoji = attrs.contains_key("__kIMEmojiImageAttributeName");
            parts.push(MessagePart {
                index,
                kind: if genmoji {
                    "genmoji"
                } else if attachment.is_some() {
                    "attachment"
                } else {
                    "text"
                }
                .into(),
                text: if attachment.is_none() && !genmoji {
                    Some(run)
                } else {
                    None
                },
                attachment_guid: attachment,
                mention,
                utf16_location: location,
                utf16_length: length,
            });
            location = end;
            index += 1;
        }
        if location != units.len() {
            return Err(DecodeError::Malformed);
        }
        let text = parts
            .iter()
            .filter_map(|v| v.text.as_deref())
            .collect::<String>();
        Ok(MessageBody {
            parts,
            text: Some(text),
            decode_status: "decoded".into(),
            raw_body_digest: None,
            formatting,
        })
    }
}

pub fn decode(data: &[u8]) -> MessageBody {
    Reader {
        data,
        position: 0,
        strings: Vec::new(),
        references: Vec::new(),
        objects: Vec::new(),
        depth: 0,
    }
    .decode()
    .unwrap_or_else(|_| MessageBody {
        parts: Vec::new(),
        text: None,
        decode_status: "undecoded".into(),
        raw_body_digest: Some(body_digest(data)),
        formatting: Vec::new(),
    })
}
