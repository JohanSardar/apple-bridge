use crate::DecodeError;
use serde_json::{Value, json};
use std::fmt::Write;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
};
type Result<T> = std::result::Result<T, DecodeError>;

#[derive(Debug, Clone, serde::Serialize)]
pub struct NotesBody {
    pub text: String,
    pub rich: bool,
    pub attachments: Vec<String>,
    pub blocks: Vec<Value>,
    pub html: String,
    pub lossless: bool,
}
#[derive(Debug, Clone)]
struct Field {
    number: u64,
    wire: u8,
    value: u64,
    bytes: Vec<u8>,
}
#[derive(Debug, Clone)]
struct Message(Vec<Field>);
impl Message {
    fn new(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(DecodeError::Limit);
        }
        let mut pos = 0;
        let mut fields = Vec::new();
        while pos < bytes.len() {
            let key = varint(bytes, &mut pos)?;
            let number = key >> 3;
            let wire = (key & 7) as u8;
            if number == 0 || number > 0x1fff_ffff {
                return Err(DecodeError::Malformed);
            }
            let mut value = 0;
            let mut data = Vec::new();
            match wire {
                0 => value = varint(bytes, &mut pos)?,
                1 | 2 | 5 => {
                    let n = if wire == 2 {
                        usize::try_from(varint(bytes, &mut pos)?)
                            .map_err(|_| DecodeError::Malformed)?
                    } else if wire == 1 {
                        8
                    } else {
                        4
                    };
                    let end = pos.checked_add(n).ok_or(DecodeError::Malformed)?;
                    data = bytes.get(pos..end).ok_or(DecodeError::Malformed)?.to_vec();
                    pos = end;
                }
                _ => return Err(DecodeError::Malformed),
            }
            fields.push(Field {
                number,
                wire,
                value,
                bytes: data,
            });
        }
        Ok(Self(fields))
    }
    fn validate(&self, types: &[(u64, u8)], repeated: &[u64]) -> Result<()> {
        let mut seen = BTreeSet::new();
        for f in &self.0 {
            if let Some((_, wire)) = types.iter().find(|(n, _)| *n == f.number)
                && (*wire != f.wire || (!repeated.contains(&f.number) && !seen.insert(f.number)))
            {
                return Err(DecodeError::Malformed);
            }
        }
        Ok(())
    }
    fn bytes(&self, n: u64) -> Option<&[u8]> {
        self.0
            .iter()
            .find(|f| f.number == n && f.wire == 2)
            .map(|f| f.bytes.as_slice())
    }
    fn integer(&self, n: u64) -> Option<u64> {
        self.0
            .iter()
            .find(|f| f.number == n && f.wire == 0)
            .map(|f| f.value)
    }
    fn repeated(&self, n: u64) -> Vec<&[u8]> {
        self.0
            .iter()
            .filter(|f| f.number == n && f.wire == 2)
            .map(|f| f.bytes.as_slice())
            .collect()
    }
    fn string(&self, n: u64) -> Result<Option<String>> {
        self.bytes(n)
            .map(|b| String::from_utf8(b.to_vec()).map_err(|_| DecodeError::Malformed))
            .transpose()
    }
    fn message(&self, n: u64) -> Result<Option<Self>> {
        self.bytes(n).map(Self::new).transpose()
    }
    fn required(&self, n: u64) -> Result<Self> {
        self.message(n)?.ok_or(DecodeError::Malformed)
    }
}
fn unpack(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() > 16 * 1024 * 1024 {
        return Err(DecodeError::Limit);
    }
    if !data.starts_with(&[0x1f, 0x8b]) {
        return Ok(data.to_vec());
    }
    let mut output = Vec::new();
    flate2::read::GzDecoder::new(data)
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut output)
        .map_err(|_| DecodeError::Malformed)?;
    if output.len() > 16 * 1024 * 1024 {
        return Err(DecodeError::Limit);
    }
    Ok(output)
}
fn run(text: &str) -> Value {
    json!({"text":text,"bold":false,"italic":false,"underline":false,"strikethrough":false})
}
fn block(kind: &str) -> Value {
    json!({"type":kind,"text":"","runs":[],"children":[]})
}
fn unsupported() -> DecodeError {
    DecodeError::Unsupported
}
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn inline(a: &Message, text: &str) -> Result<Value> {
    let mut r = run(text);
    let weight = a.integer(5).unwrap_or(0);
    if weight > 3 || a.integer(6).unwrap_or(0) > 1 || a.integer(7).unwrap_or(0) > 1 {
        return Err(unsupported());
    }
    r["bold"] = json!(weight == 1 || weight == 3);
    r["italic"] = json!(weight == 2 || weight == 3);
    r["underline"] = json!(a.integer(6) == Some(1));
    r["strikethrough"] = json!(a.integer(7) == Some(1));
    if let Some(link) = a.string(9)? {
        r["link"] = json!(link);
    }
    if let Some(font) = a.message(3)? {
        font.validate(&[(1, 2), (2, 5), (3, 0)], &[])?;
        if font.0.iter().any(|f| ![1, 2].contains(&f.number)) {
            r["unsupported"] = json!("unknown_font_attribute");
        }
        if let Some(name) = font.string(1)? {
            r["font"] = json!(name);
        }
        if let Some(size) = font.0.iter().find(|f| f.number == 2 && f.wire == 5) {
            let value = f32::from_le_bytes(
                size.bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| DecodeError::Malformed)?,
            );
            if !value.is_finite() || value <= 0.0 || value > 512.0 {
                return Err(unsupported());
            }
            r["font_size"] = json!(f64::from(value));
        }
    }
    if let Some(v) = a.integer(8).filter(|v| *v != 0) {
        if v != 1 && v != u64::MAX {
            return Err(unsupported());
        }
        r["superscript"] = json!(if v == 1 { 1 } else { -1 });
    }
    if let Some(v) = a.integer(14).filter(|v| *v != 0) {
        let color = ["purple", "pink", "orange", "mint", "blue"]
            .get(usize::try_from(v - 1).map_err(|_| unsupported())?)
            .ok_or_else(unsupported)?;
        r["highlight"] = json!(color);
    }
    if let Some(color) = a.message(10)? {
        color.validate(&[(1, 5), (2, 5), (3, 5), (4, 5)], &[])?;
        let mut result = String::from("#");
        for n in 1..=4 {
            let f = color
                .0
                .iter()
                .find(|f| f.number == n && f.wire == 5)
                .ok_or(DecodeError::Malformed)?;
            let v = f32::from_le_bytes(
                f.bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| DecodeError::Malformed)?,
            );
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(DecodeError::Malformed);
            }
            write!(result, "{:02x}", (v * 255.0).round() as u8)
                .map_err(|_| DecodeError::Malformed)?;
        }
        r["color"] = json!(result);
    }
    if let Some(attachment) = a.message(12)? {
        attachment.validate(&[(1, 2), (2, 2)], &[])?;
        for (n, k) in [(1, "attachment_id"), (2, "attachment_uti")] {
            if let Some(s) = attachment.string(n)? {
                r[k] = json!(s);
            }
        }
    }
    Ok(r)
}
#[allow(clippy::format_collect)]
fn paragraph(p: &Message) -> Result<Value> {
    p.validate(
        &[(1, 0), (2, 0), (3, 0), (4, 0), (5, 2), (8, 0), (9, 2)],
        &[],
    )?;
    let bits = p.integer(1).unwrap_or(u64::MAX).to_le_bytes();
    let style = i32::from_le_bytes([bits[0], bits[1], bits[2], bits[3]]);
    let mut b = block("paragraph");
    match style {
        -1 => {}
        0 => b["type"] = json!("title"),
        1 | 2 => {
            b["type"] = json!("heading");
            b["level"] = json!(style);
        }
        4 => b["type"] = json!("monostyled"),
        100..=102 => {
            b["type"] = json!("list_item");
            b["list_style"] = json!(
                ["bullet", "dash", "numbered"]
                    [usize::try_from(style - 100).map_err(|_| DecodeError::Malformed)?]
            );
        }
        103 => {
            b["type"] = json!("checklist_item");
            if let Some(c) = p
                .message(5)?
                .filter(|c| c.integer(2).is_some_and(|v| v <= 1))
            {
                c.validate(&[(1, 2), (2, 0)], &[])?;
                b["checked"] = json!(c.integer(2) == Some(1));
                if let Some(id) = c.bytes(1).filter(|b| b.len() == 16) {
                    b["checklist_id"] = json!(
                        id.iter()
                            .enumerate()
                            .map(|(i, b)| format!(
                                "{}{:02x}",
                                if [4, 6, 8, 10].contains(&i) { "-" } else { "" },
                                b
                            ))
                            .collect::<String>()
                    );
                } else {
                    b["unsupported"] = json!("checklist_identifier_missing");
                }
            } else {
                b["unsupported"] = json!("checklist_state_missing");
            }
        }
        _ => {
            b["type"] = json!("unknown");
            b["unsupported"] = json!("unknown_paragraph_style");
        }
    }
    if let Some(indent) = p.integer(4) {
        if indent > 128 {
            return Err(unsupported());
        }
        if b["type"] != "heading" {
            b["level"] = json!(indent);
        }
    }
    if p.integer(8).unwrap_or(0) > 1 {
        return Err(unsupported());
    }
    if p.integer(8) == Some(1) {
        b["blockquote"] = json!(true);
        if b["type"] == "paragraph" {
            b["type"] = json!("quote");
        }
    }
    if let Some(v) = p.integer(2) {
        b["alignment"] = json!(
            ["left", "center", "right", "justify"]
                .get(usize::try_from(v).map_err(|_| DecodeError::Malformed)?)
                .ok_or_else(unsupported)?
        );
    }
    if p.integer(3).is_some_and(|v| v != 1) {
        b["unsupported"] = json!("unsupported_writing_direction");
    }
    if p.bytes(9).is_some_and(|b| b.len() != 16) {
        return Err(DecodeError::Malformed);
    }
    if p.0
        .iter()
        .any(|f| ![1, 2, 3, 4, 5, 8, 9].contains(&f.number))
    {
        b["unsupported"] = json!("unknown_paragraph_attribute");
    }
    Ok(b)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn string<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}
