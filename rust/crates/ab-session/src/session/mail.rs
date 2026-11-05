use super::owned::{Driver, Probe, Refusal, Settled, conflict, native_failure, refuse, text};
use ab_wire::native::mail_recipient;
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    thread,
    time::{Duration, Instant},
};

const OPERATIONS: [&str; 6] = [
    "mail.create_draft",
    "mail.send",
    "mail.move",
    "mail.set_flag",
    "mail.mark_read",
    "mail.delete",
];
const SCRIPT: &str = "mail.script";
const POLL: Duration = Duration::from_millis(200);
const CREATE_WAIT: Duration = Duration::from_secs(30);
const CHANGE_WAIT: Duration = Duration::from_secs(15);
const SCRIPT_TIMEOUT_SECONDS: u32 = 15;
const START_RETRIES: u32 = 2;
const START_GAP: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Permission,
    Timeout,
    ConnectionLost,
    Rejected,
    Unconfirmed,
}

impl Failure {
    fn of_apple_event(number: i64) -> Self {
        match number {
            -1712 => Self::Timeout,
            -1743 | -1744 => Self::Permission,
            -600 | -609 => Self::ConnectionLost,
            _ => Self::Rejected,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Permission => {
                "Mail automation is not allowed. Enable Mail for Apple Bridge in System Settings > Privacy & Security > Automation."
            }
            Self::Timeout => {
                "Mail automation did not respond in time. Restart Mail and try again. If it still does not respond, sign out of macOS and sign in again."
            }
            Self::ConnectionLost => {
                "Mail closed before completing the command. Reopen Mail, let mailbox sync finish, and check the message before trying again."
            }
            Self::Rejected => {
                "Mail could not complete the command. Open Mail, check the account and mailbox, and try again."
            }
            Self::Unconfirmed => {
                "Mail could not confirm the message after the command. Check Sent or Drafts before trying again."
            }
        }
    }
}

fn quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

fn wrapped(body: &str) -> String {
    format!(
        "with timeout of {SCRIPT_TIMEOUT_SECONDS} seconds\ntell application id \"com.apple.mail\"\n{body}\nend tell\nend timeout"
    )
}

fn run(probe: &dyn Probe, body: &str) -> Result<String, Failure> {
    let answer = probe
        .call(SCRIPT, &json!({"source": wrapped(body)}))
        .map_err(|code| match code.as_str() {
            "permission_missing" => Failure::Permission,
            "unavailable" => Failure::Timeout,
            _ => Failure::Rejected,
        })?;
    match answer["error_number"].as_i64() {
        Some(number) => Err(Failure::of_apple_event(number)),
        None => Ok(text(&answer, "result").to_owned()),
    }
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn creates(operation: &str) -> bool {
    matches!(operation, "mail.create_draft" | "mail.send")
}

fn message_key(action: &Value) -> Option<&str> {
    let key = text(action, "native_key");
    key.parse::<u64>().ok().map(|_| key)
}

fn checked(action: &Value) -> Result<(), Refusal> {
    let operation = text(action, "op");
    if !OPERATIONS.contains(&operation) {
        return Err(refuse("unsupported"));
    }
    let args = &action["args"];
    if creates(operation) {
        let to = strings(&args["to"]);
        let cc = strings(&args["cc"]);
        let listed =
            args["to"].as_array().map_or(0, Vec::len) + args["cc"].as_array().map_or(0, Vec::len);
        if to.is_empty()
            || text(args, "subject").is_empty()
            || to.len() + cc.len() != listed
            || !to.iter().chain(&cc).all(|address| mail_recipient(address))
        {
            return Err(refuse("invalid"));
        }
        if !text(args, "reply_to").is_empty() || !text(args, "forward_of").is_empty() {
            return Err(refuse("unsupported"));
        }
    } else if message_key(action).is_none() {
        return Err(refuse("invalid"));
    }
    if args["attachments"]
        .as_array()
        .is_some_and(|files| !files.is_empty())
    {
        return Err(refuse("unsupported"));
    }
    Ok(())
}

fn folder(operation: &str) -> &'static str {
    if operation == "mail.send" {
        "sent mailbox"
    } else {
        "drafts mailbox"
    }
}

