use super::{
    Failure, Outcome,
    failure::{Reason, Step},
    modes::Modes,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{BufRead, Write},
    sync::Mutex,
};

const LABEL: usize = 16;
const MIN_WIDTH: usize = 40;

static WARNINGS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static STREAMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DEPRECATIONS: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
static CANCEL_NOTE: Mutex<Option<String>> = Mutex::new(None);
static PRINTED_STDERR: Mutex<bool> = Mutex::new(false);

fn invisible(character: char) -> bool {
    matches!(character,
        '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{061C}'
        | '\u{FEFF}'
        | '\u{180E}')
}

fn strip(text: &str, keep_newlines: bool) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\u{1b}' => match characters.peek() {
                Some('[') => {
                    characters.next();
                    for next in characters.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    characters.next();
                    while let Some(next) = characters.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' {
                            characters.next();
                            break;
                        }
                    }
                }
                _ => {
                    characters.next();
                }
            },
            '\n' if keep_newlines => output.push('\n'),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() || invisible(character) => {}
            character => output.push(character),
        }
    }
    output
}

pub fn clean(text: &str) -> String {
    strip(text, false)
}

pub fn clean_block(text: &str) -> String {
    strip(text, true)
}

fn url_parts(text: &str) -> Option<(&str, &str, &str, &str)> {
    let (scheme, after) = text.split_once(':')?;
    let mut characters = scheme.chars();
    if scheme.len() < 2
        || !characters
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        || !characters.all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    {
        return None;
    }
    let rest = after.trim_start_matches(['/', '\\']);
    let slashes = &after[..after.len() - rest.len()];
    let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if slashes.is_empty() && !authority.contains('@') {
        return None;
    }
    Some((scheme, slashes, authority, tail))
}

fn without_userinfo(authority: &str) -> String {
    match authority.rsplit_once('@') {
        Some((_, host)) => format!("***@{host}"),
        None => authority.to_owned(),
    }
}

pub fn redact_url(text: &str) -> String {
    let Some((scheme, slashes, authority, tail)) = url_parts(text) else {
        return text.to_owned();
    };
    let tail = match tail.find(['?', '#']) {
        Some(at) => format!("{}{}…", &tail[..at], &tail[at..=at]),
        None => tail.to_owned(),
    };
    format!("{scheme}:{slashes}{}{tail}", without_userinfo(authority))
}

fn scheme_start(word: &str) -> usize {
    let limit = word.find('@').unwrap_or(word.len());
    let bytes = word.as_bytes();
    let part = |b: u8| b.is_ascii_alphanumeric() || b"+.-".contains(&b);
    let mut best = None;
    for (index, byte) in bytes.iter().enumerate().take(limit) {
        if !byte.is_ascii_alphabetic() || index > 0 && part(bytes[index - 1]) {
            continue;
        }
        let mut end = index;
        while end < limit && part(bytes[end]) {
            end += 1;
        }
        if end < limit && bytes[end] == b':' && end - index >= 2 {
            best = Some(index);
            break;
        }
    }
    best.unwrap_or_else(|| {
        word.find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(word.len())
    })
}

fn each_word(text: &str, change: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end_matches(char::is_whitespace);
        let start = scheme_start(word);
        out.push_str(&word[..start]);
        out.push_str(&change(&word[start..]));
        out.push_str(&piece[word.len()..]);
    }
    out
}

pub fn redact_urls(text: &str) -> String {
    each_word(&strip_userinfo(text), redact_url)
}

fn scheme_before(text: &str, end: usize) -> bool {
    let bytes = text.as_bytes();
    let part = |b: u8| b.is_ascii_alphanumeric() || b"+.-".contains(&b);
    let mut start = end;
    while start > 0 && part(bytes[start - 1]) {
        start -= 1;
    }
    while start < end && !bytes[start].is_ascii_alphabetic() {
        start += 1;
    }
    end - start >= 2
}

fn colon_before_space(text: &str) -> bool {
    match text.find(' ') {
        Some(space) => text[..space].contains(':'),
        None => true,
    }
}

