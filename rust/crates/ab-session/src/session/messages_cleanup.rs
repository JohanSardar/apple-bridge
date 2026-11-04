use super::messages_local::{Config, write_private};
use super::messages_ui::{Failure, Ui};
use super::owned::{Probe, text};
use crate::messages_policy::{
    MediaProof, allows_late_echo, can_delete_reaction, can_delete_retracted, is_self_chat,
    normalize, tag_at_start, tag_run, tagged,
};
use ab_native::messages::Reader;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, thread,
    time::{Duration, Instant},
};

const MAX_ROUNDS: usize = 200;
const REMOVAL_WAIT: Duration = Duration::from_secs(10);
const NATIVE_WAIT: Duration = Duration::from_secs(5);

pub(super) trait Screen {
    fn clear_draft(&self, tag: &str) -> Result<bool, Failure>;
    fn open(&self, guid: &str, settle: u64, tag: &str) -> Result<(), Failure>;
    fn acknowledge(&self) -> Result<bool, Failure>;
    fn validate(&self, prepared: &Value) -> Result<(), Failure>;
    fn delete(&self, prepared: &Value) -> Result<(), Failure>;
    fn delete_owned_item(&self, guid: &str, chat: &str);
}

struct LiveScreen<'a> {
    ui: Ui<'a>,
    next: &'a dyn Probe,
}

impl Screen for LiveScreen<'_> {
    fn clear_draft(&self, tag: &str) -> Result<bool, Failure> {
        self.ui.clear_tagged_draft(tag, &[])
    }
    fn open(&self, guid: &str, settle: u64, tag: &str) -> Result<(), Failure> {
        self.ui.open_message(guid, settle, true, tag)
    }
    fn acknowledge(&self) -> Result<bool, Failure> {
        self.ui.acknowledge_deleted_info()
    }
    fn validate(&self, prepared: &Value) -> Result<(), Failure> {
        self.ui.validate(prepared)
    }
    fn delete(&self, prepared: &Value) -> Result<(), Failure> {
        self.ui.perform(prepared).map(|_| ())
    }
    fn delete_owned_item(&self, guid: &str, chat: &str) {
        let _ = self.next.call(
            "messages.delete_owned_item",
            &json!({"guid": guid, "chat": chat}),
        );
    }
}

struct Ledger {
    run: String,
    baseline: i64,
    owned: BTreeSet<String>,
    media: Option<MediaProof>,
    interactive: String,
    interactive_baseline: i64,
    creations: std::path::PathBuf,
}

