use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Gated,
    Interactive,
    OwnerSelf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    TextTagRequired,
    Invalid,
}

const TAG_OPEN: &str = "[ab-test ";

#[must_use]
pub fn tagged(text: &str) -> bool {
    text.match_indices(TAG_OPEN).any(|(at, _)| {
        let rest = &text[at + TAG_OPEN.len()..];
        let id = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(rest.len());
        id > 0 && rest[id..].starts_with(']')
    })
}

#[must_use]
pub fn normalize(raw: &str) -> String {
    let mut text = raw.trim();
    for prefix in ["iMessage:", "SMS:", "RCS:", "mailto:", "tel:", "P:", "E:"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest;
        }
    }
    if text.contains('@') {
        return text.to_lowercase();
    }
    text.chars()
        .filter(|c| c.is_numeric() || *c == '+')
        .collect()
}

#[must_use]
pub fn owns_recipients(own: Option<&BTreeSet<String>>, recipients: &[String]) -> bool {
    let Some(own) = own else {
        return false;
    };
    !recipients.is_empty()
        && recipients
            .iter()
            .all(|handle| own.contains(&normalize(handle)))
}

pub fn check(
    origin: Origin,
    text: &str,
    file_count: usize,
    own: Option<&BTreeSet<String>>,
    recipients: &[String],
) -> Result<bool, Refusal> {
    let outcome: Result<bool, Failure<()>> = check_with(
        origin,
        text,
        file_count,
        || Ok(own.cloned()),
        || Ok(recipients.to_vec()),
    );
    outcome.map_err(|failure| match failure {
        Failure::Policy(refusal) => refusal,
        Failure::Source(()) => Refusal::Invalid,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Failure<E> {
    Policy(Refusal),
    Source(E),
}

pub fn check_with<E>(
    origin: Origin,
    text: &str,
    file_count: usize,
    own: impl FnOnce() -> Result<Option<BTreeSet<String>>, E>,
    recipients: impl FnOnce() -> Result<Vec<String>, E>,
) -> Result<bool, Failure<E>> {
    let untagged = !text.is_empty() && !tagged(text);
    let owner_self = origin == Origin::OwnerSelf && untagged;
    if origin != Origin::Interactive && untagged {
        let owned = owner_self && {
            let own = own().map_err(Failure::Source)?;
            match own {
                None => false,
                Some(own) => {
                    let recipients = recipients().map_err(Failure::Source)?;
                    owns_recipients(Some(&own), &recipients)
                }
            }
        };
        if !owned {
            return Err(Failure::Policy(Refusal::TextTagRequired));
        }
    }
    if text.is_empty() && file_count == 0 {
        return Err(Failure::Policy(Refusal::Invalid));
    }
    Ok(owner_self)
}

pub const EDIT_SECONDS: f64 = 900.0;
pub const UNSEND_SECONDS: f64 = 120.0;
pub const EDIT_LIMIT: u32 = 5;

#[derive(Clone, Copy, Debug)]
pub struct ActionWindow {
    sent_at: f64,
    edit_until: Option<f64>,
    unsend_until: f64,
}

impl ActionWindow {
    #[must_use]
    pub fn new(
        service: &str,
        is_from_me: bool,
        is_message: bool,
        sent_at: Option<f64>,
        edits: u32,
    ) -> Option<Self> {
        if service != "iMessage" || !is_from_me || !is_message {
            return None;
        }
        let sent_at = sent_at?;
        Some(Self {
            sent_at,
            edit_until: (edits < EDIT_LIMIT).then_some(sent_at + EDIT_SECONDS),
            unsend_until: sent_at + UNSEND_SECONDS,
        })
    }

    #[must_use]
    pub fn allows(&self, edit: bool, now: f64) -> bool {
        let until = if edit {
            self.edit_until
        } else {
            Some(self.unsend_until)
        };
        until.is_some_and(|until| now >= self.sent_at && now < until)
    }
}

#[must_use]
pub fn read_state_matches(timestamp: f64, latest: f64, unread: bool) -> bool {
    if !timestamp.is_finite() || !latest.is_finite() || timestamp < 0.0 || latest <= 0.0 {
        return false;
    }
    let seconds = |value: f64| if value > 1e12 { value / 1e9 } else { value };
    if unread {
        seconds(timestamp) < seconds(latest)
    } else {
        seconds(timestamp) >= seconds(latest)
    }
}

fn tag_length(rest: &str) -> Option<usize> {
    let id = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))?;
    (id > 0 && rest[id..].starts_with(']')).then_some(TAG_OPEN.len() + id + 1)
}

#[must_use]
pub fn tag_at_start(text: &str) -> Option<&str> {
    let rest = text.strip_prefix(TAG_OPEN)?;
    tag_length(rest).map(|length| &text[..length])
}

#[must_use]
pub fn first_tag(text: &str) -> Option<&str> {
    text.match_indices(TAG_OPEN).find_map(|(at, _)| {
        let rest = &text[at + TAG_OPEN.len()..];
        tag_length(rest).map(|length| &text[at..at + length])
    })
}

#[must_use]
pub fn tag_run(tag: &str) -> &str {
    &tag[TAG_OPEN.len()..tag.len() - 1]
}

#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn authorizes(registration: &serde_json::Value, chat: &str, row_id: i64, text: &str) -> bool {
    let run = registration["run_id"].as_str().unwrap_or_default();
    let baseline = registration["baseline_max_row_id"]
        .as_i64()
        .or_else(|| {
            registration["baseline_max_row_id"]
                .as_f64()
                .filter(|value| value.fract() == 0.0)
                .map(|value| value as i64)
        })
        .unwrap_or(0);
    let created = registration["created_chat_key"]
        .as_str()
        .unwrap_or_default();
    !run.is_empty()
        && run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && matches!(
            registration["scenario"].as_str(),
            Some("MSG-SELFTEST" | "MSG-READSTATE-REAL")
        )
        && baseline > 0
        && row_id > baseline
        && text.starts_with(&format!("{TAG_OPEN}{run}] "))
        && !chat.is_empty()
        && (created.is_empty() || created == chat)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    email: bool,
    value: String,
}