fn array<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map_or(&[], Vec::as_slice)
}
fn safe_link(s: &str) -> bool {
    s.split_once(':').is_some_and(|(scheme, _)| {
        ["http", "https", "mailto", "notes"].contains(&scheme.to_lowercase().as_str())
    })
}
fn html(blocks: &[Value]) -> String {
    let mut out = String::new();
    for b in blocks {
        let mut text = String::new();
        for r in array(b, "runs") {
            let mut v = escape(string(r, "text")).replace('\u{2028}', "<br>");
            for (k, tag) in [
                ("bold", "b"),
                ("italic", "i"),
                ("underline", "u"),
                ("strikethrough", "s"),
            ] {
                if r[k] == true {
                    v = format!("<{tag}>{v}</{tag}>");
                }
            }
            for (n, tag) in [(1, "sup"), (-1, "sub")] {
                if r["superscript"] == n {
                    v = format!("<{tag}>{v}</{tag}>");
                }
            }
            let mut styles = Vec::new();
            if let Some(s) = r["color"].as_str() {
                styles.push(format!("color:{s}"));
            }
            if let Some(s) = r["font_size"].as_f64() {
                styles.push(format!("font-size:{s:.1}pt"));
            }
            if let Some(s) = r["font"].as_str().filter(|s| {
                s.chars().count() <= 128
                    && s.chars().all(|c| c.is_alphanumeric() || " -_.".contains(c))
            }) {
                styles.push(format!("font-family:'{s}'"));
            }
            if let Some(c) = match string(r, "highlight") {
                "purple" => Some("#d9b8ff"),
                "pink" => Some("#ffb8d6"),
                "orange" => Some("#ffcf99"),
                "mint" => Some("#b0e8cc"),
                "blue" => Some("#b8daff"),
                _ => None,
            } {
                styles.push(format!("background-color:{c}"));
            }
            if !styles.is_empty() {
                v = format!("<span style=\"{}\">{v}</span>", escape(&styles.join(";")));
            }
            let link = string(r, "link");
            if safe_link(link) {
                v = format!("<a href=\"{}\">{v}</a>", escape(link));
            }
            text.push_str(&v);
        }
        if text.is_empty() {
            text = "<br>".into();
        }
        let alignment = b["alignment"]
            .as_str()
            .map(|s| format!(" style=\"text-align:{}\"", escape(s)))
            .unwrap_or_default();
        let kind = string(b, "type");
        if kind == "table" {
            out.push_str("<table>");
            for row in array(b, "rows") {
                out.push_str("<tr>");
                for cell in row.as_array().into_iter().flatten() {
                    out.push_str("<td>");
                    out.push_str(&html(array(cell, "blocks")));
                    out.push_str("</td>");
                }
                out.push_str("</tr>");
            }
            out.push_str("</table>");
        } else if kind == "list_item" {
            let tag = if b["list_style"] == "numbered" {
                "ol"
            } else {
                "ul"
            };
            let _ = write!(out, "<{tag}><li{alignment}>{text}</li></{tag}>");
        } else if ["attachment", "checklist_item", "unknown"].contains(&kind) {
            let _ = write!(out, "<div data-notes-read-only=\"true\">{text}</div>");
        } else {
            let tag = match kind {
                "title" => "h1",
                "heading" => {
                    if b["level"] == 2 {
                        "h3"
                    } else {
                        "h2"
                    }
                }
                "monostyled" => "pre",
                "quote" => "blockquote",
                _ => "div",
            };
            let _ = write!(out, "<{tag}{alignment}>{text}</{tag}>");
        }
        out.push_str(&html(array(b, "children")));
    }
    out
}
fn append(flat: &mut Vec<Value>, current: &mut Value, r: &Value, p: Option<&Value>) {
    let pieces = string(r, "text").split('\n').collect::<Vec<_>>();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 && i == pieces.len() - 1 && piece.is_empty() {
            break;
        }
        if let Some(p) = p {
            let text = current["text"].clone();
            let runs = current["runs"].clone();
            let unsupported = current["unsupported"].clone();
            *current = p.clone();
            current["text"] = text;
            current["runs"] = runs;
            if current["unsupported"].is_null() && !unsupported.is_null() {
                current["unsupported"] = unsupported;
            }
        }
        if !piece.is_empty() {
            let mut part = r.clone();
            part["text"] = json!(piece);
            if let Some(runs) = current["runs"].as_array_mut() {
                runs.push(part);
            }
            current["text"] = json!(format!("{}{}", string(current, "text"), piece));
        }
        if i < pieces.len() - 1 {
            flat.push(current.clone());
            *current = block("paragraph");
        }
    }
}
fn sections(flat: &[Value], pos: &mut usize, level: u64) -> Vec<Value> {
    let mut result = Vec::new();
    while *pos < flat.len() {
        let mut b = flat[*pos].clone();
        let heading = if b["type"] == "heading" {
            Some(b["level"].as_u64().unwrap_or(1))
        } else {
            None
        };
        if heading.is_some_and(|h| h <= level) {
            break;
        }
        *pos += 1;
        if let Some(h) = heading {
            b["collapsible"] = json!(true);
            b["children"] = json!(sections(flat, pos, h));
        }
        result.push(b);
    }
    result
}
fn plain(r: &Value) -> bool {
    ![
        "unsupported",
        "font",
        "font_size",
        "link",
        "highlight",
        "color",
        "superscript",
        "attachment_id",
    ]
    .iter()
    .any(|k| !r[*k].is_null())
        && !["bold", "italic", "underline", "strikethrough"]
            .iter()
            .any(|k| r[*k] == true)
}
fn decode_note(
    note: &Message,
    tables: &BTreeMap<String, Vec<u8>>,
    depth: usize,
) -> Result<NotesBody> {
    note.validate(&[(2, 2), (3, 2), (4, 2), (5, 2)], &[3, 5])?;
    if depth >= 8 {
        return Err(DecodeError::Malformed);
    }
    let text = note.string(2)?.ok_or(DecodeError::Malformed)?;
    let attrs = note
        .repeated(5)
        .into_iter()
        .map(Message::new)
        .collect::<Result<Vec<_>>>()?;
    let mut attachments = Vec::new();
    for a in &attrs {
        a.validate(
            &[
                (1, 0),
                (2, 2),
                (3, 2),
                (5, 0),
                (6, 0),
                (7, 0),
                (8, 0),
                (9, 2),
                (10, 2),
                (12, 2),
                (13, 0),
                (14, 0),
            ],
            &[],
        )?;
        if let Some(id) = a.message(12)?.map(|m| m.string(1)).transpose()?.flatten()
            && !attachments.contains(&id)
        {
            attachments.push(id);
        }
    }
    if attrs.iter().any(|a| a.integer(1).is_none()) {
        let mut b = block("unknown");
        b["text"] = json!(text);
        b["runs"] = json!([run(&text)]);
        b["unsupported"] = json!("missing_run_length");
        return Ok(NotesBody {
            text: text.clone(),
            rich: true,
            attachments,
            blocks: vec![b],
            html: escape(&text),
            lossless: false,
        });
    }
    let (mut flat, mut complete) = note_blocks(
        &text,
        &attrs,
        note.0.iter().all(|f| [2, 3, 4, 5].contains(&f.number)),
    )?;
    project_attachments(&mut flat, tables, depth, &mut complete);
    let blocks = sections(&flat, &mut 0, 0);
    inventory(&blocks, &mut attachments);
    let rich = !attachments.is_empty()
        || flat.iter().enumerate().any(|(i, b)| {
            !b["unsupported"].is_null()
                || !array(b, "runs").iter().all(plain)
                || b["blockquote"] == true
                || (!b["alignment"].is_null() && b["alignment"] != "left")
                || (b["type"] != "heading" && b["level"].as_i64().unwrap_or(0) > 0)
                || (b["type"] != "paragraph" && !(i == 0 && b["type"] == "title"))
        });
    let round_trip = flat.iter().all(|b| {
        ["paragraph", "title", "heading", "monostyled"].contains(&string(b, "type"))
            && b["blockquote"] != true
            && (b["level"].as_i64().unwrap_or(0) == 0 || b["type"] == "heading")
            && array(b, "runs").iter().all(|r| {
                ["highlight", "font", "font_size", "color", "superscript"]
                    .iter()
                    .all(|k| r[*k].is_null())
                    && !string(r, "text").contains(['\0', '\u{2028}'])
                    && (r["link"].is_null() || safe_link(string(r, "link")))
            })
    });
    let html = html(&blocks);
    let lossless = complete && attachments.is_empty() && round_trip;
    Ok(NotesBody {
        text,
        rich,
        attachments,
        blocks,
        html,
        lossless,
    })
}
pub fn decode(data: &[u8], tables: &BTreeMap<String, Vec<u8>>) -> Result<NotesBody> {
    let root = Message::new(&unpack(data)?)?;
    root.validate(&[(2, 2)], &[])?;
    let document = root.required(2)?;
    document.validate(&[(2, 0), (3, 2)], &[])?;
    decode_note(&document.required(3)?, tables, 0)
}
fn table(data: &[u8], tables: &BTreeMap<String, Vec<u8>>, depth: usize) -> Result<Vec<Vec<Value>>> {
    if depth >= 8 {
        return Err(DecodeError::Malformed);
    }
    let graph = Message::new(&unpack(data)?)?.required(2)?.required(3)?;
    let objects = graph
        .repeated(3)
        .into_iter()
        .map(Message::new)
        .collect::<Result<Vec<_>>>()?;
    let strings = |n| {
        graph
            .repeated(n)
            .into_iter()
            .map(|b| String::from_utf8(b.to_vec()).map_err(|_| DecodeError::Malformed))
            .collect::<Result<Vec<_>>>()
    };
    let keys = strings(4)?;
    let types = strings(5)?;
    let uuids = graph.repeated(6);
    let object = |id| table_object(&objects, id);
    let target = |m| table_target(m, uuids.len());
    let mut root = None;
    for value in &objects {
        if let Some(map) = value.message(13)?
            && map
                .integer(1)
                .and_then(|n| usize::try_from(n).ok().and_then(|i| types.get(i)))
                .is_some_and(|s| s == "com.apple.notes.ICTable")
        {
            if root.is_some() {
                return Err(unsupported());
            }
            root = Some(map);
        }
    }
    let root = root.ok_or_else(unsupported)?;
    let mut parts = BTreeMap::new();
    for bytes in root.repeated(3) {
        let entry = Message::new(bytes)?;
        let key = keys
            .get(
                usize::try_from(entry.integer(1).ok_or(DecodeError::Malformed)?)
                    .map_err(|_| DecodeError::Malformed)?,
            )
            .ok_or(DecodeError::Malformed)?;
        let value = entry.required(2)?;
        if ["crRows", "crColumns", "cellColumns"].contains(&key.as_str()) {
            parts.insert(key.clone(), object(value.integer(6))?);
        }
    }
    let (row_count, rows) = table_ordering(
        parts.get("crRows").ok_or(DecodeError::Malformed)?,
        &objects,
        &uuids,
    )?;
    let (column_count, columns) = table_ordering(
        parts.get("crColumns").ok_or(DecodeError::Malformed)?,
        &objects,
        &uuids,
    )?;
    if row_count == 0 || column_count == 0 || row_count > 16384 / column_count {
        return Err(unsupported());
    }
    let mut result = vec![vec![json!({"blocks":[]}); column_count]; row_count];
    let mut occupied = BTreeSet::new();
    for (key, value) in dictionary(parts.get("cellColumns").ok_or(DecodeError::Malformed)?)? {
        let column = target(object(key.integer(6))?)?;
        let ci = *columns.get(&column).ok_or(DecodeError::Malformed)?;
        for (rk, rv) in dictionary(object(value.integer(6))?)? {
            let row = target(object(rk.integer(6))?)?;
            let ri = *rows.get(&row).ok_or(DecodeError::Malformed)?;
            if !occupied.insert((ri, ci)) {
                return Err(DecodeError::Malformed);
            }
            let note = object(rv.integer(6))?.required(10)?;
            result[ri][ci] = json!({"blocks":decode_note(&note,tables,depth+1)?.blocks});
        }
    }
    for value in &objects {
        if let Some(map) = value.message(13)? {
            for b in map.repeated(3) {
                let entry = Message::new(b)?;
                if entry
                    .message(2)?
                    .map(|m| m.string(4))
                    .transpose()?
                    .flatten()
                    .as_deref()
                    == Some("CRTableColumnDirectionRightToLeft")
                {
                    for row in &mut result {
                        row.reverse();
                    }
                    return Ok(result);
                }
            }
        }
    }
    Ok(result)
}

