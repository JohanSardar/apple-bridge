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
