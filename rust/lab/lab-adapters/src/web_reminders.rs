#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{Observed, await_until, converged, read_all},
    http::Client,
    readmodels::{PROTOCOLS, observation, put, reference},
    replicas,
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Map, Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const SCENARIOS: [&str; 3] = [
    "WEB-RM-REMINDERS-HARDENING",
    "WEB-RM-REMINDERS-ITEMS",
    "WEB-RM-REMINDERS-LISTS",
];
const SCOPE: &str = "reminders:read,system:read";
const HARDENING_SCOPE: &str = "reminders:read,messages:read,calendar:read,system:read";
const NY: &str = "America/New_York";
const MCP_HEADERS: [(&str, &str); 3] = [
    ("Content-Type", "application/json"),
    ("Accept", "application/json, text/event-stream"),
    ("MCP-Protocol-Version", "2025-06-18"),
];

pub fn start(world: &World, count: usize) -> Result<()> {
    replicas::start_with(world, count, SCOPE, 0)
}
pub fn run(world: &mut World, id: &str, evidence: &mut Evidence) -> Result<()> {
    match id {
        "WEB-RM-REMINDERS-HARDENING" => hardening(world, evidence),
        "WEB-RM-REMINDERS-ITEMS" => items(world, evidence),
        _ => lists(world, evidence),
    }
}

struct Env<'a> {
    world: &'a World,
    gateway: Client,
    scope: &'a str,
    replicas: bool,
}
impl Env<'_> {
    fn converged(&self) -> bool {
        converged(
            self.world,
            &Observed {
                gateway: self.gateway.clone(),
                gateway_scope: self.scope.to_owned(),
                replicas: if self.replicas {
                    replicas::endpoints()
                } else {
                    vec![]
                },
            },
        )
    }
    fn quiet(&self, seconds: u64) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut last = None;
        let mut since = Instant::now();
        while Instant::now() < deadline {
            let now = self.gateway.cursor().ok().map(|cursor| cursor.sequence);
            if now.is_none() || now != last {
                last = now;
                since = Instant::now();
            } else if since.elapsed() >= Duration::from_secs(2) {
                return;
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
    fn settle(&self, seconds: u64) -> bool {
        await_until(seconds, || self.converged())
    }
    fn change(&self, collection: &str, key: &str, data: &Value) -> bool {
        self.world
            .control(
                "/external-change",
                &json!({"collection": collection, "key": key, "data": data}),
            )
            .is_ok()
    }
    fn find(&self, collection: &str, title: Option<&str>) -> Option<String> {
        let mut found = None;
        for record in read_all(&self.gateway, false, false).ok()?.records {
            if record["collection"] == collection
                && title.is_none_or(|title| record["data"]["title"] == title)
            {
                found = record["id"].as_str().map(str::to_owned);
            }
        }
        found
    }
    fn ask(&self, protocol: &str, operation: &str, input: &Value) -> Option<Value> {
        exchange(&self.gateway, protocol, operation, input, 30).map(|x| x.value)
    }
}
fn seed(world: &World, count: u64, gateway: &Client) -> bool {
    world.control("/seed", &json!({"count": count})).is_ok()
        && await_until(10, || {
            read_all(gateway, false, false).is_ok_and(|read| !read.records.is_empty())
        })
}

struct Exchange {
    value: Value,
    envelope: Value,
    headers: Vec<(String, String)>,
}
struct Raw {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
fn raw(
    client: &Client,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    seconds: u64,
) -> Result<Raw> {
    let mut command = Command::new("curl");
    command
        .args([
            "--silent",
            "--show-error",
            "-i",
            "--max-time",
            &seconds.to_string(),
            "-X",
            method,
            "-H",
        ])
        .arg(format!("Authorization: Bearer {}", client.token()));
    for (name, value) in headers {
        command.args(["-H", &format!("{name}: {value}")]);
    }
    if body.is_some() {
        command.args(["--data-binary", "@-"]);
    }
    command
        .args(["-w", "%{http_code}"])
        .arg(format!("http://{}{path}", client.address()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = command.spawn()?;
    if let (Some(mut stdin), Some(body)) = (child.stdin.take(), body) {
        stdin.write_all(body)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() || output.stdout.len() < 3 {
        return Err(Error::Invalid(format!("request exited {}", output.status)));
    }
    let split = output.stdout.len() - 3;
    let status: u16 = std::str::from_utf8(&output.stdout[split..])
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| Error::Invalid("status".into()))?;
    let mut rest = &output.stdout[..split];
    loop {
        let end = rest
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or_else(|| Error::Invalid("response head".into()))?;
        let head = String::from_utf8_lossy(&rest[..end]).into_owned();
        rest = &rest[end + 4..];
        let interim = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .is_some_and(|code| code.starts_with('1'));
        if interim {
            continue;
        }
        let headers = head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
            .collect();
        return Ok(Raw {
            status,
            headers,
            body: rest.to_vec(),
        });
    }
}
fn exchange(
    client: &Client,
    protocol: &str,
    operation: &str,
    input: &Value,
    seconds: u64,
) -> Option<Exchange> {
    if protocol == "mcp" {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": operation.replace('.', "_"), "arguments": input}
        }))
        .ok()?;
        let reply = raw(client, "POST", "/mcp", &MCP_HEADERS, Some(&body), seconds).ok()?;
        let text = String::from_utf8(reply.body).ok()?;
        let envelope = if let Ok(value) = serde_json::from_str::<Value>(&text) {
            value
        } else {
            let messages = lab_wire::sse_messages(&text).ok()?;
            let message = messages
                .iter()
                .find(|m| m.result.is_some() || m.error.is_some())?;
            serde_json::to_value(message).ok()?
        };
        let value = envelope["result"]["structuredContent"].clone();
        (reply.status == 200
            && envelope["error"].is_null()
            && !value.is_null()
            && value["error"].is_null())
        .then_some(Exchange {
            value,
            envelope,
            headers: reply.headers,
        })
    } else {
        let body = serde_json::to_vec(input).ok()?;
        let reply = raw(
            client,
            "POST",
            &format!("/v1/op/{operation}"),
            &[("Content-Type", "application/json")],
            Some(&body),
            seconds,
        )
        .ok()?;
        let value: Value = serde_json::from_slice(&reply.body).ok()?;
        (reply.status == 200 && value["error"].is_null()).then(|| Exchange {
            envelope: value.clone(),
            value,
            headers: reply.headers,
        })
    }
}
fn text_equal(envelope: &Value) -> bool {
    let Some(text) = envelope["result"]["content"][0]["text"].as_str() else {
        return false;
    };
    serde_json::from_str::<Value>(text)
        .is_ok_and(|text| text == envelope["result"]["structuredContent"])
}
fn timing(protocol: &str, reply: &Exchange) -> (f64, usize, bool) {
    let (value, length) = if protocol == "mcp" {
        let Some(value) = reply.envelope["result"]["_meta"]["bridge/read_model_ms"].as_f64() else {
            return (0.0, 0, false);
        };
        let length = reply.envelope["result"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|content| content["text"].as_str().map_or(0, str::len))
            .sum();
        (value, length)
    } else {
        let parsed = reply
            .headers
            .iter()
            .find(|(name, _)| name == "server-timing")
            .and_then(|(_, value)| value.strip_prefix("readmodel;dur="))
            .and_then(|value| value.parse::<f64>().ok());
        let Some(value) = parsed else {
            return (0.0, 0, false);
        };
        (value, 0)
    };
    (value, length, value > 0.0 && value.is_finite())
}
fn percentile95(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let rank = (values.len() * 95).div_ceil(100);
    values.get(rank.saturating_sub(1)).copied().unwrap_or(0.0)
}
fn title(record: &Value) -> &str {
    record["data"]["title"].as_str().unwrap_or_default()
}
fn count(value: &Value) -> usize {
    value.as_array().map_or(0, Vec::len)
}
fn entries(value: &Value) -> std::slice::Iter<'_, Value> {
    value.as_array().map_or([].iter(), |items| items.iter())
}
fn all_items(page: &Value) -> Vec<&Value> {
    entries(&page["sections"])
        .flat_map(|section| entries(&section["items"]))
        .collect()
}
fn day(offset: i64) -> Result<String> {
    let zone = jiff::tz::TimeZone::get(NY).map_err(|e| Error::Invalid(format!("zone: {e}")))?;
    jiff::Timestamp::now()
        .to_zoned(zone)
        .date()
        .checked_add(jiff::Span::new().days(offset))
        .map(|date| date.to_string())
        .map_err(|e| Error::Invalid(format!("date: {e}")))
}