fn varint(bytes: &[u8], pos: &mut usize) -> Result<u64> {
    let mut n = 0;
    for shift in (0..=63).step_by(7) {
        let b = *bytes.get(*pos).ok_or(DecodeError::Malformed)?;
        *pos += 1;
        if shift == 63 && b > 1 {
            return Err(DecodeError::Malformed);
        }
        n |= u64::from(b & 127) << shift;
        if b < 128 {
            return Ok(n);
        }
    }
    Err(DecodeError::Malformed)
}

fn inventory(blocks: &[Value], ids: &mut Vec<String>) {
    for b in blocks {
        for r in array(b, "runs") {
            if let Some(id) = r["attachment_id"].as_str()
                && !ids.iter().any(|v| v == id)
            {
                ids.push(id.into());
            }
        }
        inventory(array(b, "children"), ids);
        for row in array(b, "rows") {
            for cell in row.as_array().into_iter().flatten() {
                inventory(array(cell, "blocks"), ids);
            }
        }
    }
}

fn dictionary(m: &Message) -> Result<Vec<(Message, Message)>> {
    m.required(6)?
        .repeated(1)
        .into_iter()
        .map(|b| {
            let e = Message::new(b)?;
            Ok((e.required(1)?, e.required(2)?))
        })
        .collect()
}

