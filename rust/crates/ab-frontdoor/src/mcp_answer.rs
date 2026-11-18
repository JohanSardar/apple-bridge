use serde_json::{Map, Value, json};

pub(crate) const DEFAULT_TOKENS: usize = 2_000;
pub(crate) const MAX_TOKENS: usize = 8_000;
pub(crate) const DEFAULT_ITEMS: usize = 20;
pub(crate) const MAX_ITEMS: usize = 100;
const LIST_TEXT: usize = 240;
const LONG_TEXT: usize = 1_500;
const SHORT_TEXT: usize = 160;
const HEAVY: &[&str] = &[
    "blocks",
    "html",
    "body_html",
    "store_coverage",
    "store_alarms",
    "store_recurrence",
    "store_identifier",
    "store_status",
    "native_fingerprint",
    "round_trip_status",
    "thumbnail",
];

pub(crate) fn tokens(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| (bytes.len() * 10).div_ceil(35))
}

fn bytes_for(tokens: usize) -> usize {
    tokens * 35 / 10
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Window {
    pub offset: usize,
    pub limit: usize,
}

pub(crate) fn window(input: &Map<String, Value>, default: usize) -> Window {
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(default)
        .clamp(1, MAX_ITEMS);
    let offset = input
        .get("offset")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0);
    Window { offset, limit }
}

pub(crate) fn asks_for_more(input: &Map<String, Value>) -> bool {
    ["verbose", "detail"]
        .iter()
        .any(|key| input.get(*key).and_then(Value::as_bool) == Some(true))
        || input
            .get("limit")
            .and_then(Value::as_u64)
            .is_some_and(|n| n > DEFAULT_ITEMS as u64)
}

const WHOLE: &[&str] = &[
    "records.get",
    "mail.body",
    "files.read",
    "blobs.read",
    "photos.fetch",
];

fn whole_operation(name: &str) -> bool {
    WHOLE.contains(&crate::mcp_registry::normal_name(name).as_str())
        || WHOLE.contains(&name.replace('_', ".").as_str())
}

pub(crate) fn whole(tool: &str, input: &Map<String, Value>) -> bool {
    if tool != "apple_read" {
        return whole_operation(tool);
    }
    input
        .get("operation")
        .and_then(Value::as_str)
        .is_some_and(whole_operation)
        || input
            .get("reads")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|read| read["operation"].as_str())
            .any(whole_operation)
}

pub(crate) fn budget(tool: &str, input: &Map<String, Value>) -> usize {
    let single = whole(tool, input);
    let documentation = matches!(tool, "apple_describe" | "apple_guide");
    let batch = tool == "apple_read" && input.contains_key("reads");
    let wide = input
        .get("arguments")
        .and_then(|arguments| arguments.get("limit"))
        .and_then(Value::as_u64)
        .is_some_and(|n| n > DEFAULT_ITEMS as u64);
    if single || documentation || batch || wide || asks_for_more(input) {
        MAX_TOKENS
    } else {
        DEFAULT_TOKENS
    }
}

pub(crate) fn page(items: Vec<Value>, window: Window, narrow: &str) -> (Vec<Value>, Value) {
    let total = items.len();
    let kept: Vec<Value> = items
        .into_iter()
        .skip(window.offset)
        .take(window.limit)
        .collect();
    let end = window.offset.min(total) + kept.len();
    let mut page = json!({
        "total": total,
        "returned": kept.len(),
        "truncated": end < total,
        "narrow": narrow,
    });
    if window.offset > 0 {
        page["offset"] = json!(window.offset);
    }
    if end < total {
        page["next"] = json!({"offset": end});
    }
    (kept, page)
}

pub(crate) fn cursor_page(
    returned: usize,
    next: Option<Value>,
    total: Option<usize>,
    narrow: &str,
) -> Value {
    let mut page = json!({
        "returned": returned,
        "truncated": next.is_some(),
        "narrow": narrow,
    });
    match total {
        Some(total) => page["total"] = json!(total),
        None if next.is_some() => page["total"] = json!(format!("more than {returned}")),
        None => page["total"] = json!(returned),
    }
    if let Some(next) = next {
        page["next"] = next;
    }
    page
}