fn matching(operation: &str, args: &Value) -> String {
    format!(
        "set matched to {{}}\nset expectedBody to {body}\nrepeat with candidate in (messages of {folder} whose subject is {subject})\nset candidateBody to (content of candidate) as string\nif candidateBody starts with expectedBody or candidateBody starts with (linefeed & expectedBody) or candidateBody starts with (return & expectedBody) or candidateBody starts with (return & linefeed & expectedBody) then set end of matched to contents of candidate\nend repeat\n",
        body = quote(text(args, "body")),
        folder = folder(operation),
        subject = quote(text(args, "subject")),
    )
}

fn composed(operation: &str, args: &Value) -> String {
    let mut body = format!(
        "set m to make new outgoing message with properties {{subject:{}, content:{}, visible:true}}\n",
        quote(text(args, "subject")),
        quote(text(args, "body")),
    );
    for kind in ["to", "cc"] {
        for address in strings(&args[kind]) {
            let _ = writeln!(
                body,
                "tell m to make new {kind} recipient at end of {kind} recipients with properties {{address:{}}}",
                quote(address)
            );
        }
    }
    if operation == "mail.send" {
        body.push_str("send m\n");
    } else {
        let _ = writeln!(
            body,
            "save m\nset saved to false\nrepeat 40 times\nif (count of (messages of drafts mailbox whose subject is {subject})) > 0 then\nset saved to true\nexit repeat\nend if\ndelay 0.25\nend repeat\nif saved then close m saving yes",
            subject = quote(text(args, "subject")),
        );
    }
    body
}

fn lookup(key: &str) -> String {
    format!(
        "set m to missing value\nrepeat with a in accounts\nrepeat with b in mailboxes of a\nset matches to (messages of b whose id is {key})\nif (count of matches) is greater than 0 then set m to item 1 of matches\nif m is not missing value then exit repeat\nend repeat\nif m is not missing value then exit repeat\nend repeat\nif m is missing value then error \"message unavailable\"\n"
    )
}

fn poll(
    probe: &dyn Probe,
    wait: Duration,
    mut ready: impl FnMut(&dyn Probe) -> Result<Option<String>, Failure>,
) -> Result<Option<String>, Failure> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(found) = ready(probe)? {
            return Ok(Some(found));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(POLL);
    }
}

fn create(probe: &dyn Probe, operation: &str, args: &Value) -> Result<Option<String>, Failure> {
    let matches = matching(operation, args);
    run(probe, &composed(operation, args))?;
    poll(probe, CREATE_WAIT, |probe| {
        if run(probe, &format!("{matches}return count of matched"))? != "1" {
            return Ok(None);
        }
        let key = run(
            probe,
            &format!(
                "{matches}if (count matched) is not 1 then return \"\"\nreturn (id of item 1 of matched) as string"
            ),
        )?;
        Ok(Some(key))
    })
}

fn flagged(operation: &str, args: &Value) -> (&'static str, &'static str) {
    let (property, value) = if operation == "mail.mark_read" {
        ("read status", args["read"] == true)
    } else {
        ("flagged status", args["flagged"] == true)
    };
    (property, if value { "true" } else { "false" })
}

fn set_status(
    probe: &dyn Probe,
    operation: &str,
    key: &str,
    args: &Value,
) -> Result<Option<String>, Failure> {
    let (property, value) = flagged(operation, args);
    let find = lookup(key);
    run(probe, &format!("{find}set {property} of m to {value}\n"))?;
    poll(probe, CHANGE_WAIT, |probe| {
        let answer = run(
            probe,
            &format!("{find}return ({property} of m is {value}) as string"),
        )?;
        Ok((answer == "true").then(|| key.to_owned()))
    })
}

