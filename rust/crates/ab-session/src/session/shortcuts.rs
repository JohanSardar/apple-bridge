use super::owned::{Driver, Probe, Refusal, Settled, refuse, text};
use ab_shortcuts::cli::{Cli, Input};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, time::Duration};

const MAX_NAME: usize = 1024;
const MAX_INPUT: usize = 16 * 1024;
const MAX_OUTPUT: usize = 32 * 1024;
const MAX_TIMEOUT_MS: f64 = 300_000.0;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

fn decoded_input(args: &Value) -> Result<Vec<u8>, Refusal> {
    let invalid = || refuse("invalid");
    let encoded = text(args, "input_base64");
    if encoded.is_empty() {
        return Ok(text(args, "input").as_bytes().to_vec());
    }
    if !text(args, "input").is_empty() {
        return Err(invalid());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid())?;
    if bytes.len() > MAX_INPUT {
        return Err(invalid());
    }
    Ok(bytes)
}

pub(super) fn checked(action: &Value) -> Result<(), Refusal> {
    if text(action, "op") != "shortcuts.run" {
        return Err(refuse("invalid"));
    }
    let args = &action["args"];
    let name = text(args, "shortcut");
    let timeout = args["timeout_ms"].as_f64().unwrap_or(0.0);
    if name.trim().is_empty()
        || name.len() > MAX_NAME
        || name.starts_with('-')
        || !(0.0..=MAX_TIMEOUT_MS).contains(&timeout)
        || text(args, "input").len() > MAX_INPUT
    {
        return Err(refuse("invalid"));
    }
    decoded_input(args).map(|_| ())
}

fn prepare(_probe: &dyn Probe, action: &Value, _expected: &Value) -> Result<Value, Refusal> {
    checked(action)?;
    Ok(json!({"route": "shortcuts_cli"}))
}

