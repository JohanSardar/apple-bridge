use super::{
    owned::{Driver, Probe, Refusal, Settled, native_failure, refuse, text},
    shortcuts,
};
use serde_json::{Value, json};
use std::{thread, time::Duration};

const SHORTCUT: &str = "AB Focus";
const RUN_TIMEOUT: Duration = Duration::from_secs(15);
const READBACK_TRIES: u32 = 50;
const READBACK_GAP: Duration = Duration::from_millis(200);

fn status(probe: &dyn Probe) -> Result<Value, String> {
    let fact = probe.call("focus.framework_status", &json!({}))?;
    let path = probe.call("focus.assertions_path", &json!({}))?;
    let assertions = std::fs::read(text(&path, "path")).ok();
    Ok(ab_native::focus::status(&fact, assertions.as_deref()))
}

pub(super) fn future(until: &str) -> bool {
    until
        .parse::<jiff::Timestamp>()
        .is_ok_and(|moment| moment > jiff::Timestamp::now())
}

pub(super) fn checked_args(action: &Value) -> Result<(), Refusal> {
    let args = &action["args"];
    if text(action, "op") != "focus.set"
        || text(args, "mode") != "Do Not Disturb"
        || !args["on"].is_boolean()
    {
        return Err(refuse("invalid"));
    }
    let until = text(args, "until");
    if !until.is_empty() && (args["on"] != true || !future(until)) {
        return Err(refuse("invalid"));
    }
    Ok(())
}

pub(super) fn readable(current: &Value) -> bool {
    text(current, "authorization") == "authorized" && !current["focused"].is_null()
}

fn checked(probe: &dyn Probe, action: &Value) -> Result<(), Refusal> {
    checked_args(action)?;
    let current = status(probe).map_err(|code| native_failure(&code))?;
    if readable(&current) {
        Ok(())
    } else {
        Err(refuse("forbidden"))
    }
}

fn prepare(probe: &dyn Probe, action: &Value, _expected: &Value) -> Result<Value, Refusal> {
    checked(probe, action)?;
    Ok(Value::Null)
}

fn validate(probe: &dyn Probe, action: &Value, _state: &Value) -> Result<(), Refusal> {
    checked(probe, action)
}

fn write(
    probe: &dyn Probe,
    _command: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    let args = &action["args"];
    let input = json!({"mode": args["mode"], "on": args["on"], "until": text(args, "until")});
    let ran = shortcuts::run(SHORTCUT, input.to_string().into_bytes(), RUN_TIMEOUT)?;
    if ran.exit_status != 0 || ran.timed_out {
        return Err("unavailable".to_owned());
    }
    for _ in 0..READBACK_TRIES {
        let current = status(probe)?;
        if text(&current, "authorization") == "authorized" && current["focused"] == args["on"] {
            return Ok(json!({"observed": true, "readback": current}));
        }
        thread::sleep(READBACK_GAP);
    }
    Err("unavailable".to_owned())
}

#[allow(clippy::unnecessary_wraps)]
fn settle(_operation: &str, written: &Value) -> Option<Settled> {
    Some(Settled {
        disposition: "locally_observed",
        evidence: written.clone(),
    })
}

fn resolved(_probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let prepared = &evidence["prepared"];
    let proven = text(&prepared["action"], "op") == "focus.set"
        && evidence["fingerprint"] == prepared["request_hash"]
        && evidence["observed"] == true
        && text(&evidence["readback"], "authorization") == "authorized"
        && evidence["readback"]["focused"] == prepared["action"]["args"]["on"];
    proven.then(|| evidence.clone())
}

fn reconcile_args(_found: &Value, _operation: &str) -> Value {
    json!({})
}

pub(super) const DRIVER: Driver = Driver {
    name: "focus",
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

    fn action(args: Value) -> Value {
        json!({"op": "focus.set", "args": args})
    }

    #[test]
    fn only_do_not_disturb_with_a_clear_on_or_off_passes() {
        for args in [
            json!({"mode": "Work", "on": true}),
            json!({"mode": "", "on": true}),
            json!({"on": true}),
            json!({"mode": "Do Not Disturb"}),
            json!({"mode": "Do Not Disturb", "on": "true"}),
            json!({"mode": "Do Not Disturb", "on": 1}),
        ] {
            assert_eq!(
                checked_args(&action(args.clone()))
                    .expect_err("refused")
                    .code,
                "invalid",
                "{args}"
            );
        }
        checked_args(&action(json!({"mode": "Do Not Disturb", "on": false}))).unwrap();
    }

    #[test]
    fn an_end_time_needs_on_and_the_future() {
        let later = (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1)).to_string();
        checked_args(&action(
            json!({"mode": "Do Not Disturb", "on": true, "until": later}),
        ))
        .unwrap();
        for args in [
            json!({"mode": "Do Not Disturb", "on": false, "until": later}),
            json!({"mode": "Do Not Disturb", "on": true, "until": "2001-01-01T00:00:00Z"}),
            json!({"mode": "Do Not Disturb", "on": true, "until": "tomorrow"}),
        ] {
            assert_eq!(
                checked_args(&action(args.clone()))
                    .expect_err("refused")
                    .code,
                "invalid",
                "{args}"
            );
        }
    }

    struct Facts(Value);
    impl Probe for Facts {
        fn call(&self, operation: &str, _: &Value) -> Result<Value, String> {
            match operation {
                "focus.framework_status" => Ok(self.0.clone()),
                "focus.assertions_path" => Ok(json!({"path": "/nonexistent/ab-test"})),
                _ => Err("unsupported".into()),
            }
        }
    }

    #[test]
    fn without_the_grant_nothing_runs() {
        let action = action(json!({"mode": "Do Not Disturb", "on": true}));
        for fact in [
            json!({"authorization": "denied", "center_focused": null}),
            json!({"authorization": "not_determined", "center_focused": null}),
            json!({"authorization": "authorized", "center_focused": null}),
        ] {
            let probe = Facts(fact);
            assert_eq!(
                prepare(&probe, &action, &Value::Null)
                    .expect_err("refused")
                    .code,
                "forbidden"
            );
            assert_eq!(
                validate(&probe, &action, &Value::Null)
                    .expect_err("refused")
                    .code,
                "forbidden"
            );
        }
        let probe = Facts(json!({"authorization": "authorized", "center_focused": false}));
        prepare(&probe, &action, &Value::Null).unwrap();
    }

    #[test]
    fn resolving_needs_a_matching_read_back() {
        let saved = json!({
            "fingerprint": "h", "observed": true, "readback": {"authorization": "authorized", "focused": true},
            "prepared": {"request_hash": "h", "action": {"op": "focus.set", "args": {"on": true}}},
        });
        let probe = Facts(json!({}));
        assert!(resolved(&probe, &saved).is_some());
        let mut wrong = saved.clone();
        wrong["readback"]["focused"] = json!(false);
        assert!(resolved(&probe, &wrong).is_none());
        let mut other = saved;
        other["fingerprint"] = json!("x");
        assert!(resolved(&probe, &other).is_none());
    }
}
