use super::{Failure, Outcome, tree::StatsArgs};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

const NON_INTERACTIVE_WATCH_LIMIT: Duration = Duration::from_secs(300);
const DEFAULT_SETTLE: Duration = Duration::from_secs(30);
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_secs(1);

struct Noun {
    collection: &'static str,
    one: &'static str,
    many: &'static str,
}

const fn noun(collection: &'static str, one: &'static str, many: &'static str) -> Noun {
    Noun {
        collection,
        one,
        many,
    }
}

struct Described {
    source: &'static str,
    label: &'static str,
    nouns: &'static [Noun],
}

const DESCRIBED: &[Described] = &[
    Described {
        source: "messages",
        label: "Messages",
        nouns: &[
            noun("messages.message", "message", "messages"),
            noun("messages.chat", "chat", "chats"),
            noun("messages.attachment", "attachment", "attachments"),
        ],
    },
    Described {
        source: "contacts",
        label: "Contacts",
        nouns: &[noun("contacts.contact", "contact", "contacts")],
    },
    Described {
        source: "calendar",
        label: "Calendar",
        nouns: &[
            noun("calendar.event", "event", "events"),
            noun("calendar.calendar", "calendar", "calendars"),
        ],
    },
    Described {
        source: "reminders",
        label: "Reminders",
        nouns: &[
            noun("reminders.reminder", "reminder", "reminders"),
            noun("reminders.list", "list", "lists"),
        ],
    },
    Described {
        source: "notes",
        label: "Notes",
        nouns: &[
            noun("notes.note", "note", "notes"),
            noun("notes.folder", "folder", "folders"),
        ],
    },
    Described {
        source: "mail",
        label: "Mail",
        nouns: &[
            noun("mail.message", "email", "emails"),
            noun("mail.mailbox", "mailbox", "mailboxes"),
        ],
    },
    Described {
        source: "safari",
        label: "Safari",
        nouns: &[
            noun("safari.history", "history item", "history items"),
            noun("safari.bookmark", "bookmark", "bookmarks"),
            noun("safari.tab", "open tab", "open tabs"),
        ],
    },
    Described {
        source: "phone",
        label: "Phone",
        nouns: &[noun("phone.call", "call", "calls")],
    },
    Described {
        source: "usage",
        label: "Screen Time",
        nouns: &[noun("usage.app_day", "app day", "app days")],
    },
    Described {
        source: "files",
        label: "Files",
        nouns: &[noun("files.item", "file", "files")],
    },
    Described {
        source: "shortcuts",
        label: "Shortcuts",
        nouns: &[noun("shortcuts.shortcut", "shortcut", "shortcuts")],
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceStats {
    pub source: String,
    pub label: String,
    pub state: String,
    pub counts: BTreeMap<String, i64>,
    pub total: i64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Snapshot {
    pub sources: Vec<SourceStats>,
    pub total: i64,
}

impl SourceStats {
    fn phrase(&self) -> String {
        let described = DESCRIBED.iter().find(|entry| entry.source == self.source);
        let mut parts = Vec::new();
        if let Some(described) = described {
            for noun in described.nouns {
                if let Some(count) = self.counts.get(noun.collection).filter(|count| **count > 0) {
                    let word = if *count == 1 { noun.one } else { noun.many };
                    parts.push(format!("{} {word}", thousands(*count)));
                }
            }
        }
        if parts.is_empty() && self.total > 0 {
            let word = if self.total == 1 { "item" } else { "items" };
            parts.push(format!("{} {word}", thousands(self.total)));
        }
        if parts.is_empty() {
            "nothing yet".to_owned()
        } else {
            parts.join(" · ")
        }
    }

    fn mark(&self) -> &'static str {
        match self.state.as_str() {
            "ready" => "✓",
            "degraded" => "~",
            "unknown" => "…",
            _ => "!",
        }
    }

    fn advice(&self) -> Option<&'static str> {
        match self.state.as_str() {
            "permission_missing" => Some("needs a Mac permission: run bridge permissions guide"),
            "capture_failed" => Some("could not be read: run bridge doctor"),
            "unavailable" => Some("not available on this Mac right now"),
            "degraded" => Some("partly available; if this follows an update, run bridge down, then bridge up; otherwise run bridge doctor"),
            _ => None,
        }
    }

    fn line(&self) -> String {
        let advice = self
            .advice()
            .map_or_else(String::new, |advice| format!("  ({advice})"));
        format!(
            "  {} {:<12} {}{advice}",
            self.mark(),
            self.label,
            self.phrase()
        )
    }

    fn attention(&self) -> bool {
        !["ready", "degraded", "unknown"].contains(&self.state.as_str())
    }

    fn core(&self) -> bool {
        DESCRIBED.iter().any(|entry| entry.source == self.source)
    }
}

pub fn thousands(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if value < 0 {
        out.insert(0, '-');
    }
    out
}

fn open(home: &Path) -> Outcome<Option<Connection>> {
    let path = home.join("state.db");
    if !path.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| Failure::new(format!("Could not open this home's data: {error}.")))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| Failure::new(format!("Could not open this home's data: {error}.")))?;
    Ok(Some(connection))
}