struct Fixture {
    collection: &'static str,
    key: &'static str,
    data: Value,
}
fn fixtures() -> Result<Vec<Fixture>> {
    let today = day(0)?;
    let tomorrow = day(1)?;
    let reminder = |key: &'static str, name: &str, list: &str, extra: Value| {
        let mut data = json!({"title": name, "list_id": reference("reminders.list", list), "completed": false, "flagged": false, "assignment": null, "priority": 0, "tags": [], "manual_order": 0, "created_at": "2026-01-01T12:00:00Z", "store_status": "partial", "store_coverage": {"assignment": {"status": "complete"}, "flagged": {"status": "complete"}}});
        if let (Some(target), Some(source)) = (data.as_object_mut(), extra.as_object()) {
            for (field, value) in source {
                target.insert(field.clone(), value.clone());
            }
        }
        Fixture {
            collection: "reminders.reminder",
            key,
            data,
        }
    };
    let assignment = |me: bool| json!({"assignee": {"id": if me {"fixture-self"} else {"fixture-other"}, "is_me": me}, "originator": null, "assigned_at": "2026-01-01T12:00:00Z"});
    let list = |key: &'static str, data: Value| Fixture {
        collection: "reminders.list",
        key,
        data,
    };
    Ok(vec![
        list(
            "rm-rem-group",
            json!({"title": "Bridge Test Group", "is_group": true, "display_order": 10, "pinned": false}),
        ),
        list(
            "list",
            json!({"title": "Bridge Test Read Models", "parent_id": reference("reminders.list", "rm-rem-group"), "display_order": 20, "pinned": false, "sections": [{"kind": "store", "id": "section-a", "title": "Bridge Test Section", "order": 0}, {"kind": "store", "id": "section-empty", "title": "Bridge Test Empty", "order": 1}]}),
        ),
        list(
            "rm-rem-other",
            json!({"title": "Bridge Test Other", "display_order": 30, "pinned": false, "sections": []}),
        ),
        list(
            "rm-rem-pinned",
            json!({"title": "Bridge Test Pinned", "display_order": 100, "pinned": true, "pinned_at": "2026-01-01T12:00:00Z", "sections": []}),
        ),
        reminder(
            "rm-rem-parent",
            "Bridge Test Parent",
            "list",
            json!({"flagged": true, "due": "2000-01-01", "due_all_day": true, "priority": 5, "section_id": "section-a", "tags": ["work"], "assignment": assignment(true)}),
        ),
        reminder(
            "rm-rem-child",
            "Bridge Test Child",
            "list",
            json!({"parent_id": reference("reminders.reminder", "rm-rem-parent"), "due": today, "due_all_day": true, "priority": 1, "section_id": "section-a", "tags": ["work"], "assignment": assignment(false)}),
        ),
        reminder(
            "rm-rem-completed",
            "Bridge Test Completed",
            "list",
            json!({"completed": true, "completed_at": "2026-01-01T12:00:00Z", "flagged": true, "priority": 9, "assignment": assignment(true), "tags": ["home"]}),
        ),
        reminder(
            "rm-rem-future",
            "Bridge Test Future",
            "rm-rem-other",
            json!({"due": tomorrow, "due_all_day": true, "tags": ["errands"]}),
        ),
        reminder(
            "rm-rem-undated",
            "Bridge Test Undated",
            "rm-rem-other",
            json!({"priority": 9, "assignment": assignment(false), "tags": ["home"]}),
        ),
    ])
}

fn check(evidence: &mut Evidence, name: &str, passed: bool) {
    evidence.check(name, passed, "");
}