fn userinfo_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (at, _) in text.match_indices('@') {
        let mut index = at;
        let mut slash = None;
        while index > 0 {
            let byte = bytes[index - 1];
            if matches!(byte, b'/' | b'?' | b'#') {
                if byte == b'/' {
                    slash = Some(index - 1);
                }
                break;
            }
            if byte == b'\\' && !matches!(bytes.get(index), Some(b'n' | b'r' | b't')) {
                slash = Some(index - 1);
                break;
            }
            index -= 1;
        }
        let span = if let Some(boundary) = slash {
            let mut colon = boundary;
            while colon > 0 && matches!(bytes[colon - 1], b'/' | b'\\') {
                colon -= 1;
            }
            let start = boundary + 1;
            let leading = boundary + 1 - colon;
            let schemed = colon > 0 && bytes[colon - 1] == b':' && scheme_before(text, colon - 1);
            let bare = leading >= 2
                && bytes[boundary] == b'/'
                && (colon == 0
                    || !bytes[colon - 1].is_ascii_alphanumeric() && bytes[colon - 1] != b':');
            ((schemed || bare) && colon_before_space(&text[start..at])).then_some((start, at))
        } else {
            text[index..at]
                .match_indices(':')
                .map(|(offset, _)| index + offset)
                .find(|&colon| {
                    scheme_before(text, colon)
                        && colon_before_space(&text[colon + 1..at])
                        && text[colon + 1..at].contains(':')
                        || scheme_before(text, colon) && !text[colon + 1..at].contains(' ')
                })
                .map(|colon| (colon + 1, at))
        };
        if let Some((start, end)) = span
            && start < end
        {
            match spans.last_mut() {
                Some(last) if last.1 >= start => last.1 = end,
                _ => spans.push((start, end)),
            }
        }
    }
    spans
}

pub fn strip_userinfo(text: &str) -> String {
    if !text.contains('@') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end) in userinfo_spans(text) {
        let start = start.max(last);
        out.push_str(&text[last..start]);
        out.push_str("***");
        last = end;
    }
    out.push_str(&text[last..]);
    out
}

pub fn strip_userinfo_json(value: &mut Value) {
    match value {
        Value::String(text) => *text = strip_userinfo(text),
        Value::Array(items) => items.iter_mut().for_each(strip_userinfo_json),
        Value::Object(map) => map.values_mut().for_each(strip_userinfo_json),
        _ => {}
    }
}

fn parsed_without_userinfo(text: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(text).ok()?;
    if parsed.username().is_empty() && parsed.password().is_none() {
        return None;
    }
    let host = parsed.host_str().unwrap_or_default();
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Some(format!(
        "{}://***@{host}{port}{}",
        parsed.scheme(),
        parsed.path()
    ))
}

pub fn clean_input(text: &str) -> String {
    if text.contains('@')
        && let Some(shown) = parsed_without_userinfo(text)
    {
        return clean(&shown);
    }
    if text.contains('@') && text.contains(':') && text.chars().any(char::is_whitespace) {
        return "***".to_owned();
    }
    clean(&redact_urls(text))
}

