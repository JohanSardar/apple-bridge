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
}
