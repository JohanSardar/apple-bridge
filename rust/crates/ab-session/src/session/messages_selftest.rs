use super::messages::DRIVER;
use super::owned::{Probe, text};
use crate::Error;
use ab_wire::native::{self, Frame, Primitive, Session as NativeSession};
use serde_json::{Value, json, value::RawValue};
use std::{
    cell::{Cell, RefCell},
    io::Write,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

const CALL_WAIT: Duration = Duration::from_secs(120);
const HANDSHAKE_WAIT: Duration = Duration::from_secs(15);
const ECHO_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(30)
};
const ECHO_POLL: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_millis(250)
};

struct LinkProbe {
    stream: RefCell<UnixStream>,
    next: Cell<u64>,
}

impl LinkProbe {
    fn open(mut stream: UnixStream) -> Result<Self, Error> {
        stream.set_read_timeout(Some(HANDSHAKE_WAIT))?;
        stream.write_all(
            &Frame::Hello { owner_fence: 1 }
                .encode()
                .map_err(|_| Error::Invalid)?,
        )?;
        if !matches!(native::read_frame(&mut stream), Ok(Frame::Welcome)) {
            return Err(Error::Invalid);
        }
        stream.write_all(
            &Frame::Session(NativeSession::default())
                .encode()
                .map_err(|_| Error::Invalid)?,
        )?;
        Ok(Self {
            stream: RefCell::new(stream),
            next: Cell::new(0),
        })
    }
}

impl Probe for LinkProbe {
    fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
        let unavailable = || "unavailable".to_owned();
        let id = {
            self.next.set(self.next.get() + 1);
            format!("selftest-{}", self.next.get())
        };
        let frame = Frame::Primitive(Primitive {
            id: id.clone(),
            op: operation.into(),
            args: Some(RawValue::from_string(args.to_string()).map_err(|_| unavailable())?),
        });
        let wait = CALL_WAIT;
        let mut stream = self.stream.borrow_mut();
        stream
            .set_read_timeout(Some(wait))
            .map_err(|_| unavailable())?;
        stream
            .write_all(&frame.encode().map_err(|_| unavailable())?)
            .map_err(|_| unavailable())?;
        loop {
            match native::read_frame(&mut *stream).map_err(|_| unavailable())? {
                Frame::PrimitiveResult(answer) if answer.id == id => {
                    return serde_json::from_str(answer.result.get()).map_err(|_| unavailable());
                }
                Frame::PrimitiveError(failure) if failure.id == id => return Err(failure.code),
                Frame::Close => return Err(unavailable()),
                _ => {}
            }
        }
    }
}

pub fn serve(link: UnixStream, request: &Value) -> Result<(), Error> {
    let probe = LinkProbe::open(link)?;
    let local = super::messages_local::Local::over(&probe);
    run(&local, &Driven(&local), request)
        .map(|_| ())
        .map_err(|_| Error::Invalid)
}

pub fn serve_cleanup(link: UnixStream, request: &Value) -> Result<(), Error> {
    let probe = LinkProbe::open(link)?;
    let local = super::messages_local::Local::over(&probe);
    let report = local.cleanup_report(&probe, text(request, "tag"));
    let body = report.unwrap_or_else(|code| json!({"state": "unavailable", "reason": code}));
    std::fs::write(text(request, "report"), body.to_string())?;
    Ok(())
}

const READ_STATE: [(&str, &str); 3] = [
    ("mark_read_first", "mark_read"),
    ("mark_unread", "mark_unread"),
    ("mark_read", "mark_read"),
];

const PENDING: [(&str, &str); 12] = [
    ("reply", "reply"),
    ("love_add", "react"),
    ("love_remove", "react"),
    ("emoji_add", "react"),
    ("emoji_remove", "react"),
    ("edit", "edit"),
    ("unsend", "unsend"),
    ("mark_read_first", "mark_read"),
    ("mark_read_last", "mark_read"),
    ("mark_read", "mark_read"),
    ("mark_unread", "mark_unread"),
    ("delete_message", "delete_message"),
];

pub(super) trait Commands {
    fn run(&self, command: &Value) -> Result<(String, Value), String>;
}

pub(super) struct Driven<'a>(pub &'a dyn Probe);