pub fn shell_quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:=,@%+<>".contains(c))
        && !(text.contains('<') ^ text.contains('>'))
    {
        text.to_owned()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

pub fn shell_line(argv: &[String]) -> String {
    argv.iter()
        .map(|argument| {
            if argument.starts_with('<') && argument.ends_with('>') && !argument.contains(' ') {
                argument.clone()
            } else {
                shell_quote(argument)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn paint(code: &str, text: &str) -> String {
    let modes = Modes::get();
    if modes.color() && modes.rich() {
        format!("\u{1b}[{code}m{text}\u{1b}[0m")
    } else {
        text.to_owned()
    }
}

pub fn quiet_exit_on_closed_stdout(error: &std::io::Error) {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
}

pub fn out(text: &str) {
    let mut stdout = std::io::stdout().lock();
    if let Err(error) = stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        quiet_exit_on_closed_stdout(&error);
    }
}

pub fn outln(text: &str) {
    let mut line = String::with_capacity(text.len() + 1);
    line.push_str(text);
    line.push('\n');
    out(&line);
}

fn err_raw(text: &str) {
    if let Ok(mut printed) = PRINTED_STDERR.lock() {
        *printed = true;
    }
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(text.as_bytes());
    let _ = stderr.flush();
}

pub fn err_line(text: &str) {
    if Modes::get().json {
        return;
    }
    err_raw(&format!("{text}\n"));
}

pub fn printed_stderr() -> bool {
    PRINTED_STDERR.lock().is_ok_and(|printed| *printed)
}

fn prefix(code: &str, text: &str) -> String {
    if Modes::get().color() {
        format!("\u{1b}[{code}m{text}\u{1b}[0m")
    } else {
        text.to_owned()
    }
}

pub fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

pub fn bold(text: &str) -> String {
    if Modes::get().stdout_styled() {
        format!("\u{1b}[1m{text}\u{1b}[0m")
    } else {
        text.to_owned()
    }
}

pub fn heading(text: &str) -> String {
    if Modes::get().stdout_styled() {
        format!("\u{1b}[1;35m{text}\u{1b}[0m")
    } else {
        text.to_owned()
    }
}

pub fn wrap(text: &str, width: usize, first: &str, rest: &str) -> Vec<String> {
    let width = width.max(MIN_WIDTH);
    let mut lines = Vec::new();
    let mut current = first.to_owned();
    let mut empty = true;
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        if !empty && display_width(&current) + 1 + display_width(word) > width {
            lines.push(std::mem::replace(&mut current, rest.to_owned()));
            empty = true;
        }
        if !empty {
            current.push(' ');
        }
        current.push_str(word);
        empty = false;
    }
    lines.push(current);
    lines
}

pub fn note(text: &str) {
    let modes = Modes::get();
    let indent = if modes.rich() { "  " } else { "" };
    for line in wrap(text, modes.width(), indent, indent) {
        err_line(&line);
    }
}

pub fn phase(number: usize, total: usize, text: &str) {
    let modes = Modes::get();
    if modes.accessible {
        err_line(&format!("Step {number} of {total}: {text}"));
    } else {
        note(text);
    }
}

pub fn blank() {
    err_line("");
}

fn labeled(label: &str, value: &str) -> Vec<String> {
    let width = Modes::get().width();
    let column = " ".repeat(LABEL + 2);
    if value.is_empty() {
        return vec![label.to_owned()];
    }
    if display_width(label) + 2 > LABEL {
        let mut lines = vec![label.to_owned()];
        lines.extend(wrap(value, width, &column, &column));
        return lines;
    }
    let head = format!("{label:<LABEL$}");
    if 2 + LABEL + display_width(value) <= width
        || display_width(value) <= width.saturating_sub(4) / 2
    {
        vec![format!("{head}{value}")]
    } else {
        let mut lines = vec![head.trim_end().to_owned()];
        lines.extend(wrap(value, width.saturating_sub(2), "    ", "    "));
        lines
    }
}

pub fn row(label: &str, value: &str) {
    let value = clean(value);
    if !Modes::get().rich() {
        if value.is_empty() {
            err_line(label);
        } else {
            err_line(&format!("{label}: {value}"));
        }
        return;
    }
    for (index, line) in labeled(label, &value).into_iter().enumerate() {
        if index == 0 {
            err_line(&format!("  {line}"));
        } else {
            err_line(&line);
        }
    }
}

pub fn done(label: &str, value: &str) {
    let value = clean(value);
    let sentence = if value.is_empty() {
        format!("{label}.")
    } else {
        format!("{label} {value}.")
    };
    receipt(label, &value, &sentence);
}

pub fn receipt(label: &str, value: &str, sentence: &str) {
    let value = clean(value);
    if !Modes::get().rich() {
        for line in wrap(&clean(sentence), Modes::get().width(), "", "") {
            err_line(&line);
        }
        return;
    }
    let glyph = paint("32", "✓");
    if value.is_empty() {
        err_line(&format!("{glyph} {label}"));
    } else if display_width(label) + 2 > LABEL {
        for (index, line) in wrap(&format!("{label} {value}"), Modes::get().width(), "", "  ")
            .into_iter()
            .enumerate()
        {
            if index == 0 {
                err_line(&format!("{glyph} {line}"));
            } else {
                err_line(&line);
            }
        }
    } else {
        for (index, line) in labeled(label, &value).into_iter().enumerate() {
            if index == 0 {
                err_line(&format!("{glyph} {line}"));
            } else {
                err_line(&line);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Good,
    Bad,
}

fn mark(mark: Mark) -> String {
    if !Modes::get().stdout_styled() && !Modes::get().rich() {
        return " ".to_owned();
    }
    let (code, glyph) = match mark {
        Mark::Good => ("32", "✓"),
        Mark::Bad => ("31", "✗"),
    };
    if Modes::get().color() && Modes::get().stdout_styled() {
        format!("\u{1b}[{code}m{glyph}\u{1b}[0m")
    } else {
        glyph.to_owned()
    }
}

pub fn block(first: Mark, rows: &[(String, String)]) {
    for (index, (label, value)) in rows.iter().enumerate() {
        for (line_index, line) in labeled(label, &clean(value)).into_iter().enumerate() {
            if line_index > 0 {
                outln(&line);
            } else if index == 0 {
                outln(format!("{} {}", mark(first), line.trim_end()).trim_end());
            } else {
                outln(&format!("  {}", line.trim_end()));
            }
        }
    }
}

pub fn stream_warnings() {
    STREAMING.store(true, std::sync::atomic::Ordering::Relaxed);
    if super::modes::legacy_spelling() {
        return;
    }
    for (reason, message) in warnings() {
        warning_line(&reason, &message);
    }
}

fn warning_line(reason: &str, message: &str) {
    outln(&json!({"type": "warning", "reason": reason, "message": message}).to_string());
}

pub fn warn(reason: &str, text: &str) {
    let text = &strip_userinfo(text);
    if let Ok(mut warnings) = WARNINGS.lock() {
        warnings.push((reason.to_owned(), text.to_owned()));
    }
    let modes = Modes::get();
    if modes.json {
        if STREAMING.load(std::sync::atomic::Ordering::Relaxed) && !super::modes::legacy_spelling()
        {
            warning_line(reason, text);
        }
        return;
    }
    if modes.rich() {
        let lines = wrap(text, modes.width(), "", "  ");
        for (index, line) in lines.iter().enumerate() {
            if index == 0 {
                err_line(&format!("{} {line}", paint("33", "!")));
            } else {
                err_line(line);
            }
        }
    } else {
        let lines = wrap(text, modes.width(), "", "  ");
        for (index, line) in lines.iter().enumerate() {
            if index == 0 {
                err_line(&format!("{} {line}", prefix("33", "Warning:")));
            } else {
                err_line(line);
            }
        }
    }
}

pub fn deprecated(old: &str, replacement: &str) {
    let fresh = DEPRECATIONS
        .lock()
        .is_ok_and(|mut seen| seen.insert(old.to_owned()));
    if fresh {
        warn(
            "deprecated",
            &format!("{old} is deprecated. Use {replacement}."),
        );
    }
}

static EMBEDDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn report_legacy_warnings() {
    if !Modes::get().json
        || !super::modes::legacy_spelling()
        || EMBEDDED.load(std::sync::atomic::Ordering::Relaxed)
        || STREAMING.load(std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    for (reason, message) in warnings() {
        err_raw(&format!(
            "{}\n",
            json!({"status": "warning", "reason": reason, "message": message})
        ));
    }
}

pub fn warnings() -> Vec<(String, String)> {
    WARNINGS.lock().map(|w| w.clone()).unwrap_or_default()
}

pub fn next_step(step: &Step) {
    let (lead, command) = if Modes::get().rich() {
        ("  ", "    ")
    } else {
        ("", "  ")
    };
    for line in wrap(
        &format!("{}:", step.benefit),
        Modes::get().width(),
        lead,
        lead,
    ) {
        err_line(&line);
    }
    err_line(&format!("{command}{}", paint("36", &step.command())));
}

pub fn next_steps(steps: &[Step]) {
    if steps.is_empty() || Modes::get().json {
        return;
    }
    blank();
    for step in steps {
        next_step(step);
    }
}

pub fn set_cancel_note(note: Option<&str>) {
    if let Ok(mut current) = CANCEL_NOTE.lock() {
        *current = note.map(str::to_owned);
    }
}

pub fn cancel_note() -> Option<String> {
    CANCEL_NOTE
        .lock()
        .ok()
        .and_then(|note| note.clone())
        .or_else(|| Some("Canceled.".to_owned()))
        .filter(|note| !note.is_empty())
}

pub fn render_failure(failure: &Failure) {
    let modes = Modes::get();
    if modes.json {
        if !failure.quiet_in_json {
            err_raw(&format!("{}\n", failure.to_json(&warnings())));
        }
        return;
    }
    if failure.reason == Reason::Canceled {
        if !failure.headline.is_empty() {
            err_raw(&format!("{}\n", failure.headline));
        }
        return;
    }
    let mut lines = Vec::new();
    let headline = clean(&failure.headline);
    if modes.rich() {
        let wrapped = wrap(&headline, modes.width(), "", "  ");
        for (index, line) in wrapped.into_iter().enumerate() {
            lines.push(if index == 0 {
                format!("{} {line}", paint("31", "✗"))
            } else {
                line
            });
        }
    } else {
        let wrapped = wrap(&headline, modes.width(), "", "  ");
        for (index, line) in wrapped.into_iter().enumerate() {
            lines.push(if index == 0 {
                format!("{} {line}", prefix("31", "Error:"))
            } else {
                line
            });
        }
    }
    for issue in failure.issues.iter().take(5) {
        lines.push(format!("  {}", clean(issue)));
    }
    if failure.issues.len() > 5 {
        lines.push(format!("  and {} more", failure.issues.len() - 5));
    }
    for (label, value) in &failure.rows {
        for (index, line) in labeled(label, &clean(value)).into_iter().enumerate() {
            lines.push(if index == 0 {
                format!("  {line}")
            } else {
                line
            });
        }
    }
    if let Some((source, text)) = &failure.remote {
        lines.push(format!("  {source} said: {}", clean(text)));
    }
    if let Some(sentence) = &failure.sentence {
        lines.push(String::new());
        for line in wrap(sentence, modes.width(), "  ", "  ") {
            lines.push(line);
        }
    }
    let shown: Vec<&Step> = failure.next.iter().take(3).collect();
    if !shown.is_empty() {
        lines.push(String::new());
        for (index, step) in shown.iter().enumerate() {
            let keeps_lead = ["or ", "then ", "and "]
                .iter()
                .any(|lead| step.benefit.to_lowercase().starts_with(lead));
            let benefit = if index == 0 || keeps_lead {
                step.benefit.clone()
            } else {
                format!("Or {}", lowercase_first(&step.benefit))
            };
            if index > 0 {
                lines.push(String::new());
            }
            lines.extend(wrap(&format!("{benefit}:"), modes.width(), "  ", "  "));
            lines.push(format!("    {}", paint("36", &step.command())));
        }
    }
    let lead = if printed_stderr() { "\n" } else { "" };
    err_raw(&format!("{lead}{}\n", lines.join("\n")));
}

fn lowercase_first(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_lowercase().chain(characters).collect()
    })
}

fn select_fields(value: &Value, fields: &[String]) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| select_fields(item, fields))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| fields.iter().any(|field| field == *key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn select_keyed(value: &Value, noun: &str, fields: &[String]) -> Value {
    let Value::Object(map) = value else {
        return select_fields(value, fields);
    };
    let inner: Vec<String> = fields
        .iter()
        .filter(|field| !map.contains_key(field.as_str()))
        .cloned()
        .collect();
    Value::Object(
        map.iter()
            .filter_map(|(key, child)| {
                if fields.iter().any(|field| field == key) {
                    Some((key.clone(), child.clone()))
                } else if key == noun && !inner.is_empty() {
                    Some((key.clone(), select_fields(child, &inner)))
                } else {
                    None
                }
            })
            .collect(),
    )
}

pub fn json_keyed(noun: &str, mut value: Value) {
    if let Some(fields) = &Modes::get().fields {
        value = select_keyed(&value, noun, fields);
    }
    json_document_unfiltered(value);
}

pub fn json_document(mut value: Value) {
    if let Some(fields) = &Modes::get().fields {
        value = select_fields(&value, fields);
    }
    json_document_unfiltered(value);
}

pub fn json_document_unfiltered(mut value: Value) {
    let modes = Modes::get();
    let warnings = warnings();
    if !warnings.is_empty()
        && let Value::Object(map) = &mut value
    {
        EMBEDDED.store(true, std::sync::atomic::Ordering::Relaxed);
        map.insert(
            "warnings".into(),
            Value::Array(
                warnings
                    .into_iter()
                    .map(|(reason, message)| json!({"reason": reason, "message": message}))
                    .collect(),
            ),
        );
    }
    let text = if modes.stdout_tty() {
        serde_json::to_string_pretty(&value).unwrap_or_default()
    } else {
        value.to_string()
    };
    outln(&text);
}

pub fn exact<T: serde::Serialize>(value: &T) {
    outln(&serde_json::to_string(value).unwrap_or_default());
}

pub fn exact_json(value: &Value) {
    outln(&value.to_string());
}

pub fn table(headers: &[&str], rows: &[Vec<String>]) {
    let modes = Modes::get();
    if !modes.stdout_table() {
        outln(&headers.join("\t"));
        for row in rows {
            outln(
                &row.iter()
                    .map(|cell| clean(cell))
                    .collect::<Vec<_>>()
                    .join("\t"),
            );
        }
        return;
    }
    let cleaned: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.iter().map(|cell| clean(cell)).collect())
        .collect();
    let mut widths: Vec<usize> = headers.iter().map(|header| display_width(header)).collect();
    for row in &cleaned {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(display_width(cell));
            }
        }
    }
    let total: usize = 2 + widths.iter().sum::<usize>() + 3 * widths.len().saturating_sub(1);
    if total > modes.width() && headers.len() > 1 {
        for (index, row) in cleaned.iter().enumerate() {
            if index > 0 {
                outln("");
            }
            for (header, cell) in headers.iter().zip(row) {
                for (line_index, line) in labeled(header, cell).into_iter().enumerate() {
                    outln(&if line_index == 0 {
                        format!("  {line}")
                    } else {
                        line
                    });
                }
            }
        }
        return;
    }
    let line = |cells: Vec<String>| {
        let mut text = String::from("  ");
        let last = cells.len().saturating_sub(1);
        for (index, cell) in cells.iter().enumerate() {
            text.push_str(cell);
            if index < last {
                let pad = widths[index].saturating_sub(display_width(cell)) + 3;
                text.push_str(&" ".repeat(pad));
            }
        }
        text
    };
    outln(&line(
        headers.iter().map(|header| (*header).to_owned()).collect(),
    ));
    for row in cleaned {
        outln(&line(row));
    }
}

pub fn wrapped_table(headers: &[&str], rows: &[Vec<String>]) {
    let modes = Modes::get();
    if !modes.stdout_table() || headers.len() < 2 {
        table(headers, rows);
        return;
    }
    let cleaned: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.iter().map(|cell| clean(cell)).collect())
        .collect();
    let last = headers.len() - 1;
    let mut widths: Vec<usize> = headers.iter().map(|header| display_width(header)).collect();
    for row in &cleaned {
        for (index, cell) in row.iter().enumerate().take(last) {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(display_width(cell));
            }
        }
    }
    let fixed: usize = 2 + widths
        .iter()
        .take(last)
        .map(|width| width + 3)
        .sum::<usize>();
    let room = modes.width().saturating_sub(fixed);
    if room < 16 {
        table(headers, rows);
        return;
    }
    let indent = " ".repeat(fixed);
    let line = |cells: &[String]| {
        let mut text = String::from("  ");
        for (index, cell) in cells.iter().enumerate().take(last) {
            text.push_str(cell);
            text.push_str(&" ".repeat(widths[index].saturating_sub(display_width(cell)) + 3));
        }
        let tail = cells.get(last).map(String::as_str).unwrap_or_default();
        let mut wrapped = wrap(tail, fixed + room, &indent, &indent).into_iter();
        text.push_str(wrapped.next().unwrap_or_default().trim_start());
        outln(text.trim_end());
        for more in wrapped {
            outln(&more);
        }
    };
    line(
        &headers
            .iter()
            .map(|header| (*header).to_owned())
            .collect::<Vec<_>>(),
    );
    for row in &cleaned {
        line(row);
    }
}

