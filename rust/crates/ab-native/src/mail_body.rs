use crate::{
    assets,
    capture::{Error, Result},
};

use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
const LIMIT: usize = 32 << 20;
fn result(state: &str, text: &str, html: &str) -> Value {
    json!({"state":state,"plain_text":text,"html":html})
}
fn find(root: &Path, id: &str) -> Result<Option<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    let mut count = 0;
    let mut found = None;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            count += 1;
            if count > 500_000 {
                return Err(Error::Incomplete);
            }
            let path = entry.path();
            let name = entry.file_name();
            if name == format!("{id}.emlx").as_str()
                || name == format!("{id}.partial.emlx").as_str()
            {
                if found.is_some() {
                    return Err(Error::Conflict);
                }
                found = Some(path.clone());
            }
            if entry.file_type()?.is_dir() {
                directories.push(path);
            }
        }
    }
    Ok(found)
}
pub fn read(args: &Value, root: &Path) -> Result<Value> {
    let id = args["message_id"].as_str().ok_or(Error::Corrupt)?;
    if args.as_object().is_none_or(|o| o.len() != 1)
        || id.is_empty()
        || id.len() >= 20
        || !id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Error::Corrupt);
    }
    let Some(path) = find(root, id)? else {
        return Ok(result("offloaded", "", ""));
    };
    let resolved = path.canonicalize()?;
    let root = root.canonicalize()?;
    if !resolved.starts_with(&root) || resolved == root {
        return Err(Error::Corrupt);
    }
    let metadata = std::fs::symlink_metadata(&resolved)?;
    if assets::dataless(&metadata) {
        return Ok(result("offloaded", "", ""));
    }
    if !std::fs::symlink_metadata(&path)?.is_file()
        || !metadata.is_file()
        || metadata.len() > u64::try_from(LIMIT).map_err(|_| Error::Corrupt)?
    {
        return Err(Error::Incomplete);
    }
    let mut data = Vec::new();
    assets::open_regular(&resolved)?
        .take(u64::try_from(LIMIT + 1).map_err(|_| Error::Corrupt)?)
        .read_to_end(&mut data)?;
    if data.len() > LIMIT {
        return Err(Error::Incomplete);
    }
    decode(
        &data,
        path.file_name()
            .is_some_and(|n| n.to_string_lossy().contains(".partial.")),
    )
}
pub fn decode(data: &[u8], partial: bool) -> Result<Value> {
    if data.len() > LIMIT {
        return Err(Error::Incomplete);
    }
    let newline = data
        .iter()
        .position(|b| *b == b'\n')
        .filter(|n| *n < 20)
        .ok_or(Error::Corrupt)?;
    let length = String::from_utf8_lossy(&data[..newline])
        .trim()
        .parse::<usize>()
        .map_err(|_| Error::Corrupt)?;
    if length > LIMIT {
        return Err(Error::Corrupt);
    }
    let available = length.min(data.len() - newline - 1);
    let body = latin(&data[newline + 1..newline + 1 + available]);
    let mut partial = partial || available < length;
    let (mut text, html) = mime(&body, 0, &mut partial)?;
    if text.is_empty() && html.is_empty() {
        partial = true;
    }
    if text.is_empty() && !html.is_empty() {
        let raw = html
            .replace("<pre>", "")
            .replace("</pre>", "")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&");
        text = strip_tags(&strip_scripts(&raw));
    }
    Ok(result(
        if partial { "partial" } else { "downloaded" },
        &text,
        &html,
    ))
}
fn latin(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}
fn unlatin(value: &str) -> Result<Vec<u8>> {
    value
        .chars()
        .map(|c| u8::try_from(u32::from(c)).map_err(|_| Error::Corrupt))
        .collect()
}
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn base64_foundation(input: &[u8]) -> Option<Vec<u8>> {
    fn digit(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut output = Vec::new();
    let mut offset = 0;
    while input.len() - offset >= 4 {
        let group = &input[offset..offset + 4];
        if group[0] == b'=' && input[offset..].iter().all(|b| *b == b'=') {
            if output.is_empty() {
                output.push(0);
            }
            return Some(output);
        }
        let a = digit(group[0])?;
        let b = digit(group[1])?;
        let c = if group[2] == b'=' {
            0
        } else {
            digit(group[2])?
        };
        let d = if group[3] == b'=' {
            0
        } else {
            digit(group[3])?
        };
        let padding = usize::from(group[2] == b'=') + usize::from(group[3] == b'=');
        output.push((a << 2) | (b >> 4));
        if padding < 2 {
            output.push((b << 4) | (c >> 2));
        }
        if padding == 0 {
            output.push((c << 6) | d);
        }
        offset += 4;
        if padding > 0 {
            return input[offset..].iter().all(|b| *b == b'=').then_some(output);
        }
    }
    (offset == input.len() || (!output.is_empty() && input[offset..].iter().all(|b| *b == b'=')))
        .then_some(output)
}
fn decoded(payload: &str, encoding: &str, partial: &mut bool) -> Result<Option<Vec<u8>>> {
    let mut bytes = unlatin(payload)?;
    if encoding == "base64" {
        let payload = payload
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        if let Some(decoded) = base64_foundation(payload.as_bytes()) {
            bytes = decoded;
        } else {
            *partial = true;
            return Ok(None);
        }
    } else if encoding == "quoted-printable" {
        let mut output = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'=' {
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 2;
                    continue;
                }
                if let Some(byte) = bytes
                    .get(i + 1..i + 3)
                    .and_then(|v| std::str::from_utf8(v).ok())
                    .and_then(|v| u8::from_str_radix(v, 16).ok())
                {
                    output.push(byte);
                    i += 3;
                    continue;
                }
                *partial = true;
            }
            output.push(bytes[i]);
            i += 1;
        }
        bytes = output;
    } else if !["", "7bit", "8bit", "binary"].contains(&encoding) {
        *partial = true;
        return Ok(None);
    }
    Ok(Some(bytes))
}
fn multipart(
    kind: &str,
    payload: &str,
    depth: usize,
    partial: &mut bool,
) -> Result<(String, String)> {
    let lower = kind.to_ascii_lowercase();
    let Some(index) = lower.find("boundary=") else {
        *partial = true;
        return Ok((String::new(), String::new()));
    };
    let tail = &kind[index + 9..];
    let boundary = tail.strip_prefix('"').map_or_else(
        || {
            tail.split(|c: char| c == ';' || c.is_whitespace())
                .next()
                .unwrap_or_default()
        },
        |v| v.split('"').next().unwrap_or_default(),
    );
    if boundary.is_empty() || boundary.chars().count() > 200 {
        return Err(Error::Corrupt);
    }
    if !payload.contains(&format!("--{boundary}--")) {
        *partial = true;
    }
    let mut text = String::new();
    let mut html = String::new();
    for part in payload.split(&format!("--{boundary}")).skip(1) {
        if part.starts_with("--") {
            break;
        }
        let part = part.strip_prefix('\n').unwrap_or(part);
        let part = part.strip_suffix('\n').unwrap_or(part);
        let decoded = mime(part, depth + 1, partial)?;
        text.push_str(&decoded.0);
        html.push_str(&decoded.1);
    }
    Ok((text, html))
}
fn mime(input: &str, depth: usize, partial: &mut bool) -> Result<(String, String)> {
    if depth >= 16 {
        return Err(Error::Corrupt);
    }
    let normalized = input.replace("\r\n", "\n");
    let Some((head, payload)) = normalized.split_once("\n\n") else {
        *partial = true;
        return Ok((String::new(), String::new()));
    };
    let mut headers = std::collections::BTreeMap::<String, String>::new();
    let mut key = String::new();
    for line in head.split('\n') {
        if line.starts_with([' ', '\t']) {
            let continuation = headers.entry(key.clone()).or_default();
            continuation.push(' ');
            continuation.push_str(line.trim_matches([' ', '\t']));
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        key = name.to_lowercase();
        headers.insert(key.clone(), value.trim_matches([' ', '\t']).into());
    }
    let kind = headers
        .get("content-type")
        .map_or("text/plain", String::as_str);
    let lower = kind.to_lowercase();
    if headers
        .get("content-disposition")
        .is_some_and(|v| v.to_lowercase().starts_with("attachment"))
    {
        return Ok((String::new(), String::new()));
    }
    if lower.starts_with("multipart/") {
        return multipart(kind, payload, depth, partial);
    }
    if !lower.starts_with("text/plain") && !lower.starts_with("text/html") {
        return Ok((String::new(), String::new()));
    }
    let encoding = headers
        .get("content-transfer-encoding")
        .map_or(String::new(), |v| v.to_lowercase());
    let Some(bytes) = decoded(payload, &encoding, partial)? else {
        return Ok((String::new(), String::new()));
    };
    let text = if lower.contains("iso-8859-1") {
        latin(&bytes)
    } else if let Ok(value) = String::from_utf8(bytes) {
        value
    } else {
        *partial = true;
        return Ok((String::new(), String::new()));
    };
    if lower.starts_with("text/plain") {
        Ok((text, String::new()))
    } else {
        Ok((String::new(), format!("<pre>{}</pre>", escaped(&text))))
    }
}
fn strip_scripts(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < value.len() {
        let found = ["script", "style"]
            .iter()
            .filter_map(|kind| {
                lower[cursor..]
                    .find(&format!("<{kind}"))
                    .map(|i| (cursor + i, *kind))
            })
            .min_by_key(|(i, _)| *i);
        let Some((start, kind)) = found else {
            output.push_str(&value[cursor..]);
            break;
        };
        let after = start + 1 + kind.len();
        let word = lower[after..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let closing = format!("</{kind}");
        let end = if word {
            None
        } else {
            lower[after..].find('>').and_then(|head| {
                let begin = after + head + 1;
                let mut offset = begin;
                loop {
                    let index = lower[offset..].find(&closing)? + offset;
                    let tail = index + closing.len();
                    let whitespace = lower[tail..].trim_start_matches(char::is_whitespace);
                    if whitespace.starts_with('>') {
                        return Some(lower.len() - whitespace.len() + 1);
                    }
                    offset = tail;
                }
            })
        };
        if let Some(end) = end {
            output.push_str(&value[cursor..start]);
            cursor = end;
        } else {
            output.push_str(&value[cursor..after]);
            cursor = after;
        }
    }
    output
}
fn strip_tags(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(start) = value[cursor..].find('<') {
        let start = cursor + start;
        let Some(end) = value[start..].find('>') else {
            break;
        };
        output.push_str(&value[cursor..start]);
        cursor = start + end + 1;
    }
    output.push_str(&value[cursor..]);
    output
}