fn read_error(error: &rusqlite::Error) -> Failure {
    Failure::new(format!("Could not read this home's data: {error}."))
        .next("Run `bridge doctor` to check it.")
}

fn title_case(source: &str) -> String {
    let mut characters = source.chars();
    characters.next().map_or_else(String::new, |first| {
        first
            .to_uppercase()
            .chain(characters)
            .collect::<String>()
            .replace('_', " ")
    })
}

fn source_states(connection: &Connection) -> Result<BTreeMap<String, String>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT json_extract(CAST(payload AS TEXT),'$.data.source'),json_extract(CAST(payload AS TEXT),'$.data.state') FROM records WHERE collection='system.source' AND deleted=0",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    let mut states = BTreeMap::new();
    for row in rows {
        if let (Some(source), Some(state)) = row? {
            states.insert(source, state);
        }
    }
    Ok(states)
}

pub fn read_states(connection: &Connection) -> Outcome<BTreeMap<String, String>> {
    source_states(connection).map_err(|error| read_error(&error))
}

fn collection_counts(connection: &Connection) -> Result<Vec<(String, i64)>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT collection,count(*) FROM records WHERE deleted=0 AND collection NOT LIKE 'system.%' GROUP BY collection",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

fn order(source: &str) -> usize {
    DESCRIBED
        .iter()
        .position(|entry| entry.source == source)
        .unwrap_or(DESCRIBED.len())
}

pub fn read_snapshot(connection: &Connection) -> Outcome<Snapshot> {
    let states = source_states(connection).map_err(|error| read_error(&error))?;
    let counts = collection_counts(connection).map_err(|error| read_error(&error))?;
    let mut sources = BTreeMap::<String, SourceStats>::new();
    for source in states.keys() {
        sources.insert(source.clone(), empty_source(source, &states));
    }
    for (collection, count) in counts {
        let source = collection
            .split_once('.')
            .map_or(collection.as_str(), |(source, _)| source)
            .to_owned();
        let entry = sources
            .entry(source.clone())
            .or_insert_with(|| empty_source(&source, &states));
        entry.total += count;
        entry.counts.insert(collection, count);
    }
    let mut sources: Vec<_> = sources.into_values().collect();
    sources.sort_by(|a, b| {
        order(&a.source)
            .cmp(&order(&b.source))
            .then_with(|| a.source.cmp(&b.source))
    });
    let total = sources.iter().map(|source| source.total).sum();
    Ok(Snapshot { sources, total })
}

fn empty_source(source: &str, states: &BTreeMap<String, String>) -> SourceStats {
    let label = DESCRIBED
        .iter()
        .find(|entry| entry.source == source)
        .map_or_else(|| title_case(source), |entry| entry.label.to_owned());
    SourceStats {
        source: source.to_owned(),
        label,
        state: states
            .get(source)
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned()),
        counts: BTreeMap::new(),
        total: 0,
    }
}

fn visible(source: &SourceStats, all: bool) -> bool {
    all || source.total > 0 || source.core()
}

fn print_snapshot(snapshot: &Snapshot, all: bool) {
    let mut shown = 0;
    for source in snapshot
        .sources
        .iter()
        .filter(|source| visible(source, all))
    {
        println!("{}", source.line());
        shown += 1;
    }
    if shown == 0 {
        println!("  Nothing has synced yet. Sources appear here as they connect.");
        return;
    }
    println!(
        "  {} items in total. Counts only, no content.",
        thousands(snapshot.total)
    );
}

pub fn print_once(home: &Path) -> Outcome {
    let snapshot = match open(home)? {
        Some(connection) => read_snapshot(&connection)?,
        None => Snapshot::default(),
    };
    print_snapshot(&snapshot, false);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Appeared(SourceStats),
    Progress(SourceStats),
    Finished(SourceStats, Duration),
    Attention(SourceStats),
    AllFinished(i64, Duration),
}

#[derive(Debug)]
struct Seen {
    total: i64,
    state: String,
    since: Instant,
    changed: Instant,
    reported: Instant,
    finished: bool,
}

#[derive(Debug)]
pub struct Tracker {
    settle: Duration,
    began: Instant,
    seen: BTreeMap<String, Seen>,
    announced_done: bool,
}

impl Tracker {
    pub fn new(settle: Duration, now: Instant) -> Self {
        Self {
            settle,
            began: now,
            seen: BTreeMap::new(),
            announced_done: false,
        }
    }

