use crate::{
    Error, Estimate, Local, Result, Session,
    actions::{common, disposition, sleep, text},
    failed,
    local::Handler,
    new_id,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type Seen = Arc<Mutex<HashMap<(String, String, bool), i128>>>;

fn observer(seen: &Seen, tag: &str) -> Handler {
    let seen = seen.clone();
    let prefix = format!("{tag} ");
    Box::new(move |at, batch| {
        let Ok(mut map) = seen.lock() else {
            return;
        };
        for change in batch["changes"].as_array().into_iter().flatten() {
            let record = &change["record"];
            let data = &record["data"];
            let given = data["given_name"].as_str().unwrap_or_default();
            if record["collection"] != "contacts.contact"
                || !given.starts_with(&prefix)
                || data["organization"] != "Bridge Test"
            {
                continue;
            }
            let key = (
                text(record, "id"),
                text(data, "job_title"),
                record["deleted"].as_bool().unwrap_or_default(),
            );
            map.entry(key).or_insert(at);
        }
    })
}

fn structured(reply: &crate::Reply) -> Result<Value> {
    let body = String::from_utf8_lossy(&reply.body).into_owned();
    let event = reply
        .header("content-type")
        .is_some_and(|kind| kind.starts_with("text/event-stream"));
    let payload = if event {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .find_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
    } else {
        serde_json::from_str(&body).ok()
    };
    match payload.and_then(|value| {
        let inner = value["result"]["structuredContent"].clone();
        (!inner.is_null()).then_some(inner)
    }) {
        Some(inner) => Ok(inner),
        None => failed("corrupt MCP response"),
    }
}

fn call(local: &Local, protocol: &str, name: &str, input: &Value) -> Result<Value> {
    let (status, body) = if protocol == "mcp" {
        let envelope = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name.replace('.', "_"), "arguments": input}});
        let reply = local.fetch_with(
            "POST",
            "/mcp",
            &serde_json::to_vec(&envelope)?,
            &[
                ("Accept", "application/json, text/event-stream"),
                ("MCP-Protocol-Version", "2025-06-18"),
            ],
            8 << 20,
        )?;
        (reply.status, structured(&reply)?)
    } else {
        local.post(name, input)?
    };
    if let Some(code) = body["error"]["code"].as_str() {
        return Err(Error::Api {
            operation: name.to_owned(),
            status,
            code: code.to_owned(),
        });
    }
    if status != 200 {
        return failed("unavailable");
    }
    Ok(body)
}

struct Step<'a> {
    local: &'a Local,
    seen: &'a Seen,
    tag: &'a str,
    protocol: &'a str,
}
impl Step<'_> {
    fn args(&self, operation: &str, target: &Value) -> Value {
        if operation == "contacts.create" {
            return json!({"given_name": format!("{} {}", self.tag, self.protocol), "family_name": "Testfamily", "organization": "Bridge Test", "job_title": "Testjob"});
        }
        let mut args = json!({"id": target["id"]});
        if operation == "contacts.update" {
            args["job_title"] = json!("Updated Testjob");
        }
        args
    }
    fn visible(&self, operation: &str, record: &Value) -> bool {
        let data = &record["data"];
        record["deleted"].as_bool().unwrap_or_default() == (operation == "contacts.delete")
            && text(data, "given_name") == format!("{} {}", self.tag, self.protocol)
            && (operation != "contacts.update" || data["job_title"] == "Updated Testjob")
    }
    fn run(&self, operation: &str, target: &mut Value) -> Result<Value> {
        let command = call(
            self.local,
            self.protocol,
            operation,
            &json!({"command_id": new_id("cmd"), "args": self.args(operation, target), "expected_version": target["version"].as_u64().unwrap_or_default(), "wait_ms": 15000}),
        )?;
        let ids = command["observed_record_ids"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if disposition(&command) != "locally_observed" || ids.len() != 1 {
            return failed("contact command unobserved");
        }
        let id = ids[0].as_str().unwrap_or_default().to_owned();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut first_read = 0_i128;
        while Instant::now() < deadline {
            let result = self.local.op(
                "records.get",
                &json!({"collection": "contacts.contact", "id": id, "include_deleted": true}),
            )?;
            if let Some([record]) = result["records"].as_array().map(Vec::as_slice) {
                target.clone_from(record);
                if self.visible(operation, record) {
                    if first_read == 0 {
                        first_read = jiff::Timestamp::now().as_nanosecond();
                    }
                    let key = (
                        id.clone(),
                        text(&record["data"], "job_title"),
                        record["deleted"].as_bool().unwrap_or_default(),
                    );
                    let observed = self
                        .seen
                        .lock()
                        .map_err(|_| Error::Invalid("lock"))?
                        .get(&key)
                        .copied()
                        .unwrap_or_default();
                    if observed != 0 {
                        return Ok(
                            json!({"operation": operation, "record_id": id, "wall_ns": i64::try_from(observed).unwrap_or_default(), "read_wall_ns": i64::try_from(first_read).unwrap_or_default()}),
                        );
                    }
                }
            }
            sleep(10);
        }
        failed("contact read your write failed")
    }
}

pub fn run<R: Read, W: Write>(session: &mut Session<R, W>) -> Result<()> {
    let value = session.input::<Value>()?;
    let input = common(&value)?;
    let tag = text(&value, "Tag");
    let local = Local::new(&input)?;
    let clock = Estimate::load(std::path::Path::new(&input.home))?;
    let head = local.op("records.query", &json!({"collection": "", "limit": 1}))?;
    let seen: Seen = Seen::default();
    let mut watch = local.watch(&head["cursor"], observer(&seen, &tag))?;
    let mut values = Vec::new();
    for protocol in ["rest", "mcp"] {
        let step = Step {
            local: &local,
            seen: &seen,
            tag: &tag,
            protocol,
        };
        let mut target = Value::Null;
        for operation in ["contacts.create", "contacts.update", "contacts.delete"] {
            values.push(step.run(operation, &mut target)?);
        }
    }
    watch.close();
    session.emit(&json!({"values": values, "offset_ms": clock.offset_ms(), "uncertainty_ms": clock.uncertainty_ms()}))
}
