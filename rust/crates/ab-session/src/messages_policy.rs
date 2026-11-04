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