fn lists(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let world: &World = world;
    check(evidence, "world_start", true);
    let env = Env {
        world,
        gateway: Client::new(world, SCOPE)?,
        scope: SCOPE,
        replicas: true,
    };
    let Some(list_id) = prepare(&env, evidence)? else {
        return Ok(());
    };
    let mut available = true;
    for protocol in PROTOCOLS {
        let result = env.ask(protocol, "reminders.lists", &json!({"time_zone": NY}));
        check(
            evidence,
            &format!("{protocol}_reminders.lists_available"),
            result.is_some(),
        );
        let Some(page) = result else {
            available = false;
            continue;
        };
        let sidebar = entries(&page["lists"]).collect::<Vec<_>>();
        check(
            evidence,
            &format!("{protocol}_sidebar_shape"),
            sidebar.len() == 4,
        );
        for tile in entries(&page["smart"]) {
            let id = tile["id"].as_str().unwrap_or_default();
            let expected = match id {
                "today" => Some(2),
                "scheduled" => Some(3),
                "all" => Some(4),
                "flagged" | "completed" | "assigned" => Some(1),
                _ => None,
            };
            check(
                evidence,
                &format!("{protocol}_smart_{id}_definition"),
                expected.is_some() && tile["count"].as_i64() == expected,
            );
        }
        check(
            evidence,
            &format!("{protocol}_six_smart_tiles"),
            count(&page["smart"]) == 6,
        );
        if sidebar.len() == 4 {
            check(
                evidence,
                &format!("{protocol}_native_sidebar_order"),
                title(&sidebar[0]["list"]) == "Bridge Test Pinned"
                    && sidebar[0]["pinned"] == true
                    && title(&sidebar[1]["list"]) == "Bridge Test Group"
                    && sidebar[2]["list"]["id"] == list_id
                    && title(&sidebar[3]["list"]) == "Bridge Test Other",
            );
            check(
                evidence,
                &format!("{protocol}_flat_group_and_incomplete_count"),
                sidebar[2]["group_id"].as_str().is_some()
                    && sidebar[2]["group_id"] == sidebar[1]["list"]["id"]
                    && sidebar[2]["count"].as_i64() == Some(2),
            );
        }
        let counts: Vec<(&str, i64)> = entries(&page["tags"])
            .map(|tag| {
                (
                    tag["name"].as_str().unwrap_or_default(),
                    tag["count"].as_i64().unwrap_or(0),
                )
            })
            .collect();
        let of = |name: &str| counts.iter().find(|(n, _)| *n == name).map(|(_, c)| *c);
        check(
            evidence,
            &format!("{protocol}_incomplete_tag_counts"),
            counts.len() == 3
                && of("work") == Some(2)
                && of("home") == Some(1)
                && of("errands") == Some(1),
        );
    }
    if available {
        parity(&env, evidence, "reminders.lists", &json!({"time_zone": NY}));
        schema(&env, evidence, "reminders.lists");
        replica(&env, evidence, &["reminders.lists"], &list_id);
        list_boundaries(&env, evidence);
    }
    Ok(())
}

fn prepare(env: &Env, evidence: &mut Evidence) -> Result<Option<String>> {
    if !seed(env.world, 0, &env.gateway) {
        check(evidence, "synthetic_reminders_capture", false);
        return Ok(None);
    }
    for fixture in fixtures()? {
        if !env.change(fixture.collection, fixture.key, &fixture.data) {
            check(evidence, "synthetic_reminders_capture", false);
            return Ok(None);
        }
    }
    check(
        evidence,
        "synthetic_reminders_gateway_and_replica",
        env.settle(5),
    );
    let list = env.find("reminders.list", Some("Bridge Test Read Models"));
    check(evidence, "list_reference_resolved", list.is_some());
    Ok(Some(list.unwrap_or_default()))
}

fn items(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let sibling_source: &World = world;
    check(evidence, "world_start", true);
    let env = Env {
        world: sibling_source,
        gateway: Client::new(sibling_source, SCOPE)?,
        scope: SCOPE,
        replicas: true,
    };
    let Some(list_id) = prepare(&env, evidence)? else {
        return Ok(());
    };
    let mut available = true;
    for protocol in PROTOCOLS {
        let result = env.ask(
            protocol,
            "reminders.items",
            &json!({"time_zone": NY, "list": list_id}),
        );
        check(
            evidence,
            &format!("{protocol}_reminders.items_available"),
            result.is_some(),
        );
        let Some(page) = result else {
            available = false;
            continue;
        };
        let sections = entries(&page["sections"]).collect::<Vec<_>>();
        check(
            evidence,
            &format!("{protocol}_section_shape_and_filtered_count"),
            page["count"].as_i64() == Some(2) && sections.len() == 2,
        );
        if sections.len() == 2 {
            let roots = entries(&sections[0]["items"]).collect::<Vec<_>>();
            check(
                evidence,
                &format!("{protocol}_native_and_empty_section_order"),
                sections[0]["section"]["kind"] == "store"
                    && sections[0]["section"]["id"] == "section-a"
                    && sections[1]["section"]["id"] == "section-empty"
                    && count(&sections[1]["items"]) == 0,
            );
            check(
                evidence,
                &format!("{protocol}_subtasks_nested_once"),
                roots.len() == 1
                    && title(&roots[0]["reminder"]) == "Bridge Test Parent"
                    && count(&roots[0]["subtasks"]) == 1
                    && title(&roots[0]["subtasks"][0]["reminder"]) == "Bridge Test Child",
            );
        }
    }
    if available {
        let query = json!({"time_zone": NY, "list": list_id});
        parity(&env, evidence, "reminders.items", &query);
        schema(&env, evidence, "reminders.items");
        groups(&env, evidence, &list_id);
        replica(
            &env,
            evidence,
            &["reminders.lists", "reminders.items"],
            &list_id,
        );
        zones(&env, evidence);
        item_boundaries(&env, evidence, &list_id)?;
        attachments(&env, evidence, &list_id);
        nesting(&env, evidence, &list_id)?;
        sorts(&env, evidence)?;
        perf(world, evidence)?;
    }
    Ok(())
}

fn parity(env: &Env, evidence: &mut Evidence, operation: &str, query: &Value) {
    let rest = exchange(&env.gateway, "rest", operation, query, 30);
    let mcp = exchange(&env.gateway, "mcp", operation, query, 30);
    check(
        evidence,
        &format!("{operation}_mcp_text_matches_structured_json"),
        mcp.as_ref()
            .is_some_and(|reply| text_equal(&reply.envelope)),
    );
    let strip = |reply: Exchange| {
        let mut value = reply.value;
        if let Some(map) = value.as_object_mut() {
            map.remove("cursor");
        }
        value
    };
    let (rest, mcp) = (rest.map(strip), mcp.map(strip));
    check(
        evidence,
        &format!("{operation}_rest_mcp_structured_result_exact"),
        rest.is_some() && mcp.is_some() && rest == mcp,
    );
}

