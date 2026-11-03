#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{Observed, await_until, converged, gateway_aliases},
    history,
    http::{Client, random_id},
    journey::{effect_count, observed},
    replicas,
    watch::Watcher,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::process::Command;

pub const SCOPE: &str = "messages:read,messages:write,system:read";
pub const COMMANDS: [&str; 13] = [
    "MSG-REACT",
    "MSG-READSTATE-REAL",
    "MSG-REPLY",
    "MSG-RESEND",
    "MSG-SELFTEST",
    "MSG-SEND-FILES",
    "MSG-SEND-GHOST",
    "MSG-SEND-IDENTICAL-MANUAL",
    "MSG-SEND-NEW-HANDLE",
    "MSG-SEND-SELF-EMAIL",
    "MSG-SEND-SELF-TEXT",
    "MSG-UI-INTERFERENCE",
    "MSG-UNSEND",
];
pub const READS: [&str; 2] = ["MSG-RECEIPTS", "MSG-WATCHER-REARM"];

fn action(world: &World, name: &str, key: &str, count: u64) -> bool {
    world
        .control(
            "/messages",
            &json!({"action": name, "key": key, "count": count}),
        )
        .is_ok()
}
fn timestamp_before(seconds: i64) -> String {
    jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_secs(seconds))
        .map_or_else(|_| jiff::Timestamp::now().to_string(), |t| t.to_string())
}
fn data_of(record: &Value) -> Value {
    record["data"].clone()
}

pub fn dispatch(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    if evidence.scenario == "MSG-MEDIA-CLEANUP" {
        crate::cleanup::messages_commands(world, input, evidence)
    } else {
        commands(world, input, evidence)
    }
}

pub fn read(world: &mut World, input: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let expected = id.trim_start_matches("MSG-").to_lowercase();
    if !READS.contains(&id.as_str()) || input["scenario"] != expected.as_str() {
        return Err(Error::Invalid(format!("messages_read not ported for {id}")));
    }
    let gateway = Client::new(world, history::SCOPE)?;
    let observed = Observed::new(gateway, history::SCOPE);
    let mut observers = vec![];
    for client in observed.clients() {
        observers.push(Watcher::start(&client, false)?);
    }
    evidence.check(
        "seed",
        action(world, "seed", "", history::seed_count()),
        "messages seeded in SimApple",
    );
    let settled = await_until(15, || converged(world, &observed));
    evidence.check(
        "history_equals_truth",
        settled,
        "gateway and replica history equal SimApple truth",
    );
    if !settled {
        eprintln!("{}", crate::assertions::diagnose(world, &observed));
        return Ok(());
    }
    if id == "MSG-RECEIPTS" {
        evidence.check(
            "receipts_mutate_existing_record",
            action(world, "receipts", "message-000001", 0)
                && await_until(15, || converged(world, &observed)),
            "receipts applied to an existing message",
        );
        let found = observed.replicas.first().is_some_and(|endpoint| {
            crate::assertions::read_all(&endpoint.client(), false, false).is_ok_and(|read| {
                read.records.iter().any(|record| {
                    record["collection"] == "messages.message"
                        && record["data"]["title"] == "[ab-test] message-000001"
                        && !record["data"]["delivered_at"].is_null()
                        && !record["data"]["read_at"].is_null()
                })
            })
        });
        evidence.check(
            "replica_contains_both_receipts",
            found,
            "replica carries delivered and read receipts",
        );
    } else {
        evidence.check(
            "wal_rotation_followed_by_insert",
            action(world, "watcher-rearm", "after-rearm", 0),
            "message inserted after the watcher disarmed",
        );
        evidence.check(
            "poll_fallback_catches_disarmed_events",
            action(world, "watcher-poll", "", 0) && await_until(15, || converged(world, &observed)),
            "poll fallback delivered the disarmed insert",
        );
        let output = Command::new("bash")
            .arg("packaging/dev/check-messages-capture.sh")
            .output();
        let passed = output.as_ref().is_ok_and(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout).trim() == "all Messages capture checks passed"
        });
        if let Ok(output) = &output
            && !output.status.success()
        {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
        evidence.check(
            "production_native_watcher_fault_checks",
            passed,
            "native Messages capture checks",
        );
    }
    history::tail(world, &observed, &observers, evidence)
}