const IDENTITY_PREFIXES: [&str; 10] = [
    "imessage:",
    "sms:",
    "rcs:",
    "mailto:",
    "tel:",
    "p:",
    "e:",
    "imessage;-;",
    "sms;-;",
    "rcs;-;",
];

fn percent_decoded(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let pair = raw.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[must_use]
pub fn identity(raw: &str) -> Option<Identity> {
    let decoded = percent_decoded(raw).unwrap_or_else(|| raw.to_owned());
    let lowered = decoded.trim().to_lowercase();
    let mut value = lowered.as_str();
    while let Some(rest) = IDENTITY_PREFIXES
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
    {
        value = rest.trim();
    }
    if value.contains('@') {
        let mut parts = value.split('@');
        let (local, domain) = (parts.next()?, parts.next()?);
        if parts.next().is_some()
            || local.is_empty()
            || domain.is_empty()
            || value.chars().any(char::is_whitespace)
        {
            return None;
        }
        return Some(Identity {
            email: true,
            value: value.to_owned(),
        });
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_digit() || "+().-".contains(c) || c.is_whitespace())
    {
        return None;
    }
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.strip_prefix("00").unwrap_or(&digits);
    let digits = digits.trim_start_matches('0');
    (digits.len() >= 7).then(|| Identity {
        email: false,
        value: digits.to_owned(),
    })
}

#[must_use]
pub fn unused_handles(own: &[String], participants: &[String]) -> Vec<String> {
    let known: Vec<Identity> = participants.iter().filter_map(|p| identity(p)).collect();
    own.iter()
        .filter(|alias| {
            identity(alias).is_some_and(|candidate| {
                !known.iter().any(|peer| {
                    peer.email == candidate.email
                        && if candidate.email {
                            peer.value == candidate.value
                        } else {
                            peer.value.ends_with(&candidate.value)
                                || candidate.value.ends_with(&peer.value)
                        }
                })
            })
        })
        .cloned()
        .collect()
}

#[must_use]
pub fn is_self_chat(participants: &[String], own: &[String]) -> bool {
    let [only] = participants else {
        return false;
    };
    let Some(peer) = identity(only) else {
        return false;
    };
    own.iter()
        .filter_map(|handle| identity(handle))
        .any(|mine| mine == peer)
}

#[must_use]
pub fn account_handles(raw: &[String]) -> Vec<String> {
    let mut handles: BTreeSet<String> = raw
        .iter()
        .filter(|handle| identity(handle).is_some())
        .map(|handle| normalize(handle))
        .collect();
    handles.remove("");
    handles.into_iter().collect()
}

#[derive(Debug)]
pub struct IsolationChat<'a> {
    pub guid: &'a str,
    pub identifier: &'a str,
    pub peers: Vec<String>,
}

