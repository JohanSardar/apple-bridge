use crate::mcp_registry::Registry;
use ab_catalog::AgentOperation;
use ab_operations::Principal;
use std::collections::BTreeSet;

pub(crate) struct Route {
    pub collection: &'static str,
    pub summary: &'static str,
    pub scope: &'static str,
}

pub(crate) const ROUTES: &[Route] = &[
    Route {
        collection: "voicemail.message",
        summary: "Voicemail messages: caller, date, duration, read state and transcript.",
        scope: "voicemail:read",
    },
    Route {
        collection: "phone.call",
        summary: "Phone call history: who called, when, how long, and missed calls.",
        scope: "phone:read",
    },
    Route {
        collection: "safari.history",
        summary: "Safari browsing history: pages visited and when.",
        scope: "safari:read",
    },
    Route {
        collection: "safari.bookmark",
        summary: "Safari bookmarks.",
        scope: "safari:read",
    },
    Route {
        collection: "safari.reading_list",
        summary: "Safari reading list.",
        scope: "safari:read",
    },
    Route {
        collection: "safari.tab",
        summary: "Open Safari tabs on this Mac.",
        scope: "safari:read",
    },
    Route {
        collection: "safari.cloud_tab",
        summary: "Safari tabs open on the owner's other devices.",
        scope: "safari:read",
    },
    Route {
        collection: "voice_memos.recording",
        summary: "Voice Memos recordings with title, date and duration.",
        scope: "voice_memos:read",
    },
    Route {
        collection: "usage.app_day",
        summary: "Raw Screen Time rows: one app on one device for one day.",
        scope: "usage:read",
    },
    Route {
        collection: "mail.message",
        summary: "Mail messages with sender, subject, date and mailbox; mail.body reads one in full.",
        scope: "mail:read",
    },
    Route {
        collection: "notes.note",
        summary: "Notes with title and full plain text, including notes written on the iPhone.",
        scope: "notes:read",
    },
    Route {
        collection: "iphone.location",
        summary: "Locations the iPhone recorded over time.",
        scope: "iphone:read",
    },
];

const STOP: &[&str] = &[
    "a",
    "an",
    "the",
    "my",
    "me",
    "i",
    "im",
    "someone",
    "somebody",
    "to",
    "for",
    "of",
    "in",
    "on",
    "at",
    "and",
    "or",
    "is",
    "are",
    "was",
    "be",
    "do",
    "did",
    "does",
    "can",
    "could",
    "would",
    "please",
    "you",
    "your",
    "it",
    "this",
    "that",
    "with",
    "from",
    "what",
    "whats",
    "which",
    "how",
    "much",
    "many",
    "any",
    "all",
    "some",
    "about",
    "up",
    "out",
    "there",
    "here",
    "get",
    "show",
    "tell",
    "give",
    "let",
    "want",
    "need",
    "like",
    "just",
    "now",
    "today",
    "yesterday",
    "tomorrow",
    "week",
    "day",
    "last",
    "next",
    "recent",
    "latest",
    "new",
    "via",
];

const PHRASES: &[(&str, &str)] = &[
    ("screen time", "screentime"),
    ("time spent", "screentime"),
    ("spend time", "screentime"),
    ("spent time", "screentime"),
    ("much time", "screentime"),
    ("phone usage", "screentime"),
    ("app usage", "screentime"),
    ("where am i", "location"),
    ("where i am", "location"),
    ("voice memo", "voicememo"),
    ("e-mail", "email"),
    ("to do list", "todo"),
    ("todo list", "todo"),
    ("schedule a", "add"),
    ("to do", "todo"),
    ("to-do", "todo"),
    ("wi-fi", "wifi"),
    ("phone call", "call"),
    ("do not disturb", "focus"),
    ("dark mode", "appearance"),
    ("light mode", "appearance"),
    ("look at", "preview"),
    ("what is playing", "song"),
    ("time machine", "backup"),
    ("thumbs up", "react"),
    ("read aloud", "speak"),
    ("check off", "complete"),
    ("mark done", "complete"),
    ("mark as done", "complete"),
    ("browsing history", "safari"),
    ("text message", "text"),
];