    pub fn observe(&mut self, snapshot: &Snapshot, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        for source in &snapshot.sources {
            self.observe_source(source, now, &mut events);
        }
        let everything_done = !self.seen.is_empty()
            && snapshot.sources.len() == self.seen.len()
            && self.seen.values().all(|seen| seen.finished);
        if everything_done && !self.announced_done {
            self.announced_done = true;
            let last_change = self
                .seen
                .values()
                .map(|seen| seen.changed)
                .max()
                .unwrap_or(self.began);
            events.push(Event::AllFinished(snapshot.total, last_change - self.began));
        } else if !everything_done {
            self.announced_done = false;
        }
        events
    }

    pub fn done(&self) -> bool {
        self.announced_done
    }

    fn observe_source(&mut self, source: &SourceStats, now: Instant, events: &mut Vec<Event>) {
        let settle = self.settle;
        let Some(seen) = self.seen.get_mut(&source.source) else {
            self.seen.insert(
                source.source.clone(),
                Seen {
                    total: source.total,
                    state: source.state.clone(),
                    since: now,
                    changed: now,
                    reported: now,
                    finished: source.attention(),
                },
            );
            events.push(if source.attention() {
                Event::Attention(source.clone())
            } else {
                Event::Appeared(source.clone())
            });
            return;
        };
        let state_changed = seen.state != source.state;
        if seen.total != source.total || state_changed {
            seen.total = source.total;
            seen.state.clone_from(&source.state);
            seen.changed = now;
            seen.finished = false;
        }
        if source.attention() {
            if state_changed || !seen.finished {
                events.push(Event::Attention(source.clone()));
            }
            seen.finished = true;
            return;
        }
        if seen.finished {
            return;
        }
        if source.state != "unknown" && now - seen.changed >= settle {
            seen.finished = true;
            events.push(Event::Finished(source.clone(), seen.changed - seen.since));
        } else if seen.changed > seen.reported && now - seen.reported >= PROGRESS_EVERY {
            seen.reported = now;
            events.push(Event::Progress(source.clone()));
        }
    }
}

fn emit(event: &Event, json_lines: bool, all: bool) {
    if json_lines {
        println!("{}", event_json(event));
        return;
    }
    match event {
        Event::Appeared(source) if visible(source, all) => {
            println!(
                "  … {:<12} syncing, {} so far",
                source.label,
                source.phrase()
            );
        }
        Event::Progress(source) if visible(source, all) => {
            println!("  … {:<12} {} so far", source.label, source.phrase());
        }
        Event::Finished(source, _) | Event::Attention(source) if visible(source, all) => {
            println!("{}", source.line());
        }
        Event::AllFinished(total, elapsed) => {
            println!(
                "Finished: {} items in {}. Counts only, no content.",
                thousands(*total),
                friendly_duration(*elapsed)
            );
        }
        _ => {}
    }
}

fn friendly_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 90 {
        format!("{seconds} s")
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}

fn source_json(kind: &str, source: &SourceStats) -> Value {
    json!({"event":kind,"source":source.source,"label":source.label,"state":source.state,"total":source.total,"counts":source.counts})
}

fn event_json(event: &Event) -> Value {
    match event {
        Event::Appeared(source) => source_json("syncing", source),
        Event::Progress(source) => source_json("progress", source),
        Event::Attention(source) => source_json("attention", source),
        Event::Finished(source, elapsed) => {
            let mut value = source_json("finished", source);
            value["seconds"] = json!(elapsed.as_secs_f64());
            value
        }
        Event::AllFinished(total, elapsed) => {
            json!({"event":"all_finished","total":total,"seconds":elapsed.as_secs_f64()})
        }
    }
}

async fn watch(
    home: &Path,
    settle: Duration,
    timeout: Option<Duration>,
    json_lines: bool,
    all: bool,
) -> Outcome {
    let began = Instant::now();
    let mut tracker = Tracker::new(settle, began);
    let mut connection = None;
    let mut told_waiting = false;
    loop {
        if connection.is_none() {
            connection = open(home)?;
        }
        let snapshot = match &connection {
            Some(connection) => read_snapshot(connection)?,
            None => Snapshot::default(),
        };
        if snapshot.sources.is_empty() && !told_waiting && !json_lines {
            println!("  Waiting for sources to connect…");
            told_waiting = true;
        }
        for event in tracker.observe(&snapshot, Instant::now()) {
            emit(&event, json_lines, all);
        }
        if tracker.done() {
            return Ok(());
        }
        if timeout.is_some_and(|limit| began.elapsed() >= limit) {
            if json_lines {
                println!("{}", json!({"event":"timeout","total":snapshot.total}));
            } else {
                println!(
                    "Still syncing after {}: {} items so far.",
                    friendly_duration(began.elapsed()),
                    thousands(snapshot.total)
                );
            }
            return Ok(());
        }
        tokio::time::sleep(POLL).await;
    }
}