fn schema(env: &Env, evidence: &mut Evidence, operation: &str) {
    let reply = env.gateway.request("GET", "/v1/openapi.json", &[], None, 2);
    check(evidence, "reminders_openapi_available", reply.is_ok());
    let Ok(reply) = reply else {
        return;
    };
    let document: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
    let path = &document["paths"][format!("/v1/op/{operation}")];
    let mut valid = reply.status == 200 && path.as_object().is_some_and(|map| !map.is_empty());
    let mut name = format!("{operation}_output_schema");
    if operation == "reminders.items" {
        let shape = &path["post"]["responses"]["200"]["content"]["application/json"]["schema"];
        let definitions = &shape["$defs"];
        let reference =
            definitions["contracts.ReminderItem"]["properties"]["subtasks"]["items"]["$ref"]
                .as_str()
                .unwrap_or_default();
        valid = valid
            && shape["$id"].as_str().is_some_and(|id| !id.is_empty())
            && reference == "#/$defs/contracts.ReminderItem"
            && !definitions[reference.trim_start_matches("#/$defs/")].is_null();
        name.push_str("_with_recursive_subtasks");
    }
    check(evidence, &name, valid);
}

fn groups(env: &Env, evidence: &mut Evidence, list_id: &str) {
    for protocol in PROTOCOLS {
        for (selector, value, expected, kind, sections) in [
            ("smart", "today", 2, "none", 1),
            ("smart", "scheduled", 3, "date", 3),
            ("smart", "all", 4, "list", 2),
            ("smart", "completed", 1, "list", 1),
            ("smart", "flagged", 1, "none", 1),
            ("smart", "assigned", 1, "none", 1),
            ("tag", "work", 2, "none", 1),
        ] {
            let mut query = Map::new();
            query.insert("time_zone".into(), json!(NY));
            query.insert(selector.into(), json!(value));
            let page = env.ask(protocol, "reminders.items", &Value::Object(query));
            let mut valid = page.as_ref().is_some_and(|page| {
                page["count"].as_i64() == Some(expected) && count(&page["sections"]) == sections
            });
            for section in entries(&page.clone().unwrap_or(Value::Null)["sections"]) {
                valid = valid
                    && section["section"]["kind"] == kind
                    && (kind != "list"
                        || section["section"]["list_id"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty()));
            }
            check(
                evidence,
                &format!("{protocol}_{value}_grouping_and_count"),
                valid,
            );
        }
        let completed = env.ask(
            protocol,
            "reminders.items",
            &json!({"time_zone": NY, "list": list_id, "show_completed": true}),
        );
        check(
            evidence,
            &format!("{protocol}_show_completed_before_nesting"),
            completed.is_some_and(|page| page["count"].as_i64() == Some(3)),
        );
        let priorities = env.ask(
            protocol,
            "reminders.items",
            &json!({"time_zone": NY, "smart": "all", "sort": "priority"}),
        );
        let mut found = false;
        for section in entries(&priorities.clone().unwrap_or(Value::Null)["sections"]) {
            let items = entries(&section["items"]).collect::<Vec<_>>();
            if items.len() == 2 && title(&items[0]["reminder"]) == "Bridge Test Undated" {
                found = title(&items[1]["reminder"]) == "Bridge Test Future";
            }
        }
        check(
            evidence,
            &format!("{protocol}_apple_priority_none_sorts_after_low"),
            priorities.is_some() && found,
        );
    }
}

fn replica(env: &Env, evidence: &mut Evidence, operations: &[&str], list_id: &str) {
    let added = start(env.world, 1);
    check(
        evidence,
        "replica_added_after_reminders_for_snapshot",
        added.is_ok(),
    );
    if added.is_err() {
        return;
    }
    for operation in operations {
        let mut query = json!({"time_zone": NY});
        if *operation == "reminders.items" {
            query["list"] = json!(list_id);
        }
        let expected = env.gateway.call(operation, &query, 30).ok().flatten();
        check(
            evidence,
            &format!("{operation}_gateway_before_replica"),
            expected.is_some(),
        );
        let Some(mut expected) = expected else {
            continue;
        };
        if let Some(map) = expected.as_object_mut() {
            map.remove("cursor");
        }
        let endpoints = replicas::endpoints();
        for (index, endpoint) in endpoints.iter().enumerate() {
            let matched = await_until(10, || {
                endpoint
                    .client()
                    .call(operation, &query, 30)
                    .ok()
                    .flatten()
                    .is_some_and(|mut actual| {
                        if let Some(map) = actual.as_object_mut() {
                            map.remove("cursor");
                        }
                        actual == expected
                    })
            });
            let name = if index + 1 == endpoints.len() {
                "snapshot_rebuild"
            } else {
                "live_apply"
            };
            check(
                evidence,
                &format!("{operation}_{name}_matches_gateway"),
                matched,
            );
        }
    }
}