const SYNONYMS: &[(&str, &[(&str, u32)])] = &[
    ("text", &[("messages", 8)]),
    ("sms", &[("messages", 8)]),
    ("imessag", &[("messages", 8)]),
    ("chat", &[("messages", 6)]),
    ("conversation", &[("messages", 6)]),
    ("dm", &[("messages", 6)]),
    ("email", &[("mail", 8), ("mail.send", 10)]),
    ("inbox", &[("mail", 8), ("records.query/mail.message", 10)]),
    ("screentime", &[("usage.summary", 16), ("usage", 6)]),
    ("usage", &[("usage.summary", 12)]),
    ("spend", &[("usage.summary", 10)]),
    ("spent", &[("usage.summary", 10)]),
    ("phone", &[("usage.summary", 0)]),
    ("app", &[("usage.summary", 2)]),
    (
        "location",
        &[("location.current", 12), ("iphone.location.current", 8)],
    ),
    ("wifi", &[("mac.network_check", 14)]),
    ("internet", &[("mac.network_check", 14)]),
    ("online", &[("mac.network_check", 10)]),
    ("connection", &[("mac.network_check", 8)]),
    ("file", &[("files", 8)]),
    ("document", &[("files", 8), ("files.search", 4)]),
    ("folder", &[("files", 4)]),
    ("voicemail", &[("records.query/voicemail.message", 18)]),
    ("safari", &[("records.query/safari.history", 12)]),
    ("browser", &[("records.query/safari.history", 12)]),
    ("websit", &[("records.query/safari.history", 10)]),
    ("visit", &[("records.query/safari.history", 8)]),
    ("call", &[("records.query/phone.call", 14)]),
    ("missed", &[("records.query/phone.call", 10)]),
    ("voicememo", &[("records.query/voice_memos.recording", 18)]),
    ("recording", &[("records.query/voice_memos.recording", 6)]),
    ("event", &[("calendar", 6)]),
    ("meeting", &[("calendar", 8), ("event", 0)]),
    ("appointment", &[("calendar", 8)]),
    ("calendar", &[("calendar", 6)]),
    ("schedul", &[("calendar.agenda", 4)]),
    ("busy", &[("calendar.agenda", 8)]),
    ("free", &[("calendar.agenda", 6)]),
    ("todo", &[("reminders", 8), ("reminders.items", 10)]),
    ("task", &[("reminders", 8)]),
    ("reminder", &[("reminders", 6)]),
    ("remind", &[("reminders.create", 12)]),
    ("contact", &[("contacts", 6)]),
    ("person", &[("contacts.lookup", 8)]),
    ("people", &[("contacts.lookup", 8)]),
    ("number", &[("contacts.lookup", 8)]),
    ("who", &[("contacts.lookup", 6)]),
    ("birthday", &[("contacts.lookup", 4)]),
    (
        "photo",
        &[
            ("photos", 6),
            ("photos.search", 8),
            ("photos.contact_sheet", 7),
        ],
    ),
    (
        "picture",
        &[
            ("photos", 8),
            ("photos.search", 10),
            ("photos.contact_sheet", 8),
        ],
    ),
    ("pic", &[("photos", 8), ("photos.search", 10)]),
    ("image", &[("photos", 4)]),
    ("selfi", &[("photos", 8), ("photos.search", 10)]),
    ("countr", &[("photos.places", 8)]),
    ("travel", &[("photos.places", 8)]),
    ("trip", &[("photos.places", 8)]),
    ("visit", &[("photos.places", 6)]),
    ("fre", &[("calendar.free_slots", 10)]),
    ("arriv", &[("reminders.create", 10)]),
    ("leav", &[("reminders.create", 6)]),
    ("slot", &[("calendar.free_slots", 12)]),
    ("availabl", &[("calendar.free_slots", 10)]),
    ("attachment", &[("messages.shared", 12)]),
    ("pdf", &[("messages.shared", 10)]),
    ("link", &[("messages.shared", 10)]),
    ("shar", &[("messages.shared", 8)]),
    (
        "preview",
        &[("photos.preview", 8), ("photos.contact_sheet", 8)],
    ),
    (
        "note",
        &[
            ("notes", 6),
            ("notes.list", 10),
            ("records.query/notes.note", 9),
        ],
    ),
    ("volum", &[("mac.set_volume", 12)]),
    ("loud", &[("mac.set_volume", 8)]),
    ("mute", &[("mac.set_mute", 12)]),
    ("appearance", &[("mac.set_appearance", 14)]),
    ("health", &[("health.summary", 10)]),
    ("step", &[("health.summary", 10)]),
    ("sleep", &[("health.summary", 6)]),
    ("heart", &[("health.summary", 8)]),
    ("clipboard", &[("mac.clipboard_read", 10)]),
    ("copied", &[("mac.clipboard_read", 10)]),
    ("direction", &[("maps.directions", 10)]),
    ("drive", &[("maps.eta", 8), ("maps.directions", 6)]),
    ("route", &[("maps.directions", 8)]),
    ("restaurant", &[("maps.nearby", 12)]),
    ("nearby", &[("maps.nearby", 10)]),
    ("alarm", &[("iphone.alarm.create", 6)]),
    ("wake", &[("iphone.alarm.create", 10)]),
    ("timer", &[("iphone.timer.start", 8)]),
    ("notify", &[("mac.notify", 6), ("iphone.notify", 6)]),
    ("notification", &[("mac.notify", 6), ("iphone.notify", 6)]),
    ("react", &[("messages.react", 12)]),
    ("tapback", &[("messages.react", 12)]),
    ("song", &[("mac.recognize_song", 14)]),
    ("shazam", &[("mac.recognize_song", 14)]),
    ("shortcut", &[("shortcuts.run", 8)]),
    ("speak", &[("speech.say", 12)]),
    ("say", &[("speech.say", 8)]),
    ("ocr", &[("vision.ocr", 14)]),
    ("focus", &[("focus.set", 12)]),
    ("battery", &[("iphone.device.status", 10)]),
    ("backup", &[("mac.time_machine_status", 14)]),
    ("block", &[("iphone.screentime.shield", 12)]),
    ("limit", &[("iphone.screentime.schedule", 8)]),
    ("screenshot", &[("capture.screenshot", 14)]),
    ("unread", &[("messages", 2), ("messages.chats", 9)]),
];