fn validate(_probe: &dyn Probe, action: &Value, _state: &Value) -> Result<(), Refusal> {
    checked(action)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn duration(args: &Value) -> Duration {
    match args["timeout_ms"].as_f64() {
        Some(milliseconds) if milliseconds > 0.0 => Duration::from_secs_f64(milliseconds / 1000.0),
        _ => DEFAULT_TIMEOUT,
    }
}

pub(super) struct Ran {
    pub exit_status: i32,
    pub timed_out: bool,
    pub output: Vec<u8>,
}

pub(super) fn run(name: &str, input: Vec<u8>, timeout: Duration) -> Result<Ran, String> {
    let (output, failure) = Cli::default().run_detailed(&Input {
        shortcut: name.to_owned(),
        data: Some(input),
        timeout,
        output_limit: MAX_OUTPUT,
        ..Input::default()
    });
    match failure {
        None | Some(ab_shortcuts::Error::DeadlineExceeded) => Ok(Ran {
            exit_status: output.exit_status,
            timed_out: output.timed_out,
            output: output.data,
        }),
        Some(ab_shortcuts::Error::OutputExceeded) => Ok(Ran {
            exit_status: -1,
            timed_out: false,
            output: Vec::new(),
        }),
        Some(_) => Err("unavailable".to_owned()),
    }
}

fn write(
    _probe: &dyn Probe,
    _command: &str,
    action: &Value,
    _state: &Value,
) -> Result<Value, String> {
    checked(action).map_err(|refusal| refusal.code)?;
    let args = &action["args"];
    let input = decoded_input(args).map_err(|refusal| refusal.code)?;
    let ran = run(text(args, "shortcut"), input, duration(args))?;
    Ok(json!({
        "exit_status": ran.exit_status,
        "timed_out": ran.timed_out,
        "output_base64": STANDARD.encode(&ran.output),
        "output_bytes": ran.output.len(),
        "output_sha256": hex(&Sha256::digest(&ran.output)),
        "effect_scope": "runner completion only",
    }))
}

#[allow(clippy::unnecessary_wraps)]
fn settle(_operation: &str, written: &Value) -> Option<Settled> {
    Some(Settled {
        disposition: if written["exit_status"] == 0 && written["timed_out"] != true {
            "locally_observed"
        } else {
            "indeterminate"
        },
        evidence: written.clone(),
    })
}

fn resolved(_probe: &dyn Probe, evidence: &Value) -> Option<Value> {
    let output = STANDARD.decode(text(evidence, "output_base64")).ok()?;
    let fits = output.len() <= MAX_OUTPUT;
    let proven = text(evidence, "effect_scope") == "runner completion only"
        && evidence["exit_status"] == 0
        && evidence["timed_out"] != true
        && !text(&evidence["prepared"], "request_hash").is_empty()
        && text(&evidence["prepared"]["action"], "op") == "shortcuts.run";
    (fits && proven).then(|| {
        let mut found = evidence.clone();
        found["observed"] = json!(true);
        found["disposition"] = json!("locally_observed");
        found
    })
}

fn reconcile_args(_found: &Value, _operation: &str) -> Value {
    json!({})
}

pub(super) const DRIVER: Driver = Driver {
    name: "shortcuts",
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
        json!({"op": "shortcuts.run", "args": args})
    }

    #[test]
    fn a_shortcut_name_cannot_become_a_flag_or_an_empty_run() {
        for name in ["", "   ", "\n", "-h", "--output-path=/tmp/x", "-"] {
            assert_eq!(
                checked(&action(json!({"shortcut": name})))
                    .expect_err(name)
                    .code,
                "invalid"
            );
        }
        assert!(checked(&action(json!({"shortcut": "x".repeat(MAX_NAME + 1)}))).is_err());
        checked(&action(json!({"shortcut": "AB Echo"}))).unwrap();
        assert_eq!(
            checked(&json!({"op": "focus.set", "args": {"shortcut": "x"}}))
                .expect_err("op")
                .code,
            "invalid"
        );
    }

    #[test]
    fn inputs_and_timeouts_stay_in_bounds() {
        for args in [
            json!({"shortcut": "a", "timeout_ms": 300_001}),
            json!({"shortcut": "a", "timeout_ms": -1}),
            json!({"shortcut": "a", "input": "x".repeat(MAX_INPUT + 1)}),
            json!({"shortcut": "a", "input": "x", "input_base64": "eA=="}),
            json!({"shortcut": "a", "input_base64": "not base64!"}),
            json!({"shortcut": "a", "input_base64": STANDARD.encode(vec![0; MAX_INPUT + 1])}),
        ] {
            assert_eq!(
                checked(&action(args.clone())).expect_err("refused").code,
                "invalid",
                "{args}"
            );
        }
        assert_eq!(duration(&json!({})), DEFAULT_TIMEOUT);
        assert_eq!(
            duration(&json!({"timeout_ms": 1500})),
            Duration::from_millis(1500)
        );
    }

    #[test]
    fn only_a_clean_exit_counts_as_observed() {
        let done = settle(
            "shortcuts.run",
            &json!({"exit_status": 0, "timed_out": false}),
        )
        .unwrap();
        assert_eq!(done.disposition, "locally_observed");
        for written in [
            json!({"exit_status": 1, "timed_out": false}),
            json!({"exit_status": 0, "timed_out": true}),
        ] {
            assert_eq!(
                settle("shortcuts.run", &written).unwrap().disposition,
                "indeterminate"
            );
        }
    }

    #[test]
    fn resolving_needs_the_saved_proof() {
        struct Nothing;
        impl Probe for Nothing {
            fn call(&self, _: &str, _: &Value) -> Result<Value, String> {
                Err("unavailable".into())
            }
        }
        let proof = json!({
            "effect_scope": "runner completion only", "exit_status": 0, "timed_out": false, "output_base64": "",
            "prepared": {"request_hash": "h", "action": {"op": "shortcuts.run"}},
        });
        assert_eq!(resolved(&Nothing, &proof).unwrap()["observed"], true);
        let mut failed = proof.clone();
        failed["exit_status"] = json!(1);
        assert!(resolved(&Nothing, &failed).is_none());
        let mut other = proof;
        other["prepared"]["action"]["op"] = json!("mac.notify");
        assert!(resolved(&Nothing, &other).is_none());
    }
}
