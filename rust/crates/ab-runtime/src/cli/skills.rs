use super::{Failure, Outcome, failure::Reason, modes::Modes, ui};

pub const SKILLS: &[(&str, &str, &str)] = &[
    (
        "core",
        "What Apple Bridge is, how to read safely, and the tools to start with",
        include_str!("../../skills/core.md"),
    ),
    (
        "setup",
        "How to set up Apple Bridge on this Mac and connect an agent",
        include_str!("../../skills/setup.md"),
    ),
    (
        "messages",
        "Reading chats, sending messages and attachments",
        include_str!("../../skills/messages.md"),
    ),
    (
        "reminders-calendar",
        "Reminders and calendar events, dates and versions",
        include_str!("../../skills/reminders-calendar.md"),
    ),
    (
        "photos",
        "Finding, viewing, downloading and organizing photos",
        include_str!("../../skills/photos.md"),
    ),
    (
        "files",
        "Files, documents and moving bytes through MCP",
        include_str!("../../skills/files.md"),
    ),
];

#[allow(clippy::unnecessary_wraps)]
pub fn list() -> Outcome {
    if Modes::get().json {
        ui::json_document(serde_json::Value::Array(
            SKILLS
                .iter()
                .map(|(name, summary, _)| serde_json::json!({"name": name, "summary": summary}))
                .collect(),
        ));
        return Ok(());
    }
    let rows: Vec<Vec<String>> = SKILLS
        .iter()
        .map(|(name, summary, _)| vec![(*name).to_owned(), (*summary).to_owned()])
        .collect();
    ui::table(&["Skill", "Summary"], &rows);
    Ok(())
}

pub fn get(name: &str) -> Outcome {
    let (_, _, text) = SKILLS
        .iter()
        .find(|(skill, _, _)| *skill == name)
        .ok_or_else(|| {
            let names: Vec<String> = SKILLS
                .iter()
                .map(|(skill, _, _)| (*skill).to_owned())
                .collect();
            let mut headline = format!("Unknown skill {}.", ui::clean(name));
            if let Some(close) = super::help::suggestion(&names, name) {
                headline = format!("{headline} Did you mean {close}?");
            }
            Failure::new(Reason::InvalidValue, headline)
                .then("See the skills", ["bridge", "skills", "list"])
        })?;
    if Modes::get().json {
        ui::json_document(serde_json::json!({"name": name, "text": text}));
    } else {
        ui::out(text);
    }
    Ok(())
}
