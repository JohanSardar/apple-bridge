mod tree;
pub(super) use tree::Node;
use tree::Scan;

use super::messages_local::{Config, write_private};
use super::owned::{Probe, text};
use crate::messages_policy::{
    MediaBubble, can_replace_draft, matches_conversation, media_direction, media_target,
    media_value_matches, read_state_action, tag_at_start,
};
use ab_native::messages::Reader;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fmt::Write as _,
    thread,
    time::{Duration, Instant},
};

const ROLE: &str = "AXRole";
const IDENTIFIER: &str = "AXIdentifier";
const DESCRIPTION: &str = "AXDescription";
const VALUE: &str = "AXValue";
const TITLE: &str = "AXTitle";
const BODY_FIELD: &str = "messageBodyField";
const BALLOON: &str = "CKBalloonTextView";
const BUBBLE: &str = "Sticker";
const DISMISS_REPLIES: &str = "Dismiss replies";
const ENTER: i64 = 36;
const ESCAPE: i64 = 53;
const MEDIA_ATTRIBUTES: [&str; 8] = [
    DESCRIPTION,
    VALUE,
    "AXURL",
    TITLE,
    "AXFilename",
    "AXHelp",
    "AXDocument",
    "AXValueDescription",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    TargetUnavailable,
    TargetAmbiguous,
    MenuUnavailable,
    EditFieldUnavailable,
    ComposerUnavailable,
    ValueRejected,
    DraftPresent,
    Conflict,
    Unproven,
    Native(String),
}

impl Failure {
    fn code(&self) -> String {
        match self {
            Self::Conflict => "conflict".into(),
            Self::Unproven => "capability_unproven".into(),
            Self::Native(code) => code.clone(),
            _ => "unavailable".into(),
        }
    }

    pub(super) fn name(&self) -> String {
        match self {
            Self::TargetUnavailable => "targetUnavailable".into(),
            Self::TargetAmbiguous => "targetAmbiguous".into(),
            Self::MenuUnavailable => "menuUnavailable".into(),
            Self::EditFieldUnavailable => "editFieldUnavailable".into(),
            Self::ComposerUnavailable => "composerUnavailable".into(),
            Self::ValueRejected => "messageValueRejected".into(),
            Self::DraftPresent => "draftPresent".into(),
            Self::Conflict => "conflict".into(),
            Self::Unproven => "capabilityUnproven".into(),
            Self::Native(code) => code.clone(),
        }
    }
}

impl From<String> for Failure {
    fn from(code: String) -> Self {
        if code == "conflict" {
            Self::Conflict
        } else {
            Self::Native(code)
        }
    }
}

type Result<T> = std::result::Result<T, Failure>;

pub(super) struct Ui<'a> {
    probe: &'a dyn Probe,
    config: Config,
}

fn sleep(milliseconds: u64) {
    thread::sleep(Duration::from_millis(milliseconds));
}

fn deadline(milliseconds: u64) -> Instant {
    Instant::now() + Duration::from_millis(milliseconds)
}

fn resolve<T>(milliseconds: u64, mut attempt: impl FnMut() -> Result<T>) -> Result<T> {
    let until = deadline(milliseconds);
    loop {
        match attempt() {
            Err(Failure::TargetUnavailable) if Instant::now() < until => sleep(100),
            other => return other,
        }
    }
}