pub fn mask_secret(secret: &str) {
    if Modes::get().github_actions {
        err_raw(&format!("::add-mask::{secret}\n"));
    }
}

pub fn read_answer() -> Outcome<Option<String>> {
    let mut line = String::new();
    let read = std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|_| Failure::canceled("Canceled."))?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

pub fn ask(question: &str) -> Outcome<Option<String>> {
    let modes = Modes::get();
    let glyph = if modes.rich() {
        format!("{} ", paint("36", "?"))
    } else {
        String::new()
    };
    err_raw(&format!("{glyph}{question} "));
    let answer = read_answer()?;
    if answer.is_none() {
        err_raw("\n");
    }
    Ok(answer)
}

pub fn choose(question: &str, choices: &[(&str, &str)]) -> Outcome<Option<usize>> {
    if !Modes::get().accessible {
        let keys: Vec<&str> = choices.iter().map(|(key, _)| *key).collect();
        let answer = ask(&format!("{question} ({})", keys.join("/")))?;
        return Ok(answer.map(|answer| {
            let answer = answer.to_lowercase();
            choices
                .iter()
                .position(|(key, label)| {
                    answer == key.to_lowercase()
                        || answer == label.to_lowercase()
                        || answer.is_empty() && key.chars().all(char::is_uppercase)
                })
                .unwrap_or(usize::MAX)
        }));
    }
    err_raw(&format!("{question}\n"));
    for (index, (_, label)) in choices.iter().enumerate() {
        err_raw(&format!("  {}. {label}\n", index + 1));
    }
    let answer = ask(&format!("Type a number from 1 to {}:", choices.len()))?;
    Ok(answer.map(|answer| {
        answer
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|number| (1..=choices.len()).contains(number))
            .map_or(usize::MAX, |number| number - 1)
    }))
}