pub async fn watch_first_sync(home: &Path) -> Outcome {
    watch(home, DEFAULT_SETTLE, None, false, false).await
}

pub async fn run(home: &Path, args: &StatsArgs) -> Outcome {
    if args.watch {
        let timeout = args
            .timeout
            .map(Duration::from_secs)
            .or_else(|| (!super::interactive()).then_some(NON_INTERACTIVE_WATCH_LIMIT));
        return watch(
            home,
            Duration::from_secs(args.settle),
            timeout,
            args.json,
            args.all,
        )
        .await;
    }
    let Some(connection) = open(home)? else {
        return Err(Failure::not_set_up(home));
    };
    let snapshot = read_snapshot(&connection)?;
    if args.json {
        println!("{}", serde_json::to_string(&snapshot)?);
    } else {
        print_snapshot(&snapshot, args.all);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(name: &str, state: &str, total: i64) -> SourceStats {
        SourceStats {
            source: name.into(),
            label: name.into(),
            state: state.into(),
            counts: BTreeMap::from([(format!("{name}.item"), total)]),
            total,
        }
    }

    fn snapshot(sources: Vec<SourceStats>) -> Snapshot {
        let total = sources.iter().map(|source| source.total).sum();
        Snapshot { sources, total }
    }

    fn names(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                Event::Appeared(s) => format!("appeared {}", s.source),
                Event::Progress(s) => format!("progress {} {}", s.source, s.total),
                Event::Finished(s, _) => format!("finished {} {}", s.source, s.total),
                Event::Attention(s) => format!("attention {}", s.source),
                Event::AllFinished(total, _) => format!("all {total}"),
            })
            .collect()
    }

    #[test]
    fn each_source_finishes_on_its_own_and_the_run_ends_when_all_have() {
        let start = Instant::now();
        let settle = Duration::from_secs(15);
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut tracker = Tracker::new(settle, start);
        let events = tracker.observe(
            &snapshot(vec![
                source("contacts", "ready", 10),
                source("messages", "ready", 100),
            ]),
            at(1),
        );
        assert_eq!(names(&events), ["appeared contacts", "appeared messages"]);
        let events = tracker.observe(
            &snapshot(vec![
                source("contacts", "ready", 10),
                source("messages", "ready", 200),
            ]),
            at(10),
        );
        assert_eq!(names(&events), ["progress messages 200"]);
        let events = tracker.observe(
            &snapshot(vec![
                source("contacts", "ready", 10),
                source("messages", "ready", 300),
            ]),
            at(16),
        );
        assert_eq!(
            names(&events),
            ["finished contacts 10", "progress messages 300"]
        );
        assert!(!tracker.done());
        let events = tracker.observe(
            &snapshot(vec![
                source("contacts", "ready", 10),
                source("messages", "ready", 300),
            ]),
            at(31),
        );
        assert_eq!(names(&events), ["finished messages 300", "all 310"]);
        assert!(tracker.done());
    }

    #[test]
    fn a_source_that_grows_again_is_not_finished() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut tracker = Tracker::new(Duration::from_secs(5), start);
        tracker.observe(&snapshot(vec![source("mail", "ready", 1)]), at(0));
        let events = tracker.observe(&snapshot(vec![source("mail", "ready", 1)]), at(6));
        assert_eq!(names(&events), ["finished mail 1", "all 1"]);
        tracker.observe(&snapshot(vec![source("mail", "ready", 9)]), at(7));
        assert!(!tracker.done());
    }

    #[test]
    fn a_missing_permission_does_not_hold_up_the_rest() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut tracker = Tracker::new(Duration::from_secs(5), start);
        let events = tracker.observe(
            &snapshot(vec![
                source("messages", "permission_missing", 0),
                source("notes", "ready", 3),
            ]),
            at(0),
        );
        assert_eq!(names(&events), ["attention messages", "appeared notes"]);
        let events = tracker.observe(
            &snapshot(vec![
                source("messages", "permission_missing", 0),
                source("notes", "ready", 3),
            ]),
            at(6),
        );
        assert_eq!(names(&events), ["finished notes 3", "all 3"]);
    }

    #[test]
    fn counts_are_written_for_people_without_any_content() {
        let stats = SourceStats {
            source: "messages".into(),
            label: "Messages".into(),
            state: "ready".into(),
            counts: BTreeMap::from([
                ("messages.message".into(), 163_101),
                ("messages.chat".into(), 1),
                ("messages.handle".into(), 867),
            ]),
            total: 163_969,
        };
        assert_eq!(stats.phrase(), "163,101 messages · 1 chat");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
    }
}