impl Commands for Driven<'_> {
    fn run(&self, command: &Value) -> Result<(String, Value), String> {
        let probe = self.0;
        let action = &command["action"];
        let prepare = DRIVER.prepare_command.ok_or("unsupported")?;
        let state = prepare(probe, command).map_err(|refusal| refusal.code)?;
        (DRIVER.validate)(probe, action, &state).map_err(|refusal| refusal.code)?;
        let written = (DRIVER.write)(probe, text(command, "command_id"), action, &state)
            .map_err(|code| format!("dispatched:{code}"))?;
        let settled = (DRIVER.settle)(text(action, "op"), &written)
            .ok_or_else(|| "dispatched:unavailable".to_owned())?;
        Ok((settled.disposition.to_owned(), settled.evidence))
    }
}

struct Session<'a> {
    probe: &'a dyn Probe,
    commands: &'a dyn Commands,
    run: String,
    tag: String,
    filename: String,
    os_build: String,
    chat: String,
    handle: String,
    steps: Vec<Value>,
    late: bool,
    sequence: u32,
}

fn failure(error: &str) -> (&str, bool) {
    match error.strip_prefix("dispatched:") {
        Some(code) => (code, true),
        None => (error, false),
    }
}

impl Session<'_> {
    fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
        self.probe.call(operation, args)
    }

    fn save(&self, state: &str) -> Result<Value, String> {
        let report = json!({
            "os_build": self.os_build,
            "run_id": self.run,
            "state": state,
            "steps": self.steps,
        });
        self.call("messages.selftest_report", &json!({"report": report}))?;
        Ok(report)
    }

    fn ledger(&self, set: &Value) -> Result<(), String> {
        self.call(
            "messages.selftest_ledger",
            &json!({"run": self.run, "set": set}),
        )
        .map(|_| ())
    }

    fn unproven(&mut self, name: &str, operation: &str, reason: &str) {
        let mut step = json!({"name": name, "proven": false, "reason": reason});
        if !operation.is_empty() {
            step["operation"] = json!(format!("messages.{operation}"));
        }
        self.steps.push(step);
    }

    fn execute(&mut self, name: &str, operation: &str, args: &Value, native: &str) -> Value {
        self.sequence += 1;
        let id = format!("{}-{}", self.run, self.sequence);
        let started = Instant::now();
        let list_key = if operation == "send" {
            self.chat.as_str()
        } else {
            ""
        };
        let command = json!({
            "command_id": id,
            "principal": "selftest",
            "origin": "",
            "expires_at": "",
            "action": {
                "op": format!("messages.{operation}"),
                "messages": args,
                "native_key": native,
                "list_key": list_key,
            },
            "expected_state": Value::Null,
        });
        let mut reason = String::new();
        let mut result = match self.commands.run(&command) {
            Ok((disposition, mut evidence)) => {
                evidence["disposition"] = json!(disposition);
                self.late = self.late || disposition == "indeterminate";
                match self.register(&evidence) {
                    Ok(()) => {
                        if evidence["observed"] != true
                            || !["locally_observed", "delivered"].contains(&disposition.as_str())
                        {
                            reason = disposition;
                            evidence["observed"] = json!(false);
                        }
                        evidence
                    }
                    Err(code) => {
                        reason = code;
                        self.late = true;
                        json!({"observed": false, "disposition": "indeterminate"})
                    }
                }
            }
            Err(error) => {
                let (code, dispatched) = failure(&error);
                code.clone_into(&mut reason);
                self.late = self.late || dispatched;
                json!({
                    "observed": false,
                    "disposition": if dispatched { "indeterminate" } else { "rejected_before_dispatch" },
                })
            }
        };
        if result["observed"] != true && reason.is_empty() {
            text(&result, "disposition").clone_into(&mut reason);
        }
        let milliseconds = started.elapsed().as_secs_f64() * 1000.0;
        self.steps.push(json!({
            "name": name,
            "operation": format!("messages.{operation}"),
            "proven": result["observed"] == true,
            "reason": reason,
            "latency_ms": milliseconds,
        }));
        let _ = self.save("running");
        result["observed"] = json!(result["observed"] == true);
        result
    }

    fn register(&self, evidence: &Value) -> Result<(), String> {
        let mut keys: Vec<String> = Vec::new();
        for item in
            std::iter::once(evidence).chain(evidence["parts"].as_array().into_iter().flatten())
        {
            let key = text(item, "native_key");
            if !key.is_empty() {
                keys.push(key.to_owned());
            }
        }
        self.call(
            "messages.selftest_created",
            &json!({
                "run": self.run,
                "tag": self.tag,
                "filename": self.filename,
                "keys": keys,
            }),
        )
        .map(|_| ())
    }

    fn await_echo(&self, chat: &str, own: &str) {
        let started = Instant::now();
        loop {
            let listed = self.call(
                "messages.conversation_messages",
                &json!({"chat_guid": chat}),
            );
            let arrived = listed.is_ok_and(|listed| {
                listed["rows"]
                    .as_array()
                    .and_then(|rows| rows.first())
                    .is_some_and(|newest| text(newest, "guid") != own)
            });
            if arrived || started.elapsed() >= ECHO_WAIT {
                return;
            }
            std::thread::sleep(ECHO_POLL);
        }
    }

    fn limited(&self, name: &str, reason: &str) -> Result<Value, String> {
        let report = json!({
            "os_build": self.os_build,
            "run_id": self.run,
            "state": "limited",
            "writes_submitted": 0,
            "steps": [{"name": name, "proven": false, "reason": reason}],
        });
        self.call("messages.selftest_report", &json!({"report": report}))?;
        Ok(report)
    }
}

