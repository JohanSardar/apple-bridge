use super::Config;
use crate::{
    messages_policy::{MediaProof, tag_run},
    session::owned::text,
};
use serde_json::Value;
use std::{collections::BTreeSet, fs};

pub(super) struct Ledger {
    pub(super) run: String,
    pub(super) baseline: i64,
    pub(super) owned: BTreeSet<String>,
    pub(super) media: Option<MediaProof>,
    pub(super) interactive: String,
    pub(super) interactive_baseline: i64,
    pub(super) creations: std::path::PathBuf,
}

pub(super) fn strings(value: &Value) -> impl Iterator<Item = String> + '_ {
    value.as_array().into_iter().flatten().filter_map(|item| {
        item.as_str()
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn number(value: &Value) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|float| float as i64))
        .unwrap_or(0)
}

pub(super) fn load_ledger(config: &Config, tag: &str) -> Result<Ledger, String> {
    let run = tag_run(tag).to_owned();
    let path = config
        .reports
        .join("messages-command-cleanup")
        .join(format!("{run}.json"));
    let registration: Value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("invalid")?;
    if text(&registration, "run_id") != run {
        return Err("invalid".into());
    }
    let mut owned: BTreeSet<String> = strings(&registration["created_message_ids"]).collect();
    for name in ["native_key", "incoming_echo_native_key"] {
        if !text(&registration, name).is_empty() {
            owned.insert(text(&registration, name).to_owned());
        }
    }
    for part in registration["native_parts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if !text(part, "native_key").is_empty() {
            owned.insert(text(part, "native_key").to_owned());
        }
    }
    let media = if registration["media_attachment_names"]
        .as_array()
        .is_some_and(|names| !names.is_empty())
    {
        Some(MediaProof::new(&registration, tag).ok_or("invalid")?)
    } else {
        None
    };
    let interactive = if text(&registration, "scenario") == "MSG-INTERACTIVE-GATE"
        && text(&registration, "interactive_text")
            == format!("Apple Bridge interactive self-check {run}")
    {
        text(&registration, "interactive_text").to_owned()
    } else {
        String::new()
    };
    Ok(Ledger {
        baseline: number(&registration["baseline_max_row_id"]),
        interactive_baseline: number(&registration["baseline_max_row_id"]),
        creations: path.with_extension("json.creations.json"),
        run,
        owned,
        media,
        interactive,
    })
}