struct Run<'a> {
    world: &'a World,
    client: Client,
    observed: Observed,
    tag: String,
    text: String,
}
impl Run<'_> {
    fn read(&self, collection: &str) -> Vec<Value> {
        self.client
            .call(
                "records.query",
                &json!({"collection": collection, "limit": 100}),
                20,
            )
            .ok()
            .flatten()
            .and_then(|v| v["records"].as_array().cloned())
            .unwrap_or_default()
    }
    fn record(&self, collection: &str, id: &str) -> Value {
        self.client
            .call(
                "records.get",
                &json!({"collection": collection, "id": id}),
                20,
            )
            .ok()
            .flatten()
            .and_then(|v| v["records"].as_array().and_then(|r| r.first().cloned()))
            .unwrap_or(Value::Null)
    }
    fn converged(&self) -> bool {
        converged(self.world, &self.observed)
    }
    fn command(op: &str, args: &Value, version: u64) -> Result<Value> {
        Ok(
            json!({"command_id": random_id("cmd")?, "op": format!("messages.{op}"), "args": args, "expected_version": version, "wait_ms": 15000}),
        )
    }
    fn target(op: &str, record: &Value, extras: &Value) -> Result<Value> {
        let mut args = json!({"id": record["id"]});
        if let Some(extra) = extras.as_object() {
            for (key, value) in extra {
                args[key] = value.clone();
            }
        }
        let version = if op == "edit" || op == "unsend" || op.starts_with("delete_") {
            record["version"].as_u64().unwrap_or(0)
        } else {
            0
        };
        Self::command(op, &args, version)
    }
    fn submit(&self, command: &Value, mcp: bool) -> Value {
        let op = command["op"].as_str().unwrap_or_default();
        let reply = if mcp {
            self.client.call_mcp_raw(op, command, 40)
        } else {
            self.client.call_raw(op, command, 40)
        };
        reply.ok().flatten().map_or(Value::Null, |(_, value)| value)
    }
    fn succeeded(value: &Value) -> bool {
        value.is_object() && value["error"].is_null()
    }
    fn count(&self, command: &Value, expected: u64) -> bool {
        effect_count(
            self.world,
            command["command_id"].as_str().unwrap_or_default(),
        ) == Some(expected)
    }
    fn state(value: &Value, expected: &str) -> bool {
        Self::succeeded(value) && value["state"] == expected
    }
    fn status(&self, command: &Value, resolve: bool) -> Value {
        self.client
            .call_raw(
                "commands.status",
                &json!({"command_id": command["command_id"], "resolve": resolve}),
                40,
            )
            .ok()
            .flatten()
            .map_or(Value::Null, |(_, value)| value)
    }
    fn send(&self, chat: &str) -> Result<Value> {
        Ok(self.submit(
            &Self::command("send", &json!({"chat_id": chat, "text": self.text}), 0)?,
            false,
        ))
    }
    fn find_message(&self) -> Value {
        self.read("messages.message")
            .into_iter()
            .find(|r| r["data"]["is_from_me"] == true && r["data"]["text"] == self.text.as_str())
            .unwrap_or(Value::Null)
    }
    fn fault(&self, evidence: &mut Evidence, name: &str) {
        evidence.check(
            "fault_armed",
            self.world
                .control("/fault", &json!({"name": name, "ms": 0}))
                .is_ok(),
            "fault armed in SimApple",
        );
    }
    fn mutate(&self, evidence: &mut Evidence, record: &Value, patch: &Value) -> Value {
        let aliases = gateway_aliases(&self.client).unwrap_or_default();
        let truth = self
            .world
            .control("/inspect", &Value::Null)
            .unwrap_or(Value::Null);
        for observation in truth.as_array().into_iter().flatten() {
            let collection = observation["collection"].as_str().unwrap_or_default();
            let key = format!(
                "messages/{collection}/{}",
                observation["native_key"].as_str().unwrap_or_default()
            );
            if observation["collection"] == record["collection"]
                && aliases.get(&key).map(String::as_str) == record["id"].as_str()
            {
                let mut data = observation["data"].clone();
                for (field, value) in patch.as_object().into_iter().flatten() {
                    data[field] = value.clone();
                }
                evidence.check(
                    "external_native_mutation",
                    self.world
                        .control(
                            "/external-change",
                            &json!({"key": observation["native_key"], "collection": collection, "data": data}),
                        )
                        .is_ok(),
                    "native message changed outside the bridge",
                );
                break;
            }
        }
        evidence.check(
            "native_mutation_captured",
            await_until(5, || self.converged()),
            "native mutation captured",
        );
        self.record(
            record["collection"].as_str().unwrap_or_default(),
            record["id"].as_str().unwrap_or_default(),
        )
    }
}
fn attachment_bytes(client: &Client, id: &str, expected: &[u8]) -> bool {
    await_until(10, || {
        let record = client
            .call(
                "records.get",
                &json!({"collection": "messages.attachment", "id": id}),
                20,
            )
            .ok()
            .flatten()
            .and_then(|v| v["records"].as_array().and_then(|r| r.first().cloned()));
        let Some(record) = record else { return false };
        let digest = record["data"]["blob"]["digest"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if digest.is_empty() {
            return false;
        }
        client
            .request(
                "GET",
                &format!("/v1/blobs/{digest}?record={id}"),
                &[],
                None,
                30,
            )
            .is_ok_and(|reply| reply.status == 200 && reply.body == expected)
    })
}
fn stage(client: &Client, name: &str, data: &[u8]) -> Result<Value> {
    let reply = client.request(
        "POST",
        "/v1/blobs",
        &[("Content-Type", "application/octet-stream")],
        Some(data),
        60,
    )?;
    let value: Value = serde_json::from_slice(&reply.body)?;
    match value["upload_id"].as_str() {
        Some(id) if reply.status == 200 => Ok(json!({"name": name, "upload_id": id})),
        _ => Err(Error::Invalid("stage upload rejected".into())),
    }
}
fn synthetic_file(tag: &str) -> Vec<u8> {
    const SIZE: usize = 5 * 1024 * 1024;
    let part = format!("{tag} synthetic attachment\n").into_bytes();
    let mut out = Vec::with_capacity(SIZE + part.len());
    while out.len() < SIZE {
        out.extend_from_slice(&part);
    }
    out.truncate(SIZE);
    out
}
fn suffix(remove: bool) -> &'static str {
    if remove { "_remove" } else { "_add" }
}
fn endpoints(run: &Run) -> Vec<Client> {
    let mut out = vec![run.client.clone()];
    out.extend(run.observed.replicas.iter().map(replicas::Endpoint::client));
    out
}