pub struct Consent<'a> {
    pub action: String,
    pub rows: Vec<(String, String)>,
    pub consequence: String,
    pub question: String,
    pub with_yes: Step,
    pub nothing_changed: &'a str,
}

pub fn confirm(consent: &Consent<'_>) -> Outcome {
    let modes = Modes::get();
    if modes.yes {
        return Ok(());
    }
    let mut effects: Vec<Value> = consent
        .rows
        .iter()
        .map(|(label, value)| json!({"label": label, "value": value}))
        .collect();
    if !consent.consequence.is_empty() {
        effects.push(json!({"label": "Consequence", "value": consent.consequence}));
    }
    if !modes.interactive() {
        let mut failure = Failure::new(
            Reason::ConfirmationRequired,
            format!(
                "Can't {} without --yes in non-interactive mode.",
                consent.action
            ),
        )
        .needs_action()
        .data("effects", effects);
        for (label, value) in &consent.rows {
            failure = failure.row(label.clone(), value.clone());
        }
        if !consent.consequence.is_empty() {
            failure = failure.sentence(consent.consequence.clone());
        }
        return Err(failure.step(
            Step::new(
                "Only after you approve it, run",
                consent.with_yes.argv.clone(),
            )
            .when("Only after a human approves the change shown"),
        ));
    }
    for (label, value) in &consent.rows {
        row(label, value);
    }
    if !consent.consequence.is_empty() {
        note(&consent.consequence);
    }
    if yes_or_no(&consent.question)? {
        Ok(())
    } else {
        Err(Failure::canceled(format!(
            "Canceled. {}",
            consent.nothing_changed
        )))
    }
}