#[must_use]
pub fn isolated_self_chat(
    own: &[String],
    participants: &[String],
    pins: &[String],
    chats: &[IsolationChat<'_>],
) -> Option<(String, String)> {
    if let Some(handle) = unused_handles(own, participants).into_iter().next() {
        return Some((handle, String::new()));
    }
    chats.iter().find_map(|chat| {
        let pinned = pins.iter().any(|pin| pin == chat.identifier);
        if pinned && pins.last().map(String::as_str) != Some(chat.identifier) {
            return None;
        }
        if !is_self_chat(&chat.peers, own) {
            return None;
        }
        let peer = chat.peers.first()?;
        own.iter()
            .find(|handle| is_self_chat(std::slice::from_ref(peer), std::slice::from_ref(handle)))
            .map(|handle| (handle.clone(), chat.guid.to_owned()))
    })
}

#[must_use]
pub fn qualifies_fixture_chat(chat: &str, row: i64, baseline: i64, existing: &str) -> bool {
    !chat.is_empty()
        && if existing.is_empty() {
            row > baseline
        } else {
            chat == existing
        }
}

#[must_use]
pub fn matches_conversation(
    chat: &str,
    memberships: &[String],
    expected: &str,
    visible: &[String],
) -> bool {
    !chat.is_empty()
        && memberships.len() == 1
        && memberships[0] == chat
        && !expected.is_empty()
        && visible.iter().any(|text| text == expected)
}

#[must_use]
pub fn ui_allowed(include_ui: bool, trusted: bool, screen_locked: bool) -> bool {
    include_ui && trusted && !screen_locked
}

fn exact_tag(tag: &str) -> bool {
    tag_at_start(tag).is_some_and(|found| found.len() == tag.len())
}

#[must_use]
pub fn owns_message(row: i64, text: &str, tag: &str, baseline: i64) -> bool {
    exact_tag(tag) && row > baseline && text.starts_with(&format!("{tag} "))
}

#[must_use]
pub fn owns_deleted_selection(deleted: &[String], created: &[String]) -> bool {
    let unique = |items: &[String]| items.iter().collect::<BTreeSet<_>>().len() == items.len();
    !created.is_empty()
        && unique(deleted)
        && unique(created)
        && deleted.iter().collect::<BTreeSet<_>>() == created.iter().collect::<BTreeSet<_>>()
}

#[must_use]
pub fn allows_late_echo(row: i64, baseline: i64, text: &str, tag: &str) -> bool {
    row > baseline && exact_tag(tag) && text.starts_with(tag)
}

#[must_use]
pub fn can_delete_retracted(
    row: i64,
    baseline: i64,
    guid: &str,
    owned: &BTreeSet<String>,
    has_parts: bool,
    text: &str,
) -> bool {
    row > baseline && !guid.is_empty() && owned.contains(guid) && has_parts && text.is_empty()
}

#[must_use]
pub fn can_delete_reaction(
    row: i64,
    baseline: i64,
    guid: &str,
    parent: &str,
    owned: &BTreeSet<String>,
    is_tagged: bool,
) -> bool {
    row > baseline
        && !guid.is_empty()
        && (owned.contains(guid) || (is_tagged && !parent.is_empty() && owned.contains(parent)))
}

#[must_use]
pub fn can_replace_draft(previous: &str, text: &str) -> bool {
    if previous.is_empty() {
        return true;
    }
    tag_at_start(text).is_some_and(|tag| previous.starts_with(tag))
}

#[must_use]
pub fn read_state_action(unread: bool, titles: &[String]) -> Option<&'static str> {
    let (desired, opposite) = if unread {
        ("Mark as Unread", "Mark as Read")
    } else {
        ("Mark as Read", "Mark as Unread")
    };
    let shown = titles
        .iter()
        .filter(|title| *title == desired || *title == opposite)
        .count();
    if shown != 1 {
        return None;
    }
    Some(if titles.iter().any(|title| title == desired) {
        desired
    } else {
        ""
    })
}