#[allow(clippy::cognitive_complexity)]
pub fn commands(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    if !COMMANDS.contains(&id.as_str()) {
        return Err(Error::Invalid(format!(
            "messages_commands not ported for {id}"
        )));
    }
    let tag = random_id("run")?;
    let client = Client::new(world, SCOPE)?;
    let run = Run {
        world,
        observed: Observed::new(client.clone(), SCOPE),
        client,
        text: format!("[ab-test {tag}] message"),
        tag,
    };
    let seeded = u64::from(["MSG-SELFTEST", "MSG-READSTATE-REAL"].contains(&id.as_str()));
    evidence.check(
        "seed_self_chat",
        action(world, "seed", "", seeded),
        "self chat seeded",
    );
    evidence.check(
        "fixture_captured",
        await_until(10, || run.converged()),
        "fixture captured",
    );
    let baseline = run.read("messages.message");
    let chats = run.read("messages.chat");
    let Some(chat) = chats.first().cloned() else {
        evidence.check("self_chat", false, "no self chat");
        return Ok(());
    };
    let chat_id = chat["id"].as_str().unwrap_or_default().to_owned();
    let text = run.text.clone();
    match id.as_str() {
        "MSG-SELFTEST" => {
            let mut owned = std::collections::BTreeSet::new();
            let mut own = |result: &Value| {
                for id in result["observed_record_ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    owned.insert(id.as_str().unwrap_or_default().to_owned());
                }
            };
            let created = run.send(&chat_id)?;
            own(&created);
            evidence.check("send_text", observed(&created), "text sent");
            let mut message = run.find_message();
            if message.is_null() {
                evidence.check("owned_self_message", false, "sent message not found");
                return Ok(());
            }
            let file = stage(&run.client, &format!("ab-{}.txt", run.tag), text.as_bytes())?;
            let result = run.submit(
                &Run::command("send", &json!({"chat_id": chat_id, "files": [file]}), 0)?,
                false,
            );
            own(&result);
            evidence.check("send_file", observed(&result), "file sent");
            let result = run.submit(
                &Run::target("reply", &message, &json!({"text": format!("{text} reply")}))?,
                false,
            );
            own(&result);
            evidence.check("reply", observed(&result), "reply sent");
            for kind in ["love", "emoji"] {
                for remove in [false, true] {
                    let result = run.submit(
                        &Run::target(
                            "react",
                            &message,
                            &json!({"reaction": kind, "emoji": "🧪", "remove": remove}),
                        )?,
                        false,
                    );
                    evidence.check(
                        &format!("{kind}{}", suffix(remove)),
                        observed(&result),
                        "reaction sent",
                    );
                }
            }
            message = run.record(
                "messages.message",
                message["id"].as_str().unwrap_or_default(),
            );
            let result = run.submit(
                &Run::target("edit", &message, &json!({"text": format!("{text} edited")}))?,
                false,
            );
            evidence.check("edit", observed(&result), "message edited");
            for op in ["mark_unread", "mark_read"] {
                let result = run.submit(&Run::target(op, &chat, &Value::Null)?, false);
                evidence.check(op, observed(&result), "read state changed");
            }
            let unsend = run.send(&chat_id)?;
            own(&unsend);
            let ids = unsend["observed_record_ids"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            evidence.check(
                "unsend_fixture",
                observed(&unsend) && ids.len() == 1,
                "unsend fixture sent",
            );
            if ids.len() == 1 {
                let fixture = run.record("messages.message", ids[0].as_str().unwrap_or_default());
                let result = run.submit(&Run::target("unsend", &fixture, &Value::Null)?, false);
                evidence.check("unsend", observed(&result), "message unsent");
            }
            for message in run.read("messages.message") {
                let own_id = owned.contains(message["id"].as_str().unwrap_or_default());
                let prefix = format!("[ab-test {}]", run.tag);
                let tagged = message["data"]["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with(&prefix));
                if !own_id && !tagged {
                    continue;
                }
                let result = run.submit(
                    &Run::target("delete_message", &message, &Value::Null)?,
                    false,
                );
                evidence.check(
                    "owned_message_cleanup",
                    observed(&result),
                    "owned message deleted",
                );
            }
            let remaining = run.read("messages.message");
            let preserved = remaining.len() == baseline.len()
                && baseline.iter().all(|before| {
                    remaining
                        .iter()
                        .find(|after| after["id"] == before["id"])
                        .is_some_and(|after| after["version"] == before["version"])
                });
            evidence.check(
                "existing_residue_preserved",
                preserved,
                "baseline unchanged",
            );
            evidence.check(
                "self_chat_preserved",
                run.record("messages.chat", &chat_id)["id"] == chat["id"],
                "self chat kept",
            );
        }
        "MSG-RESEND" => {
            run.fault(evidence, "delivery-failed");
            let original = Run::command("send", &json!({"chat_id": chat_id, "text": text}), 0)?;
            let failed = run.submit(&original, false);
            evidence.check(
                "deterministic_delivery_failure",
                Run::state(&failed, "delivery_failed") && run.count(&original, 1),
                "delivery failure is deterministic",
            );
            if !Run::state(&failed, "delivery_failed") {
                return Ok(());
            }
            let message = run.find_message();
            let resend = Run::command(
                "send",
                &json!({"chat_id": chat_id, "text": text, "resend_of": message["id"]}),
                0,
            )?;
            let result = run.submit(&resend, false);
            evidence.check(
                "explicit_resend_is_new_observed_effect",
                observed(&result) && run.count(&resend, 1) && run.count(&original, 1),
                "resend creates a new observed effect",
            );
            let retry = run.submit(&resend, false);
            evidence.check(
                "resend_command_replay_is_idempotent",
                Run::succeeded(&retry)
                    && retry["created_at"] == result["created_at"]
                    && run.count(&resend, 1),
                "replayed resend is idempotent",
            );
            let wrong = Run::command(
                "send",
                &json!({"chat_id": chat_id, "text": text, "resend_of": "missing"}),
                0,
            )?;
            let rejected = run.submit(&wrong, false);
            evidence.check(
                "resend_missing_target_rejected",
                !Run::succeeded(&rejected) && run.count(&wrong, 0),
                "missing resend target rejected",
            );
            let successful = run
                .read("messages.message")
                .into_iter()
                .find(|item| item["data"]["is_from_me"] == true && item["data"]["error_code"] == 0)
                .unwrap_or(Value::Null);
            let wrong = Run::command(
                "send",
                &json!({"chat_id": chat_id, "text": text, "resend_of": successful["id"]}),
                0,
            )?;
            let rejected = run.submit(&wrong, false);
            evidence.check(
                "resend_successful_target_rejected",
                !successful.is_null() && !Run::succeeded(&rejected) && run.count(&wrong, 0),
                "successful resend target rejected",
            );
        }
        "MSG-SEND-GHOST" | "MSG-SEND-IDENTICAL-MANUAL" => {
            let identical = id.contains("IDENTICAL");
            let (name, want) = if identical {
                ("identical-manual-send", "indeterminate")
            } else {
                ("ghost-send", "delivery_failed")
            };
            run.fault(evidence, name);
            let command = Run::command("send", &json!({"chat_id": chat_id, "text": text}), 0)?;
            let result = run.submit(&command, false);
            evidence.check(
                "truthful_disposition",
                Run::state(&result, want) && run.count(&command, 1),
                "disposition matches the native evidence",
            );
            let retry = run.submit(&command, false);
            evidence.check(
                "no_automatic_or_caller_redispatch",
                Run::state(&retry, want) && run.count(&command, 1),
                "no redispatch",
            );
            if identical {
                let resolved = run.status(&command, true);
                evidence.check(
                    "resolve_never_guesses_identical_candidate",
                    Run::state(&resolved, want) && run.count(&command, 1),
                    "resolve keeps the ambiguous disposition",
                );
            }
        }
        "MSG-SEND-SELF-TEXT" | "MSG-SEND-SELF-EMAIL" | "MSG-SEND-FILES" | "MSG-SEND-NEW-HANDLE" => {
            for protocol in ["rest", "mcp"] {
                let mcp = protocol == "mcp";
                let mut args = json!({"chat_id": chat_id, "text": format!("{text}{protocol}")});
                if id == "MSG-SEND-NEW-HANDLE" || id == "MSG-SEND-SELF-EMAIL" {
                    args = json!({"handles": ["second-self@example.com"], "service": "iMessage", "text": format!("{text}{protocol}")});
                }
                if id == "MSG-SEND-FILES" {
                    let large = stage(&run.client, "one.txt", &synthetic_file(&text));
                    evidence.check(
                        &format!("{protocol}_five_mib_upload_staged"),
                        large.is_ok(),
                        "5 MiB upload staged",
                    );
                    let Ok(large) = large else { return Ok(()) };
                    args["files"] = json!([large, stage(&run.client, "two.txt", b"two")?]);
                }
                let command = Run::command("send", &args, 0)?;
                let result = run.submit(&command, mcp);
                evidence.check(
                    &format!("{protocol}_locally_observed_with_checkpoint"),
                    observed(&result) && run.count(&command, 1),
                    "send observed locally with a checkpoint",
                );
                if result["state"] == "observed" {
                    let resolved = run.status(&command, true);
                    evidence.check(
                        &format!("{protocol}_delivered_only_after_native_evidence"),
                        Run::state(&resolved, "delivered"),
                        "delivered after native evidence",
                    );
                }
                evidence.check(
                    &format!("{protocol}_replica_converges"),
                    await_until(5, || run.converged()),
                    "replica converges",
                );
                if id == "MSG-SEND-FILES" {
                    let parts = result["evidence"]["parts"]
                        .as_array()
                        .or_else(|| result["parts"].as_array())
                        .map_or(0, Vec::len);
                    evidence.check(
                        &format!("{protocol}_three_part_outcomes"),
                        parts == 3,
                        "text and two files have outcomes",
                    );
                    let attachments = run.read("messages.attachment");
                    evidence.check(
                        &format!("{protocol}_attachment_records"),
                        attachments.len() >= 2,
                        "attachment records exist",
                    );
                    for attachment in &attachments {
                        let expected = if attachment["data"]["filename"] == "two.txt" {
                            b"two".to_vec()
                        } else {
                            synthetic_file(&text)
                        };
                        for endpoint in endpoints(&run) {
                            evidence.check(
                                &format!("{protocol}_authorized_attachment_bytes"),
                                attachment_bytes(
                                    &endpoint,
                                    attachment["id"].as_str().unwrap_or_default(),
                                    &expected,
                                ),
                                "attachment bytes match on every endpoint",
                            );
                        }
                    }
                }
            }
            let bad = Run::command(
                "send",
                &json!({"handles": ["other@example.com"], "service": "iMessage", "text": text}),
                0,
            )?;
            let rejected = run.submit(&bad, false);
            evidence.check(
                "non_self_refused_before_effect",
                !Run::succeeded(&rejected) && run.count(&bad, 0),
                "non-self recipient refused",
            );
        }
        _ => default_flow(&run, evidence, &id, &chat, &chat_id)?,
    }
    if id == "MSG-UNSEND" {
        window(&run, evidence, &chat_id)?;
    }
    evidence.check(
        "truth_and_replica_convergence",
        await_until(10, || run.converged()),
        "truth and replica converge",
    );
    Ok(())
}

fn default_flow(
    run: &Run,
    evidence: &mut Evidence,
    id: &str,
    chat: &Value,
    chat_id: &str,
) -> Result<()> {
    let text = run.text.clone();
    let created = run.send(chat_id)?;
    evidence.check(
        "target_created",
        observed(&created),
        "target message created",
    );
    let message = run.find_message();
    if message.is_null() {
        evidence.check("target_message", false, "target message not found");
        return Ok(());
    }
    match id {
        "MSG-REPLY" => {
            let result = run.submit(
                &Run::target("reply", &message, &json!({"text": format!("{text} reply")}))?,
                false,
            );
            evidence.check("threaded_reply", observed(&result), "reply sent");
            let linked = run.read("messages.message").iter().any(|reply| {
                reply["data"]["thread_originator_id"] == message["id"]
                    && reply["data"]["text"] == format!("{text} reply").as_str()
            });
            evidence.check(
                "reply_points_to_exact_target",
                linked,
                "reply points to target",
            );
        }
        "MSG-REACT" => {
            for reaction in ["love", "emoji"] {
                for remove in [false, true] {
                    let result = run.submit(
                        &Run::target(
                            "react",
                            &message,
                            &json!({"reaction": reaction, "emoji": "🧪", "remove": remove}),
                        )?,
                        false,
                    );
                    evidence.check(
                        &format!("reaction_{reaction}{}", suffix(remove)),
                        observed(&result),
                        "reaction submitted",
                    );
                    let matched = run.read("messages.reaction").iter().any(|record| {
                        let data = data_of(record);
                        data["message_id"] == message["id"]
                            && data["kind"] == reaction
                            && data["removed"] == remove
                            && (reaction != "emoji" || data["emoji"] == "🧪")
                    });
                    evidence.check("reaction_native_fields", matched, "reaction fields match");
                }
            }
        }
        "MSG-UNSEND" => {
            let result = run.submit(&Run::target("unsend", &message, &Value::Null)?, false);
            evidence.check("unsend_within_window", observed(&result), "unsend accepted");
            let current = run.record(
                "messages.message",
                message["id"].as_str().unwrap_or_default(),
            );
            evidence.check(
                "native_retracted_evidence",
                !current["data"]["retracted_at"].is_null(),
                "native retraction recorded",
            );
        }
        "MSG-READSTATE-REAL" => {
            for op in ["mark_unread", "mark_read"] {
                let result = run.submit(&Run::target(op, chat, &Value::Null)?, false);
                evidence.check(op, observed(&result), "read state changed");
                let current = run.record("messages.chat", chat_id);
                evidence.check(
                    "native_chat_read_state",
                    current["data"]["is_unread"] == (op == "mark_unread"),
                    "native chat read state matches",
                );
            }
        }
        "MSG-UI-INTERFERENCE" => {
            for name in ["ui-interference-before", "ui-interference"] {
                run.fault(evidence, name);
                let command = Run::target("reply", &message, &json!({"text": text}))?;
                let result = run.submit(&command, false);
                if name == "ui-interference-before" {
                    let status = run.status(&command, false);
                    evidence.check(
                        name,
                        !Run::succeeded(&result)
                            && status["state"] == "rejected"
                            && run.count(&command, 0),
                        "interference before prepare rejects without effect",
                    );
                } else {
                    evidence.check(
                        name,
                        Run::state(&result, "indeterminate") && run.count(&command, 0),
                        "interference after prepare is indeterminate",
                    );
                }
            }
        }
        _ => {
            return Err(Error::Invalid(format!(
                "messages_commands not ported for {id}"
            )));
        }
    }
    Ok(())
}

fn window(run: &Run, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let created = run.send(chat_id)?;
    evidence.check(
        "window_target_created",
        observed(&created),
        "window target sent",
    );
    let aliases = gateway_aliases(&run.client).unwrap_or_default();
    let native = created["evidence"]["native_key"]
        .as_str()
        .unwrap_or_default();
    let record_id = aliases
        .get(&format!("messages/messages.message/{native}"))
        .cloned()
        .unwrap_or_default();
    let fresh = run.record("messages.message", &record_id);
    let old = run.mutate(evidence, &fresh, &json!({"sent_at": timestamp_before(180)}));
    let command = Run::target("unsend", &old, &Value::Null)?;
    let rejected = run.submit(&command, false);
    evidence.check(
        "outside_apple_window_rejected_before_effect",
        !Run::succeeded(&rejected) && run.count(&command, 0),
        "unsend outside the window rejected before effect",
    );
    let incoming = run.mutate(
        evidence,
        &fresh,
        &json!({"sent_at": jiff::Timestamp::now().to_string(), "is_from_me": false}),
    );
    let command = Run::target("unsend", &incoming, &Value::Null)?;
    let rejected = run.submit(&command, false);
    evidence.check(
        "incoming_edit_unsend_rejected_before_effect",
        !Run::succeeded(&rejected) && run.count(&command, 0),
        "unsend of an incoming message rejected before effect",
    );
    Ok(())
}