pub fn yes_or_no(question: &str) -> Outcome<bool> {
    if Modes::get().accessible {
        return Ok(choose(question, &[("y", "Yes"), ("N", "No")])? == Some(0));
    }
    Ok(matches!(
        ask(&format!("{question} (y/N)"))?
            .map(|answer| answer.to_lowercase())
            .as_deref(),
        Some("y" | "yes")
    ))
}

pub fn size(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1_000 {
        format!("{bytes} B")
    } else if bytes < 1_000_000 {
        format!("{:.0} kB", value / 1e3)
    } else if bytes < 1_000_000_000 {
        format!("{:.0} MB", value / 1e6)
    } else {
        format!("{:.1} GB", value / 1e9)
    }
}

pub fn age_short(timestamp: &str) -> String {
    let shown = age(timestamp);
    shown
        .strip_suffix(" ago")
        .map_or(shown.clone(), str::to_owned)
}

pub fn local_time(timestamp: &str) -> String {
    let Ok(then) = timestamp.parse::<jiff::Timestamp>() else {
        return clean(timestamp);
    };
    let zoned = then.to_zoned(jiff::tz::TimeZone::system());
    let stamp = zoned.strftime("%Y-%m-%d %H:%M %Z").to_string();
    if then > jiff::Timestamp::now() {
        let left = then.as_second() - jiff::Timestamp::now().as_second();
        let left = std::time::Duration::from_secs(u64::try_from(left).unwrap_or(0));
        format!("{stamp} (in {})", super::commands::friendly(left))
    } else {
        format!("{stamp} ({})", age(timestamp))
    }
}