fn moved(
    probe: &dyn Probe,
    operation: &str,
    key: &str,
    args: &Value,
) -> Result<Option<String>, Failure> {
    let target = if operation == "mail.delete" {
        "trash mailbox".to_owned()
    } else {
        format!(
            "mailbox {} of account {}",
            quote(text(args, "mailbox")),
            quote(text(args, "account"))
        )
    };
    let find = lookup(key);
    let header = run(probe, &format!("{find}return (message id of m) as string"))?;
    if header.is_empty() {
        return Err(Failure::Unconfirmed);
    }
    run(probe, &format!("{find}move m to {target}\n"))?;
    let header = quote(&header);
    poll(probe, CHANGE_WAIT, |probe| {
        let found = run(
            probe,
            &format!(
                "set matches to (messages of {target} whose message id is {header})\nif (count of matches) is not 1 then return \"\"\nreturn (id of item 1 of matches) as string"
            ),
        )?;
        Ok((!found.is_empty()).then_some(found))
    })
}

fn effect(probe: &dyn Probe, action: &Value) -> Result<Option<String>, Failure> {
    let operation = text(action, "op");
    let args = &action["args"];
    if creates(operation) {
        return create(probe, operation, args);
    }
    let key = message_key(action).ok_or(Failure::Rejected)?;
    if matches!(operation, "mail.mark_read" | "mail.set_flag") {
        set_status(probe, operation, key, args)
    } else {
        moved(probe, operation, key, args)
    }
}

fn prepare(_probe: &dyn Probe, action: &Value, _expected: &Value) -> Result<Value, Refusal> {
    checked(action)?;
    Ok(Value::Null)
}

fn validate(probe: &dyn Probe, action: &Value, _state: &Value) -> Result<(), Refusal> {
    checked(action)?;
    let operation = text(action, "op");
    if !creates(operation) {
        return Ok(());
    }
    let question = format!(
        "{}return count of matched",
        matching(operation, &action["args"])
    );
    let mut attempt = run(probe, &question);
    for _ in 0..START_RETRIES {
        if !matches!(attempt, Err(Failure::ConnectionLost | Failure::Timeout)) {
            break;
        }
        thread::sleep(START_GAP);
        attempt = run(probe, &question);
    }
    let count = attempt.map_err(|failure| {
        if failure == Failure::Permission {
            refuse("forbidden")
        } else {
            native_failure("unavailable")
        }
    })?;
    if count == "0" {
        Ok(())
    } else {
        Err(conflict(""))
    }
}

#[allow(clippy::unnecessary_wraps)]
fn write(
    probe: &dyn Probe,
    _command: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    Ok(match effect(probe, action) {
        Ok(Some(key)) if key.parse::<u64>().is_ok() => json!({"observed": true, "native_key": key}),
        Ok(_) => json!({"observed": false, "reason": Failure::Unconfirmed.reason()}),
        Err(failure) => json!({"observed": false, "reason": failure.reason()}),
    })
}

#[allow(clippy::unnecessary_wraps)]
fn settle(_operation: &str, written: &Value) -> Option<Settled> {
    let observed = written["observed"] == true;
    Some(Settled {
        disposition: if observed {
            "locally_observed"
        } else {
            "indeterminate"
        },
        evidence: written.clone(),
    })
}

fn resolved(_probe: &dyn Probe, _evidence: &Value) -> Option<Value> {
    None
}

fn reconcile_args(_found: &Value, _operation: &str) -> Value {
    json!({})
}