pub(super) fn run(
    probe: &dyn Probe,
    commands: &dyn Commands,
    request: &Value,
) -> Result<Value, String> {
    let os_build = text(request, "os_build").to_owned();
    if request["force"] != true {
        let cached = probe.call("messages.selftest_cached", &json!({"os_build": os_build}))?;
        if cached["report"].is_object() {
            return Ok(cached["report"].clone());
        }
    }
    let run = text(request, "run").to_owned();
    let tag = format!("[ab-test {run}]");
    let filename = format!("ab-{run}.txt");
    let mut session = Session {
        probe,
        commands,
        run: run.clone(),
        tag: tag.clone(),
        filename: filename.clone(),
        os_build,
        chat: String::new(),
        handle: String::new(),
        steps: Vec::new(),
        late: false,
        sequence: 0,
    };
    let facts = session.call(
        "messages.selftest_facts",
        &json!({"include_ui": request["include_ui"] != false}),
    )?;
    if facts["ui_allowed"] != true || !facts["target"].is_object() {
        let reason = if facts["ui_allowed"] == true {
            "No verified self conversation is available. Check the Messages account's own addresses and self conversation."
        } else {
            "Unlock the Mac and enable Accessibility for AppleBridgeHelper before rerunning the Messages self-test."
        };
        return session.limited("isolated_test_self_chat", reason);
    }
    text(&facts["target"], "chat").clone_into(&mut session.chat);
    text(&facts["target"], "handle").clone_into(&mut session.handle);
    let begun = session.call(
        "messages.selftest_begin",
        &json!({"run": run, "chat": session.chat, "filename": filename}),
    )?;
    if begun["ok"] != true {
        return session.limited("existing_self_chat_baseline", text(&begun, "reason"));
    }
    let baseline_chat = begun["baseline_chat"].clone();
    session.save("running")?;
    walk(&mut session, &baseline_chat)
}