pub fn age(timestamp: &str) -> String {
    let Ok(then) = timestamp.parse::<jiff::Timestamp>() else {
        return clean(timestamp);
    };
    let seconds = jiff::Timestamp::now().as_second() - then.as_second();
    match seconds {
        s if s < 0 => "just now".to_owned(),
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s if s < 7 * 86_400 => format!("{}d ago", s / 86_400),
        _ => then.strftime("%Y-%m-%d").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo_is_stripped_and_ordinary_text_is_kept() {
        for (text, shown) in [
            ("see https://me:pw@h.test/x", "see https://***@h.test/x"),
            ("https:me:pw@h.test", "https:***@h.test"),
            ("https:/me:pw@h.test", "https:/***@h.test"),
            ("url=https://me:pw@h.test", "url=https://***@h.test"),
            ("(HTTPS://me:pw@h.test)", "(HTTPS://***@h.test)"),
            ("https://me:\\npw@h.test", "https://***@h.test"),
            ("https://me:\\tpw@h.test", "https://***@h.test"),
            ("https://me:\npw@h.test", "https://***@h.test"),
            ("https://me:\r\tpw@h.test", "https://***@h.test"),
            ("https://me:p w@h.test", "https://***@h.test"),
            ("https://me@corp:pw@h.test/x", "https://***@h.test/x"),
            ("https:me:\\npw@h.test", "https:***@h.test"),
            ("https:user:hunter2 pw@h.test", "https:***@h.test"),
            ("see //user:hunter2@h.test now", "see //***@h.test now"),
            ("//user:hunter2@h.test", "//***@h.test"),
        ] {
            assert_eq!(strip_userinfo(text), shown, "{text}");
        }
        for text in [
            "mail a@b.com now",
            "user@host:22",
            "127.0.0.1:7780",
            "at 14:03 today",
            r"C:\a@b.com\config",
            "https://login.example.test/sign-in?code=abc#x",
            "ssh me@host.test",
            "see https://h.test and mail a@b.com",
            "Error: Can't use a@b.com",
            "https://h.test and Error: contact a@b.com",
            "Note: at 14:03 mail a@b.com",
            "a/b//c@d.test",
            "名前@例え.jp と https://ü.test",
        ] {
            assert_eq!(strip_userinfo(text), text, "{text}");
        }
        assert_eq!(clean_input("https://me:\tpw@h.test"), "https://***@h.test/");
        let mut value = json!({"effects": [{"value": "https://me:pw@h.test"}]});
        strip_userinfo_json(&mut value);
        assert_eq!(value["effects"][0]["value"], "https://***@h.test");
    }

    #[test]
    fn untrusted_text_loses_controls_and_shows_newlines() {
        assert_eq!(clean("a\u{1b}[31mred\u{1b}[0m"), "ared");
        assert_eq!(clean("x\u{202E}y\u{200B}z"), "xyz");
        assert_eq!(clean("one\ntwo\tthree"), "one\\ntwo\\tthree");
        assert_eq!(clean("\u{1b}]8;;http://evil\u{7}link"), "link");
        assert_eq!(clean_block("a\nb\u{7}"), "a\nb");
    }

    #[test]
    fn commands_are_quoted_for_the_shell_and_keep_placeholders() {
        assert_eq!(
            shell_line(&[
                "bridge".into(),
                "up".into(),
                "--home".into(),
                "/tmp/a b".into()
            ]),
            "bridge up --home '/tmp/a b'"
        );
        assert_eq!(
            shell_line(&[
                "bridge".into(),
                "agents".into(),
                "revoke".into(),
                "<AGENT>".into()
            ]),
            "bridge agents revoke <AGENT>"
        );
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn prose_wraps_without_breaking_words() {
        let words = "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii";
        let lines = wrap(words, 9, "", "  ");
        assert_eq!(lines, ["aaaa bbbb cccc dddd eeee ffff gggg hhhh", "  iiii"]);
        let wide = wrap("界界界界界界界界界界界界界界界界界界界界 x", 40, "", "");
        assert_eq!(wide, ["界界界界界界界界界界界界界界界界界界界界", "x"]);
        let long = wrap("/a/very/long/path/that/never/breaks", 10, "  ", "  ");
        assert_eq!(long, ["  /a/very/long/path/that/never/breaks"]);
    }

    #[test]
    fn sizes_and_counts_read_naturally() {
        assert_eq!(size(12_400_000), "12 MB");
        assert_eq!(size(900), "900 B");
    }
}