fn zones(env: &Env, evidence: &mut Evidence) {
    for (key, due) in [
        ("before", "2026-03-08T04:30:00Z"),
        ("after", "2026-03-08T07:30:00Z"),
    ] {
        let data = json!({"title": format!("Bridge Test DST {key}"), "list_id": reference("reminders.list", "rm-rem-other"), "completed": false, "flagged": false, "due": due, "due_all_day": false, "assignment": null, "store_coverage": {"assignment": {"status": "complete"}}});
        check(
            evidence,
            &format!("dst_fixture_{key}"),
            env.change("reminders.reminder", &format!("rm-rem-dst-{key}"), &data),
        );
    }
    check(evidence, "dst_fixtures_converged", env.settle(5));
    for protocol in PROTOCOLS {
        for zone in ["UTC", NY] {
            let page = env.ask(
                protocol,
                "reminders.items",
                &json!({"smart": "scheduled", "time_zone": zone}),
            );
            let mut groups: Vec<(String, String)> = vec![];
            for section in entries(&page.clone().unwrap_or(Value::Null)["sections"]) {
                if section["section"]["kind"] != "date" {
                    continue;
                }
                for item in entries(&section["items"]) {
                    groups.push((
                        title(&item["reminder"]).to_owned(),
                        section["section"]["id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    ));
                }
            }
            let group = |name: &str| {
                groups
                    .iter()
                    .rev()
                    .find(|(title, _)| title == name)
                    .map(|(_, id)| id.clone())
                    .unwrap_or_default()
            };
            let (before, after) = (
                group("Bridge Test DST before"),
                group("Bridge Test DST after"),
            );
            let expected = if zone == NY {
                before != after
            } else {
                before == after
            };
            check(
                evidence,
                &format!("{protocol}_viewer_zone_across_dst_{zone}"),
                page.is_some() && !before.is_empty() && !after.is_empty() && expected,
            );
        }
    }
}

fn denied_client(env: &Env, evidence: &mut Evidence) -> Option<Client> {
    let denied = Client::new(env.world, "system:read");
    check(evidence, "restricted_token", denied.is_ok());
    denied.ok()
}
fn sidebar_hidden(env: &Env, evidence: &mut Evidence, protocol: &str, denied: Option<&Client>) {
    let page = denied.and_then(|client| {
        exchange(
            client,
            protocol,
            "reminders.lists",
            &json!({"time_zone": NY}),
            30,
        )
    });
    let _ = env;
    check(
        evidence,
        &format!("{protocol}_visibility_before_sidebar_counts"),
        page.is_some_and(|reply| {
            count(&reply.value["lists"]) == 0
                && count(&reply.value["smart"]) == 0
                && count(&reply.value["tags"]) == 0
        }),
    );
}
fn unsupported_custom_filter(env: &Env, evidence: &mut Evidence) {
    let data = json!({"title": "Bridge Test Unsupported Smart", "smart_list_type": "custom", "smart_filter": null, "store_coverage": {"smart_filter": {"status": "unsupported"}}});
    check(
        evidence,
        "unknown_custom_filter_capture",
        env.change("reminders.list", "rm-rem-custom", &data),
    );
    check(evidence, "unknown_custom_filter_converged", env.settle(5));
}
fn no_count(entry: &Value) -> bool {
    entry["status"] == "unsupported" && entry.get("count").is_none_or(Value::is_null)
}

fn list_boundaries(env: &Env, evidence: &mut Evidence) {
    let denied = denied_client(env, evidence);
    for protocol in PROTOCOLS {
        for zone in ["", "Local", "not/a/zone"] {
            check(
                evidence,
                &format!("{protocol}_invalid_viewer_zone"),
                env.ask(protocol, "reminders.lists", &json!({"time_zone": zone}))
                    .is_none(),
            );
        }
        sidebar_hidden(env, evidence, protocol, denied.as_ref());
    }
    let Ok(fixture) = fixtures() else {
        return;
    };
    let mut data = fixture[4].data.clone();
    data["tags"] = Value::Null;
    data["flagged"] = Value::Null;
    data["assignment"] = json!({"assignee": {"id": "fixture-self", "is_me": null}});
    check(
        evidence,
        "unknown_assignment_identity_capture",
        env.change(fixture[4].collection, fixture[4].key, &data),
    );
    check(
        evidence,
        "unknown_assignment_identity_converged",
        env.settle(5),
    );
    unsupported_custom_filter(env, evidence);
    for protocol in PROTOCOLS {
        let page = env.ask(protocol, "reminders.lists", &json!({"time_zone": NY}));
        let page_ok = page.is_some();
        let page = page.unwrap_or(Value::Null);
        let tile = |id: &str| {
            entries(&page["smart"])
                .rfind(|tile| tile["id"] == id)
                .is_some_and(no_count)
        };
        check(
            evidence,
            &format!("{protocol}_assigned_identity_unknown_has_no_count"),
            page_ok && tile("assigned"),
        );
        check(
            evidence,
            &format!("{protocol}_flag_unknown_has_no_count"),
            page_ok && tile("flagged"),
        );
        check(
            evidence,
            &format!("{protocol}_unknown_tags_hide_partial_counts"),
            page_ok && count(&page["tags"]) == 0,
        );
        let custom = entries(&page["lists"])
            .rfind(|list| title(&list["list"]) == "Bridge Test Unsupported Smart")
            .is_some_and(no_count);
        check(
            evidence,
            &format!("{protocol}_custom_filter_unknown_has_no_count"),
            page_ok && custom,
        );
    }
}

fn item_boundaries(env: &Env, evidence: &mut Evidence, list_id: &str) -> Result<()> {
    let denied = denied_client(env, evidence);
    for protocol in PROTOCOLS {
        for query in [
            json!({"list": list_id}),
            json!({"list": list_id, "time_zone": "not/a/zone"}),
            json!({"time_zone": NY}),
            json!({"list": list_id, "smart": "all", "time_zone": NY}),
            json!({"smart": "invented", "time_zone": NY}),
            json!({"list": list_id, "sort": "invented", "time_zone": NY}),
        ] {
            check(
                evidence,
                &format!("{protocol}_invalid_selector_zone_or_sort"),
                env.ask(protocol, "reminders.items", &query).is_none(),
            );
        }
        sidebar_hidden(env, evidence, protocol, denied.as_ref());
        let hidden = denied.as_ref().and_then(|client| {
            exchange(
                client,
                protocol,
                "reminders.items",
                &json!({"list": list_id, "time_zone": NY}),
                30,
            )
        });
        check(
            evidence,
            &format!("{protocol}_visibility_before_items_and_nesting"),
            hidden.is_some_and(|reply| {
                reply.value["count"].as_i64().unwrap_or(0) == 0
                    && count(&reply.value["sections"]) == 0
            }),
        );
    }
    let fixture = fixtures()?;
    let mut data = fixture[4].data.clone();
    data["assignment"] = json!({"assignee": {"id": "fixture-self", "is_me": null}});
    check(
        evidence,
        "unknown_assignment_identity_capture",
        env.change(fixture[4].collection, fixture[4].key, &data),
    );
    check(
        evidence,
        "unknown_assignment_identity_converged",
        env.settle(5),
    );
    unsupported_custom_filter(env, evidence);
    for protocol in PROTOCOLS {
        let page = env.ask(protocol, "reminders.lists", &json!({"time_zone": NY}));
        let page_ok = page.is_some();
        let page = page.unwrap_or(Value::Null);
        let assigned = entries(&page["smart"])
            .rfind(|tile| tile["id"] == "assigned")
            .is_some_and(no_count);
        check(
            evidence,
            &format!("{protocol}_assigned_identity_unknown_has_no_count"),
            page_ok && assigned,
        );
        let custom = entries(&page["lists"])
            .rfind(|list| title(&list["list"]) == "Bridge Test Unsupported Smart")
            .is_some_and(no_count);
        check(
            evidence,
            &format!("{protocol}_custom_filter_unknown_has_no_count"),
            page_ok && custom,
        );
    }
    Ok(())
}

fn attachments(env: &Env, evidence: &mut Evidence, list_id: &str) {
    for (key, owner) in [("own", "rm-rem-parent"), ("other", "rm-rem-future")] {
        let data = json!({"reminder_id": reference("reminders.reminder", owner), "filename": format!("Bridge Test {key}.png"), "uti": "public.png", "size": 0, "media": null});
        check(
            evidence,
            &format!("attachment_fixture_{key}"),
            env.change(
                "reminders.attachment",
                &format!("rm-rem-attachment-{key}"),
                &data,
            ),
        );
    }
    check(evidence, "attachments_converged", env.settle(5));
    for protocol in PROTOCOLS {
        let page = env.ask(
            protocol,
            "reminders.items",
            &json!({"list": list_id, "time_zone": NY}),
        );
        let mut valid = false;
        let page_ok = page.is_some();
        let page = page.unwrap_or(Value::Null);
        for item in all_items(&page) {
            if title(&item["reminder"]) == "Bridge Test Parent" && count(&item["attachments"]) == 1
            {
                let attachment = &item["attachments"][0];
                valid = attachment["collection"] == "reminders.attachment"
                    && attachment["data"]["reminder_id"] == item["reminder"]["id"]
                    && attachment["data"]["filename"] == "Bridge Test own.png";
            }
        }
        check(
            evidence,
            &format!("{protocol}_attachments_are_owned_records"),
            page_ok && valid,
        );
    }
}

fn walk(items: &[Value], depth: usize, seen: &mut Vec<String>, flags: &mut (bool, bool)) {
    for entry in items {
        let id = entry["reminder"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        flags.0 |= seen.contains(&id);
        seen.push(id);
        flags.1 |= depth == 3 && title(&entry["reminder"]) == "Bridge Test grandchild";
        walk(
            entry["subtasks"].as_array().map_or(&[], Vec::as_slice),
            depth + 1,
            seen,
            flags,
        );
    }
}

fn nesting(env: &Env, evidence: &mut Evidence, list_id: &str) -> Result<()> {
    let parents = [
        ("grandchild", "rm-rem-child"),
        ("cycle-a", "rm-rem-cycle-b"),
        ("cycle-b", "rm-rem-cycle-a"),
    ];
    let data_for = |key: &str| json!({"title": format!("Bridge Test {key}"), "list_id": reference("reminders.list", "list"), "completed": false, "section_id": "section-a"});
    for (key, _) in parents {
        check(
            evidence,
            &format!("nested_fixture_{key}"),
            env.change(
                "reminders.reminder",
                &format!("rm-rem-{key}"),
                &data_for(key),
            ),
        );
    }
    check(evidence, "nested_fixture_aliases_converged", env.settle(5));
    for (key, parent) in parents {
        let mut data = data_for(key);
        data["parent_id"] = reference("reminders.reminder", parent);
        check(
            evidence,
            &format!("nested_fixture_link_{key}"),
            env.change("reminders.reminder", &format!("rm-rem-{key}"), &data),
        );
    }
    check(evidence, "nested_fixtures_converged", env.settle(5));
    let query = json!({"time_zone": NY, "list": list_id});
    for protocol in PROTOCOLS {
        let first = env.ask(protocol, "reminders.items", &query);
        let second = env.ask(protocol, "reminders.items", &query);
        let mut seen: Vec<String> = vec![];
        let mut flags = (false, false);
        let first_page = first.clone().unwrap_or(Value::Null);
        for section in entries(&first_page["sections"]) {
            walk(
                section["items"].as_array().map_or(&[], Vec::as_slice),
                1,
                &mut seen,
                &mut flags,
            );
        }
        let (duplicate, deep) = flags;
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        let same = first
            .as_ref()
            .zip(second.as_ref())
            .is_some_and(|(a, b)| a["count"] == b["count"] && a["sections"] == b["sections"]);
        check(
            evidence,
            &format!("{protocol}_recursive_subtasks_and_deterministic_cycle_break"),
            first.is_some()
                && second.is_some()
                && first_page["count"].as_i64() == Some(5)
                && unique.len() == 5
                && !duplicate
                && deep
                && same,
        );
    }
    let fixture = fixtures()?;
    let mut data = fixture[4].data.clone();
    data["completed"] = json!(true);
    check(
        evidence,
        "completed_parent_capture",
        env.change(fixture[4].collection, fixture[4].key, &data),
    );
    check(evidence, "completed_parent_converged", env.settle(5));
    let filtered = env.ask("rest", "reminders.items", &query);
    let ok = filtered.is_some();
    let filtered = filtered.unwrap_or(Value::Null);
    let orphan = all_items(&filtered).iter().any(|root| {
        title(&root["reminder"]) == "Bridge Test Child" && count(&root["subtasks"]) == 1
    });
    check(
        evidence,
        "completion_filter_before_recursive_nesting",
        ok && filtered["count"].as_i64() == Some(4) && orphan,
    );
    Ok(())
}

fn sorts(env: &Env, evidence: &mut Evidence) -> Result<()> {
    let fixture = fixtures()?;
    let sort_list = json!({"title": "Bridge Test Sorts", "sections": []});
    check(
        evidence,
        "sort_list_capture",
        env.change("reminders.list", "rm-rem-sort-list", &sort_list),
    );
    check(evidence, "sort_list_converged", env.settle(5));
    let list_id = env.find("reminders.list", Some("Bridge Test Sorts"));
    check(evidence, "sort_list_reference", list_id.is_some());
    let list_id = list_id.unwrap_or_default();
    for (index, key) in [
        "rm-rem-sort-future",
        "rm-rem-sort-undated",
        "rm-rem-sort-high",
        "rm-rem-sort-medium",
    ]
    .into_iter()
    .enumerate()
    {
        let mut data = fixture[[7, 8, 7, 7][index]].data.clone();
        data["list_id"] = reference("reminders.list", "rm-rem-sort-list");
        match index {
            0 => {
                data["manual_order"] = json!(2);
                data["created_at"] = json!("2026-01-02T12:00:00Z");
            }
            1 => {
                data["manual_order"] = json!(1);
                data["created_at"] = Value::Null;
            }
            2 => {
                data["title"] = json!("Bridge Test High");
                data["priority"] = json!(1);
                data["manual_order"] = json!(3);
                data["created_at"] = json!("2026-01-01T12:00:00Z");
                data["due"] = Value::Null;
            }
            _ => {
                data["title"] = json!("Bridge Test Medium");
                data["priority"] = json!(5);
                data["manual_order"] = Value::Null;
                data["created_at"] = json!("2026-01-03T12:00:00Z");
                data["due"] = Value::Null;
            }
        }
        check(
            evidence,
            "sort_fixture_capture",
            env.change("reminders.reminder", key, &data),
        );
    }
    check(evidence, "sort_fixtures_converged", env.settle(5));
    for protocol in PROTOCOLS {
        for (sort, titles) in [
            ("manual", Some(["Undated", "Future", "High", "Medium"])),
            ("created", Some(["High", "Future", "Medium", "Undated"])),
            ("priority", Some(["High", "Medium", "Undated", "Future"])),
            ("title", Some(["Future", "High", "Medium", "Undated"])),
            ("due", None),
        ] {
            let page = env.ask(
                protocol,
                "reminders.items",
                &json!({"list": list_id, "time_zone": NY, "sort": sort}),
            );
            let page_ok = page.is_some();
            let page = page.unwrap_or(Value::Null);
            let sections = entries(&page["sections"]).collect::<Vec<_>>();
            let mut valid = page_ok
                && page["count"].as_i64() == Some(4)
                && sections.len() == 1
                && count(&sections[0]["items"]) == 4;
            if valid {
                for (position, item) in entries(&sections[0]["items"]).enumerate() {
                    let name = title(&item["reminder"]);
                    valid = valid
                        && titles.map_or(position != 0 || name == "Bridge Test Future", |titles| {
                            name == format!("Bridge Test {}", titles[position])
                        });
                }
            }
            check(
                evidence,
                &format!("{protocol}_{sort}_sort_and_missing_values_last"),
                valid,
            );
        }
    }
    Ok(())
}

fn perf(world: &World, evidence: &mut Evidence) -> Result<()> {
    let sibling = world.sibling("perf");
    check(evidence, "1000_reminders_perf_world", sibling.is_ok());
    let Ok(sibling) = sibling else {
        return Ok(());
    };
    let env = Env {
        world: &sibling,
        gateway: Client::new(&sibling, SCOPE)?,
        scope: SCOPE,
        replicas: false,
    };
    let seeded = seed(&sibling, 1000, &env.gateway);
    if !seeded {
        check(evidence, "1000_reminders_perf_world", false);
        return Ok(());
    }
    check(evidence, "1000_reminders_perf_convergence", env.settle(20));
    let list_id = env.find("reminders.list", None);
    check(evidence, "1000_reminders_perf_list", list_id.is_some());
    let query = json!({"list": list_id.unwrap_or_default(), "time_zone": NY});
    for protocol in PROTOCOLS {
        let client = Client::new(&sibling, SCOPE);
        check(evidence, &format!("{protocol}_perf_token"), client.is_ok());
        let Ok(client) = client else {
            continue;
        };
        let first = exchange(&client, protocol, "reminders.items", &query, 30);
        let exact = first
            .as_ref()
            .is_some_and(|reply| reply.value["count"].as_i64() == Some(1000));
        check(
            evidence,
            &format!("{protocol}_perf_exactly_1000_items"),
            exact,
        );
        if !exact {
            continue;
        }
        let (mut values, mut server_values) = (vec![], vec![]);
        let (mut failures, mut missing, mut longest_text, mut invalid_text) = (0, 0, 0, 0);
        for _ in 0..100 {
            let started = Instant::now();
            let reply = exchange(&client, protocol, "reminders.items", &query, 30);
            let milliseconds = started.elapsed().as_secs_f64() * 1000.0;
            let Some(reply) = reply else {
                failures += 1;
                continue;
            };
            values.push(milliseconds);
            let (server, text_length, known) = timing(protocol, &reply);
            if known {
                server_values.push(server);
            } else {
                missing += 1;
            }
            if protocol == "mcp" && !text_equal(&reply.envelope) {
                invalid_text += 1;
            }
            longest_text = longest_text.max(text_length);
        }
        check(
            evidence,
            &format!("{protocol}_1000_items_gateway_p95_under_10ms"),
            server_values.len() == 100 && missing == 0 && percentile95(&mut server_values) < 10.0,
        );
        check(
            evidence,
            &format!("{protocol}_1000_items_roundtrip_complete"),
            values.len() == 100 && failures == 0,
        );
        if protocol == "mcp" {
            check(
                evidence,
                "mcp_large_result_text_matches_structured_json",
                longest_text > 0 && invalid_text == 0,
            );
        }
    }
    Ok(())
}

fn hardening(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    natural_titles(world, evidence)?;
    large_selection(world, evidence)
}

fn natural_titles(world: &World, evidence: &mut Evidence) -> Result<()> {
    check(evidence, "natural_title_world", true);
    let env = Env {
        world,
        gateway: Client::new(world, HARDENING_SCOPE)?,
        scope: HARDENING_SCOPE,
        replicas: false,
    };
    if !seed(world, 0, &env.gateway) {
        return Ok(());
    }
    check(
        evidence,
        "natural_title_list",
        env.change(
            "reminders.list",
            "rm-natural-list",
            &json!({"title": "Bridge Test Numeric Titles"}),
        ),
    );
    for (index, name) in ["Item 10", "Item 2", "item 1", "Item 12"]
        .into_iter()
        .enumerate()
    {
        let data = json!({"title": name, "list_id": reference("reminders.list", "rm-natural-list"), "completed": false});
        check(
            evidence,
            "natural_title_fixture",
            env.change("reminders.reminder", &format!("rm-natural-{index}"), &data),
        );
    }
    check(evidence, "natural_title_convergence", env.settle(10));
    let list = env.find("reminders.list", Some("Bridge Test Numeric Titles"));
    check(
        evidence,
        "natural_title_list_resolved",
        list.as_ref().is_some_and(|id| !id.is_empty()),
    );
    let list_id = list.unwrap_or_default();
    for protocol in PROTOCOLS {
        let page = env.ask(
            protocol,
            "reminders.items",
            &json!({"list": list_id, "time_zone": NY, "sort": "title"}),
        );
        let page_ok = page.is_some();
        let page = page.unwrap_or(Value::Null);
        let titles: Vec<&str> = all_items(&page)
            .iter()
            .map(|item| title(&item["reminder"]))
            .collect();
        check(
            evidence,
            &format!("{protocol}_numeric_case_insensitive_title_order"),
            page_ok && titles == ["item 1", "Item 2", "Item 10", "Item 12"],
        );
        for operation in ["messages.chats", "sources.status"] {
            let reply = exchange(&env.gateway, protocol, operation, &json!({}), 30);
            let measured = reply
                .as_ref()
                .is_some_and(|reply| timing(protocol, reply).2);
            check(
                evidence,
                &format!("{protocol}_descriptor_timing_{operation}"),
                reply.is_some() && measured == (operation == "messages.chats"),
            );
        }
    }
    other_projection_parity(&env, evidence)
}

fn other_projection_parity(env: &Env, evidence: &mut Evidence) -> Result<()> {
    let seconds = jiff::Timestamp::now().as_second();
    let at = jiff::Timestamp::from_second(seconds - seconds % 60 + 3600)
        .map_err(|e| Error::Invalid(format!("time: {e}")))?;
    let stamp = at.to_string();
    let fixture = put(
        env.world,
        &[
            observation(
                "messages.chat",
                "rm-timing-chat",
                &json!({"service": "iMessage", "display_name": "Bridge Test Timing", "handles": [], "is_group": false}),
            ),
            observation(
                "messages.message",
                "rm-timing-message",
                &json!({"chat_id": reference("messages.chat", "rm-timing-chat"), "sent_at": stamp, "text": "[ab-test] timing parity", "kind": "message", "is_from_me": true, "parts": []}),
            ),
        ],
    );
    check(evidence, "timed_messages_fixture", fixture.is_ok());
    check(
        evidence,
        "timed_calendar_fixture",
        env.world
            .control(
                "/calendar",
                &json!({"action": "calendar", "key": "test-calendar"}),
            )
            .is_ok(),
    );
    let end = jiff::Timestamp::from_second(at.as_second() + 1800)
        .map_err(|e| Error::Invalid(format!("time: {e}")))?;
    check(
        evidence,
        "timed_event_fixture",
        env.world
            .control(
                "/calendar",
                &json!({"action": "create", "key": "rm-timing-event", "data": {"title": "Bridge Test Timing", "start": stamp, "end": end.to_string()}}),
            )
            .is_ok(),
    );
    check(evidence, "timed_projection_convergence", env.settle(10));
    let window = |delta: i64| {
        jiff::Timestamp::from_second(at.as_second() + delta).map(|time| time.to_string())
    };
    let (start, finish) = (
        window(-3600).map_err(|e| Error::Invalid(format!("time: {e}")))?,
        window(3600).map_err(|e| Error::Invalid(format!("time: {e}")))?,
    );
    for (operation, query, key) in [
        ("messages.chats", json!({}), "chats"),
        (
            "calendar.agenda",
            json!({"start": start, "end": finish}),
            "records",
        ),
    ] {
        let response = env.gateway.call(operation, &query, 30).ok().flatten();
        check(
            evidence,
            &format!("{operation}_nonempty_timed_projection"),
            response
                .as_ref()
                .is_some_and(|response| count(&response[key]) == 1),
        );
        parity(env, evidence, operation, &query);
    }
    Ok(())
}

fn large_selection(world: &World, evidence: &mut Evidence) -> Result<()> {
    const COUNT: usize = 33_000;
    let sibling = world.sibling("large");
    check(evidence, "large_selection_world", sibling.is_ok());
    let Ok(sibling) = sibling else {
        return Ok(());
    };
    let env = Env {
        world: &sibling,
        gateway: Client::new(&sibling, "reminders:read,system:read")?,
        scope: "reminders:read,system:read",
        replicas: false,
    };
    if !seed(&sibling, COUNT as u64, &env.gateway) {
        return Ok(());
    }
    env.quiet(60);
    check(evidence, "large_selection_convergence", env.settle(60));
    check(evidence, "large_selection_read_token", true);
    let records = read_all(&env.gateway, false, false);
    let mut list_id = String::new();
    let mut ids: Vec<String> = vec![];
    for record in records.iter().flat_map(|read| read.records.iter()) {
        if record["collection"] == "reminders.list" {
            record["id"]
                .as_str()
                .unwrap_or_default()
                .clone_into(&mut list_id);
        }
        if record["collection"] == "reminders.reminder" {
            ids.push(record["id"].as_str().unwrap_or_default().to_owned());
        }
    }
    check(
        evidence,
        "large_selection_above_sqlite_variable_limit",
        records.is_ok() && ids.len() == COUNT && !list_id.is_empty(),
    );
    if ids.len() != COUNT || list_id.is_empty() {
        return Ok(());
    }
    let owners = [ids[0].clone(), ids[COUNT - 1].clone()];
    for (index, owner) in owners.iter().enumerate() {
        let data = json!({"reminder_id": owner, "filename": format!("Bridge Test Boundary {index}.png"), "uti": "public.png", "size": 0, "media": null});
        check(
            evidence,
            "large_selection_attachment",
            env.change(
                "reminders.attachment",
                &format!("rm-boundary-attachment-{index}"),
                &data,
            ),
        );
    }
    check(evidence, "large_selection_attachment_convergence", {
        env.quiet(60);
        env.settle(60)
    });
    let query = Client::new(&sibling, "reminders:read,system:read");
    check(evidence, "large_selection_query_token", query.is_ok());
    let Ok(query) = query else {
        return Ok(());
    };
    let page = exchange(
        &query,
        "rest",
        "reminders.items",
        &json!({"list": list_id, "time_zone": NY}),
        120,
    );
    let page_ok = page.is_some();
    let page = page.map_or(Value::Null, |reply| reply.value);
    let mut attachments: Vec<(String, String)> = vec![];
    let mut total = 0;
    for item in all_items(&page) {
        total += 1;
        for attachment in entries(&item["attachments"]) {
            if attachment["data"]["reminder_id"] == item["reminder"]["id"] {
                attachments.push((
                    attachment["data"]["reminder_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    attachment["data"]["filename"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                ));
            }
        }
    }
    let named = |owner: &String, name: &str| {
        attachments
            .iter()
            .any(|(id, filename)| id == owner && filename == name)
    };
    check(
        evidence,
        "rest_large_selection_and_attachment_owners_exact",
        page_ok
            && page["count"].as_u64() == Some(COUNT as u64)
            && total == COUNT
            && attachments.len() == 2
            && named(&owners[0], "Bridge Test Boundary 0.png")
            && named(&owners[1], "Bridge Test Boundary 1.png"),
    );
    Ok(())
}