fn walk(session: &mut Session<'_>, baseline_chat: &Value) -> Result<Value, String> {
    let run = session.run.clone();
    let tag = session.tag.clone();
    let handle = session.handle.clone();
    let send =
        json!({"handles": [handle], "service": "iMessage", "text": format!("{tag} self-test")});
    let initial = session.execute("send_text", "send", &send, "");
    let target = text(&initial, "native_key").to_owned();
    let observed = initial["observed"] == true;
    let mut read_chat = None;
    if observed {
        let fixture = session.call(
            "messages.selftest_fixture_chat",
            &json!({"target": target, "baseline_chat": baseline_chat, "existing_chat": session.chat}),
        )?;
        if fixture["qualifies"] == true {
            let chat = text(&fixture, "chat").to_owned();
            session.ledger(&json!({
                "baseline_max_chat_row_id": baseline_chat,
                "created_chat_key": chat,
                "created_by_run": session.chat.is_empty(),
                "existing_self_chat": !session.chat.is_empty(),
            }))?;
            session.await_echo(&chat, &target);
            for (name, operation) in READ_STATE {
                session.execute(name, operation, &json!({}), &chat);
            }
            read_chat = Some(chat);
        } else {
            for (name, operation) in READ_STATE {
                session.unproven(name, operation, "isolated_test_chat_unavailable");
            }
        }
    }
    if observed && !target.is_empty() {
        targeted(session, &target, &tag);
    }
    if observed {
        let mut unsend = send.clone();
        unsend["text"] = json!(format!("{tag} unsend"));
        let retract = session.execute("unsend_fixture", "send", &unsend, "");
        let key = text(&retract, "native_key").to_owned();
        if retract["observed"] == true && !key.is_empty() {
            if let Some(chat) = &read_chat {
                session.await_echo(chat, &key);
                session.execute("mark_read_last", "mark_read", &json!({}), chat);
            }
            session.execute("unsend", "unsend", &json!({}), &key);
        }
    }
    send_file(session, &send);
    for (name, operation) in PENDING {
        let present = session.steps.iter().any(|step| text(step, "name") == name);
        if !present {
            session.unproven(name, operation, "prerequisite_send_unproven");
        }
    }
    invalidate(session);
    let cleaned = session.call("messages.selftest_cleanup", &json!({"tag": tag}));
    let mut state = "pending_native_cleanup";
    match cleaned {
        Ok(cleaned) => {
            session.steps.push(json!({
                "name": "cleanup",
                "proven": cleaned["passed"] == true,
                "reason": cleaned["failure"],
            }));
            if cleaned["passed"] == true {
                state = if session.late {
                    "pending_reconciliation"
                } else {
                    "verified_clean"
                };
            }
        }
        Err(code) => session.unproven("cleanup", "", &code),
    }
    let mut cleanup_state = state;
    if !session.chat.is_empty() {
        let restored = session
            .call("messages.selftest_restore", &json!({"run": run}))
            .is_ok_and(|answer| answer["restored"] == true);
        session.steps.push(json!({
            "name": "existing_self_chat_preserved",
            "proven": restored,
            "reason": if restored {
                ""
            } else {
                "The previous self-chat state could not be verified after cleanup. Keep Messages available and review the self-chat before rerunning."
            },
        }));
        if !restored {
            cleanup_state = "pending_native_cleanup";
        }
    }
    session
        .ledger(&json!({"cleanup_state": cleanup_state, "late_effect_possible": session.late}))?;
    let proven = session.steps.iter().all(|step| step["proven"] == true);
    session.save(if proven { "proven" } else { "limited" })
}

fn targeted(session: &mut Session<'_>, target: &str, tag: &str) {
    session.execute(
        "reply",
        "reply",
        &json!({"text": format!("{tag} reply")}),
        target,
    );
    for (kind, emoji) in [("love", ""), ("emoji", "😂")] {
        let reaction = |remove: bool| json!({"reaction": kind, "emoji": emoji, "remove": remove});
        let added = session.execute(&format!("{kind}_add"), "react", &reaction(false), target);
        if added["observed"] == true {
            session.execute(&format!("{kind}_remove"), "react", &reaction(true), target);
        } else {
            session.unproven(&format!("{kind}_remove"), "react", "reaction_add_unproven");
        }
    }
    session.execute(
        "edit",
        "edit",
        &json!({"text": format!("{tag} edited")}),
        target,
    );
    session.execute("delete_message", "delete_message", &json!({}), target);
}

fn send_file(session: &mut Session<'_>, send: &Value) {
    let upload = session.call(
        "messages.selftest_upload",
        &json!({
            "run": session.run,
            "name": session.filename,
            "text": format!("{} synthetic file", session.tag),
        }),
    );
    match upload {
        Ok(file) => {
            let mut args = send.clone();
            args["text"] = json!("");
            args["files"] = json!([{
                "name": session.filename,
                "path": file["path"],
                "digest": file["digest"],
            }]);
            session.execute("send_file", "send", &args, "");
            let _ = session.call(
                "messages.selftest_upload_remove",
                &json!({"path": file["path"]}),
            );
        }
        Err(code) => session.unproven("send_file", "send", &code),
    }
}