const VERBS: &[(&str, &str)] = &[
    ("add", "create"),
    ("make", "create"),
    ("creat", "create"),
    ("new", "create"),
    ("write", "create"),
    ("put", "create"),
    ("set", "create"),
    ("remov", "delete"),
    ("cancel", "delete"),
    ("trash", "delete"),
    ("eras", "delete"),
    ("done", "complete"),
    ("finish", "complete"),
    ("tick", "complete"),
    ("complet", "complete"),
    ("move", "update"),
    ("reschedul", "update"),
    ("chang", "update"),
    ("edit", "update"),
    ("postpon", "update"),
    ("find", "search"),
    ("look", "search"),
    ("lookup", "search"),
    ("seen", "search"),
    ("text", "send"),
    ("messag", "send"),
    ("email", "send"),
    ("tell", "send"),
    ("list", "list"),
    ("read", "read"),
    ("check", "status"),
    ("open", "open"),
    ("star", "favorite"),
    ("favourit", "favorite"),
    ("favorit", "favorite"),
];

pub(crate) enum Hit<'a> {
    Operation(&'a AgentOperation),
    Route(&'a Route),
}

pub(crate) fn stem(word: &str) -> String {
    let word = word.to_lowercase();
    for suffix in ["ings", "ing", "ied", "ies", "ed", "es", "s", "e"] {
        if word.len() > suffix.len() + 2
            && let Some(base) = word.strip_suffix(suffix)
        {
            return match suffix {
                "ied" | "ies" => format!("{base}y"),
                _ => base.to_owned(),
            };
        }
    }
    word
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(stem)
        .collect()
}

fn query_words(query: &str) -> Vec<String> {
    let mut text = format!(" {} ", query.to_lowercase());
    for (phrase, token) in PHRASES {
        text = text.replace(&format!(" {phrase} "), &format!(" {token} "));
        text = text.replace(phrase, token);
    }
    let mut seen = BTreeSet::new();
    text.split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .map(|w| w.trim_matches('-'))
        .filter(|w| !w.is_empty() && !STOP.contains(w))
        .map(stem)
        .filter(|w| seen.insert(w.clone()))
        .collect()
}

fn targets(target: &str, name: &str, domain: &str) -> bool {
    target == name || target == domain
}

fn score(words: &[String], name: &str, domain: &str, summary: &str, about: &str) -> u32 {
    let domain_tokens: Vec<String> = tokens(domain);
    let name_tokens: Vec<String> = tokens(name)
        .into_iter()
        .filter(|t| !domain_tokens.contains(t))
        .collect();
    let summary_tokens: BTreeSet<String> = tokens(summary).into_iter().collect();
    let about_tokens: BTreeSet<String> = tokens(about).into_iter().collect();
    let mut total = 0;
    for word in words {
        let verb = VERBS
            .iter()
            .find(|(from, _)| stem(from) == *word)
            .map(|(_, to)| stem(to));
        let mut best = 0;
        if domain_tokens.iter().any(|t| t == word) {
            best = best.max(6);
        }
        if name_tokens
            .iter()
            .any(|t| t == word || verb.as_ref().is_some_and(|v| t == v))
        {
            best = best.max(7);
        }
        if summary_tokens.contains(word) {
            best = best.max(2);
        }
        if best == 0 && about_tokens.contains(word) {
            best = 1;
        }
        for (key, boosts) in SYNONYMS {
            if stem(key) == *word {
                for (target, weight) in *boosts {
                    if targets(target, name, domain) {
                        best = best.max(*weight);
                    }
                }
            }
        }
        total += best;
    }
    total
}

const LOOKING: &[&str] = &[
    "find", "show", "look", "see", "read", "list", "what", "which", "how", "many", "view", "get",
    "check", "open", "search", "where", "who", "when", "did",
];

const ACTING: &[&str] = &[
    "send", "take", "captur", "snap", "record", "shoot", "reply", "mark", "flag", "rename",
];

fn looking(words: &[String]) -> bool {
    let acting = words.iter().any(|word| {
        ACTING.iter().any(|a| stem(a) == *word)
            || VERBS.iter().any(|(from, to)| {
                stem(from) == *word
                    && matches!(
                        *to,
                        "create" | "delete" | "complete" | "update" | "send" | "favorite"
                    )
            })
    });
    !acting
        && words
            .iter()
            .any(|word| LOOKING.iter().any(|l| stem(l) == *word))
}

fn domain_named(words: &[String], domain: &str) -> bool {
    let domain_stem = stem(domain);
    words.iter().any(|word| {
        *word == domain_stem
            || SYNONYMS.iter().any(|(key, boosts)| {
                stem(key) == *word
                    && boosts
                        .iter()
                        .any(|(target, _)| target.split('.').next() == Some(domain))
            })
    })
}

pub(crate) fn find<'a>(
    registry: &'a Registry,
    principal: Option<&'a Principal>,
    query: &str,
) -> (Vec<Hit<'a>>, usize) {
    let words = query_words(query);
    if words.is_empty() {
        return (Vec::new(), 0);
    }
    let about = |domain: &str| {
        registry
            .domains
            .iter()
            .find(|(d, _)| d == domain)
            .map_or("", |(_, about)| about.as_str())
    };
    let mut scored: Vec<(u32, u8, String, Hit<'a>)> = registry
        .visible(principal)
        .filter_map(|operation| {
            let points = score(
                &words,
                &operation.name,
                &operation.domain,
                &operation.summary,
                about(&operation.domain),
            );
            (points > 0).then(|| (points, 0, operation.name.clone(), Hit::Operation(operation)))
        })
        .collect();
    for route in ROUTES {
        if principal.is_some_and(|p| !crate::mcp_registry::can(p, route.scope)) {
            continue;
        }
        let name = format!("records.query/{}", route.collection);
        let domain = route.collection.split('.').next().unwrap_or_default();
        let points = score(&words, &name, domain, route.summary, "");
        if points > 0 {
            scored.push((points, 1, name, Hit::Route(route)));
        }
    }
    if looking(&tokens(query)) {
        for (points, _, name, hit) in &mut scored {
            let (changes, domain) = match hit {
                Hit::Operation(o) => (o.changes && !o.read_only(), o.domain.as_str()),
                Hit::Route(r) => (false, r.collection.split('.').next().unwrap_or_default()),
            };
            if changes || name.contains("capture") {
                *points /= 3;
            } else if domain_named(&words, domain) {
                *points += 3;
            }
        }
        scored.retain(|(p, ..)| *p > 0);
    }
    let best = scored.iter().map(|(p, ..)| *p).max().unwrap_or(0);
    scored.retain(|(p, ..)| *p * 4 >= best);
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let total = scored.len();
    (
        scored.into_iter().take(10).map(|(.., hit)| hit).collect(),
        total,
    )
}