fn note_blocks(text: &str, attrs: &[Message], known: bool) -> Result<(Vec<Value>, bool)> {
    let units = text.encode_utf16().collect::<Vec<_>>();
    let mut offset = 0;
    let mut complete = known;
    let mut flat = Vec::new();
    let mut current = block("paragraph");
    for a in attrs {
        let n = usize::try_from(a.integer(1).ok_or(DecodeError::Malformed)?)
            .map_err(|_| DecodeError::Malformed)?;
        if n > units.len() - offset {
            return Err(DecodeError::Malformed);
        }
        let end = offset + n;
        let segment =
            String::from_utf16(&units[offset..end]).map_err(|_| DecodeError::Malformed)?;
        offset = end;
        let parsed = (|| {
            Ok((
                inline(a, &segment)?,
                a.message(2)?.map(|p| paragraph(&p)).transpose()?,
            ))
        })();
        let (r, mut p) = match parsed {
            Ok(v) => v,
            Err(DecodeError::Unsupported) => {
                let mut p = block("unknown");
                p["unsupported"] = json!("unsupported_attributes");
                (run(&segment), Some(p))
            }
            Err(e) => return Err(e),
        };
        if a.0
            .iter()
            .any(|f| ![1, 2, 3, 5, 6, 7, 8, 9, 10, 12, 13, 14].contains(&f.number))
        {
            complete = false;
            p.get_or_insert_with(|| block("paragraph"))["unsupported"] = json!("unknown_attribute");
        }
        if !r["unsupported"].is_null() {
            p.get_or_insert_with(|| block("paragraph"))["unsupported"] = r["unsupported"].clone();
        }
        if p.as_ref().is_some_and(|p| !p["unsupported"].is_null()) {
            complete = false;
        }
        append(&mut flat, &mut current, &r, p.as_ref());
    }
    if offset < units.len() {
        append(
            &mut flat,
            &mut current,
            &run(&String::from_utf16_lossy(&units[offset..])),
            None,
        );
        complete &= attrs.is_empty();
    }
    if !array(&current, "runs").is_empty() || flat.is_empty() {
        flat.push(current);
    }
    if !known {
        let mut b = block("unknown");
        b["unsupported"] = json!("unknown_note_field");
        flat.push(b);
    }

    Ok((flat, complete))
}