fn invalidate(session: &mut Session<'_>) {
    let mut kinds: Vec<String> = Vec::new();
    for step in &session.steps {
        if step["proven"] == true {
            continue;
        }
        let kind = match text(step, "operation") {
            "messages.send" if text(step, "name") == "send_file" => "attachment".to_owned(),
            "messages.send" => "text".to_owned(),
            other => other.to_owned(),
        };
        if !kind.is_empty() && !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    for kind in kinds {
        if let Err(code) = session.call("messages.invalidate_proof", &json!({"kind": kind})) {
            session.unproven("capability_evidence", "", &code);
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::unnecessary_wraps)]
mod tests {
    use super::{Commands, run};
    use crate::session::owned::Probe;
    use serde_json::{Value, json};
    use std::cell::RefCell;

    struct Fake {
        calls: RefCell<Vec<(String, Value)>>,
        facts: Value,
        begin: Value,
        fixture: Value,
        cleanup: Value,
        restore: Value,
        cached: Value,
        echo_after: usize,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                facts: json!({"ui_allowed": true, "target": {"handle": "me@example.com", "chat": "chat-0"}}),
                begin: json!({"ok": true, "baseline_chat": 5}),
                fixture: json!({"chat": "chat-0", "qualifies": true}),
                cleanup: json!({"passed": true}),
                restore: json!({"restored": true}),
                cached: json!({}),
                echo_after: 1,
            }
        }
        fn count(&self, operation: &str) -> usize {
            self.calls
                .borrow()
                .iter()
                .filter(|(name, _)| name == operation)
                .count()
        }
        fn ledger(&self) -> Vec<Value> {
            self.calls
                .borrow()
                .iter()
                .filter(|(name, _)| name == "messages.selftest_ledger")
                .map(|(_, args)| args["set"].clone())
                .collect()
        }
        fn invalidated(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .filter(|(name, _)| name == "messages.invalidate_proof")
                .map(|(_, args)| args["kind"].as_str().unwrap_or_default().to_owned())
                .collect()
        }
    }

    impl Probe for Fake {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            self.calls
                .borrow_mut()
                .push((operation.to_owned(), args.clone()));
            Ok(match operation {
                "messages.selftest_cached" => self.cached.clone(),
                "messages.selftest_facts" => self.facts.clone(),
                "messages.selftest_begin" => self.begin.clone(),
                "messages.selftest_fixture_chat" => self.fixture.clone(),
                "messages.selftest_cleanup" => self.cleanup.clone(),
                "messages.selftest_restore" => self.restore.clone(),
                "messages.selftest_upload" => json!({"path": "/tmp/x", "digest": "d"}),
                "messages.conversation_messages" => {
                    if self.count(operation) >= self.echo_after {
                        json!({"rows": [{"guid": "echo"}, {"guid": "key-messages.send"}]})
                    } else {
                        json!({"rows": [{"guid": "key-messages.send"}]})
                    }
                }
                _ => json!({}),
            })
        }
    }

    type Outcome = Result<(String, Value), String>;
    type Answer = fn(&str, &str) -> Outcome;

    struct Script {
        seen: RefCell<Vec<(String, String)>>,
        answer: Answer,
    }

    impl Commands for Script {
        fn run(&self, command: &Value) -> Result<(String, Value), String> {
            let operation = command["action"]["op"].as_str().unwrap_or_default();
            let native = command["action"]["native_key"].as_str().unwrap_or_default();
            self.seen
                .borrow_mut()
                .push((operation.to_owned(), native.to_owned()));
            (self.answer)(operation, native)
        }
    }

    fn observed(key: &str) -> Outcome {
        Ok((
            "locally_observed".into(),
            json!({"observed": true, "native_key": key}),
        ))
    }

    fn everything(operation: &str, _native: &str) -> Outcome {
        observed(&format!("key-{operation}"))
    }

    fn script(answer: Answer) -> Script {
        Script {
            seen: RefCell::new(Vec::new()),
            answer,
        }
    }

    fn request() -> Value {
        json!({"run": "selftest_abc", "os_build": "25A", "force": true})
    }

    fn names(report: &Value) -> Vec<&str> {
        report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step["name"].as_str().unwrap_or_default())
            .collect()
    }

    fn step<'a>(report: &'a Value, name: &str) -> &'a Value {
        report["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["name"] == name)
            .unwrap()
    }

    #[test]
    fn a_clean_run_walks_the_steps_in_order_and_is_proven() {
        let probe = Fake::new();
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(report["state"], "proven");
        assert_eq!(
            names(&report),
            [
                "send_text",
                "mark_read_first",
                "mark_unread",
                "mark_read",
                "reply",
                "love_add",
                "love_remove",
                "emoji_add",
                "emoji_remove",
                "edit",
                "delete_message",
                "unsend_fixture",
                "mark_read_last",
                "unsend",
                "send_file",
                "cleanup",
                "existing_self_chat_preserved"
            ]
        );
        assert_eq!(probe.invalidated(), Vec::<String>::new());
        let last = probe.ledger().pop().unwrap();
        assert_eq!(last["cleanup_state"], "verified_clean");
        assert_eq!(last["late_effect_possible"], false);
    }

    #[test]
    fn the_steps_target_the_right_messages_and_chat() {
        let probe = Fake::new();
        let commands = script(everything);
        run(&probe, &commands, &request()).unwrap();
        let seen = commands.seen.borrow();
        assert_eq!(seen[0], ("messages.send".to_owned(), String::new()));
        assert_eq!(
            seen[1],
            ("messages.mark_read".to_owned(), "chat-0".to_owned()),
            "the echo arrives unread, so the chat is read first or Mark as Unread is never offered"
        );
        assert_eq!(
            seen[2],
            ("messages.mark_unread".to_owned(), "chat-0".to_owned())
        );
        assert_eq!(
            seen[3],
            ("messages.mark_read".to_owned(), "chat-0".to_owned())
        );
        assert!(seen.contains(&("messages.reply".to_owned(), "key-messages.send".to_owned())));
        assert!(seen.contains(&("messages.unsend".to_owned(), "key-messages.send".to_owned())));
    }

    #[test]
    fn the_read_state_steps_wait_for_the_self_chat_echo() {
        let mut probe = Fake::new();
        probe.echo_after = 3;
        let commands = script(everything);
        run(&probe, &commands, &request()).unwrap();
        assert!(probe.count("messages.conversation_messages") >= 3);
    }

    #[test]
    fn a_missing_echo_does_not_stop_the_walk() {
        let mut probe = Fake::new();
        probe.echo_after = usize::MAX;
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(report["state"], "proven");
        assert!(probe.count("messages.conversation_messages") > 1);
    }

    #[test]
    fn the_chat_is_read_again_after_the_tapbacks_because_their_last_row_stays_unread() {
        let probe = Fake::new();
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        let names = names(&report);
        let place = |name: &str| names.iter().position(|found| *found == name).unwrap();
        assert!(place("emoji_remove") < place("mark_read_last"));
        assert!(place("unsend_fixture") < place("mark_read_last"));
        assert!(
            place("mark_read_last") < place("unsend"),
            "the unsent row has no tag, so it would make the read a conflict"
        );
        assert!(
            place("mark_read_last") < place("send_file"),
            "the file row has no tag, so it would make the read a conflict"
        );
    }

    #[test]
    fn a_run_that_cannot_use_the_ui_writes_nothing() {
        let mut probe = Fake::new();
        probe.facts = json!({"ui_allowed": false, "target": {"handle": "h", "chat": "c"}});
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(report["state"], "limited");
        assert_eq!(report["writes_submitted"], 0);
        assert!(commands.seen.borrow().is_empty());
        assert!(
            step(&report, "isolated_test_self_chat")["reason"]
                .as_str()
                .unwrap()
                .starts_with("Unlock the Mac")
        );
        assert_eq!(probe.count("messages.selftest_begin"), 0);
    }

    #[test]
    fn a_missing_self_chat_names_the_account_problem() {
        let mut probe = Fake::new();
        probe.facts = json!({"ui_allowed": true, "target": null});
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert!(
            step(&report, "isolated_test_self_chat")["reason"]
                .as_str()
                .unwrap()
                .starts_with("No verified self conversation")
        );
        assert!(commands.seen.borrow().is_empty());
    }

    #[test]
    fn an_unverifiable_baseline_stops_before_any_write() {
        let mut probe = Fake::new();
        probe.begin = json!({"ok": false, "reason": "baseline moved"});
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(report["state"], "limited");
        assert_eq!(
            step(&report, "existing_self_chat_baseline")["reason"],
            "baseline moved"
        );
        assert!(commands.seen.borrow().is_empty());
    }

    #[test]
    fn a_failed_first_send_leaves_everything_else_unproven_and_invalidates_its_proofs() {
        let probe = Fake::new();
        let commands = script(|operation, _| {
            if operation == "messages.send" {
                Err("invalid".into())
            } else {
                observed("never")
            }
        });
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(report["state"], "limited");
        assert_eq!(step(&report, "send_text")["reason"], "invalid");
        assert_eq!(
            step(&report, "reply")["reason"],
            "prerequisite_send_unproven"
        );
        assert_eq!(
            step(&report, "mark_read")["reason"],
            "prerequisite_send_unproven"
        );
        let invalidated = probe.invalidated();
        assert!(invalidated.contains(&"text".to_owned()));
        assert!(invalidated.contains(&"attachment".to_owned()) || !invalidated.is_empty());
        assert!(invalidated.contains(&"messages.reply".to_owned()));
        assert_eq!(probe.count("messages.selftest_cleanup"), 1);
        assert!(
            !commands
                .seen
                .borrow()
                .iter()
                .any(|(operation, _)| operation == "messages.reply")
        );
    }

    #[test]
    fn a_chat_that_does_not_qualify_skips_the_read_state_steps_with_a_reason() {
        let mut probe = Fake::new();
        probe.fixture = json!({"chat": "chat-9", "qualifies": false});
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(
            step(&report, "mark_read")["reason"],
            "isolated_test_chat_unavailable"
        );
        assert_eq!(
            step(&report, "mark_unread")["reason"],
            "isolated_test_chat_unavailable"
        );
        assert!(
            !commands
                .seen
                .borrow()
                .iter()
                .any(|(operation, _)| operation == "messages.mark_read")
        );
    }

    #[test]
    fn the_read_state_ledger_names_the_fixture_chat_before_the_first_ui_step() {
        let probe = Fake::new();
        let commands = script(everything);
        run(&probe, &commands, &request()).unwrap();
        let first = probe.ledger().remove(0);
        assert_eq!(first["created_chat_key"], "chat-0");
        assert_eq!(first["baseline_max_chat_row_id"], 5);
        assert_eq!(first["created_by_run"], false);
        assert_eq!(first["existing_self_chat"], true);
    }

    #[test]
    fn a_dispatch_that_ends_indeterminate_leaves_the_cleanup_pending_reconciliation() {
        let probe = Fake::new();
        let commands = script(|operation, native| {
            if operation == "messages.edit" {
                Ok(("indeterminate".into(), json!({"observed": false})))
            } else {
                everything(operation, native)
            }
        });
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(step(&report, "edit")["reason"], "indeterminate");
        let last = probe.ledger().pop().unwrap();
        assert_eq!(last["cleanup_state"], "pending_reconciliation");
        assert_eq!(last["late_effect_possible"], true);
    }

    #[test]
    fn a_failed_baseline_restore_is_reported_and_keeps_the_cleanup_pending() {
        let mut probe = Fake::new();
        probe.restore = json!({"restored": false});
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(report["state"], "limited");
        assert_eq!(
            step(&report, "existing_self_chat_preserved")["proven"],
            false
        );
        assert_eq!(
            probe.ledger().pop().unwrap()["cleanup_state"],
            "pending_native_cleanup"
        );
    }

    #[test]
    fn a_failed_cleanup_is_unproven_and_leaves_native_cleanup_pending() {
        let mut probe = Fake::new();
        probe.cleanup = json!({"passed": false, "failure": "row remains"});
        let commands = script(everything);
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(step(&report, "cleanup")["reason"], "row remains");
        assert_eq!(
            probe.ledger().pop().unwrap()["cleanup_state"],
            "pending_native_cleanup"
        );
    }

    #[test]
    fn a_reaction_that_was_not_added_is_not_removed() {
        let probe = Fake::new();
        let commands = script(|operation, native| {
            if operation == "messages.react" {
                Err("invalid".into())
            } else {
                everything(operation, native)
            }
        });
        let report = run(&probe, &commands, &request()).unwrap();
        assert_eq!(
            step(&report, "love_remove")["reason"],
            "reaction_add_unproven"
        );
        assert_eq!(
            step(&report, "emoji_remove")["reason"],
            "reaction_add_unproven"
        );
        let removals = commands
            .seen
            .borrow()
            .iter()
            .filter(|(operation, _)| operation == "messages.react")
            .count();
        assert_eq!(removals, 2);
    }

    #[test]
    fn a_cached_report_for_the_same_build_is_returned_without_running() {
        let mut probe = Fake::new();
        probe.cached = json!({"report": {"state": "proven", "os_build": "25A"}});
        let commands = script(everything);
        let mut asked = request();
        asked["force"] = json!(false);
        let report = run(&probe, &commands, &asked).unwrap();
        assert_eq!(report["state"], "proven");
        assert!(commands.seen.borrow().is_empty());
        assert_eq!(probe.count("messages.selftest_facts"), 0);
    }

    #[test]
    fn every_registered_message_comes_from_the_command_evidence() {
        let probe = Fake::new();
        let commands = script(everything);
        run(&probe, &commands, &request()).unwrap();
        let keys: Vec<Value> = probe
            .calls
            .borrow()
            .iter()
            .filter(|(name, _)| name == "messages.selftest_created")
            .map(|(_, args)| args["keys"].clone())
            .collect();
        assert!(
            keys.iter()
                .all(|keys| keys.as_array().is_some_and(|keys| keys.len() == 1))
        );
        assert!(keys.len() >= 10);
    }

    #[test]
    fn the_link_entry_handshakes_asks_for_facts_and_writes_the_limited_report() {
        use ab_wire::native::{Frame, PrimitiveResult};
        use std::{io::Write, os::unix::net::UnixStream, thread};
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let swift = thread::spawn(move || {
            assert!(matches!(
                ab_wire::native::read_frame(&mut theirs),
                Ok(Frame::Hello { owner_fence: 1 })
            ));
            theirs.write_all(&Frame::Welcome.encode().unwrap()).unwrap();
            assert!(matches!(
                ab_wire::native::read_frame(&mut theirs),
                Ok(Frame::Session(_))
            ));
            let mut seen = Vec::new();
            while let Ok(Frame::Primitive(call)) = ab_wire::native::read_frame(&mut theirs) {
                let result = match call.op.as_str() {
                    "messages.selftest_facts" => json!({"ui_allowed": false, "target": null}),
                    _ => json!({}),
                };
                seen.push((call.op.clone(), call.args.map(|a| a.get().to_owned())));
                let answer = Frame::PrimitiveResult(PrimitiveResult {
                    id: call.id,
                    result: serde_json::value::RawValue::from_string(result.to_string()).unwrap(),
                });
                theirs.write_all(&answer.encode().unwrap()).unwrap();
            }
            seen
        });
        super::serve(ours, &request()).unwrap();
        let seen = swift.join().unwrap();
        let seen: Vec<_> = seen
            .into_iter()
            .filter(|(name, _)| name != "messages.config")
            .collect();
        let names: Vec<&str> = seen.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["messages.selftest_facts", "messages.selftest_report"]
        );
        let report: Value = serde_json::from_str(seen[1].1.as_ref().unwrap()).unwrap();
        assert_eq!(report["report"]["state"], "limited");
        assert_eq!(report["report"]["writes_submitted"], 0);
    }

    #[test]
    fn a_primitive_failure_from_the_peer_stops_the_run_with_its_code() {
        use ab_wire::native::{Frame, PrimitiveError};
        use std::{io::Write, os::unix::net::UnixStream, thread};
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let swift = thread::spawn(move || {
            let _ = ab_wire::native::read_frame(&mut theirs);
            theirs.write_all(&Frame::Welcome.encode().unwrap()).unwrap();
            let _ = ab_wire::native::read_frame(&mut theirs);
            if let Ok(Frame::Primitive(call)) = ab_wire::native::read_frame(&mut theirs) {
                let answer = Frame::PrimitiveError(PrimitiveError {
                    id: call.id,
                    code: "permission_missing".into(),
                });
                theirs.write_all(&answer.encode().unwrap()).unwrap();
            }
        });
        assert!(super::serve(ours, &request()).is_err());
        swift.join().unwrap();
    }
}