pub(super) const DRIVER: Driver = Driver {
    name: "mail",
    prepare,
    prepare_command: None,
    validate,
    write,
    settle,
    resolved,
    reconcile_args,
};

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps
)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    struct Scripted {
        answers: RefCell<VecDeque<Result<Value, String>>>,
        seen: RefCell<Vec<String>>,
    }

    impl Scripted {
        fn new(answers: Vec<Result<Value, String>>) -> Self {
            Self {
                answers: RefCell::new(answers.into()),
                seen: RefCell::default(),
            }
        }
        fn says(text: &str) -> Result<Value, String> {
            Ok(json!({"result": text}))
        }
    }

    impl Probe for Scripted {
        fn call(&self, operation: &str, args: &Value) -> Result<Value, String> {
            assert_eq!(operation, SCRIPT);
            self.seen.borrow_mut().push(text(args, "source").to_owned());
            self.answers
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Scripted::says("0"))
        }
    }

    fn draft() -> Value {
        json!({"op": "mail.create_draft", "args": {
            "to": ["a@example.com"], "cc": ["b@example.com"], "subject": "s\"x", "body": "line\nbody"
        }})
    }

    #[test]
    fn who_mail_can_reach_is_decided_here() {
        let probe = Scripted::new(vec![]);
        for args in [
            json!({"subject": "s"}),
            json!({"to": [], "subject": "s"}),
            json!({"to": ["a@example.com"]}),
            json!({"to": ["a@example.com, b@example.com"], "subject": "s"}),
            json!({"to": ["a@example.com\nbcc: c@example.com"], "subject": "s"}),
            json!({"to": ["Name <a@example.com>"], "subject": "s"}),
            json!({"to": ["a@example.com"], "cc": ["x"], "subject": "s"}),
            json!({"to": ["a@example.com", 7], "subject": "s"}),
        ] {
            for op in ["mail.create_draft", "mail.send"] {
                let refusal = prepare(&probe, &json!({"op": op, "args": args}), &Value::Null)
                    .expect_err("refused");
                assert_eq!(refusal.code, "invalid", "{op} {args}");
            }
        }
        for args in [
            json!({"to": ["a@example.com"], "subject": "s", "attachments": ["f"]}),
            json!({"to": ["a@example.com"], "subject": "s", "reply_to": "1"}),
            json!({"to": ["a@example.com"], "subject": "s", "forward_of": "1"}),
        ] {
            let refusal = prepare(
                &probe,
                &json!({"op": "mail.send", "args": args}),
                &Value::Null,
            )
            .expect_err("refused");
            assert_eq!(refusal.code, "unsupported");
        }
        assert!(probe.seen.borrow().is_empty(), "nothing reached Mail");
    }

    #[test]
    fn a_message_command_needs_a_numeric_key() {
        let probe = Scripted::new(vec![]);
        for key in ["", "abc", "1 or 1", "1\ndelete"] {
            let action = json!({"op": "mail.delete", "native_key": key, "args": {}});
            assert_eq!(
                prepare(&probe, &action, &Value::Null)
                    .expect_err("refused")
                    .code,
                "invalid"
            );
        }
        let other = json!({"op": "mail.forward", "native_key": "1", "args": {}});
        assert_eq!(
            prepare(&probe, &other, &Value::Null)
                .expect_err("refused")
                .code,
            "unsupported"
        );
    }

    #[test]
    fn text_cannot_leave_its_applescript_string() {
        let script = composed(
            "mail.create_draft",
            &json!({"subject": "\" & (do shell script \"x\") & \"", "body": "\\\"\n", "to": ["a@example.com"]}),
        );
        assert!(script.contains("subject:\"\\\" & (do shell script \\\"x\\\") & \\\"\""));
        assert!(script.contains("content:\"\\\\\\\"\\n\""));
    }

    #[test]
    fn a_draft_stays_open_until_it_is_saved() {
        let script = composed("mail.create_draft", &draft()["args"]);
        let saved = script.find("save m").expect("save");
        let waited = script.find("messages of drafts mailbox").expect("wait");
        let closed = script
            .find("if saved then close m saving yes")
            .expect("close");
        assert!(saved < waited && waited < closed);
        assert!(script.contains(
            "to recipient at end of to recipients with properties {address:\"a@example.com\"}"
        ));
        assert!(script.contains(
            "cc recipient at end of cc recipients with properties {address:\"b@example.com\"}"
        ));
        let sent = composed("mail.send", &draft()["args"]);
        assert!(sent.contains("send m") && !sent.contains("close m"));
    }

    #[test]
    fn an_existing_copy_refuses_before_any_effect() {
        let probe = Scripted::new(vec![Scripted::says("1")]);
        let refusal = validate(&probe, &draft(), &Value::Null).expect_err("refused");
        assert_eq!(refusal.code, "conflict");
        assert_eq!(probe.seen.borrow().len(), 1);
        assert!(!probe.seen.borrow()[0].contains("make new"));
    }

    #[test]
    fn a_mail_that_is_still_starting_gets_more_chances_before_a_refusal() {
        let starting = Scripted::new(vec![Ok(json!({"error_number": -600})), Scripted::says("0")]);
        validate(&starting, &draft(), &Value::Null).unwrap();
        let down = Scripted::new(vec![
            Ok(json!({"error_number": -600})),
            Ok(json!({"error_number": -600})),
            Ok(json!({"error_number": -600})),
        ]);
        let refusal = validate(&down, &draft(), &Value::Null).expect_err("refused");
        assert_eq!(refusal.code, "unavailable");
        assert_eq!(down.seen.borrow().len(), 3);
    }

    #[test]
    fn a_created_message_is_found_by_subject_and_body() {
        let probe = Scripted::new(vec![
            Scripted::says(""),
            Scripted::says("0"),
            Scripted::says("1"),
            Scripted::says("4242"),
        ]);
        let written = write(&probe, "c", &draft(), &Value::Null).unwrap();
        assert_eq!(written, json!({"observed": true, "native_key": "4242"}));
        let settled = settle("mail.create_draft", &written).unwrap();
        assert_eq!(settled.disposition, "locally_observed");
    }

    #[test]
    fn apple_event_failures_stay_indeterminate_with_a_reason() {
        for (number, wanted) in [
            (-1743, Failure::Permission),
            (-1712, Failure::Timeout),
            (-609, Failure::ConnectionLost),
            (-10000, Failure::Rejected),
        ] {
            let probe = Scripted::new(vec![Ok(json!({"error_number": number}))]);
            let written = write(&probe, "c", &draft(), &Value::Null).unwrap();
            assert_eq!(written["observed"], false);
            assert_eq!(written["reason"], wanted.reason());
            assert_eq!(
                settle("mail.send", &written).unwrap().disposition,
                "indeterminate"
            );
        }
    }

    #[test]
    fn a_move_follows_the_message_id_to_the_new_row() {
        let probe = Scripted::new(vec![
            Scripted::says("<x@y>"),
            Scripted::says(""),
            Scripted::says("77"),
        ]);
        let action = json!({"op": "mail.delete", "native_key": "12", "args": {"id": "r"}});
        let written = write(&probe, "c", &action, &Value::Null).unwrap();
        assert_eq!(written, json!({"observed": true, "native_key": "77"}));
        let seen = probe.seen.borrow();
        assert!(seen[1].contains("move m to trash mailbox"));
        assert!(seen[2].contains("messages of trash mailbox whose message id is \"<x@y>\""));
    }

    #[test]
    fn flags_read_back_the_requested_value() {
        let probe = Scripted::new(vec![Scripted::says(""), Scripted::says("true")]);
        let action = json!({"op": "mail.set_flag", "native_key": "12", "args": {"flagged": false}});
        let written = write(&probe, "c", &action, &Value::Null).unwrap();
        assert_eq!(written["native_key"], "12");
        let seen = probe.seen.borrow();
        assert!(seen[0].contains("set flagged status of m to false"));
        assert!(seen[1].contains("(flagged status of m is false)"));
    }
}
