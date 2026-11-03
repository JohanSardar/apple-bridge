use crate::{
    assertions::await_until,
    files_native,
    http::{Client, random_id},
    journey,
};
use base64::Engine;
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const SIM: [&str; 3] = ["FILE-COMMANDS", "FILE-CRUD-REAL", "FILE-OFFLOAD-REAL"];
pub const REAL: [&str; 3] = ["FILE-CRUD-REAL", "FILE-OFFLOAD-REAL", "FILE-SEARCH-REAL"];
const SCOPE: &str = "files:read,files:write,reminders:read,system:read";
const ROOT: &str = "/Bridge Test";

fn digest(content: &str) -> String {
    hex::encode(Sha256::digest(content.as_bytes()))
}

fn encode(content: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(content)
}

struct Session<'a> {
    client: &'a Client,
    mcp: bool,
    prefix: &'a str,
}

impl Session<'_> {
    fn raw(&self, op: &str, input: &Value) -> Result<Option<(u16, Value)>> {
        if self.mcp {
            self.client.call_mcp_raw(op, input, 40)
        } else {
            self.client.call_raw(op, input, 40)
        }
    }

    fn check(&self, evidence: &mut Evidence, name: &str, ok: bool) -> bool {
        evidence.check(&format!("{}{name}", self.prefix), ok, "");
        ok
    }

    fn run(&self, evidence: &mut Evidence, op: &str, mut args: Value) -> Result<bool> {
        args["root"] = json!(ROOT);
        let command = journey::command(op, &args)?;
        let reply = self.raw(op, &command)?;
        let observed = reply.as_ref().is_some_and(|(status, value)| {
            *status == 200 && value["error"].is_null() && value["state"] == "observed"
        });
        if !observed {
            eprintln!("{op} {reply:?}");
        }
        Ok(self.check(evidence, op, observed))
    }

    fn query(&self, op: &str, args: &Value) -> Result<Option<Value>> {
        let body = json!({"id": random_id("qry")?, "args": args, "deadline_ms": 15000});
        Ok(self.raw(op, &body)?.and_then(|(status, value)| {
            (status == 200 && value["error"].is_null()).then_some(value)
        }))
    }
}

fn protocol(world: &World, mcp: bool, prefix: &str, evidence: &mut Evidence) -> Result<bool> {
    let start = evidence.assertions.len();
    steps(world, mcp, prefix, evidence)?;
    Ok(evidence.assertions[start..].iter().all(|a| a.passed))
}

#[allow(clippy::too_many_lines)]
fn steps(world: &World, mcp: bool, prefix: &str, evidence: &mut Evidence) -> Result<()> {
    let mut world = world.sibling(if mcp { "mcp" } else { "rest" })?;
    evidence.check(&format!("{prefix}gateway_start"), true, "");
    let folder = json!({"root": ROOT, "path": "folder", "name": "folder", "kind": "directory",
        "size": 0, "created": null, "modified": null, "tags": [], "downloaded": true, "icloud_status": "local"});
    let fixture = world.control(
        "/dataset",
        &json!({"records": [{"kind": "upsert", "collection": "files.item", "native_key": "/Bridge Test/folder",
            "data": folder, "observed_at": jiff::Timestamp::now().to_string()}]}),
    );
    let client = Client::new(&world, SCOPE)?;
    let session = Session {
        client: &client,
        mcp,
        prefix,
    };
    if !session.check(evidence, "directory_fixture", fixture.is_ok()) {
        return Ok(());
    }
    let ready = await_until(5, || {
        session
            .query("helper.version", &json!({}))
            .is_ok_and(|reply| reply.is_some())
    });
    if !session.check(evidence, "helper_ready", ready) {
        return Ok(());
    }
    let first = "first";
    if !session.run(
        evidence,
        "files.create",
        json!({"name": "a.txt", "content_base64": encode(first), "digest": digest(first)}),
    )? {
        return Ok(());
    }
    let second = "second";
    if !session.run(
        evidence,
        "files.write",
        json!({"name": "a.txt", "content_base64": encode(second), "digest": digest(second)}),
    )? {
        return Ok(());
    }
    let search = session.query("files.search", &json!({"query": "a.txt", "field": "name"}))?;
    let found = search.as_ref().is_some_and(|value| {
        value["result_json"]["items"]
            .as_array()
            .is_some_and(|items| items.len() == 1)
    });
    if !session.check(evidence, "created_item_search", found) {
        return Ok(());
    }
    let read = session.query("files.read", &json!({"root": ROOT, "path": "a.txt"}))?;
    let readback = read.as_ref().is_some_and(|value| {
        value["blobs"]
            .as_array()
            .is_some_and(|blobs| blobs.len() == 1 && blobs[0]["digest"] == json!(digest(second)))
    });
    if !session.check(evidence, "content_digest_readback", readback) {
        return Ok(());
    }
    for (op, args) in [
        ("files.rename", json!({"path": "a.txt", "name": "b.txt"})),
        (
            "files.move",
            json!({"path": "b.txt", "parent": "folder", "name": "c.txt"}),
        ),
        (
            "files.set_tags",
            json!({"path": "folder/c.txt", "tags": ["Bridge Test"]}),
        ),
        ("files.evict", json!({"path": "folder/c.txt"})),
        ("files.download", json!({"path": "folder/c.txt"})),
    ] {
        if !session.run(evidence, op, args)? {
            return Ok(());
        }
    }
    let records = journey::records(&client);
    let found = records.as_ref().is_ok_and(|(_, rows)| {
        rows.iter().any(|row| {
            row["collection"] == "files.item"
                && row["data"]["path"] == "folder/c.txt"
                && row["data"]["tags"] == json!(["Bridge Test"])
                && row["data"]["downloaded"] == true
        })
    });
    if !session.check(
        evidence,
        "captured_metadata_readback",
        records.is_ok() && found,
    ) {
        return Ok(());
    }
    for path in ["../escape", "/outside"] {
        let command = journey::command("files.trash", &json!({"root": ROOT, "path": path}))?;
        let rejected = match session.raw("files.trash", &command)? {
            None => true,
            Some((status, value)) => {
                status != 200 || !value["error"].is_null() || value["state"] == "rejected"
            }
        };
        session.check(evidence, "escape_rejected", rejected);
    }
    session.run(evidence, "files.trash", json!({"path": "folder/c.txt"}))?;
    world.close();
    Ok(())
}

pub fn commands(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    if !SIM.contains(&evidence.scenario.as_str()) {
        return Err(Error::Invalid(format!(
            "file commands not ported for {}",
            evidence.scenario
        )));
    }
    if protocol(world, false, "", evidence)? {
        protocol(world, true, "mcp_", evidence)?;
    }
    Ok(())
}

pub fn real(evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    files_native::checks(evidence, &[&id]);
    Ok(())
}
