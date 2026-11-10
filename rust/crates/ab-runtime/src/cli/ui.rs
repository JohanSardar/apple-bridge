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

static WARNINGS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
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
    if Modes::get().color() {
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

pub fn display_width(text: &str) -> usize {
    text.chars().count()
}

pub fn wrap(text: &str, width: usize, first: &str, rest: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = first.to_owned();
    let mut empty = true;
    for word in text.split(' ') {
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
    for line in wrap(text, modes.width(), "  ", "  ") {
        err_line(&line);
    }
}

pub fn blank() {
    err_line("");
}

fn labeled(label: &str, value: &str) -> Vec<String> {
    let width = Modes::get().width();
    let head = if display_width(label) + 2 > LABEL {
        format!("{label}  ")
    } else {
        format!("{label:<LABEL$}")
    };
    if 2 + LABEL + display_width(value) <= width
        || display_width(value) <= width.saturating_sub(4) / 2
    {
        vec![format!("{head}{value}")]
    } else {
        vec![head.trim_end().to_owned(), format!("    {value}")]
    }
}

pub fn row(label: &str, value: &str) {
    for (index, line) in labeled(label, &clean(value)).into_iter().enumerate() {
        if index == 0 {
            err_line(&format!("  {line}"));
        } else {
            err_line(&line);
        }
    }
}

pub fn done(label: &str, value: &str) {
    let value = clean(value);
    if Modes::get().rich() {
        for (index, line) in labeled(label, &value).into_iter().enumerate() {
            if index == 0 {
                err_line(&format!("{} {line}", paint("32", "✓")));
            } else {
                err_line(&line);
            }
        }
    } else if value.is_empty() {
        err_line(label);
    } else {
        err_line(&format!("{label} {value}"));
    }
}

pub fn warn(reason: &str, text: &str) {
    if let Ok(mut warnings) = WARNINGS.lock() {
        warnings.push((reason.to_owned(), text.to_owned()));
    }
    let modes = Modes::get();
    if modes.json {
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
        err_line(&format!("Warning: {text}"));
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

pub fn warnings() -> Vec<(String, String)> {
    WARNINGS.lock().map(|w| w.clone()).unwrap_or_default()
}

pub fn next_step(step: &Step) {
    err_line(&format!("  {}:", step.benefit));
    err_line(&format!("    {}", paint("36", &step.command())));
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
        lines.push(format!("Error: {headline}"));
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
    let shown: Vec<&Step> = failure.next.iter().take(2).collect();
    if !shown.is_empty() {
        lines.push(String::new());
        for (index, step) in shown.iter().enumerate() {
            let benefit = if index == 0 {
                step.benefit.clone()
            } else {
                format!("Or {}", lowercase_first(&step.benefit))
            };
            lines.push(format!("  {benefit}:"));
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

pub fn json_document(mut value: Value) {
    let modes = Modes::get();
    if let Some(fields) = &modes.fields {
        value = select_fields(&value, fields);
    }
    let warnings = warnings();
    if !warnings.is_empty()
        && let Value::Object(map) = &mut value
    {
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

pub fn confirm(
    preview: &[String],
    question: &str,
    with_yes: &Step,
    nothing_changed: &str,
) -> Outcome {
    let modes = Modes::get();
    if modes.yes {
        return Ok(());
    }
    if !modes.interactive() {
        let mut failure = Failure::new(
            Reason::ConfirmationRequired,
            format!(
                "{} {} needs --yes.",
                question.trim_end_matches('?'),
                modes.why_not_interactive()
            ),
        )
        .needs_action()
        .data(
            "effects",
            preview.iter().map(|line| json!(line)).collect::<Vec<_>>(),
        )
        .step(
            Step::new("Only after you approve it, run", with_yes.argv.clone()).when(format!(
                "Only after a human approves: {}",
                preview.join(" ")
            )),
        );
        for line in preview {
            failure = failure.issue(line.clone());
        }
        return Err(failure);
    }
    for line in preview {
        note(line);
    }
    match ask(&format!("{question} (y/N)"))? {
        Some(answer) if matches!(answer.to_lowercase().as_str(), "y" | "yes") => Ok(()),
        _ => Err(Failure::canceled(format!("Canceled. {nothing_changed}"))),
    }
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
        let lines = wrap("aaaa bbbb cccc", 9, "", "  ");
        assert_eq!(lines, ["aaaa bbbb", "  cccc"]);
        let long = wrap("/a/very/long/path/that/never/breaks", 10, "  ", "  ");
        assert_eq!(long, ["  /a/very/long/path/that/never/breaks"]);
    }

    #[test]
    fn sizes_and_counts_read_naturally() {
        assert_eq!(size(12_400_000), "12 MB");
        assert_eq!(size(900), "900 B");
    }
}