#[must_use]
pub fn pin_index(value: &serde_json::Value) -> Option<i64> {
    let number = value.as_f64()?;
    #[allow(clippy::cast_possible_truncation)]
    (number.fract() == 0.0 && (-1.0..=8.0).contains(&number)).then_some(number as i64)
}

const MEDIA_SCENARIOS: [&str; 4] = [
    "WEB-MEDIA-VARIANTS",
    "MSG-SELFTEST",
    "WEB-UPLOAD-NATIVE-STAGING",
    "MSG-AUDIO-MESSAGE",
];

fn last_path_component(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

#[derive(Clone, Debug)]
pub struct MediaProof {
    pub baseline: i64,
    pub names: BTreeSet<String>,
    display: std::collections::BTreeMap<String, BTreeSet<String>>,
}

impl MediaProof {
    #[must_use]
    pub fn new(registration: &serde_json::Value, tag: &str) -> Option<Self> {
        if !exact_tag(tag) {
            return None;
        }
        let run = tag_run(tag);
        let baseline = whole(&registration["baseline_max_row_id"]).filter(|row| *row >= 0)?;
        if !MEDIA_SCENARIOS.contains(&registration["scenario"].as_str()?)
            || registration["run_id"].as_str()? != run
        {
            return None;
        }
        let files = registration["media_attachment_names"].as_array()?;
        if files.is_empty() {
            return None;
        }
        let mut names = BTreeSet::new();
        let mut display = std::collections::BTreeMap::new();
        for (index, file) in files.iter().enumerate() {
            let name = file.as_str()?;
            if !name.starts_with(&format!("ab-{run}."))
                || name != last_path_component(name)
                || name.contains('%')
                || name.contains('\n')
            {
                return None;
            }
            let staged = format!("{index}-{name}");
            names.insert(name.to_owned());
            names.insert(staged.clone());
            let both: BTreeSet<String> = [name.to_owned(), staged.clone()].into();
            display.insert(name.to_owned(), both.clone());
            display.insert(staged, both);
        }
        Some(Self {
            baseline,
            names,
            display,
        })
    }

    #[must_use]
    pub fn authorizes(
        &self,
        row: i64,
        attachments: &[String],
        peers: &[String],
        own: &BTreeSet<String>,
    ) -> bool {
        row > self.baseline
            && !peers.is_empty()
            && peers.iter().all(|peer| own.contains(&normalize(peer)))
            && !attachments.is_empty()
            && attachments
                .iter()
                .all(|name| self.names.contains(last_path_component(name)))
    }

    #[must_use]
    pub fn target_names(&self, attachments: &[String]) -> Vec<String> {
        let mut names = BTreeSet::new();
        for attachment in attachments {
            if let Some(found) = self.display.get(last_path_component(attachment)) {
                names.extend(found.iter().cloned());
            }
        }
        names.into_iter().collect()
    }
}

fn whole(value: &serde_json::Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        let number = value.as_f64()?;
        #[allow(clippy::cast_possible_truncation)]
        (number.fract() == 0.0).then_some(number as i64)
    })
}