fn encoded(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

impl<'a> Ui<'a> {
    pub(super) fn new(probe: &'a dyn Probe, config: Config) -> Self {
        Self { probe, config }
    }

    fn call(&self, operation: &str, args: &Value) -> Result<Value> {
        self.probe.call(operation, args).map_err(Failure::from)
    }

    fn fence(&self) -> Result<()> {
        self.call("messages.ax_fence", &json!({})).map(|_| ())
    }

    fn press_key(&self, code: i64) -> Result<()> {
        self.call("messages.ax_key", &json!({"code": code}))
            .map(|_| ())
    }

    fn activate(&self) -> Result<()> {
        self.call("messages.ax_activate", &json!({})).map(|_| ())
    }

    fn perform_action(&self, handle: i64, action: &str) -> Result<()> {
        self.call(
            "messages.ax_action",
            &json!({"handle": handle, "action": action}),
        )
        .map(|_| ())
    }

    fn value_of(&self, handle: i64) -> Result<String> {
        let answer = self.call(
            "messages.ax_value",
            &json!({"handle": handle, "attribute": VALUE}),
        )?;
        Ok(text(&answer, "value").to_owned())
    }

    fn action_names(&self, handle: i64) -> Result<Vec<String>> {
        let answer = self.call("messages.ax_actions", &json!({"handle": handle}))?;
        Ok(answer["actions"]
            .as_array()
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| name.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    fn focused(&self) -> Result<Option<Node>> {
        let answer = self.call(
            "messages.ax_focused",
            &json!({"attributes": [IDENTIFIER, ROLE]}),
        )?;
        Ok(answer["node"]
            .is_object()
            .then(|| Node::parse(&answer["node"])))
    }

    fn focus_signature(&self) -> Result<String> {
        Ok(self.focused()?.map_or_else(String::new, |node| {
            format!("{}|{}", node.get(IDENTIFIER), node.get(ROLE))
        }))
    }

    fn opened_preview_values(&self, bubble: i64, names: &BTreeSet<String>) -> Result<Vec<String>> {
        let actions = self.action_names(bubble)?;
        let previews: Vec<&String> = actions
            .iter()
            .filter(|action| action.starts_with("Name:Quick Look\n"))
            .collect();
        let [preview] = previews.as_slice() else {
            return Ok(Vec::new());
        };
        self.call("messages.ax_preview_begin", &json!({}))?;
        self.fence()?;
        if self.perform_action(bubble, preview).is_err() {
            return Ok(Vec::new());
        }
        let until = deadline(1000);
        let mut found = Vec::new();
        loop {
            let answer = self.call("messages.ax_preview_values", &json!({"budget": 1000}));
            if let Ok(answer) = answer {
                found = answer["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|value| media_value_matches(value, names))
                    .map(ToOwned::to_owned)
                    .collect();
            }
            if !found.is_empty() || Instant::now() >= until {
                break;
            }
            sleep(50);
        }
        let _ = self.press_key(ESCAPE);
        sleep(100);
        Ok(found)
    }

    fn target(&self, prepared: &Value) -> Result<i64> {
        resolve(3000, || self.target_now(prepared))
    }

    fn target_now(&self, prepared: &Value) -> Result<i64> {
        let state = &prepared["native_state"];
        let guid = text(&prepared["action"], "native_key");
        if guid.is_empty() {
            return Err(Failure::Unproven);
        }
        let target = &state["target"];
        let expected = text(target, "text");
        let media: Vec<String> = if state["cleanup_authorized"] == true {
            target["cleanup_media_names"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| name.as_str().map(ToOwned::to_owned))
                .collect()
        } else {
            Vec::new()
        };
        if expected.is_empty() && media.is_empty() {
            return Err(Failure::Conflict);
        }
        let outgoing = target["is_from_me"] == true;
        if state["conversation_target"] == true {
            return self.conversation_row(expected);
        }
        if !media.is_empty() {
            return self.media_bubble(guid, target, &media, outgoing);
        }
        self.text_bubble(expected, outgoing, state["cleanup_authorized"] == true)
    }

    fn conversation_row(&self, expected: &str) -> Result<i64> {
        let (rows, left) = self.scan(&Scan {
            role: Some(&["AXRow"]),
            attributes: &[ROLE],
            budget: 4000,
            nested_attributes: &[VALUE, TITLE, DESCRIPTION],
            nested_budget: 100,
            ..Scan::default()
        })?;
        let you = format!("You: {expected}");
        let matching: Vec<&Node> = rows
            .iter()
            .filter(|row| {
                row.nested_left > 0
                    && row.nested.iter().any(|node| {
                        [VALUE, TITLE, DESCRIPTION].iter().any(|attribute| {
                            let shown = node.get(attribute);
                            shown == expected || shown == you
                        })
                    })
            })
            .collect();
        match matching.as_slice() {
            [row] if left > 0 => Ok(row.handle),
            _ => Err(Failure::TargetUnavailable),
        }
    }

    fn media_bubble(
        &self,
        guid: &str,
        target: &Value,
        media: &[String],
        outgoing: bool,
    ) -> Result<i64> {
        let identifiers = [
            BUBBLE.to_owned(),
            guid.to_owned(),
            format!("message-{guid}"),
        ];
        let identifiers: Vec<&str> = identifiers.iter().map(String::as_str).collect();
        let (bubbles, left) = self.scan(&Scan {
            identifier: Some(&identifiers),
            attributes: &[IDENTIFIER, DESCRIPTION],
            budget: 4000,
            nested_attributes: &MEDIA_ATTRIBUTES,
            nested_budget: 150,
            ..Scan::default()
        })?;
        let names: BTreeSet<String> = media.iter().cloned().collect();
        let mut candidates: Vec<MediaBubble> = bubbles
            .iter()
            .map(|bubble| {
                let mut values: Vec<String> = Vec::new();
                if bubble.nested_left > 0 {
                    for node in &bubble.nested {
                        for attribute in MEDIA_ATTRIBUTES {
                            values.push(node.get(attribute).to_owned());
                        }
                    }
                    if identifiers[1..].contains(&bubble.get(IDENTIFIER)) {
                        values.extend(media.iter().cloned());
                    }
                }
                MediaBubble {
                    outgoing: media_direction(bubble.get(DESCRIPTION)),
                    values,
                }
            })
            .collect();
        let copies = usize::try_from(target["cleanup_media_owned_copies"].as_i64().unwrap_or(1))
            .unwrap_or(1)
            .max(1);
        let echo = target["cleanup_self_echo"] == true;
        let equivalent = target["cleanup_equivalent_owned_media"] == true;
        let mut selected = media_target(&candidates, &names, outgoing, echo, equivalent, copies);
        if selected.is_none() && left > 0 {
            let until = deadline(6000);
            for (index, bubble) in bubbles.iter().enumerate() {
                if Instant::now() >= until {
                    return Err(Failure::TargetUnavailable);
                }
                let shown = self.opened_preview_values(bubble.handle, &names)?;
                if !shown.is_empty() {
                    candidates[index].values.extend(shown);
                    selected = media_target(&candidates, &names, outgoing, echo, false, copies);
                    if selected.is_some() {
                        break;
                    }
                }
            }
            selected = media_target(&candidates, &names, outgoing, echo, equivalent, copies);
        }
        match selected {
            Some(index) if left > 0 => Ok(bubbles[index].handle),
            _ => {
                self.media_diagnostic(&bubbles, &candidates, &names, left, outgoing);
                Err(Failure::TargetUnavailable)
            }
        }
    }

    fn media_diagnostic(
        &self,
        bubbles: &[Node],
        candidates: &[MediaBubble],
        names: &BTreeSet<String>,
        left: i64,
        outgoing: bool,
    ) {
        let rows: Vec<Value> = bubbles
            .iter()
            .zip(candidates)
            .map(|(bubble, candidate)| {
                let description = bubble.get(DESCRIPTION);
                let actions = self.action_names(bubble.handle).unwrap_or_default();
                let titles: Vec<&str> = actions
                    .iter()
                    .map(|name| name.lines().next().unwrap_or_default())
                    .filter(|title| title.starts_with("Name:Quick") || *title == "AXShowMenu")
                    .collect();
                json!({
                    "first_field": description.split(", ").next().unwrap_or_default(),
                    "fields": description.split(", ").count(),
                    "outgoing": candidate.outgoing,
                    "is_generic_bubble": bubble.get(IDENTIFIER) == BUBBLE,
                    "values": candidate.values.iter().filter(|value| !value.is_empty()).count(),
                    "values_naming_file": candidate.values.iter().filter(|value| media_value_matches(value, names)).count(),
                    "actions": titles,
                })
            })
            .collect();
        let body = json!({
            "stage": "media_target",
            "budget_left": left,
            "want_outgoing": outgoing,
            "bubbles": rows,
        });
        let _ = write_private(
            &self.config.reports.join("messages-target-diagnostic.json"),
            body.to_string().as_bytes(),
        );
    }

    fn text_bubble(&self, expected: &str, outgoing: bool, cleanup: bool) -> Result<i64> {
        let (bubbles, left) = self.scan(&Scan {
            identifier: Some(&[BUBBLE]),
            attributes: &[DESCRIPTION],
            budget: 4000,
            nested_identifier: Some(&[BALLOON]),
            nested_attributes: &[VALUE],
            nested_budget: 100,
            ..Scan::default()
        })?;
        let prefix = if outgoing {
            "Your iMessage,"
        } else {
            "iMessage,"
        };
        let targets: Vec<&Node> = bubbles
            .iter()
            .filter(|bubble| {
                let exact = bubble
                    .nested
                    .iter()
                    .filter(|node| node.get(VALUE) == expected)
                    .count()
                    == 1;
                exact && (cleanup || bubble.get(DESCRIPTION).starts_with(prefix))
            })
            .collect();
        if targets.is_empty() || left <= 0 {
            return Err(Failure::TargetUnavailable);
        }
        if targets.len() != 1 && !cleanup {
            return Err(Failure::TargetAmbiguous);
        }
        Ok(targets
            .iter()
            .find(|bubble| bubble.get(DESCRIPTION).starts_with(prefix))
            .unwrap_or(&targets[0])
            .handle)
    }

    pub(super) fn validate(&self, prepared: &Value) -> Result<()> {
        let until = deadline(4000);
        let bubble = loop {
            if let Ok(bubble) = self.target(prepared) {
                break bubble;
            }
            if Instant::now() >= until {
                return Err(Failure::TargetUnavailable);
            }
            sleep(100);
        };
        let action = &prepared["action"];
        if text(action, "op") == "messages.react"
            && text(&action["messages"], "reaction") == "emoji"
        {
            let wanted = format!("Name:{}\n", text(&action["messages"], "emoji"));
            let shown = self.action_names(bubble).map_err(|_| Failure::Unproven)?;
            if !shown.iter().any(|name| name.starts_with(&wanted)) {
                return Err(Failure::Unproven);
            }
        }
        Ok(())
    }

    fn validate_conversation(&self, prepared: &Value) -> Result<()> {
        let chat = text(&prepared["action"], "native_key");
        let message = text(&prepared["native_state"], "message_guid");
        let expected = text(&prepared["native_state"]["target"], "text");
        let memberships = Reader::open(&self.config.chat_db, &self.config.home)
            .and_then(|reader| reader.memberships(message))
            .map_err(|_| Failure::TargetUnavailable)?;
        let until = deadline(4000);
        loop {
            let (texts, left) = self.scan(&Scan {
                identifier: Some(&[BALLOON]),
                attributes: &[VALUE],
                budget: 4000,
                ..Scan::default()
            })?;
            let visible: Vec<String> = texts
                .iter()
                .map(|node| node.get(VALUE).to_owned())
                .collect();
            if left > 0 && matches_conversation(chat, &memberships, expected, &visible) {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(Failure::TargetUnavailable);
            }
            sleep(100);
        }
    }

    fn menu_items(&self, menu: i64) -> Result<Vec<Node>> {
        let (items, _) = self.scan(&Scan {
            within: Some(menu),
            role: Some(&["AXMenuItem"]),
            attributes: &[TITLE, DESCRIPTION],
            budget: 5000,
            menu_bar: true,
            ..Scan::default()
        })?;
        Ok(items)
    }

    fn perform_read_state(&self, prepared: &Value) -> Result<Value> {
        self.open_message(
            text(&prepared["native_state"], "message_guid"),
            500,
            false,
            "",
        )?;
        self.validate_conversation(prepared)?;
        let (menus, _) = self.scan(&Scan {
            role: Some(&["AXMenuBarItem"]),
            attributes: &[TITLE, DESCRIPTION],
            budget: 5000,
            menu_bar: true,
            ..Scan::default()
        })?;
        let menus: Vec<&Node> = menus
            .iter()
            .filter(|menu| {
                menu.get(TITLE) == "Conversation" || menu.get(DESCRIPTION) == "Conversation"
            })
            .collect();
        let [menu] = menus.as_slice() else {
            return Err(Failure::MenuUnavailable);
        };
        self.fence()?;
        self.perform_action(menu.handle, "AXPress")?;
        sleep(150);
        let unread = text(&prepared["action"], "op") == "messages.mark_unread";
        let chosen = resolve(3000, || {
            let titles: Vec<String> = self
                .menu_items(menu.handle)?
                .iter()
                .map(|item| {
                    let title = item.get(TITLE);
                    if title.is_empty() {
                        item.get(DESCRIPTION)
                    } else {
                        title
                    }
                    .to_owned()
                })
                .collect();
            read_state_action(unread, &titles).ok_or(Failure::TargetUnavailable)
        });
        let finish = chosen.and_then(|action| {
            if action.is_empty() {
                let _ = self.press_key(ESCAPE);
                return Ok(());
            }
            let items = self.menu_items(menu.handle)?;
            let items: Vec<&Node> = items
                .iter()
                .filter(|item| item.get(TITLE) == action || item.get(DESCRIPTION) == action)
                .collect();
            let [item] = items.as_slice() else {
                return Err(Failure::MenuUnavailable);
            };
            self.fence()?;
            self.perform_action(item.handle, "AXPress")?;
            sleep(150);
            Ok(())
        });
        if let Err(failure) = finish {
            let _ = self.press_key(ESCAPE);
            return Err(failure);
        }
        Ok(json!({"target_verified": true}))
    }

    fn press_title(&self, title: &str) -> Result<()> {
        let plain = title.replace('…', "");
        let until = deadline(2000);
        loop {
            let (controls, left) = self.scan(&Scan {
                role: Some(&["AXMenuItem", "AXButton"]),
                attributes: &[TITLE, DESCRIPTION],
                budget: 4000,
                ..Scan::default()
            })?;
            let matches: Vec<&Node> = controls
                .iter()
                .filter(|node| {
                    left > 0
                        && (node.get(TITLE) == title
                            || node.get(TITLE) == plain
                            || node.get(DESCRIPTION) == title)
                })
                .collect();
            if let [only] = matches.as_slice() {
                self.fence()?;
                self.perform_action(only.handle, "AXPress")
                    .map_err(|_| Failure::Conflict)?;
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(Failure::MenuUnavailable);
            }
            sleep(50);
        }
    }

    fn reply_shown(&self) -> Result<Option<i64>> {
        let (controls, _) = self.scan(&Scan {
            description: Some(&[DISMISS_REPLIES]),
            attributes: &[DESCRIPTION],
            budget: 4000,
            ..Scan::default()
        })?;
        Ok(match controls.as_slice() {
            [only] => Some(only.handle),
            _ => None,
        })
    }

    fn enter_reply_mode(&self) -> Result<()> {
        let until = deadline(2000);
        while self.reply_shown()?.is_none() {
            if Instant::now() >= until {
                let _ = self.press_key(ESCAPE);
                return Err(Failure::MenuUnavailable);
            }
            sleep(50);
        }
        Ok(())
    }

    fn dismiss_replies(&self) {
        if self.press_key(ESCAPE).is_err() {
            return;
        }
        let until = deadline(3000);
        while self.reply_shown().is_ok_and(|shown| shown.is_some()) && Instant::now() < until {
            sleep(100);
        }
    }

    fn type_text(&self, handle: i64, value: &str) -> Result<()> {
        self.call(
            "messages.ax_replace_text",
            &json!({"handle": handle, "text": value}),
        )?;
        let until = deadline(3000);
        loop {
            if self.value_of(handle)? == value {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(Failure::ValueRejected);
            }
            sleep(100);
        }
    }

    fn edit(&self, value: &str, previous: &str) -> Result<()> {
        let until = deadline(3000);
        loop {
            let (fields, _) = self.scan(&Scan {
                role: Some(&["AXTextField"]),
                attributes: &[IDENTIFIER, VALUE],
                budget: 2000,
                ..Scan::default()
            })?;
            let matching: Vec<&Node> = fields
                .iter()
                .filter(|field| field.get(IDENTIFIER) != BODY_FIELD && field.get(VALUE) == previous)
                .collect();
            if let [only] = matching.as_slice() {
                self.type_text(only.handle, value)?;
                return self.press_key(ENTER);
            }
            if Instant::now() >= until {
                return Err(Failure::EditFieldUnavailable);
            }
            sleep(50);
        }
    }

    fn composer(&self) -> Result<Option<i64>> {
        if let Some(node) = self.focused()?
            && node.get(IDENTIFIER) == BODY_FIELD
        {
            return Ok(Some(node.handle));
        }
        let (fields, _) = self.scan(&Scan {
            identifier: Some(&[BODY_FIELD]),
            attributes: &[IDENTIFIER],
            budget: 4000,
            ..Scan::default()
        })?;
        Ok(match fields.as_slice() {
            [only] => Some(only.handle),
            _ => None,
        })
    }

    fn replace(&self, value: &str) -> Result<()> {
        let until = deadline(2000);
        loop {
            if let Some(editor) = self.composer()? {
                if !can_replace_draft(&self.value_of(editor)?, value) {
                    return Err(Failure::DraftPresent);
                }
                return self.type_text(editor, value);
            }
            if Instant::now() >= until {
                return Err(Failure::ComposerUnavailable);
            }
            sleep(50);
        }
    }

    fn snapshot(&self, prepared: &Value) -> Result<Value> {
        let (nodes, _) = self.scan(&Scan {
            identifier: Some(&["TranscriptCollectionView"]),
            attributes: &[IDENTIFIER],
            budget: 4000,
            ..Scan::default()
        })?;
        let target = text(&prepared["native_state"]["target"], "text");
        Ok(json!({
            "transcript_present": nodes.len() == 1,
            "target_text": format!("{:x}", Sha256::digest(target.as_bytes())),
        }))
    }

    pub(super) fn acknowledge_deleted_info(&self) -> Result<bool> {
        let (sheets, _) = self.scan(&Scan {
            role: Some(&["AXSheet"]),
            attributes: &[ROLE],
            budget: 200,
            depth: Some(2),
            nested_attributes: &[ROLE, VALUE],
            nested_budget: 200,
            ..Scan::default()
        })?;
        let notices = sheets
            .iter()
            .flat_map(|sheet| &sheet.nested)
            .filter(|node| {
                node.get(ROLE) == "AXStaticText"
                    && node
                        .get(VALUE)
                        .starts_with("Deleted messages are moved to Recently")
            })
            .count();
        if notices != 1 {
            return Ok(false);
        }
        self.press_title("OK")?;
        sleep(200);
        Ok(true)
    }

    fn reset_tagged_editor(&self, tag: &str) -> Result<bool> {
        let (cancel, _) = self.scan(&Scan {
            description: Some(&["Cancel edit"]),
            attributes: &[DESCRIPTION],
            budget: 2000,
            ..Scan::default()
        })?;
        let [cancel] = cancel.as_slice() else {
            return Ok(false);
        };
        if tag_at_start(tag).is_none_or(|found| found.len() != tag.len()) {
            return Err(Failure::DraftPresent);
        }
        let (fields, _) = self.scan(&Scan {
            role: Some(&["AXTextField"]),
            attributes: &[IDENTIFIER, VALUE],
            budget: 2000,
            ..Scan::default()
        })?;
        if !fields
            .iter()
            .any(|field| field.get(IDENTIFIER) != BODY_FIELD && field.get(VALUE).starts_with(tag))
        {
            return Err(Failure::DraftPresent);
        }
        self.perform_action(cancel.handle, "AXPress")
            .map_err(|_| Failure::Conflict)?;
        self.press_key(ESCAPE)?;
        sleep(350);
        Ok(true)
    }

    pub(super) fn open_message(
        &self,
        guid: &str,
        settle: u64,
        reset_editor: bool,
        owned_tag: &str,
    ) -> Result<()> {
        self.activate()?;
        if reset_editor {
            self.reset_tagged_editor(owned_tag)?;
        } else {
            let (cancel, left) = self.scan(&Scan {
                description: Some(&["Cancel edit"]),
                attributes: &[DESCRIPTION],
                budget: 2000,
                ..Scan::default()
            })?;
            if !cancel.is_empty() || left <= 0 {
                return Err(Failure::Conflict);
            }
        }
        let url = format!("imessage://open?message-guid={}", encoded(guid));
        self.call("messages.ax_open_url", &json!({"url": url}))?;
        sleep(settle);
        Ok(())
    }

    pub(super) fn clear_tagged_draft(&self, tag: &str, group: &[String]) -> Result<bool> {
        self.activate()?;
        if !group.is_empty() {
            let addresses = group
                .iter()
                .map(|handle| encoded(handle))
                .collect::<Vec<_>>()
                .join(",");
            self.call(
                "messages.ax_open_url",
                &json!({"url": format!("imessage:open?addresses={addresses}")}),
            )?;
            sleep(3000);
        }
        let (fields, _) = self.scan(&Scan {
            identifier: Some(&[BODY_FIELD]),
            attributes: &[VALUE],
            budget: 4000,
            ..Scan::default()
        })?;
        let [field] = fields.as_slice() else {
            return Ok(false);
        };
        let draft = field.get(VALUE);
        if tag_at_start(draft).is_none() || !(tag.is_empty() || draft.contains(tag)) {
            return Ok(false);
        }
        self.call("messages.ax_clear_text", &json!({"handle": field.handle}))?;
        let until = deadline(3000);
        loop {
            if self.value_of(field.handle)?.is_empty() {
                return Ok(true);
            }
            if Instant::now() >= until {
                return Err(Failure::ValueRejected);
            }
            sleep(100);
        }
    }

    pub(super) fn perform(&self, prepared: &Value) -> Result<Value> {
        let action = &prepared["action"];
        let operation = text(action, "op");
        let args = &action["messages"];
        if matches!(operation, "messages.mark_read" | "messages.mark_unread") {
            return self.perform_read_state(prepared);
        }
        let bubble = self.target(prepared)?;
        let initial = self.focus_signature()?;
        let start = Instant::now();
        let before = self.snapshot(prepared)?;
        self.fence()?;
        if self.focus_signature()? != initial {
            return Err(Failure::Conflict);
        }
        let menu: String = match operation {
            "messages.reply" => "Reply…".into(),
            "messages.edit" => "Edit".into(),
            "messages.unsend" => "Undo Send".into(),
            "messages.delete_message" | "messages.delete_chat" => "Delete…".into(),
            "messages.react" => reaction_menu(args).ok_or(Failure::Unproven)?,
            _ => return Err(Failure::Unproven),
        };
        self.fence()?;
        if start.elapsed() >= Duration::from_secs(10) {
            return Err(Failure::Conflict);
        }
        let plain = menu.replace('…', "");
        let shown = self.action_names(bubble);
        let direct: Vec<String> = shown
            .as_ref()
            .map(|names| {
                names
                    .iter()
                    .filter(|name| {
                        name.lines()
                            .next()
                            .unwrap_or_default()
                            .replace("Name:", "")
                            .replace('…', "")
                            == plain
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if operation == "messages.reply" {
            self.perform_action(bubble, "AXShowMenu")
                .map_err(|_| Failure::MenuUnavailable)?;
            self.press_title(&menu)?;
            self.enter_reply_mode()?;
        } else if let [only] = direct.as_slice() {
            self.perform_action(bubble, only)
                .map_err(|_| Failure::Conflict)?;
        } else {
            self.call("messages.ax_context_menu", &json!({"handle": bubble}))?;
            self.press_title(&menu)?;
        }
        sleep(250);
        match operation {
            "messages.edit" => self.edit(
                text(args, "text"),
                text(&prepared["native_state"]["target"], "text"),
            )?,
            "messages.reply" => {
                let sent = self
                    .replace(text(args, "text"))
                    .and_then(|()| self.press_key(ENTER));
                sleep(500);
                self.dismiss_replies();
                sent?;
            }
            "messages.delete_message" | "messages.delete_chat" => {
                if let Err(failure) = self.press_title("Delete") {
                    let _ = self.press_title("Cancel");
                    return Err(failure);
                }
                sleep(250);
                self.acknowledge_deleted_info()?;
            }
            _ => {}
        }
        let after = self.snapshot(prepared)?;
        self.fence()?;
        Ok(
            json!({"before_accessibility": before, "after_accessibility": after, "target_verified": true}),
        )
    }
}

fn reaction_menu(args: &Value) -> Option<String> {
    let reaction = text(args, "reaction");
    if reaction == "emoji" {
        return Some(text(args, "emoji").to_owned());
    }
    Some(
        match reaction {
            "love" => "Heart",
            "like" => "Thumbs up",
            "dislike" => "Thumbs down",
            "laugh" => "Ha ha!",
            "emphasize" => "Exclamation mark",
            "question" => "Question mark",
            _ => return None,
        }
        .to_owned(),
    )
}

pub(super) fn answer(
    next: &dyn Probe,
    config: &Config,
    operation: &str,
    args: &Value,
) -> std::result::Result<Value, String> {
    let ui = Ui::new(next, config.clone());
    let prepared = &args["prepared"];
    let code = |failure: Failure| failure.code();
    match operation {
        "messages.ui_available" => {
            let state = next.call("messages.ax_state", &json!({}))?;
            Ok(json!({"available": state["trusted"] == true}))
        }
        "messages.ui_prepare" => {
            let guid = text(args, "guid");
            ui.open_message(guid, 3000, true, "").map_err(code)?;
            if ui.acknowledge_deleted_info().map_err(code)? {
                ui.open_message(guid, 3000, true, "").map_err(code)?;
            }
            ui.validate(prepared).map_err(code)?;
            Ok(json!({}))
        }
        "messages.ui_validate" => ui.validate(prepared).map(|()| json!({})).map_err(code),
        "messages.ui_activate_validate" => {
            ui.activate().map_err(code)?;
            ui.validate(prepared).map(|()| json!({})).map_err(code)
        }
        "messages.ui_perform" => Ok(json!({"ui": ui.perform(prepared).unwrap_or_else(|failure| {
            json!({"target_verified": false, "ui_error": failure.name()})
        })})),
        _ => Err("unsupported".into()),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Tree {
        nodes: Vec<Value>,
        calls: RefCell<Vec<(String, Value)>>,
        typed: RefCell<String>,
    }

    fn node(handle: i64, attrs: &Value, nested: &[Value]) -> Value {
        json!({
            "handle": handle,
            "attrs": attrs,
            "nested": {"nodes": nested, "budget_left": 90},
        })
    }

    fn bubble(handle: i64, description: &str, texts: &[&str]) -> Value {
        let nested: Vec<Value> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| {
                node(
                    handle * 100 + i64::try_from(index).unwrap(),
                    &json!({IDENTIFIER: BALLOON, VALUE: text}),
                    &[],
                )
            })
            .collect();
        node(
            handle,
            &json!({IDENTIFIER: BUBBLE, DESCRIPTION: description}),
            &nested,
        )
    }

    impl Probe for Tree {
        fn call(&self, operation: &str, args: &Value) -> std::result::Result<Value, String> {
            self.calls
                .borrow_mut()
                .push((operation.to_owned(), args.clone()));
            match operation {
                "messages.ax_find" => {
                    let escaped =
                        self.calls.borrow().iter().any(|(name, args)| {
                            name == "messages.ax_key" && args["code"] == ESCAPE
                        });
                    let nodes: Vec<&Value> = self
                        .nodes
                        .iter()
                        .filter(|node| !(escaped && node["attrs"][DESCRIPTION] == DISMISS_REPLIES))
                        .filter(|node| {
                            args["match"]
                                .as_object()
                                .into_iter()
                                .flatten()
                                .all(|(key, wanted)| {
                                    wanted.as_array().is_some_and(|list| {
                                        list.iter().any(|item| item == &node["attrs"][key])
                                    })
                                })
                        })
                        .collect();
                    Ok(json!({"nodes": nodes, "budget_left": 100}))
                }
                "messages.ax_actions" => {
                    Ok(json!({"actions": ["Name:Reply\nTarget:0", "Name:😂\nTarget:0"]}))
                }
                "messages.ax_focused" => Ok(json!({"node": null})),
                "messages.ax_replace_text" => {
                    text(args, "text").clone_into(&mut self.typed.borrow_mut());
                    Ok(json!({}))
                }
                "messages.ax_value" => Ok(json!({"value": self.typed.borrow().clone()})),
                _ => Ok(json!({})),
            }
        }
    }

    fn config() -> Config {
        Config {
            lab: true,
            chat_db: "/nonexistent/chat.db".into(),
            home: "/nonexistent".into(),
            reports: std::env::temp_dir(),
            os_build: "test".into(),
            agent_policy: None,
        }
    }

    fn prepared(op: &str, text_body: &str, outgoing: bool, cleanup: bool) -> Value {
        json!({
            "native_state": {"target": {"text": text_body, "is_from_me": outgoing}, "cleanup_authorized": cleanup},
            "action": {"op": op, "native_key": "guid-1", "messages": {"reaction": "emoji", "emoji": "😂"}},
        })
    }

    fn actions(tree: &Tree) -> Vec<Value> {
        tree.calls
            .borrow()
            .iter()
            .filter(|(name, _)| name == "messages.ax_action")
            .map(|(_, args)| args.clone())
            .collect()
    }

    #[test]
    fn the_exact_bubble_in_the_right_direction_is_chosen() {
        let tree = Tree {
            nodes: vec![
                bubble(1, "iMessage, from a friend", &["[ab-test r] hi"]),
                bubble(2, "Your iMessage, today", &["something else"]),
                bubble(3, "Your iMessage, today", &["[ab-test r] hi"]),
            ],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        let target = prepared("messages.edit", "[ab-test r] hi", true, false);
        assert_eq!(ui.target_now(&target), Ok(3));
    }

    #[test]
    fn two_equal_bubbles_are_ambiguous_and_never_retried_or_guessed() {
        let tree = Tree {
            nodes: vec![
                bubble(1, "Your iMessage, today", &["[ab-test r] hi"]),
                bubble(2, "Your iMessage, today", &["[ab-test r] hi"]),
            ],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        let target = prepared("messages.edit", "[ab-test r] hi", true, false);
        assert_eq!(ui.target_now(&target), Err(Failure::TargetAmbiguous));
        let finds_before = tree.calls.borrow().len();
        assert_eq!(ui.target(&target), Err(Failure::TargetAmbiguous));
        assert_eq!(tree.calls.borrow().len(), finds_before + 1, "not retried");
        assert_eq!(actions(&tree), Vec::<Value>::new());
    }

    #[test]
    fn a_message_of_the_other_direction_or_no_exact_text_is_not_a_target() {
        let tree = Tree {
            nodes: vec![
                bubble(1, "iMessage, from a friend", &["[ab-test r] hi"]),
                bubble(2, "Your iMessage, today", &["[ab-test r] hi there"]),
            ],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        let target = prepared("messages.edit", "[ab-test r] hi", true, false);
        assert_eq!(ui.target_now(&target), Err(Failure::TargetUnavailable));
    }

    #[test]
    fn an_empty_expected_text_is_a_conflict_and_no_key_is_unproven() {
        let tree = Tree::default();
        let ui = Ui::new(&tree, config());
        assert_eq!(
            ui.target_now(&prepared("messages.edit", "", true, false)),
            Err(Failure::Conflict)
        );
        let mut keyless = prepared("messages.edit", "x", true, false);
        keyless["action"]["native_key"] = json!("");
        assert_eq!(ui.target_now(&keyless), Err(Failure::Unproven));
    }

    #[test]
    fn a_conversation_row_must_be_the_only_one_showing_the_text() {
        let row = |handle, value: &str| {
            node(
                handle,
                &json!({ROLE: "AXRow"}),
                &[node(handle * 10, &json!({VALUE: value}), &[])],
            )
        };
        let tree = Tree {
            nodes: vec![row(1, "other chat"), row(2, "You: [ab-test r] hi")],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        assert_eq!(ui.conversation_row("[ab-test r] hi"), Ok(2));
        let tree = Tree {
            nodes: vec![row(1, "[ab-test r] hi"), row(2, "You: [ab-test r] hi")],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        assert_eq!(
            ui.conversation_row("[ab-test r] hi"),
            Err(Failure::TargetUnavailable)
        );
    }

    #[test]
    fn a_cancel_edit_button_is_only_pressed_for_this_runs_edit() {
        let cancel = node(1, &json!({DESCRIPTION: "Cancel edit"}), &[]);
        let field = |value: &str| {
            node(
                2,
                &json!({ROLE: "AXTextField", IDENTIFIER: "edit", VALUE: value}),
                &[],
            )
        };
        let tree = Tree {
            nodes: vec![cancel.clone(), field("[ab-test r] editing")],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        assert_eq!(ui.reset_tagged_editor("[ab-test r]"), Ok(true));
        assert_eq!(actions(&tree).len(), 1);

        let tree = Tree {
            nodes: vec![cancel.clone(), field("a person editing a real message")],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        assert_eq!(
            ui.reset_tagged_editor("[ab-test r]"),
            Err(Failure::DraftPresent)
        );
        assert_eq!(actions(&tree), Vec::<Value>::new());
        let tree = Tree {
            nodes: vec![cancel, field("[ab-test r] editing")],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        assert_eq!(
            ui.reset_tagged_editor(""),
            Err(Failure::DraftPresent),
            "no tag"
        );
        assert_eq!(actions(&tree), Vec::<Value>::new());
    }

    #[test]
    fn a_draft_is_cleared_only_when_it_is_tagged_with_this_run() {
        let composer = |value: &str| node(5, &json!({IDENTIFIER: BODY_FIELD, VALUE: value}), &[]);
        let cleared = |tree: &Tree| {
            tree.calls
                .borrow()
                .iter()
                .any(|(name, _)| name == "messages.ax_clear_text")
        };
        for (draft, tag) in [
            ("a real draft", "[ab-test r]"),
            ("[ab-test other] draft", "[ab-test r]"),
        ] {
            let tree = Tree {
                nodes: vec![composer(draft)],
                ..Tree::default()
            };
            let ui = Ui::new(&tree, config());
            assert_eq!(ui.clear_tagged_draft(tag, &[]), Ok(false), "{draft}");
            assert!(!cleared(&tree), "{draft}");
        }
    }

    #[test]
    fn an_emoji_reaction_needs_the_emoji_in_the_action_list() {
        let tree = Tree {
            nodes: vec![bubble(1, "Your iMessage, today", &["[ab-test r] hi"])],
            ..Tree::default()
        };
        let ui = Ui::new(&tree, config());
        let mut target = prepared("messages.react", "[ab-test r] hi", true, false);
        assert_eq!(ui.validate(&target), Ok(()));
        target["action"]["messages"]["emoji"] = json!("🦀");
        assert_eq!(ui.validate(&target), Err(Failure::Unproven));
    }

    fn reply_tree(in_reply_mode: bool) -> Tree {
        let mut nodes = vec![
            bubble(1, "Your iMessage, today", &["[ab-test r] hi"]),
            node(7, &json!({ROLE: "AXMenuItem", TITLE: "Reply…"}), &[]),
            node(5, &json!({IDENTIFIER: BODY_FIELD}), &[]),
        ];
        if in_reply_mode {
            nodes.push(node(
                8,
                &json!({ROLE: "AXButton", DESCRIPTION: "Dismiss replies"}),
                &[],
            ));
        }
        Tree {
            nodes,
            ..Tree::default()
        }
    }

    fn reply_request() -> Value {
        let mut prepared = prepared("messages.reply", "[ab-test r] hi", true, false);
        prepared["action"]["messages"] = json!({"text": "[ab-test r] reply"});
        prepared
    }

    fn keys(tree: &Tree) -> Vec<i64> {
        tree.calls
            .borrow()
            .iter()
            .filter(|(name, _)| name == "messages.ax_key")
            .map(|(_, args)| args["code"].as_i64().unwrap())
            .collect()
    }

    #[test]
    fn a_reply_goes_through_the_bubble_menu_and_reply_mode_is_closed_after_sending() {
        let tree = reply_tree(true);
        let ui = Ui::new(&tree, config());
        ui.perform(&reply_request()).unwrap();
        assert_eq!(
            actions(&tree),
            [
                json!({"handle": 1, "action": "AXShowMenu"}),
                json!({"handle": 7, "action": "AXPress"}),
            ],
            "the bubble's named Reply action reports success but sends an unthreaded message"
        );
        assert_eq!(*tree.typed.borrow(), "[ab-test r] reply");
        assert_eq!(keys(&tree), [ENTER, ESCAPE], "Escape closes reply mode");
    }

    #[test]
    fn a_reply_that_never_enters_reply_mode_sends_nothing() {
        let tree = reply_tree(false);
        let ui = Ui::new(&tree, config());
        assert_eq!(ui.perform(&reply_request()), Err(Failure::MenuUnavailable));
        assert_eq!(*tree.typed.borrow(), "", "nothing is typed");
        assert!(!keys(&tree).contains(&ENTER), "nothing is sent");
    }

    #[test]
    fn a_failed_perform_reports_it_without_pressing_anything_more() {
        let tree = Tree::default();
        let answer = answer(
            &tree,
            &config(),
            "messages.ui_perform",
            &json!({"prepared": prepared("messages.edit", "", true, false)}),
        )
        .unwrap();
        assert_eq!(answer["ui"]["target_verified"], false);
        assert_eq!(answer["ui"]["ui_error"], "conflict");
        assert_eq!(actions(&tree), Vec::<Value>::new());
    }

    #[test]
    fn identifiers_in_urls_are_percent_encoded() {
        assert_eq!(encoded("p:0/AB-C"), "p%3A0%2FAB-C");
    }
}
