use super::messages::route_proof;
use super::messages_ui;
use super::owned::{Probe, text};
use crate::messages_policy::{
    IsolationChat, account_handles, is_self_chat, isolated_self_chat, normalize,
    qualifies_fixture_chat, ui_allowed,
};
use ab_native::messages::{Reader, facts};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const UNAVAILABLE: &str = "unavailable";
const INVALID: &str = "invalid";
const UNREAD_REASON: &str = "The self conversation contains unread existing messages. This check cannot open it without changing their read state. Read those messages in Messages before rerunning the check.";
const BASELINE_REASON: &str = "The self conversation baseline could not be verified. Keep Messages available and rerun the check.";
const TEST_CONTACT_KEYS: [&str; 2] = ["REMY_IMESSAGE_TEST_HANDLE", "REMY_IMESSAGE_TEST_EMAIL"];

#[derive(Clone, Debug)]
pub(super) struct Config {
    pub lab: bool,
    pub chat_db: PathBuf,
    pub home: PathBuf,
    pub reports: PathBuf,
    pub os_build: String,
    pub agent_policy: Option<PathBuf>,
}

impl Config {
    fn parse(answer: &Value) -> Option<Self> {
        let path = |key: &str| answer[key].as_str().filter(|value| !value.is_empty());
        Some(Self {
            lab: answer["lab_commands"] == true,
            chat_db: path("chat_db")?.into(),
            home: path("home")?.into(),
            reports: path("report_dir")?.into(),
            os_build: path("os_build")?.to_owned(),
            agent_policy: path("agent_policy").map(PathBuf::from),
        })
    }
}

struct Baseline {
    chat: String,
    through: i64,
    messages: String,
    preferences: String,
    pins: Vec<String>,
}