#[must_use]
pub fn media_value_matches(value: &str, names: &BTreeSet<String>) -> bool {
    if value.is_empty() {
        return false;
    }
    let decoded = percent_decoded(value).unwrap_or_else(|| value.to_owned());
    if names.contains(&decoded) {
        return true;
    }
    let fields: Vec<&str> = decoded.split(", ").collect();
    if fields.len() >= 3 && fields[1..].iter().any(|field| names.contains(*field)) {
        return true;
    }
    if value.starts_with("file:") {
        return names.contains(last_path_component(&decoded));
    }
    decoded.starts_with('/') && names.contains(last_path_component(&decoded))
}

#[must_use]
pub fn media_direction(description: &str) -> Option<bool> {
    let fields: Vec<&str> = description.split(", ").collect();
    if fields.len() < 2 || fields[0].is_empty() {
        return None;
    }
    if ["Your iMessage", "Your SMS", "Your RCS"].contains(&fields[0]) {
        return Some(true);
    }
    (!fields[0].starts_with("Your ")).then_some(false)
}

#[derive(Clone, Debug, Default)]
pub struct MediaBubble {
    pub outgoing: Option<bool>,
    pub values: Vec<String>,
}

#[must_use]
pub fn media_target(
    bubbles: &[MediaBubble],
    names: &BTreeSet<String>,
    outgoing: bool,
    allow_self_echo: bool,
    equivalent_owned: bool,
    owned_copies: usize,
) -> Option<usize> {
    let naming = |bubble: &MediaBubble| {
        bubble
            .values
            .iter()
            .any(|value| media_value_matches(value, names))
    };
    let targets: Vec<usize> = bubbles
        .iter()
        .enumerate()
        .filter(|(_, bubble)| bubble.outgoing == Some(outgoing) && naming(bubble))
        .map(|(index, _)| index)
        .collect();
    if !targets.is_empty() && (targets.len() <= owned_copies || equivalent_owned) {
        return targets.first().copied();
    }
    if !targets.is_empty() || !allow_self_echo {
        return None;
    }
    let echoes: Vec<usize> = bubbles
        .iter()
        .enumerate()
        .filter(|(_, bubble)| naming(bubble))
        .map(|(index, _)| index)
        .collect();
    if !echoes.is_empty() && (echoes.len() <= owned_copies || equivalent_owned) {
        echoes.first().copied()
    } else {
        None
    }
}

#[must_use]
pub fn media_cleanup_check(fixture: &serde_json::Value) -> Option<serde_json::Value> {
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let proof = MediaProof::new(&fixture["registration"], fixture["tag"].as_str()?)?;
    let row = &fixture["row"];
    let attachments = strings(&row["attachments"]);
    let own: BTreeSet<String> = strings(&fixture["own"])
        .iter()
        .map(|h| normalize(h))
        .collect();
    let authorized = proof.authorizes(
        whole(&row["row_id"])?,
        &attachments,
        &strings(&row["peers"]),
        &own,
    );
    let names: BTreeSet<String> = proof.target_names(&attachments).into_iter().collect();
    let bubbles: Vec<MediaBubble> = fixture["candidates"]
        .as_array()?
        .iter()
        .map(|candidate| {
            let description = candidate["description"].as_str().unwrap_or_default();
            let mut values = strings(&candidate["values"]);
            let outgoing = if description.is_empty() {
                candidate["outgoing"].as_bool()
            } else {
                values.push(description.to_owned());
                media_direction(description)
            };
            MediaBubble { outgoing, values }
        })
        .collect();
    let copies = usize::try_from(whole(&fixture["owned_copies"]).unwrap_or(1))
        .unwrap_or(1)
        .max(1);
    let target = media_target(
        &bubbles,
        &names,
        row["is_from_me"] == true,
        fixture["allow_self_echo"] == true,
        fixture["equivalent_owned"] == true,
        copies,
    );
    Some(serde_json::json!({
        "authorized": authorized,
        "target_verified": authorized && target.is_some(),
    }))
}

