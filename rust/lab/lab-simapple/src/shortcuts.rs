use super::{Duration, Instant, Result, Session, Shared, Ticket, Value, json, lock, now, send, string};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::thread;

const SLOW_RUNNER: &str = "[ab-test slow runner]";

impl Session<'_> {
    pub(super) fn shortcuts_prepare(&mut self, frame: &Value) -> Result<()> {
        if frame["action"]["op"] != "shortcuts.run" {
            return self.reject(frame, "unsupported");
        }
        let ticket = super::id("ticket")?;
        self.tickets.insert(
            ticket.clone(),
            Ticket {
                prepare: frame.clone(),
                before: Value::Null,
                deadline: Instant::now() + Duration::from_secs(30),
            },
        );
        send(
            &mut self.stream,
            &json!({"t":"Prepared","command_id":frame["command_id"],"ticket":ticket,"route":"shortcuts_cli","fingerprint":frame["request_hash"]}),
        )?;
        Ok(())
    }

    pub(super) fn shortcuts_dispatch(&mut self, state: &Shared, frame: &Value) -> Result<()> {
        let command = string(frame, "command_id");
        let Some(ticket) = self.tickets.remove(&string(frame, "ticket")) else {
            return self.reject(frame, "expired");
        };
        if ticket.prepare["command_id"] != command
            || frame["owner_fence"].as_u64() != Some(self.fence)
            || Instant::now() >= ticket.deadline
        {
            return self.reject(frame, "expired");
        }
        lock(state)?.guard(&json!({"command_id":command,"ticket":frame["ticket"],"started_at":now()}))?;
        let args = &ticket.prepare["action"]["args"];
        if string(args, "shortcut") == SLOW_RUNNER {
            for _ in 0..4 {
                thread::sleep(Duration::from_secs(4));
                send(&mut self.stream, &json!({"t":"Pong"}))?;
            }
        }
        let data = match args["input_base64"].as_str().filter(|s| !s.is_empty()) {
            Some(encoded) => STANDARD.decode(encoded).unwrap_or_default(),
            None => string(args, "input").into_bytes(),
        };
        let evidence = json!({
            "output_sha256": hex(&Sha256::digest(&data)),
            "output_bytes": data.len(),
            "exit_status": 0,
            "output_base64": STANDARD.encode(&data),
            "effect_scope": "runner completion only",
            "prepared": {"request_hash": ticket.prepare["request_hash"]},
        });
        let outcome = json!({"t":"Outcome","command_id":command,"disposition":"locally_observed","evidence":evidence});
        lock(state)?.guard(&json!({"command_id":command,"ticket":frame["ticket"],"outcome":outcome}))?;
        send(&mut self.stream, &outcome)?;
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}