pub(super) struct Local<'a> {
    next: &'a dyn Probe,
    config: Option<Config>,
    baselines: RefCell<BTreeMap<String, Baseline>>,
    rows: RefCell<BTreeMap<String, i64>>,
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn safe_run(run: &str) -> bool {
    !run.is_empty()
        && run
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |_| UNAVAILABLE.to_owned();
    let parent = path.parent().ok_or(UNAVAILABLE)?;
    fs::create_dir_all(parent).map_err(fail)?;
    let temporary = parent.join(format!(
        ".{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("report"),
        std::process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    fs::rename(&temporary, path).map_err(fail)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

impl<'a> Local<'a> {
    pub(super) fn over(next: &'a dyn Probe) -> Self {
        let config = next
            .call("messages.config", &json!({}))
            .ok()
            .and_then(|answer| Config::parse(&answer));
        Self::with(next, config)
    }

    pub(super) fn with(next: &'a dyn Probe, config: Option<Config>) -> Self {
        Self {
            next,
            config,
            baselines: RefCell::new(BTreeMap::new()),
            rows: RefCell::new(BTreeMap::new()),
        }
    }

    fn reader(config: &Config) -> Result<Reader, String> {
        Reader::open(&config.chat_db, &config.home).map_err(|_| UNAVAILABLE.to_owned())
    }

    fn proofs_path(config: &Config) -> PathBuf {
        config.reports.join("messages-command-proofs.json")
    }

    fn ledger_path(config: &Config, run: &str) -> PathBuf {
        config
            .reports
            .join("messages-command-cleanup")
            .join(format!("{run}.json"))
    }

    fn report_path(config: &Config) -> PathBuf {
        config.reports.join("messages-selftest.json")
    }

    fn uploads(config: &Config) -> PathBuf {
        config.reports.join("messages-selftest/message-uploads")
    }

    fn proofs(config: &Config) -> BTreeSet<String> {
        read_json(&Self::proofs_path(config))
            .filter(|proofs| text(proofs, "os_build") == config.os_build)
            .map(|proofs| strings(&proofs["routes"]).into_iter().collect())
            .unwrap_or_default()
    }

    fn save_proofs(config: &Config, routes: &BTreeSet<String>) -> Result<(), String> {
        let body = json!({"os_build": config.os_build, "routes": routes});
        write_private(&Self::proofs_path(config), body.to_string().as_bytes())
    }

    fn account(config: &Config) -> Result<Vec<String>, String> {
        let reader = Self::reader(config)?;
        let raw = reader
            .account_handles()
            .map_err(|_| UNAVAILABLE.to_owned())?;
        Ok(account_handles(&raw))
    }

    fn own_handles(&self, config: &Config) -> Option<Vec<String>> {
        let card = self
            .next
            .call("messages.me_card", &json!({}))
            .ok()
            .map(|answer| strings(&answer["handles"]))
            .unwrap_or_default();
        let reader = Self::reader(config).ok()?;
        let recorded = reader.account_handles().ok()?;
        let handles: BTreeSet<String> = card
            .iter()
            .chain(&recorded)
            .map(|handle| normalize(handle))
            .filter(|handle| !handle.is_empty())
            .collect();
        (!handles.is_empty()).then(|| handles.into_iter().collect())
    }

    fn approved(&self, config: &Config) -> Result<Vec<String>, String> {
        let mut handles: BTreeSet<String> = self
            .own_handles(config)
            .ok_or(UNAVAILABLE)?
            .into_iter()
            .collect();
        if config.lab {
            let file = config.home.join(".config/apple-bridge/test-contacts.env");
            if let Ok(contents) = fs::read_to_string(file) {
                for line in contents.lines() {
                    let line = line.replace("export ", "");
                    let Some((key, value)) = line.split_once('=') else {
                        continue;
                    };
                    if TEST_CONTACT_KEYS.contains(&key.trim()) {
                        handles.insert(normalize(value.trim_matches([' ', '"', '\''])));
                    }
                }
            }
        }
        Ok(handles.into_iter().collect())
    }

    fn stage(config: &Config, args: &Value) -> Result<Value, String> {
        let digest = sha(text(args, "command_id").as_bytes());
        let directory = config
            .home
            .join("Library/Messages/Attachments/AppleBridgeOutgoing")
            .join(digest);
        let messages = &args["messages"];
        let mut parts = Vec::new();
        if !text(messages, "text").is_empty() {
            parts.push(json!({"kind": "text", "text": messages["text"]}));
        }
        let files = messages["files"].as_array().cloned().unwrap_or_default();
        if !files.is_empty() {
            fs::create_dir_all(&directory).map_err(|_| INVALID.to_owned())?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| INVALID.to_owned())?;
        }
        for (index, file) in files.iter().enumerate() {
            let name = text(file, "name");
            let digest = text(file, "digest");
            if !plain_name(name) || !text(file, "data").is_empty() {
                return Err(INVALID.into());
            }
            let source = confined_upload(text(file, "path")).ok_or(INVALID)?;
            let staged = directory.join(format!("{index}-{name}"));
            copy_new(&source, &staged).map_err(|_| INVALID.to_owned())?;
            if file_digest(&staged).is_none_or(|found| found != digest) {
                let _ = fs::remove_file(&staged);
                return Err(INVALID.into());
            }
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o600))
                .map_err(|_| INVALID.to_owned())?;
            parts.push(json!({
                "kind": "attachment",
                "path": staged.to_string_lossy(),
                "name": staged.file_name().and_then(|n| n.to_str()),
                "digest": digest,
            }));
        }
        Ok(json!({"parts": parts}))
    }

    fn selftest_facts(&self, config: &Config, args: &Value) -> Result<Value, String> {
        let state = self.next.call("messages.ax_state", &json!({}))?;
        let allowed = ui_allowed(
            args["include_ui"] != false,
            state["trusted"] == true,
            state["screen_locked"] == true,
        );
        let reader = Self::reader(config)?;
        let isolation = reader
            .isolation_facts()
            .map_err(|_| UNAVAILABLE.to_owned())?;
        let own = account_handles(&strings(&isolation["account_handles"]));
        let participants = strings(&isolation["participants"]);
        let pins = facts::pins(&config.home).ok_or(UNAVAILABLE)?;
        let chats: Vec<IsolationChat<'_>> = isolation["chats"]
            .as_array()
            .map(|chats| {
                chats
                    .iter()
                    .map(|chat| IsolationChat {
                        guid: text(chat, "guid"),
                        identifier: text(chat, "identifier"),
                        peers: strings(&chat["peers"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let target = isolated_self_chat(&own, &participants, &pins, &chats)
            .map(|(handle, chat)| json!({"handle": handle, "chat": chat}));
        Ok(json!({"ui_allowed": allowed, "target": target}))
    }

    fn capture_baseline(config: &Config, reader: &Reader, chat: &str) -> Result<Baseline, String> {
        let baseline = || BASELINE_REASON.to_owned();
        let own = Self::account(config).map_err(|_| baseline())?;
        let peers = reader
            .fact("messages.chat_members", &json!({"chat_guid": chat}))
            .ok()
            .flatten()
            .ok_or_else(baseline)?;
        if !is_self_chat(&strings(&peers["handles"]), &own) {
            return Err(baseline());
        }
        if reader.has_unread(chat, None).map_err(|_| baseline())? {
            return Err(UNREAD_REASON.to_owned());
        }
        let through = reader.max_row().map_err(|_| baseline())?;
        Ok(Baseline {
            chat: chat.to_owned(),
            through,
            messages: reader
                .content_digest(chat, through)
                .map_err(|_| baseline())?,
            preferences: reader
                .preferences_digest(chat)
                .map_err(|_| baseline())?
                .ok_or_else(baseline)?,
            pins: facts::pins(&config.home).ok_or_else(baseline)?,
        })
    }

    fn selftest_begin(&self, config: &Config, args: &Value) -> Result<Value, String> {
        let run = text(args, "run");
        let chat = text(args, "chat");
        let reader = Self::reader(config)?;
        let baseline = if chat.is_empty() {
            None
        } else {
            match Self::capture_baseline(config, &reader, chat) {
                Ok(baseline) => Some(baseline),
                Err(reason) => return Ok(json!({"ok": false, "reason": reason})),
            }
        };
        let chat_row = reader.max_chat_row().map_err(|_| UNAVAILABLE.to_owned())?;
        let message_row = reader.max_row().map_err(|_| UNAVAILABLE.to_owned())?;
        let registration = json!({
            "scenario": "MSG-SELFTEST",
            "run_id": run,
            "baseline_max_row_id": message_row,
            "media_attachment_names": [text(args, "filename")],
            "cleanup_state": "pending_native_cleanup",
        });
        Self::save_ledger(config, run, &registration)?;
        if let Some(baseline) = baseline {
            self.baselines.borrow_mut().insert(run.to_owned(), baseline);
        }
        self.rows.borrow_mut().insert(run.to_owned(), message_row);
        Ok(json!({"ok": true, "baseline_chat": chat_row, "baseline_row": message_row}))
    }

    fn save_ledger(config: &Config, run: &str, registration: &Value) -> Result<(), String> {
        if !run.starts_with("selftest_") || !safe_run(run) {
            return Err(INVALID.into());
        }
        let bytes = serde_json::to_vec_pretty(registration).map_err(|_| UNAVAILABLE.to_owned())?;
        write_private(&Self::ledger_path(config, run), &bytes)
    }

    fn load_ledger(config: &Config, run: &str) -> Result<Map<String, Value>, String> {
        if !run.starts_with("selftest_") || !safe_run(run) {
            return Err(INVALID.into());
        }
        Ok(read_json(&Self::ledger_path(config, run))
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default())
    }

    fn selftest_created(&self, config: &Config, args: &Value) -> Result<Value, String> {
        let run = text(args, "run");
        let mut registration = Self::load_ledger(config, run)?;
        let baseline = self.rows.borrow().get(run).copied().unwrap_or(0);
        let mut created: BTreeSet<String> = registration
            .get("created_message_ids")
            .map(strings)
            .unwrap_or_default()
            .into_iter()
            .collect();
        created.extend(
            strings(&args["keys"])
                .into_iter()
                .filter(|key| !key.is_empty()),
        );
        let reader = Self::reader(config)?;
        created.extend(
            reader
                .created_rows(baseline, text(args, "tag"), text(args, "filename"))
                .map_err(|_| UNAVAILABLE.to_owned())?,
        );
        registration.insert("created_message_ids".into(), json!(created));
        Self::save_ledger(config, run, &Value::Object(registration))?;
        Ok(json!({}))
    }

    fn selftest_fixture_chat(config: &Config, args: &Value) -> Result<Value, String> {
        let reader = Self::reader(config)?;
        let found = reader
            .chat_of_message(text(args, "target"))
            .ok()
            .flatten()
            .filter(|(chat, row)| {
                qualifies_fixture_chat(
                    chat,
                    *row,
                    args["baseline_chat"].as_i64().unwrap_or(i64::MAX),
                    text(args, "existing_chat"),
                )
            });
        Ok(match found {
            Some((chat, _)) => json!({"qualifies": true, "chat": chat}),
            None => json!({"qualifies": false}),
        })
    }

    fn selftest_restore(&self, config: &Config, args: &Value) -> Value {
        let baselines = self.baselines.borrow();
        let Some(baseline) = baselines.get(text(args, "run")) else {
            return json!({"restored": true});
        };
        let restored = Self::reader(config).is_ok_and(|reader| {
            reader
                .content_digest(&baseline.chat, baseline.through)
                .is_ok_and(|digest| digest == baseline.messages)
                && reader
                    .preferences_digest(&baseline.chat)
                    .is_ok_and(|digest| digest.as_ref() == Some(&baseline.preferences))
                && facts::pins(&config.home).as_deref() == Some(baseline.pins.as_slice())
        });
        json!({"restored": restored})
    }

    fn answer(
        &self,
        config: &Config,
        operation: &str,
        args: &Value,
    ) -> Option<Result<Value, String>> {
        Some(match operation {
            "messages.own_handles" => Ok(json!({"handles": self.own_handles(config)})),
            "messages.approved_handles" => self
                .approved(config)
                .map(|handles| json!({"handles": handles})),
            "messages.route_proofs" => Ok(json!({"routes": Self::proofs(config)})),
            "messages.record_proof" => {
                if !config.lab {
                    return Some(Err("unsupported".into()));
                }
                let mut routes = Self::proofs(config);
                for part in args["parts"].as_array().into_iter().flatten() {
                    routes.insert(route_proof(&args["route"], text(part, "kind")));
                }
                Self::save_proofs(config, &routes).map(|()| json!({}))
            }
            "messages.invalidate_proof" => {
                let kind = text(args, "kind");
                let suffixes = if kind == "messages.send" {
                    vec![":text".to_owned(), ":attachment".to_owned()]
                } else {
                    vec![format!(":{kind}")]
                };
                let routes: BTreeSet<String> = Self::proofs(config)
                    .into_iter()
                    .filter(|route| {
                        !suffixes
                            .iter()
                            .any(|suffix| route.ends_with(suffix.as_str()))
                    })
                    .collect();
                Self::save_proofs(config, &routes).map(|()| json!({}))
            }
            "messages.agent_policy" => agent_policy(config),
            "messages.mark_policy_used" => mark_policy_used(config, text(args, "command_id")),
            "messages.ledger" => {
                let run = text(args, "run");
                if !safe_run(run) {
                    return Some(Err(INVALID.into()));
                }
                read_json(&Self::ledger_path(config, run)).ok_or_else(|| UNAVAILABLE.to_owned())
            }
            "messages.is_self_chat" => Self::account(config).map(
                |own| json!({"self_chat": is_self_chat(&strings(&args["participants"]), &own)}),
            ),
            "messages.payload_fingerprint" => {
                Ok(json!({"digest": sha(args["messages"].to_string().as_bytes())}))
            }
            "messages.stage" => Ok(Self::stage(config, args)
                .unwrap_or_else(|_| json!({"failed": {"domain": "staging", "code": 1}}))),
            "messages.selftest_cached" => {
                let report = read_json(&Self::report_path(config))
                    .filter(|report| text(report, "os_build") == text(args, "os_build"));
                Ok(report.map_or_else(|| json!({}), |report| json!({"report": report})))
            }
            "messages.selftest_report" => {
                let bytes = args["report"].to_string();
                write_private(&Self::report_path(config), bytes.as_bytes()).map(|()| json!({}))
            }
            "messages.selftest_facts" => self.selftest_facts(config, args),
            "messages.selftest_begin" => self.selftest_begin(config, args),
            "messages.selftest_ledger" => {
                let run = text(args, "run");
                Self::load_ledger(config, run).and_then(|mut registration| {
                    for (key, value) in args["set"].as_object().into_iter().flatten() {
                        registration.insert(key.clone(), value.clone());
                    }
                    Self::save_ledger(config, run, &Value::Object(registration)).map(|()| json!({}))
                })
            }
            "messages.selftest_created" => self.selftest_created(config, args),
            "messages.selftest_fixture_chat" => Self::selftest_fixture_chat(config, args),
            "messages.selftest_upload" => selftest_upload(config, args),
            "messages.selftest_upload_remove" => selftest_upload_remove(config, args),
            "messages.selftest_restore" => Ok(self.selftest_restore(config, args)),
            "messages.ui_available"
            | "messages.ui_prepare"
            | "messages.ui_validate"
            | "messages.ui_activate_validate"
            | "messages.ui_perform" => messages_ui::answer(self.next, config, operation, args),
            "messages.selftest_cleanup" => {
                super::messages_cleanup::selftest(self.next, self, config, text(args, "tag"))
            }
            _ if is_chat_fact(operation) => Self::reader(config).and_then(|reader| {
                reader
                    .fact(operation, args)
                    .map_err(|_| UNAVAILABLE.to_owned())?
                    .ok_or_else(|| "unsupported".to_owned())
            }),
            _ => return None,
        })
    }

    pub(super) fn cleanup_report(&self, next: &dyn Probe, tag: &str) -> Result<Value, String> {
        let config = self.config.as_ref().ok_or("unsupported")?;
        super::messages_cleanup::report(next, self, config, tag)
    }
}

fn is_chat_fact(operation: &str) -> bool {
    matches!(
        operation,
        "messages.chat_members"
            | "messages.direct_chats"
            | "messages.message"
            | "messages.max_row_id"
            | "messages.rows_after"
            | "messages.target"
            | "messages.reactions"
            | "messages.reaction_rows"
            | "messages.replies"
            | "messages.conversation"
            | "messages.conversation_messages"
            | "messages.chat_recoverable"
            | "messages.foreign_activity"
    )
}

impl Probe for Local<'_> {
    fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
        if let Some(config) = &self.config
            && let Some(answer) = self.answer(config, operation, args)
        {
            return answer;
        }
        self.next.call(operation, args)
    }
}

fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

fn confined_upload(path: &str) -> Option<PathBuf> {
    let source = fs::canonicalize(path).ok()?;
    let parent = source.parent()?;
    if parent.file_name()?.to_str()? != "message-uploads" {
        return None;
    }
    let uid = ab_sys::user_id();
    let directory = fs::metadata(parent).ok()?;
    let file = fs::symlink_metadata(&source).ok()?;
    (directory.mode() & 0o777 == 0o700
        && directory.uid() == uid
        && file.uid() == uid
        && file.file_type().is_file())
    .then_some(source)
}

fn copy_new(source: &Path, target: &Path) -> std::io::Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()
}

fn file_digest(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Some(format!("{:x}", digest.finalize()))
}

fn agent_policy(config: &Config) -> Result<Value, String> {
    let Some(path) = config.agent_policy.as_ref().filter(|path| path.exists()) else {
        return Ok(json!({"configured": false}));
    };
    let policy = read_json(path).ok_or(UNAVAILABLE)?;
    let used = PathBuf::from(format!("{}.used", path.display())).exists();
    Ok(json!({"configured": true, "policy": policy, "used": used}))
}

fn mark_policy_used(config: &Config, command: &str) -> Result<Value, String> {
    let path = config
        .agent_policy
        .as_ref()
        .filter(|path| path.exists())
        .ok_or("conflict")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(format!("{}.used", path.display()))
        .map_err(|_| UNAVAILABLE.to_owned())?;
    file.write_all(command.as_bytes())
        .map_err(|_| UNAVAILABLE.to_owned())?;
    Ok(json!({}))
}

fn selftest_upload(config: &Config, args: &Value) -> Result<Value, String> {
    let name = text(args, "name");
    if !plain_name(name) {
        return Err(INVALID.into());
    }
    let directory = Local::uploads(config);
    fs::create_dir_all(&directory).map_err(|_| UNAVAILABLE.to_owned())?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
        .map_err(|_| UNAVAILABLE.to_owned())?;
    let bytes = text(args, "text").as_bytes();
    let path = directory.join(name);
    write_private(&path, bytes)?;
    Ok(json!({"path": path.to_string_lossy(), "digest": sha(bytes)}))
}

fn selftest_upload_remove(config: &Config, args: &Value) -> Result<Value, String> {
    let uploads = fs::canonicalize(Local::uploads(config)).map_err(|_| INVALID.to_owned())?;
    let file = fs::canonicalize(text(args, "path")).map_err(|_| INVALID.to_owned())?;
    if file.parent() != Some(uploads.as_path()) {
        return Err(INVALID.into());
    }
    let _ = fs::remove_file(file);
    Ok(json!({}))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::os::unix::fs::symlink;

    struct NoLink;

    impl Probe for NoLink {
        fn call(&self, operation: &str, _: &Value) -> Result<Value, String> {
            match operation {
                "messages.me_card" => Ok(json!({"handles": ["Card@Example.com"]})),
                "messages.ax_state" => Ok(json!({"trusted": true, "screen_locked": false})),
                "messages.config" => Ok(json!({"lab_commands": true})),
                _ => Err("unsupported".into()),
            }
        }
    }

    struct Fixture {
        directory: PathBuf,
        config: Config,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn fixture(name: &str) -> Fixture {
        let directory = std::env::temp_dir().join(format!(
            "ab-local-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory_ready =
            fs::create_dir_all(directory.join("reports/messages-command-cleanup"));
        directory_ready.unwrap();
        let directory = fs::canonicalize(directory).unwrap();
        fs::create_dir_all(directory.join("Library/Preferences")).unwrap();
        let chat_db = directory.join("chat.db");
        Connection::open(&chat_db)
            .unwrap()
            .execute_batch("
                CREATE TABLE message(ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB, is_from_me INTEGER DEFAULT 1, is_read INTEGER DEFAULT 1, is_delivered INTEGER DEFAULT 0, service TEXT DEFAULT 'iMessage', item_type INTEGER DEFAULT 0, associated_message_type INTEGER DEFAULT 0, associated_message_guid TEXT, associated_message_emoji TEXT, date INTEGER DEFAULT 1, date_read INTEGER DEFAULT 0, error INTEGER DEFAULT 0, handle_id INTEGER DEFAULT 0, other_handle INTEGER DEFAULT 0, destination_caller_id TEXT);
                CREATE TABLE chat(ROWID INTEGER PRIMARY KEY, guid TEXT, chat_identifier TEXT, service_name TEXT DEFAULT 'iMessage', properties BLOB, last_read_message_timestamp INTEGER DEFAULT 0);
                CREATE TABLE handle(ROWID INTEGER PRIMARY KEY, id TEXT, service TEXT DEFAULT 'iMessage');
                CREATE TABLE chat_handle_join(chat_id INTEGER, handle_id INTEGER);
                CREATE TABLE chat_message_join(chat_id INTEGER, message_id INTEGER);
                CREATE TABLE message_attachment_join(message_id INTEGER, attachment_id INTEGER);
                CREATE TABLE attachment(ROWID INTEGER PRIMARY KEY, guid TEXT, filename TEXT, transfer_name TEXT);
                CREATE TABLE chat_recoverable_message_join(chat_id INTEGER, message_id INTEGER, delete_date INTEGER);
                CREATE TABLE recoverable_message_part(chat_id INTEGER, message_id INTEGER, part_index INTEGER, delete_date INTEGER, part_text BLOB);
                INSERT INTO handle(ROWID,id) VALUES (1,'me@example.com'),(2,'friend@example.com');
                INSERT INTO chat(ROWID,guid,chat_identifier) VALUES (1,'self','me@example.com'),(2,'friend','friend@example.com');
                INSERT INTO chat_handle_join VALUES (1,1),(2,2);
                INSERT INTO message(ROWID,guid,text,destination_caller_id) VALUES (1,'m1','hello','me@example.com'),(2,'m2','hi friend','me@example.com');
                INSERT INTO chat_message_join VALUES (1,1),(2,2);
            ")
            .unwrap();
        let config = Config {
            lab: true,
            chat_db,
            home: directory.clone(),
            reports: directory.join("reports"),
            os_build: "build-1".into(),
            agent_policy: None,
        };
        Fixture { directory, config }
    }

    fn write_pins(fixture: &Fixture, pins: &[&str]) {
        let root = plist::Value::Dictionary(
            [(
                "pD".to_owned(),
                plist::Value::Dictionary(
                    [(
                        "pP".to_owned(),
                        plist::Value::Array(
                            pins.iter()
                                .map(|pin| plist::Value::String((*pin).to_owned()))
                                .collect(),
                        ),
                    )]
                    .into_iter()
                    .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        );
        root.to_file_xml(
            fixture
                .directory
                .join("Library/Preferences/com.apple.messages.pinning.plist"),
        )
        .unwrap();
    }

    fn upload(fixture: &Fixture, directory: &str, mode: u32) -> (PathBuf, String) {
        let parent = fixture.directory.join(directory);
        fs::create_dir_all(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(mode)).unwrap();
        let path = parent.join("note.txt");
        fs::write(&path, b"bytes").unwrap();
        (path, sha(b"bytes"))
    }

    fn stage(fixture: &Fixture, path: &Path, digest: &str, name: &str) -> Value {
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        fs::create_dir_all(fixture.directory.join("Library/Messages/Attachments")).unwrap();
        local
            .call(
                "messages.stage",
                &json!({"command_id": "c1", "messages": {"files": [{"name": name, "path": path, "digest": digest}]}}),
            )
            .unwrap()
    }

    #[test]
    fn a_peer_without_a_chat_db_answers_everything_itself() {
        let local = Local::over(&NoLink);
        assert_eq!(
            local.call("messages.chat_members", &json!({"chat_guid": "self"})),
            Err("unsupported".to_owned())
        );
    }

    #[test]
    fn staging_copies_only_a_private_upload_of_this_user() {
        let fixture = fixture("stage");
        let (good, digest) = upload(&fixture, "message-uploads", 0o700);
        let staged = stage(&fixture, &good, &digest, "note.txt");
        assert_eq!(staged["parts"][0]["kind"], "attachment", "{staged}");
        let copied = staged["parts"][0]["path"].as_str().unwrap().to_owned();
        assert_eq!(fs::read(&copied).unwrap(), b"bytes");
        assert!(copied.ends_with("/0-note.txt"));
        assert_eq!(fs::metadata(&copied).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn staging_refuses_everything_else() {
        let fixture = fixture("refuse");
        let (good, digest) = upload(&fixture, "message-uploads", 0o700);
        let refused = |staged: Value| staged["failed"].is_object();
        assert!(
            refused(stage(&fixture, &good, "wrong-digest", "a.txt")),
            "digest"
        );
        assert!(
            refused(stage(&fixture, &good, &digest, "../a.txt")),
            "name escapes"
        );
        assert!(refused(stage(&fixture, &good, &digest, "")), "no name");
        let (other, digest_other) = upload(&fixture, "other-directory", 0o700);
        assert!(
            refused(stage(&fixture, &other, &digest_other, "b.txt")),
            "wrong directory"
        );
        let open_fixture = self::fixture("open");
        let (open, digest_open) = upload(&open_fixture, "message-uploads", 0o755);
        assert!(
            refused(stage(&open_fixture, &open, &digest_open, "c.txt")),
            "not private"
        );
        let link_fixture = self::fixture("link");
        let (_, digest_link) = upload(&link_fixture, "message-uploads", 0o700);
        let secret = link_fixture.directory.join("secret.txt");
        fs::write(&secret, b"bytes").unwrap();
        let link = link_fixture.directory.join("message-uploads/link.txt");
        symlink(&secret, &link).unwrap();
        assert!(
            refused(stage(&link_fixture, &link, &digest_link, "d.txt")),
            "symlink out"
        );
    }

    #[test]
    fn route_proofs_count_only_for_this_os_build_and_only_the_lab_records_them() {
        let mut fixture = fixture("proofs");
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        let record =
            json!({"route": {"kind": "chat_id_raw", "chat": "self"}, "parts": [{"kind": "text"}]});
        local.call("messages.record_proof", &record).unwrap();
        let routes = local.call("messages.route_proofs", &json!({})).unwrap();
        assert_eq!(routes["routes"].as_array().unwrap().len(), 1);
        local
            .call(
                "messages.invalidate_proof",
                &json!({"kind": "messages.send"}),
            )
            .unwrap();
        let routes = local.call("messages.route_proofs", &json!({})).unwrap();
        assert_eq!(routes["routes"], json!([]));
        local.call("messages.record_proof", &record).unwrap();
        fixture.config.os_build = "build-2".into();
        let upgraded = Local::with(&NoLink, Some(fixture.config.clone()));
        let routes = upgraded.call("messages.route_proofs", &json!({})).unwrap();
        assert_eq!(routes["routes"], json!([]), "a new OS needs new proof");
        fixture.config.lab = false;
        let product = Local::with(&NoLink, Some(fixture.config.clone()));
        assert_eq!(
            product.call("messages.record_proof", &record),
            Err("unsupported".to_owned())
        );
    }

    #[test]
    fn an_unreadable_agent_policy_is_a_failure_and_never_no_policy() {
        let mut fixture = fixture("policy");
        let path = fixture.directory.join("policy.json");
        fixture.config.agent_policy = Some(path.clone());
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        let answer = local.call("messages.agent_policy", &json!({})).unwrap();
        assert_eq!(answer["configured"], false);
        fs::write(&path, b"{not json").unwrap();
        assert!(local.call("messages.agent_policy", &json!({})).is_err());
        fs::write(&path, br#"{"handle":"x"}"#).unwrap();
        let answer = local.call("messages.agent_policy", &json!({})).unwrap();
        assert_eq!(answer["configured"], true);
        assert_eq!(answer["used"], false);
        local
            .call("messages.mark_policy_used", &json!({"command_id": "c1"}))
            .unwrap();
        let answer = local.call("messages.agent_policy", &json!({})).unwrap();
        assert_eq!(answer["used"], true);
        assert!(
            local
                .call("messages.mark_policy_used", &json!({"command_id": "c2"}))
                .is_err(),
            "the policy pays out once"
        );
    }

    #[test]
    fn own_handles_join_the_me_card_and_what_chat_db_records() {
        let fixture = fixture("own");
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        let own = local.call("messages.own_handles", &json!({})).unwrap();
        assert_eq!(
            own["handles"],
            json!(["card@example.com", "me@example.com"])
        );
    }

    #[test]
    fn a_conversation_reports_unread_from_its_incoming_rows() {
        let fixture = fixture("unread");
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        let unread = |local: &Local<'_>| {
            local
                .call("messages.conversation", &json!({"chat_guid": "self"}))
                .unwrap()["unread"]
                .clone()
        };
        assert_eq!(unread(&local), false);
        let db = Connection::open(&fixture.config.chat_db).unwrap();
        db.execute_batch(
            "INSERT INTO message(ROWID,guid,text,is_from_me,is_read) VALUES (3,'echo','x',0,0);
             INSERT INTO chat_message_join VALUES (1,3);",
        )
        .unwrap();
        assert_eq!(unread(&local), true, "an incoming unread row");
        db.execute("UPDATE message SET is_from_me=1 WHERE guid='echo'", [])
            .unwrap();
        assert_eq!(unread(&local), false, "my own row is not unread");
        db.execute_batch(
            "UPDATE message SET is_from_me=0, associated_message_type=3006 WHERE guid='echo'",
        )
        .unwrap();
        assert_eq!(
            unread(&local),
            true,
            "Messages marks a trailing tapback row unread too"
        );
        db.execute_batch(
            "INSERT INTO chat_recoverable_message_join VALUES (1,3,1);
             DELETE FROM chat_message_join WHERE message_id=3;",
        )
        .unwrap();
        assert_eq!(unread(&local), false, "a recoverable row is gone");
    }

    #[test]
    fn the_self_test_takes_only_the_self_chat_and_refuses_unread_messages() {
        let fixture = fixture("selftest");
        write_pins(&fixture, &[]);
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        let facts = local
            .call("messages.selftest_facts", &json!({"include_ui": true}))
            .unwrap();
        assert_eq!(facts["ui_allowed"], true);
        assert_eq!(facts["target"]["chat"], "self");
        let begin = json!({"run": "selftest_r", "chat": "self", "filename": "ab-selftest_r.txt"});
        let started = local.call("messages.selftest_begin", &begin).unwrap();
        assert_eq!(started["ok"], true, "{started}");
        let restored = local
            .call("messages.selftest_restore", &json!({"run": "selftest_r"}))
            .unwrap();
        assert_eq!(restored["restored"], true);
        let db = Connection::open(&fixture.config.chat_db).unwrap();
        db.execute("UPDATE message SET text='changed' WHERE guid='m1'", [])
            .unwrap();
        let restored = local
            .call("messages.selftest_restore", &json!({"run": "selftest_r"}))
            .unwrap();
        assert_eq!(
            restored["restored"], false,
            "a changed row is not a restored chat"
        );
        db.execute(
            "INSERT INTO message(ROWID,guid,text,is_from_me,is_read) VALUES (9,'unread','x',0,0)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO chat_message_join VALUES (1,9)", [])
            .unwrap();
        let refused = local.call("messages.selftest_begin", &begin).unwrap();
        assert_eq!(refused["ok"], false);
        assert!(text(&refused, "reason").contains("unread"));
        let friend =
            json!({"run": "selftest_s", "chat": "friend", "filename": "ab-selftest_s.txt"});
        let refused = local.call("messages.selftest_begin", &friend).unwrap();
        assert_eq!(
            refused["ok"], false,
            "someone else's chat is never a baseline"
        );
    }

    #[test]
    fn the_ledger_names_only_plain_runs() {
        let fixture = fixture("ledger");
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        assert_eq!(
            local.call("messages.ledger", &json!({"run": "../../secrets"})),
            Err("invalid".to_owned())
        );
        assert_eq!(
            local.call("messages.ledger", &json!({"run": ""})),
            Err("invalid".to_owned())
        );
        assert_eq!(
            local.call(
                "messages.selftest_ledger",
                &json!({"run": "not-a-selftest", "set": {}})
            ),
            Err("invalid".to_owned())
        );
        assert_eq!(
            local.call(
                "messages.selftest_upload",
                &json!({"name": "../x", "text": "y"})
            ),
            Err("invalid".to_owned())
        );
    }

    #[test]
    fn chat_facts_read_the_real_tables() {
        let fixture = fixture("facts");
        let local = Local::with(&NoLink, Some(fixture.config.clone()));
        let members = local
            .call("messages.chat_members", &json!({"chat_guid": "self"}))
            .unwrap();
        assert_eq!(members["joined"], json!(["me@example.com"]));
        let chats = local.call("messages.direct_chats", &json!({})).unwrap();
        assert_eq!(chats["chats"][0]["guid"], "friend", "newest first");
        let max = local.call("messages.max_row_id", &json!({})).unwrap();
        assert_eq!(max["value"], 2);
        let rows = local
            .call("messages.rows_after", &json!({"maximum": 1, "limit": 10}))
            .unwrap();
        assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
        assert_eq!(rows["rows"][0]["guid"], "m2");
        assert_eq!(rows["rows"][0]["peers"], json!(["friend@example.com"]));
        assert_eq!(rows["rows"][0]["has_body"], false);
        let target = local
            .call("messages.target", &json!({"guid": "m1"}))
            .unwrap();
        assert_eq!(target["exists"], true);
        assert_eq!(target["target"]["text"], "hello");
        assert_eq!(target["row"]["capture_chat_guid"], "self");
        let missing = local
            .call("messages.target", &json!({"guid": "nope"}))
            .unwrap();
        assert_eq!(missing["exists"], false);
    }
}