#[cfg(test)]
mod conversation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_read_state_matcher_follows_the_swift_checks() {
        assert!(read_state_matches(20.0, 20.0, false));
        assert!(read_state_matches(19.0, 20.0, true));
        assert!(!read_state_matches(19.0, 20.0, false));
        assert!(read_state_matches(2_000_000_000_000.0, 2000.0, false));
        assert!(!read_state_matches(f64::NAN, 20.0, true));
        assert!(!read_state_matches(f64::INFINITY, 20.0, false));
        assert!(!read_state_matches(-1.0, 20.0, true));
        assert!(!read_state_matches(5.0, 0.0, true));
        assert!(!read_state_matches(20.0, 20.0, true));
        assert!(read_state_matches(1.0e9 * 1.9e3, 1.9e3, false));
    }

    #[test]
    fn tags_are_found_at_the_start_or_anywhere_as_asked() {
        assert_eq!(tag_at_start("[ab-test run-1] hi"), Some("[ab-test run-1]"));
        assert_eq!(tag_at_start("x [ab-test run-1]"), None);
        assert_eq!(
            first_tag("x [ab-test run-1] y [ab-test z]"),
            Some("[ab-test run-1]")
        );
        assert_eq!(first_tag("[ab-test ] [ab-test ok]"), Some("[ab-test ok]"));
        assert_eq!(tag_at_start("[ab-test ]"), None);
        assert_eq!(tag_run("[ab-test run-1]"), "run-1");
    }

    #[test]
    fn a_ledger_authorises_only_its_own_run_scenario_row_and_chat() {
        let registration = json!({"run_id": "r1", "scenario": "MSG-SELFTEST", "baseline_max_row_id": 10, "created_chat_key": "chat-1"});
        let text = "[ab-test r1] hello";
        assert!(authorizes(&registration, "chat-1", 11, text));
        assert!(!authorizes(&registration, "chat-2", 11, text));
        assert!(!authorizes(&registration, "chat-1", 10, text));
        assert!(!authorizes(
            &registration,
            "chat-1",
            11,
            "[ab-test r2] hello"
        ));
        assert!(!authorizes(
            &registration,
            "chat-1",
            11,
            "[ab-test r1]hello"
        ));
        assert!(!authorizes(&registration, "", 11, text));
        let mut other = registration.clone();
        other["scenario"] = json!("MSG-SEND-SELF-TEXT");
        assert!(!authorizes(&other, "chat-1", 11, text));
        other = registration.clone();
        other["baseline_max_row_id"] = json!(0);
        assert!(!authorizes(&other, "chat-1", 1, text));
        other = registration.clone();
        other["run_id"] = json!("r 1");
        assert!(!authorizes(&other, "chat-1", 11, "[ab-test r 1] hello"));
        other = registration.clone();
        other["created_chat_key"] = json!("");
        assert!(authorizes(&other, "chat-9", 11, text));
        other["baseline_max_row_id"] = json!(10.5);
        assert!(!authorizes(&other, "chat-9", 11, text));
    }

    fn own(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    #[test]
    fn every_spelling_of_an_own_address_is_already_in_use() {
        for (name, handle, participants, expected) in [
            (
                "national phone",
                "2025550123",
                vec!["+1 (202) 555-0123"],
                false,
            ),
            ("country phone", "+12025550123", vec!["202-555-0123"], false),
            (
                "international prefix",
                "00442079460000",
                vec!["+44 20 7946 0000"],
                false,
            ),
            (
                "national trunk",
                "02079460000",
                vec!["+442079460000"],
                false,
            ),
            (
                "service prefixes",
                "+12025550123",
                vec!["SMS:2025550123", "RCS;-;2025550123"],
                false,
            ),
            (
                "email prefix and case",
                "Self@Example.com",
                vec!["iMessage:E:mailto:self%40example.com"],
                false,
            ),
            (
                "distinct email",
                "unused@example.com",
                vec!["self@example.com"],
                true,
            ),
            (
                "distinct phone",
                "+12025550123",
                vec!["+442079460000"],
                true,
            ),
            ("empty inventory", "self@example.com", vec![], true),
        ] {
            let list = unused_handles(&own(&[handle]), &own(&participants));
            assert_eq!(!list.is_empty(), expected, "{name}");
        }
        assert_eq!(
            unused_handles(&own(&["", "+12", "unknown", "a@@b"]), &[]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_self_chat_has_one_participant_who_is_the_account() {
        let mine = own(&["me@example.com", "+12025550123"]);
        assert!(is_self_chat(&own(&["ME@example.com"]), &mine));
        assert!(is_self_chat(&own(&["tel:+1 202 555 0123"]), &mine));
        assert!(
            !is_self_chat(&own(&["other@example.com"]), &mine),
            "someone else"
        );
        assert!(
            !is_self_chat(&own(&["me@example.com", "+12025550123"]), &mine),
            "two members"
        );
        assert!(!is_self_chat(&[], &mine), "no member");
        assert!(
            !is_self_chat(&own(&["me@example.com"]), &[]),
            "no own address"
        );
        assert!(!is_self_chat(&own(&["not an address"]), &mine));
        assert!(
            !is_self_chat(&own(&["2025550123"]), &mine),
            "a shorter number is not exact"
        );
    }

    #[test]
    fn the_self_test_never_picks_someone_elses_chat_or_a_pinned_one() {
        let mine = own(&["me@example.com"]);
        let used = own(&["me@example.com", "friend@example.com"]);
        let chat = |guid, identifier, peer: &str| IsolationChat {
            guid,
            identifier,
            peers: vec![peer.to_owned()],
        };
        assert_eq!(
            isolated_self_chat(&own(&["fresh@example.com"]), &used, &[], &[]),
            Some(("fresh@example.com".to_owned(), String::new())),
            "an address nobody uses comes first"
        );
        let chats = [
            chat("friend", "friend@example.com", "friend@example.com"),
            chat("self", "me@example.com", "me@example.com"),
        ];
        assert_eq!(
            isolated_self_chat(&mine, &used, &[], &chats),
            Some(("me@example.com".to_owned(), "self".to_owned()))
        );
        assert_eq!(
            isolated_self_chat(&mine, &used, &own(&["me@example.com", "x"]), &chats),
            None,
            "a pinned self chat that is not the last pin is left alone"
        );
        assert!(isolated_self_chat(&mine, &used, &own(&["x", "me@example.com"]), &chats).is_some());
        assert_eq!(isolated_self_chat(&mine, &used, &[], &chats[..1]), None);
    }

    #[test]
    fn the_fixture_chat_rule_refuses_older_and_other_chats() {
        assert!(qualifies_fixture_chat("new", 11, 10, ""));
        assert!(!qualifies_fixture_chat("old", 10, 10, ""));
        assert!(qualifies_fixture_chat("self", 3, 10, "self"));
        assert!(!qualifies_fixture_chat("other", 11, 10, "self"));
        assert!(!qualifies_fixture_chat("", 11, 10, ""));
    }

    #[test]
    fn the_visible_conversation_must_be_the_one_chat_with_the_text() {
        let self_chat = own(&["self"]);
        let text = "[ab-test current] fixture";
        assert!(matches_conversation(
            "self",
            &self_chat,
            text,
            &own(&[text, "x"])
        ));
        assert!(!matches_conversation(
            "other",
            &self_chat,
            text,
            &own(&[text])
        ));
        assert!(!matches_conversation(
            "self",
            &self_chat,
            text,
            &own(&["different"])
        ));
        assert!(!matches_conversation(
            "self",
            &own(&["self", "b"]),
            text,
            &own(&[text])
        ));
        assert!(!matches_conversation("self", &self_chat, "", &own(&[""])));
    }

    #[test]
    fn cleanup_rules_protect_everything_the_run_did_not_create() {
        let owned: BTreeSet<String> = [
            "current".to_owned(),
            "parent".to_owned(),
            "reaction".to_owned(),
        ]
        .into();
        assert!(can_delete_reaction(11, 10, "reaction", "", &owned, false));
        assert!(can_delete_reaction(11, 10, "late", "parent", &owned, true));
        assert!(
            !can_delete_reaction(10, 10, "reaction", "parent", &owned, true),
            "old"
        );
        assert!(
            !can_delete_reaction(11, 10, "late", "parent", &owned, false),
            "untagged foreign"
        );
        assert!(
            !can_delete_reaction(11, 10, "late", "other", &owned, true),
            "foreign parent"
        );
        assert!(can_delete_retracted(11, 10, "current", &owned, true, ""));
        assert!(
            !can_delete_retracted(10, 10, "current", &owned, true, ""),
            "existing"
        );
        assert!(
            !can_delete_retracted(11, 10, "other", &owned, true, ""),
            "foreign"
        );
        assert!(
            !can_delete_retracted(11, 10, "current", &owned, false, ""),
            "blank ordinary"
        );
        assert!(!can_delete_retracted(
            11,
            10,
            "current",
            &owned,
            true,
            "text left"
        ));
        assert!(allows_late_echo(11, 10, "[ab-test r] echo", "[ab-test r]"));
        assert!(!allows_late_echo(10, 10, "[ab-test r] echo", "[ab-test r]"));
        assert!(!allows_late_echo(
            11,
            10,
            "[ab-test other] echo",
            "[ab-test r]"
        ));
        assert!(
            !allows_late_echo(11, 10, "echo", "[ab-test r] extra"),
            "a bad tag allows nothing"
        );
        assert!(owns_message(11, "[ab-test r] x", "[ab-test r]", 10));
        assert!(!owns_message(10, "[ab-test r] x", "[ab-test r]", 10));
        assert!(!owns_message(11, "[ab-test r]x", "[ab-test r]", 10));
        assert!(!owns_message(11, "[ab-test r] x", "r", 10));
        assert!(owns_deleted_selection(&own(&["a", "b"]), &own(&["b", "a"])));
        assert!(
            !owns_deleted_selection(&own(&["a"]), &own(&["a", "b"])),
            "partial"
        );
        assert!(
            !owns_deleted_selection(&own(&["a", "c"]), &own(&["a"])),
            "extra"
        );
        assert!(
            !owns_deleted_selection(&own(&["a", "a"]), &own(&["a"])),
            "duplicate"
        );
        assert!(!owns_deleted_selection(&[], &[]));
    }

    #[test]
    fn ui_rules_refuse_what_is_not_clearly_right() {
        assert!(ui_allowed(true, true, false));
        assert!(!ui_allowed(false, true, false));
        assert!(!ui_allowed(true, false, false));
        assert!(!ui_allowed(true, true, true));
        assert!(can_replace_draft("", "anything"));
        assert!(can_replace_draft("[ab-test r] old", "[ab-test r] new"));
        assert!(
            !can_replace_draft("a real draft", "[ab-test r] new"),
            "a person's draft"
        );
        assert!(!can_replace_draft("[ab-test other] old", "[ab-test r] new"));
        assert!(!can_replace_draft("a real draft", "no tag"));
        assert_eq!(
            read_state_action(false, &own(&["Mark as Read"])),
            Some("Mark as Read")
        );
        assert_eq!(
            read_state_action(true, &own(&["Mark as Unread"])),
            Some("Mark as Unread")
        );
        assert_eq!(
            read_state_action(false, &own(&["Mark as Unread"])),
            Some("")
        );
        assert_eq!(read_state_action(true, &own(&["Mark as Read"])), Some(""));
        assert_eq!(read_state_action(false, &[]), None);
        assert_eq!(
            read_state_action(false, &own(&["Mark as Read", "Mark as Unread"])),
            None
        );
        assert_eq!(pin_index(&json!(-1)), Some(-1));
        assert_eq!(pin_index(&json!(8)), Some(8));
        assert_eq!(pin_index(&json!(9)), None);
        assert_eq!(pin_index(&json!(1.5)), None);
        assert_eq!(pin_index(&json!("1")), None);
    }
}