pub(crate) fn clip(text: &str, chars: usize) -> (String, bool) {
    if text.chars().count() <= chars {
        return (text.to_owned(), false);
    }
    let mut out: String = text.chars().take(chars).collect();
    out.push('…');
    (out, true)
}

pub(crate) fn short(text: &str) -> String {
    clip(text.trim(), SHORT_TEXT).0
}

pub(crate) fn list_item(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut out = Map::new();
            let mut clipped = Vec::new();
            for (key, child) in fields {
                if HEAVY.contains(&key.as_str()) {
                    continue;
                }
                match child {
                    Value::String(text) => {
                        let (kept, cut) = clip(text, LIST_TEXT);
                        if cut {
                            clipped.push(json!(key));
                        }
                        out.insert(key.clone(), json!(kept));
                    }
                    Value::Object(_) => {
                        out.insert(key.clone(), list_item(child));
                    }
                    other => {
                        out.insert(key.clone(), other.clone());
                    }
                }
            }
            if !clipped.is_empty() {
                out.insert("clipped".into(), Value::Array(clipped));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

pub(crate) fn upstream_cursor(raw: &Value) -> Option<Value> {
    [("next", "after"), ("next_cursor", "cursor")]
        .iter()
        .find_map(|(key, argument)| {
            raw.get(*key)
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .map(|cursor| json!({ *argument: cursor }))
        })
}

pub(crate) fn paged(value: &mut Value, window: Window, narrow: &str, upstream: Option<&Value>) {
    let Some(fields) = value.as_object_mut() else {
        return;
    };
    let keys: Vec<String> = fields
        .iter()
        .filter(|(_, child)| child.as_array().is_some_and(|items| items.len() > 1))
        .map(|(key, _)| key.clone())
        .collect();
    let mut pages = Map::new();
    for key in keys {
        let Some(Value::Array(items)) = fields.remove(&key) else {
            continue;
        };
        let items: Vec<Value> = items.iter().map(list_item).collect();
        let fetched = items.len();
        let (kept, mut info) = page(items, window, narrow);
        if let Some(cursor) = upstream {
            info["truncated"] = json!(true);
            info["total"] = json!(format!("more than {fetched}"));
            if info.get("next").is_none() {
                info["next"] = json!({"arguments": cursor, "offset": 0});
            }
        }
        fields.insert(key.clone(), Value::Array(kept));
        pages.insert(key, info);
    }
    match pages.len() {
        0 => {}
        1 => {
            if let Some((_, info)) = pages.into_iter().next() {
                fields.insert("page".into(), info);
            }
        }
        _ => {
            fields.insert("pages".into(), Value::Object(pages));
        }
    }
}

enum Largest<'a> {
    Array(&'a mut Vec<Value>, String),
    Text(&'a mut String, String),
}

fn find(
    value: &Value,
    path: &str,
    arrays: &mut Vec<(usize, String)>,
    texts: &mut Vec<(usize, String)>,
) {
    match value {
        Value::Array(items) => {
            if items.len() > 1 {
                arrays.push((
                    serde_json::to_vec(value).map_or(0, |b| b.len()),
                    path.to_owned(),
                ));
            }
            for (index, item) in items.iter().enumerate() {
                find(item, &format!("{path}/{index}"), arrays, texts);
            }
        }
        Value::Object(fields) => {
            for (key, child) in fields {
                if key == "cut" || key == "page" || key == "pages" {
                    continue;
                }
                find(child, &format!("{path}/{}", escape(key)), arrays, texts);
            }
        }
        Value::String(text) if text.len() > LONG_TEXT => {
            texts.push((text.len(), path.to_owned()));
        }
        _ => {}
    }
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn label(path: &str) -> String {
    path.trim_start_matches('/').replace('/', ".")
}

fn largest<'a>(value: &'a mut Value, settled: &[String]) -> Option<Largest<'a>> {
    let mut arrays = Vec::new();
    let mut texts = Vec::new();
    find(value, "", &mut arrays, &mut texts);
    texts.retain(|(_, path)| !settled.contains(path));
    let array = arrays.into_iter().max_by_key(|(size, _)| *size);
    let text = texts.into_iter().max_by_key(|(size, _)| *size);
    let prefer_text = match (&array, &text) {
        (Some((array_size, _)), Some((text_size, _))) => text_size * 2 >= *array_size,
        (None, Some(_)) => true,
        _ => false,
    };
    if prefer_text {
        let (_, path) = text?;
        let Value::String(target) = value.pointer_mut(&path)? else {
            return None;
        };
        return Some(Largest::Text(target, path));
    }
    let (_, path) = array?;
    let Value::Array(target) = value.pointer_mut(&path)? else {
        return None;
    };
    Some(Largest::Array(target, path))
}

pub(crate) fn fit(value: &mut Value, budget: usize) {
    let limit = bytes_for(budget);
    let mut dropped: Map<String, Value> = Map::new();
    let mut shortened: Vec<Value> = Vec::new();
    let mut settled: Vec<String> = Vec::new();
    let mut unpaged = false;
    for _ in 0..256 {
        let size = serde_json::to_vec(value).map_or(0, |b| b.len());
        let reserve = 400;
        if size + reserve <= limit {
            break;
        }
        let excess = size + reserve - limit;
        match largest(value, &settled) {
            Some(Largest::Array(items, path)) => {
                let mut removed = 0;
                let mut count = 0;
                while items.len() > 1 && removed < excess {
                    if let Some(last) = items.pop() {
                        removed += serde_json::to_vec(&last).map_or(0, |b| b.len()) + 1;
                        count += 1;
                    }
                }
                let key = label(&path);
                let before = dropped.get(&key).and_then(Value::as_u64).unwrap_or(0);
                dropped.insert(key, json!(before + count as u64));
                if !adjust_page(value, &path, count) {
                    unpaged = true;
                }
            }
            Some(Largest::Text(text, path)) => {
                let chars = text.chars().count();
                let keep = chars.saturating_sub(excess + 8).max(LONG_TEXT / 2);
                if keep >= chars {
                    settled.push(path);
                    continue;
                }
                let (kept, _) = clip(text, keep);
                *text = kept;
                let key = json!(label(&path));
                if !shortened.contains(&key) {
                    shortened.push(key);
                }
            }
            None => break,
        }
    }
    if dropped.is_empty() && shortened.is_empty() {
        return;
    }
    let mut cut = json!({
        "reason": format!("This answer is kept under about {budget} tokens."),
        "next": "Ask for fewer items with limit, narrow the request, keep only some fields with fields, or read one item by id for its full text.",
    });
    if !dropped.is_empty() {
        cut["dropped_items"] = Value::Object(dropped);
    }
    if unpaged {
        cut["unpaged"] = json!(
            "Some dropped items are in a list without page; ask without verbose, or with a smaller limit or narrower arguments, to see them."
        );
    }
    if !shortened.is_empty() {
        cut["shortened_text"] = Value::Array(shortened);
    }
    if let Some(fields) = value.as_object_mut() {
        fields.insert("cut".into(), cut);
    } else {
        *value = json!({"items": value.clone(), "cut": cut});
    }
}

fn adjust_page(value: &mut Value, path: &str, count: usize) -> bool {
    let Some((parent, key)) = path.rsplit_once('/') else {
        return false;
    };
    let Some(parent) = value.pointer_mut(parent) else {
        return false;
    };
    let Some(container) = parent.as_object_mut() else {
        return false;
    };
    let own = format!("{key}_page");
    let target = if container.contains_key(&own) {
        container.get_mut(&own)
    } else if container.get("page").is_some() {
        container.get_mut("page")
    } else {
        container
            .get_mut("pages")
            .and_then(|pages| pages.get_mut(key))
    };
    let Some(info) = target.and_then(Value::as_object_mut) else {
        return false;
    };
    let returned = info
        .get("returned")
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .saturating_sub(count as u64);
    info.insert("returned".into(), json!(returned));
    info.insert("truncated".into(), json!(true));
    let resumable = info
        .get("next")
        .is_none_or(|next| next.get("offset").and_then(Value::as_u64).is_some());
    if resumable {
        let start = info.get("offset").and_then(Value::as_u64).unwrap_or(0);
        info.insert("next".into(), json!({"offset": start + returned}));
    } else {
        let left_out = info
            .get("left_out")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            + count as u64;
        info.insert("left_out".into(), json!(left_out));
        info.insert(
            "gap".into(),
            json!(
                "left_out items were dropped to fit; ask again with a smaller limit to see them."
            ),
        );
    }
    true
}