fn project_attachments(
    flat: &mut [Value],
    tables: &BTreeMap<String, Vec<u8>>,
    depth: usize,
    complete: &mut bool,
) {
    for b in flat {
        let ids = array(b, "runs")
            .iter()
            .filter_map(|r| r["attachment_id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        if b["text"] == "\u{fffc}" && ids.len() == 1 {
            b["attachment_id"] = json!(ids[0]);
            if array(b, "runs")
                .first()
                .is_some_and(|r| r["attachment_uti"] == "com.apple.notes.table")
            {
                b["type"] = json!("table");
                if let Some(rows) = tables
                    .get(&ids[0])
                    .and_then(|data| table(data, tables, depth + 1).ok())
                {
                    b["rows"] = json!(rows);
                } else {
                    b["unsupported"] = json!("table_data_unavailable");
                }
            } else if b["type"] == "paragraph" {
                b["type"] = json!("attachment");
            }
            *complete = false;
        } else if string(b, "text").contains('\u{fffc}') && ids.is_empty() {
            b["type"] = json!("unknown");
            b["unsupported"] = json!("attachment_identifier_missing");
            *complete = false;
        }
    }
}

fn table_ordering(
    m: &Message,
    objects: &[Message],
    uuids: &[&[u8]],
) -> Result<(usize, BTreeMap<u64, usize>)> {
    let order = m.required(16)?.required(1)?;
    let arr = order.required(1)?;
    let mut entries = Vec::new();
    for b in arr.repeated(2) {
        let item = Message::new(b)?;
        let pos = item.integer(1).ok_or(DecodeError::Malformed)?;
        let uuid = item
            .bytes(2)
            .filter(|b| b.len() == 16)
            .ok_or(DecodeError::Malformed)?
            .to_vec();
        entries.push((pos, uuid));
    }
    entries.sort_by_key(|e| e.0);
    if entries.len() > 16384 || entries.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err(unsupported());
    }
    let mut indices = BTreeMap::new();
    for (i, (_, uuid)) in entries.iter().enumerate() {
        let id = uuids
            .iter()
            .position(|v| *v == uuid)
            .ok_or(DecodeError::Malformed)? as u64;
        if indices.insert(id, i).is_some() {
            return Err(DecodeError::Malformed);
        }
    }
    if let Some(aliases) = order.message(2)? {
        let mut pending = Vec::new();
        for b in aliases.repeated(1) {
            let pair = Message::new(b)?;
            pending.push((
                table_target(
                    table_object(objects, pair.required(1)?.integer(6))?,
                    uuids.len(),
                )?,
                table_target(
                    table_object(objects, pair.required(2)?.integer(6))?,
                    uuids.len(),
                )?,
            ));
        }
        while !pending.is_empty() {
            let previous = pending.len();
            let mut next = Vec::new();
            for (k, v) in pending {
                if let Some(pos) = indices.get(&k).copied() {
                    if indices.get(&v).is_some_and(|p| *p != pos) {
                        return Err(DecodeError::Malformed);
                    }
                    indices.insert(v, pos);
                } else {
                    next.push((k, v));
                }
            }
            if next.len() >= previous {
                return Err(DecodeError::Malformed);
            }
            pending = next;
        }
    }
    Ok((entries.len(), indices))
}

fn table_object(objects: &[Message], id: Option<u64>) -> Result<&Message> {
    objects
        .get(
            usize::try_from(id.ok_or(DecodeError::Malformed)?)
                .map_err(|_| DecodeError::Malformed)?,
        )
        .ok_or(DecodeError::Malformed)
}
fn table_target(m: &Message, count: usize) -> Result<u64> {
    let map = m.required(13)?;
    let entry = Message::new(map.repeated(3).first().ok_or(DecodeError::Malformed)?)?;
    let id = entry
        .required(2)?
        .integer(2)
        .ok_or(DecodeError::Malformed)?;
    if id >= count as u64 {
        return Err(DecodeError::Malformed);
    }
    Ok(id)
}
