use crate::{
    assertions::{Observed, await_until, converged, gateway_aliases},
    http::{Client, random_id},
    journey::effect_count,
    replicas::{self, Endpoint},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub const SCOPE: &str = "messages:read,messages:write,system:read";
pub const ALL: [&str; 9] = [
    "MSG-CAPABILITY-MATRIX",
    "MSG-COMMAND-OBSERVED-IDS",
    "MSG-COMMAND-PARTS",
    "MSG-DELETE",
    "MSG-DELETE-REAL",
    "MSG-EDIT",
    "MSG-FAILED-OBSERVED-IDS",
    "MSG-MARK-READ",
    "MSG-OWNER-TOKEN-SELFCHAT",
];
const OPERATIONS: [&str; 9] = [
    "messages.send",
    "messages.reply",
    "messages.react",
    "messages.edit",
    "messages.unsend",
    "messages.mark_read",
    "messages.mark_unread",
    "messages.delete_message",
    "messages.delete_chat",
];

#[derive(Clone)]
struct Api {
    client: Client,
    mcp: bool,
}
impl Api {
    fn rest(client: Client) -> Self {
        Self { client, mcp: false }
    }
    fn call(&self, op: &str, input: &Value) -> Option<Value> {
        if self.mcp {
            self.client.call_mcp(op, input, 40).ok().flatten()
        } else {
            self.client.call(op, input, 40).ok().flatten()
        }
    }
    fn raw(&self, op: &str, input: &Value) -> Value {
        let reply = if self.mcp {
            self.client.call_mcp_raw(op, input, 40)
        } else {
            self.client.call_raw(op, input, 40)
        };
        reply.ok().flatten().map_or(Value::Null, |(_, value)| value)
    }
    fn status(&self, id: &str, resolve: bool) -> Option<Value> {
        self.call(
            "commands.status",
            &json!({"command_id": id, "resolve": resolve}),
        )
    }
}
struct Ctx<'a> {
    world: &'a World,
    evidence: &'a mut Evidence,
    run: String,
    observed: Observed,
    gateway: Api,
}
impl Ctx<'_> {
    fn add(&mut self, name: &str, passed: bool) {
        self.evidence.check(name, passed, name);
    }
    fn settle(&self, seconds: u64) -> bool {
        await_until(seconds, || converged(self.world, &self.observed))
    }
    fn fault(&self, name: &str) -> bool {
        self.world
            .control("/fault", &json!({"name": name, "ms": 0}))
            .is_ok()
    }
    fn count(&self, command: &Value, expected: u64) -> bool {
        effect_count(self.world, &id_of(command)) == Some(expected)
    }
    fn tag(&self, suffix: &str) -> String {
        format!("[ab-test {}] {suffix}", self.run)
    }
}
fn id_of(command: &Value) -> String {
    command["command_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
fn command(op: &str, args: &Value, version: u64) -> Result<Value> {
    let mut value = json!({"command_id": random_id("cmd")?, "op": format!("messages.{op}"), "args": args, "wait_ms": 15000});
    if version != 0 {
        value["expected_version"] = json!(version);
    }
    Ok(value)
}
fn observed(reply: &Value) -> bool {
    reply["error"].is_null()
        && reply["state"] == "observed"
        && reply["commit_seq"].as_u64().unwrap_or(0) > 0
}
fn rejected(reply: &Value) -> bool {
    !reply["error"].is_null()
}
fn list(api: &Api, collection: &str) -> Vec<Value> {
    api.call(
        "records.query",
        &json!({"collection": collection, "limit": 100}),
    )
    .and_then(|page| page["records"].as_array().cloned())
    .unwrap_or_default()
}
fn get_page(api: &Api, collection: &str, id: &str) -> Option<Value> {
    api.call("records.get", &json!({"collection": collection, "id": id}))
}
fn record(api: &Api, collection: &str, id: &str) -> Value {
    get_page(api, collection, id)
        .and_then(|page| page["records"].as_array().and_then(|r| r.first().cloned()))
        .unwrap_or(Value::Null)
}
fn rid(record: &Value) -> String {
    record["id"].as_str().unwrap_or_default().to_owned()
}
fn ids(value: &Value) -> Vec<String> {
    value["observed_record_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect()
}
fn seq(value: &Value) -> u64 {
    value["commit_seq"].as_u64().unwrap_or(0)
}
fn cursor_seq(value: &Value) -> u64 {
    value["cursor"]["sequence"].as_u64().unwrap_or(0)
}
fn target(op: &str, record: &Value, extras: &Value) -> Result<Value> {
    let mut args = json!({"id": record["id"]});
    for (key, value) in extras.as_object().into_iter().flatten() {
        args[key] = value.clone();
    }
    let version = if op == "edit" || op == "unsend" || op.starts_with("delete_") {
        record["version"].as_u64().unwrap_or(0)
    } else {
        0
    };
    command(op, &args, version)
}
fn alias(aliases: &crate::assertions::Aliases, native: &str) -> String {
    aliases
        .get(&format!("messages/messages.message/{native}"))
        .cloned()
        .unwrap_or_default()
}
fn endpoint_label(index: usize) -> &'static str {
    if index == 0 { "gateway" } else { "replica" }
}
fn stage(client: &Client, data: &[u8]) -> Option<String> {
    let reply = client
        .request(
            "POST",
            "/v1/blobs",
            &[("Content-Type", "application/octet-stream")],
            Some(data),
            60,
        )
        .ok()?;
    let value: Value = serde_json::from_slice(&reply.body).ok()?;
    if reply.status != 200 {
        return None;
    }
    value["upload_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}
fn upload(client: &Client, name: &str, data: &[u8]) -> Option<Value> {
    stage(client, data).map(|id| json!({"name": name, "upload_id": id}))
}
fn inline(name: &str, data: &[u8]) -> Value {
    use base64::Engine;
    json!({"name": name, "data": base64::engine::general_purpose::STANDARD.encode(data)})
}
fn await_ms(ms: u64, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(25));
    }
}
fn effects_of(world: &World, ids: &[&str]) -> Option<Vec<u64>> {
    let effects = world.control("/effects", &Value::Null).ok()?;
    Some(
        ids.iter()
            .map(|id| effects[*id].as_u64().unwrap_or(0))
            .collect(),
    )
}
fn data_of(record: &Value) -> &Value {
    &record["data"]
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let id = evidence.scenario.clone();
    let gateway_client = Client::new(world, SCOPE)?;
    let observed_set = Observed::new(gateway_client.clone(), SCOPE);
    let mut ctx = Ctx {
        world,
        evidence,
        run: random_id("run")?,
        observed: observed_set,
        gateway: Api::rest(gateway_client),
    };
    let seed = u64::from(id == "MSG-DELETE-REAL");
    let seeded = ctx
        .world
        .control("/messages", &json!({"action": "seed", "count": seed}))
        .is_ok();
    ctx.add("seed_self_chat", seeded);
    let captured = ctx.settle(10);
    ctx.add("fixture_captured", captured);
    let baseline = list(&ctx.gateway, "messages.message");
    let chats = list(&ctx.gateway, "messages.chat");
    let Some(chat) = chats.first().cloned() else {
        ctx.add("self_chat", false);
        return Ok(());
    };
    let text = ctx.tag("message");
    let api = ctx.gateway.clone();
    match id.as_str() {
        "MSG-OWNER-TOKEN-SELFCHAT" => owner_token(&mut ctx, &api, &chat)?,
        "MSG-COMMAND-PARTS" => parts(&mut ctx, &chat)?,
        "MSG-FAILED-OBSERVED-IDS" => failed_ids(&mut ctx, &chat)?,
        "MSG-COMMAND-OBSERVED-IDS" => observed_ids(&mut ctx, &chat)?,
        "MSG-CAPABILITY-MATRIX" => {
            capability_matrix(&mut ctx, &api, &chat, &text)?;
        }
        _ => {
            let first = command(
                "send",
                &json!({"chat_id": chat["id"], "text": text.clone()}),
                0,
            )?;
            let reply = api.raw("messages.send", &first);
            ctx.add("target_created", observed(&reply));
            let message = list(&api, "messages.message")
                .into_iter()
                .find(|r| data_of(r)["is_from_me"] == true && data_of(r)["text"] == text);
            let Some(message) = message else {
                ctx.add("target_message", false);
                return Ok(());
            };
            match id.as_str() {
                "MSG-EDIT" => edit(&mut ctx, &api, message, &text)?,
                "MSG-MARK-READ" => mark_read(&mut ctx, &api, &chat)?,
                _ => delete(&mut ctx, &api, &message, &chat, &baseline, &id)?,
            }
        }
    }
    if id == "MSG-EDIT" {
        window(&mut ctx, &api, &chat, &text)?;
    }
    let converged = ctx.settle(10);
    ctx.add("truth_and_replica_convergence", converged);
    Ok(())
}

fn owner_token(ctx: &mut Ctx, api: &Api, chat: &Value) -> Result<()> {
    let untagged = command(
        "send",
        &json!({"chat_id": chat["id"], "text": "synthetic owner self-chat"}),
        0,
    )?;
    let result = api.raw("messages.send", &untagged);
    ctx.add("local_token_untagged_self_chat_observed", observed(&result));
    let single = ctx.count(&untagged, 1);
    ctx.add("self_chat_single_effect", single);
    let fixture = ctx
        .world
        .control("/messages", &json!({"action": "nonself-chat"}))
        .is_ok();
    ctx.add("nonself_fixture", fixture);
    let captured = ctx.settle(5);
    ctx.add("nonself_captured", captured);
    let other = list(api, "messages.chat")
        .into_iter()
        .map(|r| rid(&r))
        .rfind(|id| *id != rid(chat))
        .unwrap_or_default();
    ctx.add("nonself_chat_available", !other.is_empty());
    let mut denied = command(
        "send",
        &json!({"chat_id": other, "text": "synthetic untagged text"}),
        0,
    )?;
    denied["origin"] = json!("interactive");
    let reply = api.raw("messages.send", &denied);
    let spoof = rejected(&reply)
        && reply["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("verified own recipients"));
    ctx.evidence.check(
        "token_cannot_spoof_interactive_for_nonself",
        spoof,
        &reply.to_string(),
    );
    let zero = ctx.count(&denied, 0);
    ctx.add("nonself_zero_effects", zero);
    Ok(())
}

fn edit(ctx: &mut Ctx, api: &Api, mut message: Value, text: &str) -> Result<()> {
    for i in 0..6 {
        message = record(api, "messages.message", &rid(&message));
        let edited = format!("{text}{}", "!".repeat(i + 1));
        let c = target("edit", &message, &json!({"text": edited}))?;
        let reply = api.raw("messages.edit", &c);
        if i < 5 {
            ctx.add("edit_within_limits", observed(&reply));
            let current = record(api, "messages.message", &rid(&message));
            let data = data_of(&current);
            let history = data["edit_history"].as_array().map_or(0, Vec::len);
            ctx.add(
                "edit_history_and_new_text",
                data["text"] == edited && history == i + 1,
            );
            let parts = data["parts"].as_array().cloned().unwrap_or_default();
            ctx.add(
                "edited_part_text_matches_capture",
                parts.len() == 1 && parts[0]["index"] == 0 && parts[0]["text"] == data["text"],
            );
        } else {
            let zero = ctx.count(&c, 0);
            ctx.add("sixth_edit_rejected", rejected(&reply) && zero);
        }
    }
    Ok(())
}

fn mark_read(ctx: &mut Ctx, api: &Api, chat: &Value) -> Result<()> {
    for op in ["mark_unread", "mark_read"] {
        let c = target(op, chat, &Value::Null)?;
        let reply = api.raw(&format!("messages.{op}"), &c);
        ctx.add(op, observed(&reply));
        let current = record(api, "messages.chat", &rid(chat));
        ctx.add(
            "native_chat_read_state",
            data_of(&current)["is_unread"] == (op == "mark_unread"),
        );
    }
    Ok(())
}

fn delete(
    ctx: &mut Ctx,
    api: &Api,
    message: &Value,
    chat: &Value,
    baseline: &[Value],
    id: &str,
) -> Result<()> {
    let c = target("delete_message", message, &Value::Null)?;
    let reply = api.raw("messages.delete_message", &c);
    let gone = record(api, "messages.message", &rid(message)).is_null();
    ctx.add("delete_message_complete_scan", observed(&reply) && gone);
    if id == "MSG-DELETE-REAL" {
        let prefix = format!("[ab-test {}]", ctx.run);
        for item in list(api, "messages.message") {
            if !data_of(&item)["text"]
                .as_str()
                .is_some_and(|t| t.starts_with(&prefix))
            {
                continue;
            }
            let c = target("delete_message", &item, &Value::Null)?;
            let reply = api.raw("messages.delete_message", &c);
            ctx.add("owned_echo_cleanup", observed(&reply));
        }
        let current = list(api, "messages.message");
        let missing = baseline
            .iter()
            .filter(|before| !current.iter().any(|after| after["id"] == before["id"]))
            .count();
        let changed = baseline
            .iter()
            .filter(|before| {
                current.iter().any(|after| {
                    after["id"] == before["id"] && after["version"] != before["version"]
                })
            })
            .count();
        ctx.add(
            "existing_residue_preserved",
            current.len() == baseline.len() && missing == 0 && changed == 0,
        );
    }
    if id == "MSG-DELETE" {
        let chat = record(api, "messages.chat", &rid(chat));
        let c = target("delete_chat", &chat, &Value::Null)?;
        let reply = api.raw("messages.delete_chat", &c);
        let gone = record(api, "messages.chat", &rid(&chat)).is_null();
        ctx.add("delete_chat_complete_scan", observed(&reply) && gone);
    }
    Ok(())
}

fn mutate(ctx: &mut Ctx, api: &Api, fresh: &Value, patch: &Value) -> Value {
    let aliases = gateway_aliases(&ctx.observed.gateway).unwrap_or_default();
    let truth = ctx
        .world
        .control("/inspect", &Value::Null)
        .unwrap_or_default();
    for observation in truth.as_array().into_iter().flatten() {
        let collection = observation["collection"].as_str().unwrap_or_default();
        let native = observation["native_key"].as_str().unwrap_or_default();
        let key = format!("messages/{collection}/{native}");
        if collection != fresh["collection"]
            || aliases.get(&key) != fresh["id"].as_str().map(str::to_owned).as_ref()
        {
            continue;
        }
        let mut data = observation["data"].clone();
        for (field, value) in patch.as_object().into_iter().flatten() {
            data[field] = value.clone();
        }
        let done = ctx
            .world
            .control(
                "/external-change",
                &json!({"key": native, "collection": collection, "data": data}),
            )
            .is_ok();
        ctx.add("external_native_mutation", done);
        break;
    }
    let captured = ctx.settle(5);
    ctx.add("native_mutation_captured", captured);
    record(
        api,
        fresh["collection"].as_str().unwrap_or_default(),
        &rid(fresh),
    )
}

fn timestamp(offset_seconds: i64) -> String {
    jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(offset_seconds))
        .map(|t| t.to_string())
        .unwrap_or_default()
}

fn window(ctx: &mut Ctx, api: &Api, chat: &Value, text: &str) -> Result<()> {
    let c = command("send", &json!({"chat_id": chat["id"], "text": text}), 0)?;
    let reply = api.raw("messages.send", &c);
    ctx.add("window_target_created", observed(&reply));
    let native = reply["evidence"]["native_key"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let aliases = gateway_aliases(&ctx.observed.gateway).unwrap_or_default();
    let fresh = record(api, "messages.message", &alias(&aliases, &native));
    let extra = json!({"text": format!("{text} outside window")});
    let old = mutate(ctx, api, &fresh, &json!({"sent_at": timestamp(-16 * 60)}));
    let c = target("edit", &old, &extra)?;
    let reply = api.raw("messages.edit", &c);
    let zero = ctx.count(&c, 0);
    ctx.add(
        "outside_apple_window_rejected_before_effect",
        rejected(&reply) && zero,
    );
    let incoming = mutate(
        ctx,
        api,
        &fresh,
        &json!({"sent_at": timestamp(0), "is_from_me": false}),
    );
    let c = target("edit", &incoming, &extra)?;
    let reply = api.raw("messages.edit", &c);
    let zero = ctx.count(&c, 0);
    ctx.add(
        "incoming_edit_unsend_rejected_before_effect",
        rejected(&reply) && zero,
    );
    Ok(())
}

fn operation_catalog() -> Option<Value> {
    let probe =
        std::env::temp_dir().join(format!("operation-catalog-probe-{}", random_id("p").ok()?));
    let output = Command::new(replicas::binary().ok()?)
        .args(["lab-operation-catalog", "--home"])
        .arg(&probe)
        .env("APPLE_BRIDGE_SECRET_STORE", "file")
        .output()
        .ok()?;
    if !output.status.success() || probe.exists() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}
fn messages_coverage(api: &Api) -> (bool, Vec<Value>) {
    let status = api.call("sources.status", &json!({}));
    let operations = status
        .as_ref()
        .and_then(|page| page["records"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|record| data_of(record)["source"] == "messages")
        .flat_map(|record| {
            data_of(&record)["coverage"]["operations"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .collect();
    (status.is_some(), operations)
}
fn capability_matrix(ctx: &mut Ctx, api: &Api, chat: &Value, text: &str) -> Result<()> {
    let (available, operations) = messages_coverage(api);
    ctx.add("status_available", available);
    for op in OPERATIONS {
        let capability = operations.iter().find(|o| o["operation"] == op);
        let ok = capability.is_some_and(|c| {
            c["proven"] == true && c["reason"].as_str().is_some_and(|r| !r.is_empty())
        });
        ctx.add(&format!("capability_reason_{op}"), ok);
    }
    let catalog = operation_catalog();
    for op in OPERATIONS {
        let ok = catalog.as_ref().is_some_and(|catalog| {
            let descriptor = &catalog[op];
            descriptor["scope"] == "messages:write"
                && descriptor["schema"]
                    .as_object()
                    .is_some_and(|schema| !schema.is_empty())
        });
        ctx.add(&format!("descriptor_{op}"), ok);
    }
    let armed = ctx
        .world
        .control("/messages", &json!({"action": "capability-unproven"}))
        .is_ok();
    ctx.add("capability_unproven", armed);
    let c = command("send", &json!({"chat_id": chat["id"], "text": text}), 0)?;
    let reply = api.raw("messages.send", &c);
    let zero = ctx.count(&c, 0);
    ctx.evidence.check(
        "unproven_rejects_without_effect",
        reply["error"]["code"] == "capability_unproven" && zero,
        &format!("{reply} zero={zero}"),
    );
    let (_, after) = messages_coverage(api);
    let rejected_capabilities = after
        .iter()
        .filter(|c| c["proven"] != true && c["reason"] == "capability_unproven")
        .count();
    ctx.add(
        "unproven_matrix_has_per_operation_reasons",
        rejected_capabilities == OPERATIONS.len(),
    );
    Ok(())
}

fn observed_ids(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    let replicas = replicas::endpoints();
    let mut clients = vec![ctx.gateway.client.clone()];
    clients.extend(replicas.iter().map(Endpoint::client));
    for (index, client) in clients.into_iter().enumerate() {
        let mut api = Api::rest(client);
        let label = endpoint_label(index);
        let mut previous = Value::Null;
        for op in ["send", "reply", "edit"] {
            let mut args = json!({"text": ctx.tag(&format!("{label} {op}"))});
            let mut version = 0;
            if op == "send" {
                args["chat_id"] = chat["id"].clone();
                let armed = ctx.fault("receipt-before-outcome");
                ctx.add(&format!("{label}_receipt_before_outcome_armed"), armed);
            } else {
                args["id"] = previous["id"].clone();
            }
            if op == "edit" {
                version = previous["version"].as_u64().unwrap_or(0);
            }
            let c = command(op, &args, version)?;
            let result = api.raw(&format!("messages.{op}"), &c);
            ctx.add(&format!("{label}_{op}_observed"), observed(&result));
            let native = result["evidence"]["native_key"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let aliases = gateway_aliases(&ctx.observed.gateway);
            let expected = aliases
                .as_ref()
                .map(|a| alias(a, &native))
                .unwrap_or_default();
            ctx.add(
                &format!("{label}_{op}_native_alias_exists"),
                aliases.is_ok() && !expected.is_empty(),
            );
            let mut prior: Vec<String> = vec![];
            for protocol in ["rest", "mcp"] {
                api.mcp = protocol == "mcp";
                let status = api.status(&id_of(&c), false);
                let got = status.as_ref().map(ids).unwrap_or_default();
                let exact = status.as_ref().is_some_and(|s| {
                    !expected.is_empty() && got == [expected.clone()] && seq(s) == seq(&result)
                });
                ctx.add(&format!("{label}_{op}_{protocol}_exact_observed_id"), exact);
                if protocol == "mcp" {
                    ctx.add(&format!("{label}_{op}_rest_mcp_parity"), prior == got);
                }
                prior = got;
            }
            let page = get_page(&api, "messages.message", &expected);
            let records = page
                .as_ref()
                .and_then(|p| p["records"].as_array().cloned())
                .unwrap_or_default();
            ctx.add(
                &format!("{label}_{op}_immediately_readable_at_commit"),
                page.as_ref()
                    .is_some_and(|p| records.len() == 1 && cursor_seq(p) >= seq(&result)),
            );
            if records.len() != 1 {
                return Ok(());
            }
            previous = records[0].clone();
            if op == "send" {
                let delivered = data_of(&previous)["delivered_at"]
                    .as_str()
                    .is_some_and(|d| !d.is_empty());
                ctx.add(
                    &format!("{label}_receipt_update_precedes_resolution"),
                    delivered && previous["version"].as_u64().unwrap_or(0) > 1,
                );
            }
            let converged = ctx.settle(5);
            ctx.add(&format!("{label}_{op}_replicas_converge"), converged);
            for replica in &replicas {
                let page = get_page(&Api::rest(replica.client()), "messages.message", &expected);
                let ok = page.as_ref().is_some_and(|p| {
                    p["records"]
                        .as_array()
                        .is_some_and(|r| r.len() == 1 && r[0]["id"] == expected.as_str())
                        && cursor_seq(p) >= seq(&result)
                });
                ctx.add(&format!("{label}_{op}_replicated_observed_record"), ok);
            }
        }
    }
    let api = ctx.gateway.clone();
    let armed = ctx.fault("identical-manual-send");
    ctx.add("ambiguous_send_fault_armed", armed);
    let c = command(
        "send",
        &json!({"chat_id": chat["id"], "text": ctx.tag("ambiguous")}),
        0,
    )?;
    let first = api.call("messages.send", &c);
    ctx.add(
        "ambiguous_send_does_not_invent_observed_ids",
        first
            .as_ref()
            .is_some_and(|r| r["state"] == "indeterminate" && ids(r).is_empty()),
    );
    let second = api.call("messages.send", &c);
    let one = ctx.count(&c, 1);
    ctx.add(
        "ambiguous_retry_is_lookup_only",
        second
            .as_ref()
            .is_some_and(|r| r["state"] == "indeterminate" && ids(r).is_empty())
            && one,
    );
    Ok(())
}

fn failed_ids(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    let replicas = replicas::endpoints();
    let mut clients = vec![ctx.gateway.client.clone()];
    clients.extend(replicas.iter().map(Endpoint::client));
    for (index, client) in clients.into_iter().enumerate() {
        let mut api = Api::rest(client);
        let label = endpoint_label(index);
        for fault in ["delivery-failed", "ghost-send"] {
            let armed = ctx.fault(fault);
            ctx.add(&format!("{label}_{fault}_armed"), armed);
            let c = command(
                "send",
                &json!({"chat_id": chat["id"], "text": ctx.tag(fault)}),
                0,
            )?;
            let result = api.call("messages.send", &c);
            let state = result.clone().unwrap_or(Value::Null);
            ctx.add(
                &format!("{label}_{fault}_failure_is_honest"),
                result.is_some() && state["state"] == "delivery_failed",
            );
            if fault == "ghost-send" {
                let proof = &state["evidence"];
                ctx.add(
                    &format!("{label}_ghost_has_no_matching_native_record"),
                    proof["native_key"].as_str().is_none_or(str::is_empty)
                        && proof["observed"] != true
                        && proof["reason"] == "empty_unjoined_sms",
                );
                ctx.add(
                    &format!("{label}_ghost_no_guessed_record"),
                    seq(&state) == 0 && ids(&state).is_empty(),
                );
            } else {
                let native = state["evidence"]["native_key"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let aliases = gateway_aliases(&ctx.observed.gateway);
                let expected = aliases
                    .as_ref()
                    .map(|a| alias(a, &native))
                    .unwrap_or_default();
                ctx.add(
                    &format!("{label}_concrete_failure_has_exact_id"),
                    aliases.is_ok()
                        && !expected.is_empty()
                        && seq(&state) > 0
                        && ids(&state) == [expected.clone()],
                );
                let page = get_page(&api, "messages.message", &expected);
                ctx.add(
                    &format!("{label}_concrete_failure_readable_at_commit"),
                    page.as_ref().is_some_and(|p| {
                        p["records"].as_array().is_some_and(|r| r.len() == 1)
                            && cursor_seq(p) >= seq(&state)
                    }),
                );
                if ids(&state).len() == 1 {
                    let resend = command(
                        "send",
                        &json!({"chat_id": chat["id"], "text": ctx.tag(fault), "resend_of": ids(&state)[0]}),
                        0,
                    )?;
                    let sent = api.call("messages.send", &resend);
                    ctx.add(
                        &format!("{label}_explicit_resend_targets_proven_failure"),
                        sent.is_some_and(|s| s["state"] == "observed"),
                    );
                }
            }
            for protocol in ["rest", "mcp"] {
                api.mcp = protocol == "mcp";
                let status = api.status(&id_of(&c), false);
                ctx.add(
                    &format!("{label}_{fault}_{protocol}_status_parity"),
                    status
                        .as_ref()
                        .is_some_and(|s| seq(s) == seq(&state) && ids(s) == ids(&state)),
                );
            }
            let retry = api.call("messages.send", &c);
            let one = ctx.count(&c, 1);
            ctx.add(
                &format!("{label}_{fault}_retry_never_redispatches"),
                retry
                    .as_ref()
                    .is_some_and(|r| r["state"] == state["state"] && ids(r) == ids(&state))
                    && one,
            );
        }
    }
    Ok(())
}

fn part_view(value: &Value, key: &str) -> Vec<Value> {
    value[key].as_array().cloned().unwrap_or_default()
}
fn evidence_parts(value: &Value) -> Vec<Value> {
    value["evidence"]["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}
fn parts_shape(state: &Value) -> Value {
    json!(part_view(state, "parts"))
}
fn history_len(value: &Value) -> usize {
    value["history"].as_array().map_or(0, Vec::len)
}
fn last_history(value: &Value, key: &str) -> Value {
    value["history"]
        .as_array()
        .and_then(|h| h.last())
        .map_or(Value::Null, |h| h[key].clone())
}
fn delivered_state(value: &Value) -> bool {
    value["state"] == "observed" || value["state"] == "delivered"
}

#[allow(clippy::too_many_lines)]
fn parts(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    let replicas = replicas::endpoints();
    let mut clients = vec![ctx.gateway.client.clone()];
    clients.extend(replicas.iter().map(Endpoint::client));
    for (index, client) in clients.iter().enumerate() {
        let mut api = Api::rest(client.clone());
        let label = endpoint_label(index);
        let outgoing = |ctx: &mut Ctx, api: &Api| -> Option<usize> {
            let page = api.call(
                "records.query",
                &json!({"collection": "messages.message", "limit": 100}),
            );
            let records = page
                .as_ref()
                .and_then(|p| p["records"].as_array().cloned())
                .unwrap_or_default();
            ctx.add(
                &format!("{label}_cardinality_window_complete"),
                page.is_some() && records.len() < 100,
            );
            page.map(|_| {
                records
                    .iter()
                    .filter(|r| data_of(r)["is_from_me"] == true)
                    .count()
            })
        };
        for fault in ["", "part-delivery-failed", "part-no-effect"] {
            let mixed = !fault.is_empty();
            let mut prefix = format!("{label}_complete");
            if mixed {
                prefix = format!("{label}_{fault}");
                let armed = ctx.fault(fault);
                ctx.add(&format!("{prefix}_fault_armed"), armed);
            }
            let mut files = vec![];
            for i in 0..2 {
                let data = format!("[ab-test {}] part {i}", ctx.run).into_bytes();
                let file = if index == 0 {
                    upload(client, "same.txt", &data)
                } else {
                    Some(inline("same.txt", &data))
                };
                ctx.add(&format!("{prefix}_upload_{i}_prepared"), file.is_some());
                let Some(file) = file else {
                    return Ok(());
                };
                files.push(file);
            }
            let c = command(
                "send",
                &json!({"chat_id": chat["id"], "text": ctx.tag("multipart"), "files": files}),
                0,
            )?;
            let before = outgoing(ctx, &api);
            let result = api.call("messages.send", &c);
            let result_ok = result.is_some();
            let result = result.unwrap_or(Value::Null);
            let want_state = if mixed { "delivery_failed" } else { "observed" };
            ctx.add(
                &format!("{prefix}_truthful_aggregate"),
                result_ok && result["state"] == want_state && seq(&result) > 0,
            );
            let after = outgoing(ctx, &api);
            let want_rows = if fault == "part-no-effect" { 2 } else { 3 };
            ctx.add(
                &format!("{prefix}_captured_message_cardinality"),
                matches!((before, after), (Some(b), Some(a)) if a.saturating_sub(b) == want_rows),
            );
            let helper_parts = evidence_parts(&result);
            ctx.add(
                &format!("{prefix}_three_helper_parts"),
                helper_parts.len() == 3,
            );
            ctx.add(
                &format!("{prefix}_three_public_parts"),
                part_view(&result, "parts").len() == 3,
            );
            let compatible = |cursor_holder: &Value| -> bool {
                seq(&result) > 0
                    && cursor_holder["cursor"]["epoch"] == result["cursor"]["epoch"]
                    && cursor_holder["cursor"]["subscription_revision"]
                        == result["cursor"]["subscription_revision"]
                    && cursor_seq(cursor_holder) >= seq(&result)
            };
            let aliases = gateway_aliases(&ctx.observed.gateway);
            ctx.add(&format!("{prefix}_aliases_readable"), aliases.is_ok());
            let aliases = aliases.unwrap_or_default();
            let mut expected: Vec<Value> = vec![];
            let mut seen = BTreeSet::new();
            for (i, part) in helper_parts.iter().enumerate() {
                let native = part["native_key"].as_str().unwrap_or_default();
                let record_id = alias(&aliases, native);
                let no_effect = part["disposition"] == "failed_no_effect";
                ctx.add(
                    &format!("{prefix}_unique_part_identity"),
                    part["index"] == i
                        && (no_effect && record_id.is_empty()
                            || !no_effect && !record_id.is_empty() && !seen.contains(&record_id)),
                );
                seen.insert(record_id.clone());
                let disposition = if part["disposition"] == "locally_observed" {
                    json!("observed")
                } else {
                    part["disposition"].clone()
                };
                let mut entry =
                    json!({"index": i, "kind": part["kind"], "disposition": disposition});
                if !record_id.is_empty() {
                    entry["record_id"] = json!(record_id);
                }
                if i > 0 && i <= files.len() && !files[i - 1]["upload_id"].is_null() {
                    entry["upload_id"] = files[i - 1]["upload_id"].clone();
                }
                expected.push(entry);
                if no_effect {
                    continue;
                }
                let page = get_page(&api, "messages.message", &record_id);
                ctx.add(
                    &format!("{prefix}_part_readable_at_commit"),
                    page.as_ref().is_some_and(|p| {
                        p["records"].as_array().is_some_and(|r| r.len() == 1) && compatible(p)
                    }),
                );
            }
            ctx.add(
                &format!("{prefix}_exact_parts_and_original_upload_order"),
                parts_shape(&result) == json!(expected),
            );
            ctx.add(
                &format!("{prefix}_legacy_primary_id_unchanged"),
                expected.len() == 3
                    && ids(&result) == [expected[0]["record_id"].as_str().unwrap_or_default()],
            );
            if mixed {
                let last_ok = if fault == "part-delivery-failed" {
                    expected.len() == 3 && expected[2]["disposition"] == "delivery_failed"
                } else {
                    expected.len() == 3
                        && expected[2]["disposition"] == "failed_no_effect"
                        && expected[2]["record_id"].is_null()
                };
                ctx.add(
                    &format!("{prefix}_only_last_part_failed"),
                    expected.len() == 3
                        && expected[0]["disposition"] == "observed"
                        && expected[1]["disposition"] == "observed"
                        && last_ok,
                );
                if expected.len() == 3 {
                    let mut retry_file = files[1].clone();
                    if index == 0 {
                        let restaged = upload(
                            client,
                            "same.txt",
                            format!("[ab-test {}] part 1", ctx.run).as_bytes(),
                        );
                        let fresh = restaged
                            .as_ref()
                            .is_some_and(|f| f["upload_id"] != files[1]["upload_id"]);
                        ctx.add(&format!("{prefix}_selected_retry_file_restaged"), fresh);
                        if let Some(file) = restaged {
                            retry_file = file;
                        }
                    }
                    let mut retry_args = json!({"chat_id": chat["id"], "files": [retry_file]});
                    if !expected[2]["record_id"].is_null() {
                        retry_args["resend_of"] = expected[2]["record_id"].clone();
                    }
                    let resend = command("send", &retry_args, 0)?;
                    let retried = api.call("messages.send", &resend);
                    let ok = retried.as_ref().is_some_and(|r| {
                        let public = part_view(r, "parts");
                        let record_id = public
                            .first()
                            .and_then(|p| p["record_id"].as_str())
                            .unwrap_or_default();
                        r["state"] == "observed"
                            && evidence_parts(r).len() == 1
                            && evidence_parts(r)[0]["kind"] == "attachment"
                            && public.len() == 1
                            && public[0]["index"] == 0
                            && public[0]["kind"] == "attachment"
                            && !record_id.is_empty()
                            && !seen.contains(record_id)
                            && public[0]["upload_id"] == retry_file["upload_id"]
                    });
                    ctx.add(
                        &format!("{prefix}_fresh_resend_contains_only_failed_attachment"),
                        ok,
                    );
                    let one = effects_of(ctx.world, &[&id_of(&c), &id_of(&resend)])
                        .is_some_and(|n| n == [1, 1]);
                    ctx.add(
                        &format!("{prefix}_resend_does_not_repeat_original_dispatch"),
                        one,
                    );
                }
            }
            for protocol in ["rest", "mcp"] {
                api.mcp = protocol == "mcp";
                let status = api.status(&id_of(&c), false);
                ctx.add(
                    &format!("{prefix}_{protocol}_parts_parity"),
                    status.is_some_and(|s| parts_shape(&s) == json!(expected)),
                );
            }
            let converged = ctx.settle(5);
            ctx.add(&format!("{prefix}_replicas_reach_commit"), converged);
            let observers: Vec<Client> = clients.clone();
            for observer in observers {
                for part in &expected {
                    let Some(record_id) = part["record_id"].as_str() else {
                        continue;
                    };
                    let page =
                        get_page(&Api::rest(observer.clone()), "messages.message", record_id);
                    ctx.add(
                        &format!("{prefix}_replicated_part_at_compatible_cursor"),
                        page.as_ref().is_some_and(|p| {
                            p["records"].as_array().is_some_and(|r| r.len() == 1) && compatible(p)
                        }),
                    );
                }
            }
            let replay = api.call("messages.send", &c);
            let one = ctx.count(&c, 1);
            ctx.add(
                &format!("{prefix}_replay_preserves_parts_without_effect"),
                replay.is_some_and(|r| parts_shape(&r) == json!(expected)) && one,
            );
        }
    }
    delayed_capture(ctx, chat)?;
    ghost_part(ctx, chat)?;
    ambiguous_part(ctx, chat)?;
    receipt_failure(ctx, chat)?;
    progress(ctx, chat)?;
    capture_bound(ctx, chat)?;
    Ok(())
}

fn delayed_capture(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    delayed_capture_case(ctx, chat, true, "")?;
    delayed_capture_case(ctx, chat, false, "no_action_")
}
fn delayed_capture_case(ctx: &mut Ctx, chat: &Value, resolve: bool, prefix: &str) -> Result<()> {
    let api = ctx.gateway.clone();
    let armed = ctx.fault("outcome-before-capture");
    ctx.add(&format!("{prefix}delayed_capture_fault_armed"), armed);
    let c = command(
        "send",
        &json!({"chat_id": chat["id"], "text": ctx.tag("delayed capture")}),
        0,
    )?;
    let started = Instant::now();
    let initial = api.call("messages.send", &c);
    let initial_ok = initial.is_some();
    let initial = initial.unwrap_or(Value::Null);
    let public = part_view(&initial, "parts");
    ctx.add(
        &format!("{prefix}delayed_capture_initial_only_claims_bound_identity"),
        initial_ok
            && public.len() == 1
            && evidence_parts(&initial).len() == 1
            && (initial["state"] == "indeterminate"
                && ids(&initial).is_empty()
                && public[0]["record_id"].is_null()
                || delivered_state(&initial) && !public[0]["record_id"].is_null()),
    );
    if !initial_ok || evidence_parts(&initial).len() != 1 {
        return Ok(());
    }
    let native = evidence_parts(&initial)[0]["native_key"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut expected = String::new();
    let arrived = await_ms(1000, || {
        expected = gateway_aliases(&ctx.observed.gateway)
            .map(|a| alias(&a, &native))
            .unwrap_or_default();
        !expected.is_empty()
    });
    ctx.add(
        &format!("{prefix}delayed_capture_exact_alias_arrives"),
        arrived && !expected.is_empty(),
    );
    let mut resolved = Value::Null;
    let resolved_ok = if resolve {
        let reply = api.status(&id_of(&c), true);
        let ok = reply.is_some();
        resolved = reply.unwrap_or(Value::Null);
        ok
    } else {
        let remaining =
            1000u64.saturating_sub(u64::try_from(started.elapsed().as_millis()).unwrap_or(1000));
        await_ms(remaining, || {
            let reply = api.status(&id_of(&c), false);
            let done = reply.as_ref().is_some_and(delivered_state);
            if let Some(reply) = reply {
                resolved = reply;
            }
            done
        })
    };
    let elapsed = started.elapsed();
    let public = part_view(&resolved, "parts");
    ctx.add(
        &format!("{prefix}delayed_capture_resolve_is_sent_with_exact_identity"),
        resolved_ok
            && delivered_state(&resolved)
            && public.len() == 1
            && public[0]["record_id"] == expected.as_str()
            && ids(&resolved) == [expected.clone()]
            && seq(&resolved) > 0,
    );
    ctx.add(
        &format!("{prefix}delayed_capture_no_prolonged_unknown"),
        elapsed < Duration::from_secs(1),
    );
    let page = get_page(&api, "messages.message", &expected);
    ctx.add(
        &format!("{prefix}delayed_capture_read_your_write"),
        page.as_ref().is_some_and(|p| {
            p["records"].as_array().is_some_and(|r| r.len() == 1)
                && p["cursor"]["epoch"] == resolved["cursor"]["epoch"]
                && cursor_seq(p) >= seq(&resolved)
        }),
    );
    let one = ctx.count(&c, 1);
    ctx.add(
        &format!("{prefix}delayed_capture_resolve_effect_remains_one"),
        one,
    );
    Ok(())
}

fn ghost_part(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    let api = ctx.gateway.clone();
    let armed = ctx.fault("ghost-send");
    ctx.add("ghost_part_fault_armed", armed);
    let c = command(
        "send",
        &json!({"chat_id": chat["id"], "text": ctx.tag("ghost part")}),
        0,
    )?;
    let result = api.call("messages.send", &c);
    let ok = result.is_some();
    let result = result.unwrap_or(Value::Null);
    let public = part_view(&result, "parts");
    ctx.add(
        "ghost_part_failure_never_invents_identity",
        ok && result["state"] == "delivery_failed"
            && seq(&result) == 0
            && ids(&result).is_empty()
            && public.len() == 1
            && public[0]["disposition"] == "delivery_failed"
            && public[0]["record_id"].is_null(),
    );
    let checked = api.status(&id_of(&c), true);
    let one = ctx.count(&c, 1);
    ctx.add(
        "ghost_part_check_preserves_failure_without_redispatch",
        checked.is_some_and(|s| {
            s["state"] == "delivery_failed"
                && ids(&s).is_empty()
                && parts_shape(&s) == parts_shape(&result)
        }) && one,
    );
    Ok(())
}

fn ambiguous_part(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    let api = ctx.gateway.clone();
    let armed = ctx.fault("identical-manual-send");
    ctx.add("ambiguous_part_fault_armed", armed);
    let c = command(
        "send",
        &json!({"chat_id": chat["id"], "text": ctx.tag("ambiguous part")}),
        0,
    )?;
    let result = api.call("messages.send", &c);
    let ok = result.is_some();
    let result = result.unwrap_or(Value::Null);
    let public = part_view(&result, "parts");
    ctx.add(
        "ambiguous_part_never_overrides_multiple_candidate_evidence",
        ok && result["state"] == "indeterminate"
            && seq(&result) == 0
            && ids(&result).is_empty()
            && public.len() == 1
            && public[0]["disposition"] == "indeterminate"
            && public[0]["record_id"].is_null(),
    );
    let one = ctx.count(&c, 1);
    ctx.add("ambiguous_part_effect_remains_one", one);
    Ok(())
}

fn receipt_failure(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    let api = ctx.gateway.clone();
    let file = upload(
        &api.client,
        "receipt.txt",
        format!("[ab-test {}] receipt", ctx.run).as_bytes(),
    );
    ctx.add("receipt_part_file_ready", file.is_some());
    let Some(file) = file else {
        return Ok(());
    };
    let c = command(
        "send",
        &json!({"chat_id": chat["id"], "text": ctx.tag("receipt"), "files": [file]}),
        0,
    )?;
    let initial = api.call("messages.send", &c);
    let initial_ok = initial.is_some();
    let initial = initial.unwrap_or(Value::Null);
    ctx.add(
        "receipt_parts_initially_observed",
        initial_ok && initial["state"] == "observed" && part_view(&initial, "parts").len() == 2,
    );
    let initial_parts = part_view(&initial, "parts");
    if initial_parts.len() != 2 {
        return Ok(());
    }
    let armed = ctx.fault("part-unbound-failure");
    ctx.add("receipt_unbound_failure_armed", armed);
    let unbound = api.status(&id_of(&c), true);
    ctx.add(
        "receipt_unbound_failure_never_borrows_observed_identity",
        unbound.is_some_and(|u| {
            u["state"] == "observed"
                && part_view(&u, "parts") == initial_parts
                && seq(&u) == seq(&initial)
                && history_len(&u) == history_len(&initial)
        }),
    );
    let armed = ctx.fault("part-receipt-failed");
    ctx.add("receipt_part_failure_armed", armed);
    let failed = api.status(&id_of(&c), true);
    let failed_ok = failed.is_some();
    let failed = failed.unwrap_or(Value::Null);
    let failed_parts = part_view(&failed, "parts");
    ctx.add(
        "receipt_part_strengthens_to_failed_with_same_identity",
        failed_ok
            && failed["state"] == "delivery_failed"
            && failed_parts.len() == 2
            && failed_parts[0] == initial_parts[0]
            && failed_parts[1]["disposition"] == "delivery_failed"
            && failed_parts[1]["record_id"] == initial_parts[1]["record_id"]
            && seq(&failed) > seq(&initial),
    );
    let record_id = initial_parts[1]["record_id"].as_str().unwrap_or_default();
    let page = get_page(&api, "messages.message", record_id);
    let code = page
        .as_ref()
        .and_then(|p| p["records"].as_array().cloned())
        .filter(|r| r.len() == 1)
        .map_or(0, |r| data_of(&r[0])["error_code"].as_i64().unwrap_or(0));
    ctx.add(
        "receipt_part_captured_error_is_readable",
        page.is_some() && code == 100,
    );
    if failed["state"] == "delivery_failed" && failed_parts.len() == 2 {
        let armed = ctx.fault("part-receipt-failed-first");
        ctx.add("receipt_remaining_part_failure_armed", armed);
        let strengthened = api.status(&id_of(&c), true);
        let ok = strengthened.is_some_and(|s| {
            let parts = part_view(&s, "parts");
            s["state"] == "delivery_failed"
                && parts.len() == 2
                && parts[0]["disposition"] == "delivery_failed"
                && parts[0]["record_id"] == initial_parts[0]["record_id"]
                && parts[1] == failed_parts[1]
                && seq(&s) > seq(&failed)
                && history_len(&s) == history_len(&failed) + 1
                && last_history(&s, "transition") == "parts_updated"
        });
        ctx.add(
            "receipt_failed_aggregate_accepts_remaining_part_evidence",
            ok,
        );
    }
    let one = ctx.count(&c, 1);
    ctx.add("receipt_part_resolve_does_not_redispatch", one);
    Ok(())
}

fn progress(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    for fault in ["part-progress", "part-progress-failure"] {
        progress_case(ctx, chat, fault)?;
    }
    Ok(())
}
fn progress_case(ctx: &mut Ctx, chat: &Value, fault: &str) -> Result<()> {
    let (want_state, want_later) = if fault == "part-progress-failure" {
        ("delivery_failed", "failed_no_effect")
    } else {
        ("indeterminate", "indeterminate")
    };
    let mut api = ctx.gateway.clone();
    let file = upload(
        &api.client,
        "progress.txt",
        format!("[ab-test {}] progress", ctx.run).as_bytes(),
    );
    let name = |suffix: &str| format!("{fault}_{suffix}");
    ctx.add(&name("progress_file_ready"), file.is_some());
    let Some(file) = file else {
        return Ok(());
    };
    let armed = ctx.fault(fault);
    ctx.add(&name("progress_fault_armed"), armed);
    let c = command(
        "send",
        &json!({"chat_id": chat["id"], "text": ctx.tag("progress"), "files": [file]}),
        0,
    )?;
    let initial = api.call("messages.send", &c);
    let initial_ok = initial.is_some();
    let initial = initial.unwrap_or(Value::Null);
    let parts = part_view(&initial, "parts");
    ctx.add(
        &name("progress_initial_expected_unresolved_parts"),
        initial_ok
            && initial["state"] == want_state
            && parts.len() == 2
            && parts[0]["disposition"] == "indeterminate"
            && parts[1]["disposition"] == want_later
            && parts[0]["record_id"].is_null()
            && parts[1]["record_id"].is_null(),
    );
    let resolved = api.status(&id_of(&c), true);
    let resolved_ok = resolved.is_some();
    let resolved = resolved.unwrap_or(Value::Null);
    let aliases = gateway_aliases(&ctx.observed.gateway);
    let expected = evidence_parts(&resolved)
        .first()
        .map(|p| {
            aliases
                .as_ref()
                .map(|a| alias(a, p["native_key"].as_str().unwrap_or_default()))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let resolved_parts = part_view(&resolved, "parts");
    ctx.add(
        &name("progress_earlier_part_observed_later_unchanged"),
        resolved_ok
            && aliases.is_ok()
            && !expected.is_empty()
            && resolved["state"] == want_state
            && resolved_parts.len() == 2
            && resolved_parts[0]["disposition"] == "observed"
            && resolved_parts[0]["record_id"] == expected.as_str()
            && resolved_parts[1]["disposition"] == want_later
            && resolved_parts[1]["record_id"].is_null(),
    );
    ctx.add(
        &name("progress_same_aggregate_explicit_history_and_commit"),
        seq(&resolved) > seq(&initial)
            && cursor_seq(&resolved) > cursor_seq(&initial)
            && history_len(&resolved) == history_len(&initial) + 1
            && last_history(&resolved, "state") == want_state
            && last_history(&resolved, "transition") == "parts_updated",
    );
    for protocol in ["rest", "mcp"] {
        api.mcp = protocol == "mcp";
        let read = api.status(&id_of(&c), false);
        ctx.add(
            &name(&format!("progress_{protocol}_read_only_evidence_visible")),
            read.is_some_and(|r| {
                part_view(&r, "parts") == resolved_parts && seq(&r) == seq(&resolved)
            }),
        );
    }
    let repeated = api.status(&id_of(&c), true);
    ctx.add(
        &name("progress_identical_proof_does_not_append_history"),
        repeated.is_some_and(|r| {
            part_view(&r, "parts") == resolved_parts
                && history_len(&r) == history_len(&resolved)
                && seq(&r) == seq(&resolved)
        }),
    );
    let armed = ctx.fault("part-progress-stale");
    ctx.add(&name("progress_stale_proof_armed"), armed);
    let stale = api.status(&id_of(&c), true);
    ctx.add(
        &name("progress_stale_unknown_never_erases_observation"),
        stale.is_some_and(|s| {
            part_view(&s, "parts") == resolved_parts
                && history_len(&s) == history_len(&resolved)
                && seq(&s) == seq(&resolved)
        }),
    );
    let one = ctx.count(&c, 1);
    ctx.add(&name("progress_reconcile_never_redispatches"), one);
    Ok(())
}

fn capture_bound(ctx: &mut Ctx, chat: &Value) -> Result<()> {
    let api = ctx.gateway.clone();
    let armed = ctx.fault("outcome-without-capture");
    ctx.add("capture_bound_fault_armed", armed);
    let c = command(
        "send",
        &json!({"chat_id": chat["id"], "text": ctx.tag("missing capture")}),
        0,
    )?;
    let started = Instant::now();
    let initial = api.call("messages.send", &c);
    let elapsed = started.elapsed();
    let initial_ok = initial.is_some();
    let initial = initial.unwrap_or(Value::Null);
    let public = part_view(&initial, "parts");
    ctx.add(
        "capture_bound_stops_with_honest_unknown",
        initial_ok
            && initial["state"] == "indeterminate"
            && ids(&initial).is_empty()
            && public.len() == 1
            && public[0]["record_id"].is_null()
            && elapsed >= Duration::from_secs(9)
            && elapsed < Duration::from_secs(12),
    );
    if !initial_ok || initial["state"] != "indeterminate" || evidence_parts(&initial).len() != 1 {
        return Ok(());
    }
    let truth = ctx.world.control("/inspect", &Value::Null);
    ctx.add("capture_bound_later_capture_available", truth.is_ok());
    let observations: Vec<Value> = truth
        .unwrap_or_default()
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| {
            o["collection"]
                .as_str()
                .is_some_and(|c| c.starts_with("messages."))
        })
        .cloned()
        .collect();
    let committed = ctx
        .world
        .control(
            "/capture",
            &json!({"source": "messages", "observations": observations}),
        )
        .is_ok();
    ctx.add("capture_bound_later_capture_committed", committed);
    thread::sleep(Duration::from_millis(350));
    let unchanged = api.status(&id_of(&c), false);
    ctx.add(
        "capture_bound_does_not_follow_after_deadline",
        unchanged.is_some_and(|u| {
            u["state"] == "indeterminate" && u["cursor"] == initial["cursor"] && ids(&u).is_empty()
        }),
    );
    let checked = api.status(&id_of(&c), true);
    ctx.add(
        "capture_bound_manual_check_remains_available",
        checked.is_some_and(|s| {
            let public = part_view(&s, "parts");
            delivered_state(&s)
                && ids(&s).len() == 1
                && public.len() == 1
                && public[0]["record_id"] == ids(&s)[0].as_str()
        }),
    );
    let one = ctx.count(&c, 1);
    ctx.add("capture_bound_read_only_checks_effect_one", one);
    Ok(())
}

pub fn commands(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    if !ALL.contains(&evidence.scenario.as_str()) {
        return Err(Error::Invalid(format!(
            "messages commands not ported for {}",
            evidence.scenario
        )));
    }
    run(world, evidence)
}