fn strings(value: &Value) -> impl Iterator<Item = String> + '_ {
    value.as_array().into_iter().flatten().filter_map(|item| {
        item.as_str()
            .filter(|s| !s.is_empty())
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

fn load_ledger(config: &Config, tag: &str) -> Result<Ledger, String> {
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

fn last_component(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

struct Run<'a> {
    reader: Reader,
    screen: &'a dyn Screen,
    tag: String,
    own: BTreeSet<String>,
    ledger: Ledger,
    unreachable: BTreeSet<String>,
    native_wait: Duration,
}

impl Run<'_> {
    fn sql<T>(result: Result<T, ab_native::messages::Error>) -> Result<T, String> {
        result.map_err(|_| "unavailable".to_owned())
    }

    fn all_own(&self, peers: &[String]) -> bool {
        !peers.is_empty() && peers.iter().all(|peer| self.own.contains(&normalize(peer)))
    }

    fn remove_native(&mut self, guid: &str, chat: &str) -> Result<(), String> {
        if !self.unreachable.insert(guid.to_owned()) {
            return Ok(());
        }
        self.screen.delete_owned_item(guid, chat);
        let started = Instant::now();
        loop {
            if !Self::sql(self.reader.is_live(guid))? {
                self.unreachable.remove(guid);
                return Ok(());
            }
            if started.elapsed() >= self.native_wait {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn candidates(&mut self) -> Result<Vec<(String, Value)>, String> {
        if let Some(report) = fs::read(&self.ledger.creations)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|report| text(report, "run_id") == self.ledger.run)
        {
            self.ledger.owned.extend(strings(&report["ids"]));
        }
        let baseline = self.ledger.baseline;
        let mut keys = Self::sql(self.reader.tagged_guids())?;
        let seen: BTreeSet<String> = keys.iter().cloned().collect();
        if !self.ledger.owned.is_empty() {
            for guid in Self::sql(self.reader.live_guids_after(baseline))? {
                if self.ledger.owned.contains(&guid) && !seen.contains(&guid) {
                    keys.push(guid);
                }
            }
        }
        if !self.ledger.interactive.is_empty() && self.ledger.interactive_baseline > 0 {
            keys.extend(Self::sql(self.reader.guids_with_text_after(
                self.ledger.interactive_baseline,
                &self.ledger.interactive,
            ))?);
        }
        let (media, copies, equivalent) = self.media_rows(&mut keys)?;
        let mut result = Vec::new();
        for key in keys {
            if let Some(found) = self.candidate(&key, &media, &copies, equivalent)? {
                result.push(found);
            }
        }
        Ok(result)
    }

    #[allow(clippy::type_complexity)]
    fn media_rows(
        &self,
        keys: &mut Vec<String>,
    ) -> Result<(BTreeMap<String, Vec<String>>, BTreeMap<String, usize>, bool), String> {
        let Some(proof) = &self.ledger.media else {
            return Ok((BTreeMap::new(), BTreeMap::new(), false));
        };
        for (filename, transfer) in Self::sql(self.reader.attachment_names_through(proof.baseline))?
        {
            if proof.names.contains(last_component(&filename)) || proof.names.contains(&transfer) {
                return Err("conflict".into());
            }
        }
        let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut positions: BTreeMap<String, i64> = BTreeMap::new();
        for (guid, row, name) in Self::sql(self.reader.attachments_after(proof.baseline))? {
            names.entry(guid.clone()).or_default().push(name);
            positions.insert(guid, row);
        }
        let mut media: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (guid, attachments) in &names {
            let peers = Self::sql(self.reader.visible_peers(guid))?;
            let row = positions.get(guid).copied().unwrap_or(0);
            if proof.authorizes(row, attachments, &peers, &self.own) {
                media.insert(guid.clone(), proof.target_names(attachments));
            }
        }
        let mut copies = BTreeMap::new();
        for (guid, wanted) in &media {
            let matching: Vec<&String> = names
                .iter()
                .filter(|(_, attachments)| {
                    attachments
                        .iter()
                        .any(|name| wanted.iter().any(|want| want == last_component(name)))
                })
                .map(|(key, _)| key)
                .collect();
            if matching.iter().all(|key| media.contains_key(*key)) {
                copies.insert(guid.clone(), matching.len());
            }
        }
        for guid in Self::sql(self.reader.live_guids_after(proof.baseline))? {
            if media.contains_key(&guid) && !keys.contains(&guid) {
                keys.push(guid);
            }
        }
        Ok((media, copies, true))
    }

    fn candidate(
        &mut self,
        key: &str,
        media: &BTreeMap<String, Vec<String>>,
        copies: &BTreeMap<String, usize>,
        equivalent: bool,
    ) -> Result<Option<(String, Value)>, String> {
        let baseline = self.ledger.baseline;
        let Some(position) = Self::sql(self.reader.row_of(key))? else {
            return Ok(None);
        };
        if position <= baseline {
            return Ok(None);
        }
        let Some(row) = Self::sql(self.reader.cleanup_row(key))? else {
            return Ok(None);
        };
        let peers = Self::sql(self.reader.chat_peers(&row.chat))?;
        if !self.all_own(&peers) {
            return Ok(None);
        }
        let own: Vec<String> = self.own.iter().cloned().collect();
        let all_peers = Self::sql(self.reader.all_peers(key))?;
        if row.collection == "messages.reaction" {
            if can_delete_reaction(
                row.row,
                baseline,
                &row.guid,
                &row.parent,
                &self.ledger.owned,
                row.raw_text.contains(&self.tag),
            ) && is_self_chat(&peers, &own)
                && self.all_own(&all_peers)
            {
                self.ledger.owned.insert(row.guid.clone());
                self.remove_native(&row.guid, &row.chat)?;
            }
            return Ok(None);
        }
        if row.collection != "messages.message" || !self.all_own(&all_peers) {
            return Ok(None);
        }
        let body = text(&row.data, "text").to_owned();
        if !self.ledger.owned.contains(key)
            && !media.contains_key(key)
            && !allows_late_echo(row.row, baseline, &body, &self.tag)
        {
            return Ok(None);
        }
        self.ledger.owned.insert(row.guid.clone());
        let mut target = row.data.clone();
        if let (Some(names), Some(count)) = (media.get(key), copies.get(key)) {
            target["cleanup_media_names"] = json!(names);
            target["cleanup_media_owned_copies"] = json!(count);
            target["cleanup_self_echo"] = json!(true);
            target["cleanup_equivalent_owned_media"] = json!(equivalent);
            return Ok(Some((row.guid, target)));
        }
        if !self.ledger.interactive.is_empty() && body == self.ledger.interactive {
            return Ok(Some((row.guid, target)));
        }
        if !tagged(&body) {
            if !row.retracted {
                let parts = target["retracted_parts"]
                    .as_array()
                    .is_some_and(|parts| !parts.is_empty());
                if can_delete_retracted(
                    row.row,
                    baseline,
                    &row.guid,
                    &self.ledger.owned,
                    parts,
                    &body,
                ) && is_self_chat(&peers, &own)
                {
                    self.remove_native(&row.guid, &row.chat)?;
                    return Ok(None);
                }
                return Err("conflict".into());
            }
            return Ok(None);
        }
        if !body.contains(&self.tag) {
            return Err("conflict".into());
        }
        Ok(Some((row.guid, target)))
    }
}

fn progress(config: &Config, initial: usize, deleted: usize, remaining: usize, phase: &str) {
    let body = json!({
        "initial_tagged_self_messages": initial,
        "deleted": deleted,
        "remaining_tagged_self_messages": remaining,
        "phase": phase,
    });
    let _ = write_private(
        &config.reports.join("messages-cleanup-progress.json"),
        body.to_string().as_bytes(),
    );
}

enum Step {
    Deleted,
    SkipRow(String),
    Stop(String),
}

fn delete_row(run: &mut Run<'_>, guid: &str, target: &Value, tag: &str) -> Result<Step, String> {
    let prepared = json!({
        "native_state": {"target": target, "cleanup_authorized": true},
        "fingerprint": "authorized-tagged-self-cleanup",
        "action": {"op": "messages.delete_message", "native_key": guid},
    });
    let screen = run.screen;
    let mut phase = "open";
    let attempt = (|| {
        screen.open(guid, 1000, tag)?;
        if screen.acknowledge()? {
            screen.open(guid, 250, tag)?;
        }
        phase = "validate";
        screen.validate(&prepared)?;
        phase = "delete";
        screen.delete(&prepared)
    })();
    let Err(error) = attempt else {
        return Ok(Step::Deleted);
    };
    if !Run::sql(run.reader.is_live(guid))? {
        return Ok(Step::Deleted);
    }
    let row = Run::sql(run.reader.cleanup_row(guid))?;
    match row {
        Some(row)
            if run.ledger.media.is_some()
                && phase == "validate"
                && error == Failure::TargetUnavailable =>
        {
            run.remove_native(guid, &row.chat)?;
            Ok(if run.unreachable.contains(guid) {
                Step::SkipRow("owned_media_native_item_not_removed".into())
            } else {
                Step::Deleted
            })
        }
        _ => Ok(Step::Stop(format!("{phase}:{}", error.name()))),
    }
}

pub(super) fn run(
    screen: &dyn Screen,
    config: &Config,
    own: BTreeSet<String>,
    tag: &str,
) -> Result<Value, String> {
    if tag_at_start(tag).is_none_or(|found| found.len() != tag.len()) {
        return Err("invalid".into());
    }
    let ledger = load_ledger(config, tag)?;
    let reader =
        Reader::open(&config.chat_db, &config.home).map_err(|_| "unavailable".to_owned())?;
    let drafts = screen.clear_draft(tag).map_err(|f| f.name())?;
    let mut run = Run {
        reader,
        screen,
        tag: tag.to_owned(),
        own,
        ledger,
        unreachable: BTreeSet::new(),
        native_wait: NATIVE_WAIT,
    };
    let initial = run.candidates()?.len();
    let mut deleted = 0;
    let mut failure = String::new();
    let mut unavailable: BTreeSet<String> = BTreeSet::new();
    progress(config, initial, deleted, initial, "starting");
    for _ in 0..MAX_ROUNDS {
        let remaining = run.candidates()?;
        let Some((guid, target)) = remaining
            .iter()
            .find(|(guid, _)| !unavailable.contains(guid))
            .cloned()
        else {
            break;
        };
        match delete_row(&mut run, &guid, &target, tag)? {
            Step::Deleted => {}
            Step::SkipRow(reason) => {
                failure = reason;
                unavailable.insert(guid);
                continue;
            }
            Step::Stop(reason) => {
                failure = reason;
                break;
            }
        }
        let before: BTreeSet<&String> = remaining.iter().map(|(guid, _)| guid).collect();
        let started = Instant::now();
        let (removed, left) = loop {
            let after = run.candidates()?;
            let now: BTreeSet<&String> = after.iter().map(|(guid, _)| guid).collect();
            let removed = before.difference(&now).count();
            if removed > 0 || started.elapsed() >= REMOVAL_WAIT {
                break (removed, after.len());
            }
            thread::sleep(Duration::from_millis(50));
        };
        if removed == 0 {
            failure = "native_deletion_not_observed".into();
            break;
        }
        deleted += removed;
        progress(config, initial, deleted, left, "deleting");
    }
    let candidates = run.candidates()?.len() + run.unreachable.len();
    let mut owned_left = 0;
    for guid in &run.ledger.owned {
        if Run::sql(run.reader.is_live_after(guid, run.ledger.baseline))? {
            owned_left += 1;
        }
    }
    let remaining = candidates.max(owned_left);
    if !run.unreachable.is_empty() && failure.is_empty() {
        "owned_native_item_not_removed".clone_into(&mut failure);
    }
    progress(
        config,
        initial,
        deleted,
        remaining,
        if failure.is_empty() {
            "finished"
        } else {
            &failure
        },
    );
    Ok(json!({
        "initial_tagged_self_messages": initial,
        "deleted": deleted,
        "remaining_tagged_self_messages": remaining,
        "drafts_cleared": u8::from(drafts),
        "only_tagged_self_messages": true,
        "tag_filtered": true,
        "failure": failure,
        "passed": remaining == 0,
    }))
}

pub(super) fn own_addresses(next: &dyn Probe) -> Result<BTreeSet<String>, String> {
    let own = next.call("messages.own_handles", &json!({}))?;
    let handles: BTreeSet<String> = own["handles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect();
    if handles.is_empty() {
        return Err("unavailable".into());
    }
    Ok(handles)
}

pub(super) fn selftest(
    next: &dyn Probe,
    local: &dyn Probe,
    config: &Config,
    tag: &str,
) -> Result<Value, String> {
    let screen = LiveScreen {
        ui: Ui::new(next, config.clone()),
        next,
    };
    let own = own_addresses(local)?;
    let result = run(&screen, config, own, tag)?;
    Ok(json!({"passed": result["passed"], "failure": result["failure"]}))
}

pub(super) fn report(
    next: &dyn Probe,
    local: &dyn Probe,
    config: &Config,
    tag: &str,
) -> Result<Value, String> {
    let screen = LiveScreen {
        ui: Ui::new(next, config.clone()),
        next,
    };
    let own = own_addresses(local)?;
    run(&screen, config, own, tag)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::type_complexity)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::{cell::RefCell, path::Path};

    const OWN: &str = "me@example.com";
    const BASELINE: i64 = 10;

    struct Fixture {
        directory: std::path::PathBuf,
        config: Config,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "ab-cleanup-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let reports = directory.join("reports");
            fs::create_dir_all(reports.join("messages-command-cleanup")).unwrap();
            let chat_db = directory.join("chat.db");
            let db = Connection::open(&chat_db).unwrap();
            db.execute_batch("
                CREATE TABLE message(ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB, is_from_me INTEGER DEFAULT 1, service TEXT DEFAULT 'iMessage', item_type INTEGER DEFAULT 0, associated_message_type INTEGER DEFAULT 0, associated_message_guid TEXT, associated_message_emoji TEXT, date INTEGER DEFAULT 1, date_retracted INTEGER DEFAULT 0, error INTEGER DEFAULT 0, is_read INTEGER DEFAULT 1, handle_id INTEGER DEFAULT 0, other_handle INTEGER DEFAULT 0, thread_originator_guid TEXT, reply_to_guid TEXT);
                CREATE TABLE chat(ROWID INTEGER PRIMARY KEY, guid TEXT, chat_identifier TEXT, service_name TEXT DEFAULT 'iMessage', properties BLOB, last_read_message_timestamp INTEGER DEFAULT 0);
                CREATE TABLE handle(ROWID INTEGER PRIMARY KEY, id TEXT, service TEXT DEFAULT 'iMessage');
                CREATE TABLE chat_handle_join(chat_id INTEGER, handle_id INTEGER);
                CREATE TABLE chat_message_join(chat_id INTEGER, message_id INTEGER);
                CREATE TABLE message_attachment_join(message_id INTEGER, attachment_id INTEGER);
                CREATE TABLE attachment(ROWID INTEGER PRIMARY KEY, guid TEXT, filename TEXT, transfer_name TEXT);
                CREATE TABLE chat_recoverable_message_join(chat_id INTEGER, message_id INTEGER, delete_date INTEGER);
                CREATE TABLE recoverable_message_part(chat_id INTEGER, message_id INTEGER, part_index INTEGER, delete_date INTEGER, part_text BLOB);
                INSERT INTO handle(ROWID,id) VALUES (1,'me@example.com'),(2,'friend@example.com');
                INSERT INTO chat(ROWID,guid,chat_identifier) VALUES (1,'self','me@example.com'),(2,'friend','friend@example.com'),(3,'group','group');
                INSERT INTO chat_handle_join VALUES (1,1),(2,2),(3,1),(3,2);
            ").unwrap();
            drop(db);
            let config = Config {
                lab: true,
                chat_db,
                home: directory.clone(),
                reports,
                os_build: "test".into(),
                agent_policy: None,
            };
            Self { directory, config }
        }

        fn db(&self) -> Connection {
            Connection::open(&self.config.chat_db).unwrap()
        }

        fn message(&self, row: i64, guid: &str, body: &str, chat: i64) {
            let db = self.db();
            db.execute(
                "INSERT INTO message(ROWID,guid,text) VALUES (?,?,?)",
                params![row, guid, body],
            )
            .unwrap();
            db.execute(
                "INSERT INTO chat_message_join VALUES (?,?)",
                params![chat, row],
            )
            .unwrap();
        }

        fn ledger(&self, run: &str, created: &[&str], extra: &Value) {
            let mut registration = json!({
                "scenario": "MSG-SELFTEST",
                "run_id": run,
                "baseline_max_row_id": BASELINE,
                "created_message_ids": created,
            });
            for (key, value) in extra.as_object().into_iter().flatten() {
                registration[key] = value.clone();
            }
            fs::write(
                self.config
                    .reports
                    .join("messages-command-cleanup")
                    .join(format!("{run}.json")),
                registration.to_string(),
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    struct Recorder<'a> {
        db: &'a Path,
        deleted: RefCell<Vec<String>>,
        validate: RefCell<Option<Failure>>,
    }

    impl<'a> Recorder<'a> {
        fn new(db: &'a Path) -> Self {
            Self {
                db,
                deleted: RefCell::new(Vec::new()),
                validate: RefCell::new(None),
            }
        }
        fn remove(&self, guid: &str) {
            self.deleted.borrow_mut().push(guid.to_owned());
            let db = Connection::open(self.db).unwrap();
            db.execute(
                "DELETE FROM chat_message_join WHERE message_id=(SELECT ROWID FROM message WHERE guid=?)",
                params![guid],
            )
            .unwrap();
            db.execute("DELETE FROM message WHERE guid=?", params![guid])
                .unwrap();
        }
    }

    impl Screen for Recorder<'_> {
        fn clear_draft(&self, _: &str) -> Result<bool, Failure> {
            Ok(false)
        }
        fn open(&self, _: &str, _: u64, _: &str) -> Result<(), Failure> {
            Ok(())
        }
        fn acknowledge(&self) -> Result<bool, Failure> {
            Ok(false)
        }
        fn validate(&self, _: &Value) -> Result<(), Failure> {
            match self.validate.borrow().clone() {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        }
        fn delete(&self, prepared: &Value) -> Result<(), Failure> {
            self.remove(text(&prepared["action"], "native_key"));
            Ok(())
        }
        fn delete_owned_item(&self, guid: &str, _: &str) {
            self.remove(guid);
        }
    }

    fn own() -> BTreeSet<String> {
        [OWN.to_owned()].into()
    }

    fn live(fixture: &Fixture) -> Vec<String> {
        let db = fixture.db();
        let mut statement = db
            .prepare("SELECT guid FROM message ORDER BY ROWID")
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn only_rows_the_run_created_are_deleted() {
        let fixture = Fixture::new("owned");
        fixture.message(5, "older-tagged", "[ab-test r1] from before the run", 1);
        fixture.message(6, "person", "a real message", 1);
        fixture.message(11, "mine", "[ab-test r1] mine", 1);
        fixture.message(12, "other-run", "[ab-test r2] someone else's run", 1);
        fixture.message(13, "friend-tagged", "[ab-test r1] in a friend's chat", 2);
        fixture.message(14, "group-tagged", "[ab-test r1] in a group", 3);
        fixture.message(15, "fresh-untagged", "written by hand while the run ran", 1);
        fixture.ledger("r1", &["mine"], &json!({}));
        let screen = Recorder::new(&fixture.config.chat_db);
        let report = run(&screen, &fixture.config, own(), "[ab-test r1]").unwrap();
        assert_eq!(report["passed"], true, "{report}");
        assert_eq!(*screen.deleted.borrow(), ["mine"]);
        let left = live(&fixture);
        for protected in [
            "older-tagged",
            "person",
            "friend-tagged",
            "group-tagged",
            "fresh-untagged",
            "other-run",
        ] {
            assert!(
                left.iter().any(|guid| guid == protected),
                "{protected} must stay"
            );
        }
    }

    #[test]
    fn a_clean_run_removes_its_rows_and_passes() {
        let fixture = Fixture::new("clean");
        fixture.message(5, "person", "a real message", 1);
        fixture.message(11, "mine", "[ab-test r1] mine", 1);
        fixture.message(12, "echo", "[ab-test r1] late echo of my send", 1);
        fixture.ledger("r1", &["mine"], &json!({}));
        let screen = Recorder::new(&fixture.config.chat_db);
        let report = run(&screen, &fixture.config, own(), "[ab-test r1]").unwrap();
        assert_eq!(report["passed"], true, "{report}");
        assert_eq!(live(&fixture), ["person"]);
    }

    #[test]
    fn a_row_recorded_in_the_ledger_but_older_than_the_baseline_stays() {
        let fixture = Fixture::new("older");
        fixture.message(5, "old", "[ab-test r1] old", 1);
        fixture.ledger("r1", &["old"], &json!({}));
        let screen = Recorder::new(&fixture.config.chat_db);
        let report = run(&screen, &fixture.config, own(), "[ab-test r1]").unwrap();
        assert!(screen.deleted.borrow().is_empty());
        assert_eq!(report["deleted"], 0);
        assert_eq!(live(&fixture), ["old"]);
    }

    #[test]
    fn a_recorded_row_in_someone_elses_chat_stays() {
        let fixture = Fixture::new("friend");
        fixture.message(11, "mine", "[ab-test r1] sent to a friend", 2);
        fixture.ledger("r1", &["mine"], &json!({}));
        let screen = Recorder::new(&fixture.config.chat_db);
        run(&screen, &fixture.config, own(), "[ab-test r1]").unwrap();
        assert!(screen.deleted.borrow().is_empty());
        assert_eq!(live(&fixture), ["mine"]);
    }

    #[test]
    fn a_recorded_row_whose_text_lost_its_tag_stops_the_run() {
        let fixture = Fixture::new("untagged");
        fixture.message(11, "mine", "plain text", 1);
        fixture.ledger("r1", &["mine"], &json!({}));
        let screen = Recorder::new(&fixture.config.chat_db);
        assert_eq!(
            run(&screen, &fixture.config, own(), "[ab-test r1]"),
            Err("conflict".to_owned())
        );
        assert!(screen.deleted.borrow().is_empty());
    }

    #[test]
    fn no_ledger_and_loose_tags_delete_nothing() {
        let fixture = Fixture::new("noledger");
        fixture.message(11, "mine", "[ab-test r1] mine", 1);
        let screen = Recorder::new(&fixture.config.chat_db);
        assert!(
            run(&screen, &fixture.config, own(), "[ab-test r1]").is_err(),
            "no ledger"
        );
        fixture.ledger("r1", &["mine"], &json!({}));
        assert!(run(&screen, &fixture.config, own(), "r1").is_err());
        assert!(run(&screen, &fixture.config, own(), "[ab-test r1] extra").is_err());
        assert!(run(&screen, &fixture.config, own(), "").is_err());
        assert!(screen.deleted.borrow().is_empty());
    }

    #[test]
    fn a_ledger_of_another_run_authorises_nothing() {
        let fixture = Fixture::new("otherledger");
        fixture.message(11, "mine", "[ab-test r1] mine", 1);
        let mut registration = json!({"run_id": "someone", "baseline_max_row_id": BASELINE, "created_message_ids": ["mine"]});
        registration["scenario"] = json!("MSG-SELFTEST");
        fs::write(
            fixture
                .config
                .reports
                .join("messages-command-cleanup/r1.json"),
            registration.to_string(),
        )
        .unwrap();
        let screen = Recorder::new(&fixture.config.chat_db);
        assert!(run(&screen, &fixture.config, own(), "[ab-test r1]").is_err());
        assert!(screen.deleted.borrow().is_empty());
    }

    #[test]
    fn a_blank_row_the_run_never_recorded_stops_nothing_and_stays() {
        let fixture = Fixture::new("blank");
        fixture.message(11, "unknown", "", 1);
        fixture.message(12, "mine", "[ab-test r1] mine", 1);
        fixture.ledger("r1", &["mine"], &json!({}));
        let screen = Recorder::new(&fixture.config.chat_db);
        let report = run(&screen, &fixture.config, own(), "[ab-test r1]").unwrap();
        assert_eq!(report["passed"], true, "{report}");
        assert_eq!(live(&fixture), ["unknown"]);
    }

    #[test]
    fn a_screen_that_cannot_verify_the_target_deletes_nothing_and_reports_it() {
        let fixture = Fixture::new("validate");
        fixture.message(11, "mine", "[ab-test r1] mine", 1);
        fixture.ledger("r1", &["mine"], &json!({}));
        let screen = Recorder::new(&fixture.config.chat_db);
        *screen.validate.borrow_mut() = Some(Failure::TargetAmbiguous);
        let report = run(&screen, &fixture.config, own(), "[ab-test r1]").unwrap();
        assert_eq!(report["passed"], false);
        assert_eq!(report["failure"], "validate:targetAmbiguous");
        assert_eq!(live(&fixture), ["mine"]);
    }

    #[test]
    fn media_is_removed_only_when_every_attachment_is_named_by_the_run_and_was_not_there_before() {
        let attach = |fixture: &Fixture, row: i64, id: i64, name: &str| {
            let db = fixture.db();
            db.execute(
                "INSERT INTO attachment(ROWID,guid,filename,transfer_name) VALUES (?,?,?,?)",
                params![id, format!("a{id}"), format!("/x/{name}"), name],
            )
            .unwrap();
            db.execute(
                "INSERT INTO message_attachment_join VALUES (?,?)",
                params![row, id],
            )
            .unwrap();
        };
        let name = "ab-r1.txt";
        let fixture = Fixture::new("media");
        fixture.message(11, "file", "", 1);
        attach(&fixture, 11, 1, name);
        fixture.message(12, "other-file", "", 1);
        attach(&fixture, 12, 2, "holiday.jpg");
        fixture.ledger("r1", &[], &json!({"media_attachment_names": [name]}));
        let screen = Recorder::new(&fixture.config.chat_db);
        let report = run(&screen, &fixture.config, own(), "[ab-test r1]").unwrap();
        assert_eq!(report["passed"], true, "{report}");
        assert_eq!(live(&fixture), ["other-file"]);

        let fixture = Fixture::new("media-old");
        fixture.message(5, "old-file", "", 1);
        attach(&fixture, 5, 1, name);
        fixture.message(11, "file", "", 1);
        attach(&fixture, 11, 2, name);
        fixture.ledger("r1", &[], &json!({"media_attachment_names": [name]}));
        let screen = Recorder::new(&fixture.config.chat_db);
        assert_eq!(
            run(&screen, &fixture.config, own(), "[ab-test r1]"),
            Err("conflict".to_owned()),
            "the name existed before the run, so it proves nothing"
        );
        assert!(screen.deleted.borrow().is_empty());
    }
}
