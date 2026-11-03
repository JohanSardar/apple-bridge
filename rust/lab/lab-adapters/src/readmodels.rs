#![allow(clippy::too_many_lines)]
use crate::{assertions::gateway_aliases, http::Client};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Map, Value, json};
use std::time::Instant;

pub const SCENARIOS: [&str; 5] = [
    "WEB-RM-MEDIA-COLLECTION",
    "WEB-RM-SEARCH",
    "WEB-RM-THREAD-ORIGINATOR",
    "WEB-RM-THREAD-ORIGINATOR-PERF",
    "WEB-RM-REMINDERS-DEMO",
];
pub const PROTOCOLS: [&str; 2] = ["rest", "mcp"];

#[must_use]
pub fn reference(collection: &str, key: &str) -> Value {
    json!({"$ref": {"collection": collection, "native_key": key}})
}
#[must_use]
pub fn observation(collection: &str, key: &str, data: &Value) -> Value {
    json!({"kind": "upsert", "collection": collection, "native_key": key, "data": data})
}
pub fn put(world: &World, records: &[Value]) -> Result<()> {
    world
        .control("/readmodels", &json!({"action": "put", "records": records}))
        .map(|_| ())
}
#[must_use]
pub fn message(key: &str, sent_at: &str, extra: &Value) -> Value {
    let mut data = Map::new();
    data.insert("chat_id".into(), reference("messages.chat", "rm-chat-000"));
    data.insert("sent_at".into(), json!(sent_at));
    data.insert("kind".into(), json!("message"));
    data.insert("text".into(), json!(format!("[ab-test] {key}")));
    data.insert("is_from_me".into(), json!(false));
    data.insert("parts".into(), json!([]));
    for (name, value) in extra.as_object().into_iter().flatten() {
        data.insert(name.clone(), value.clone());
    }
    observation("messages.message", key, &Value::Object(data))
}
pub fn call(client: &Client, protocol: &str, operation: &str, input: &Value) -> Option<Value> {
    let reply = if protocol == "mcp" {
        client.call_mcp(operation, input, 30)
    } else {
        client.call(operation, input, 30)
    };
    reply.ok().flatten()
}
pub fn at(second: i64) -> Result<String> {
    offset("2026-05-01T00:00:00Z", second)
}
pub fn offset(base: &str, second: i64) -> Result<String> {
    let base = base
        .parse::<jiff::Timestamp>()
        .map_err(|e| Error::Invalid(format!("base time: {e}")))?;
    Ok(base
        .checked_add(jiff::SignedDuration::from_secs(second))
        .map_err(|e| Error::Invalid(format!("time: {e}")))?
        .to_string())
}
fn ids(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default().to_owned())
        .collect()
}
fn media_collection(world: &World, evidence: &mut Evidence) -> Result<()> {
    evidence.check("world_start", true, "world");
    let base = 1_777_593_600_i64;
    let mut records = vec![observation(
        "messages.chat",
        "rm-chat-000",
        &json!({"display_name": "Bridge Test Media"}),
    )];
    for i in 0..9 {
        let key = format!("media-message-{i}");
        let mime = if i % 2 == 1 {
            "video/quicktime"
        } else {
            "image/png"
        };
        let mut extra = json!({});
        if i == 7 {
            extra["deleted_at"] = json!(at(3600)?);
        }
        records.push(message(&key, &at(i)?, &extra));
        records.push(observation(
            "messages.attachment",
            &format!("media-attachment-{i}"),
            &json!({"message_id": reference("messages.message", &key), "mime_type": mime, "filename": format!("synthetic-{i}"), "hidden": i == 8}),
        ));
    }
    let committed = put(world, &records);
    evidence.check(
        "interleaved_synthetic_sequence_committed",
        committed.is_ok(),
        "put",
    );
    if committed.is_err() {
        return Ok(());
    }
    let gateway = Client::new(world, "messages:read,system:read")?;
    let aliases = gateway_aliases(&gateway);
    evidence.check("aliases_resolved", aliases.is_ok(), "aliases");
    let Ok(aliases) = aliases else {
        return Ok(());
    };
    let alias = |collection: &str, key: &str| {
        aliases
            .get(&format!("messages/{collection}/{key}"))
            .cloned()
            .unwrap_or_default()
    };
    let chat_id = alias("messages.chat", "rm-chat-000");
    let expected: Vec<String> = (0..7)
        .rev()
        .map(|i| alias("messages.attachment", &format!("media-attachment-{i}")))
        .collect();
    let mut parity: Vec<Value> = vec![];
    for protocol in PROTOCOLS {
        let start = Instant::now();
        let query = |kind: &str, limit: u64, after: &str| {
            let mut input = json!({"chat_id": chat_id, "kind": kind});
            if limit > 0 {
                input["limit"] = json!(limit);
            }
            if !after.is_empty() {
                input["after"] = json!(after);
            }
            call(&gateway, protocol, "messages.collection", &input)
        };
        let page = query("media", 2, "");
        evidence.check(
            &format!("{protocol}_media_kind_accepted"),
            page.is_some(),
            "media",
        );
        let Some(mut page) = page else { continue };
        let mut all: Vec<Value> = page["items"].as_array().cloned().unwrap_or_default();
        let first_cursor = page["next"].as_str().unwrap_or_default().to_owned();
        let live_key = format!("media-live-{protocol}");
        let live = put(
            world,
            &[
                message(&live_key, &at(3600)?, &json!({})),
                observation(
                    "messages.attachment",
                    &format!("{live_key}-attachment"),
                    &json!({"message_id": reference("messages.message", &live_key), "mime_type": "video/mp4", "filename": live_key}),
                ),
            ],
        );
        let mut valid = live.is_ok();
        let mut pages = 0;
        while page["next"].as_str().is_some_and(|next| !next.is_empty()) && pages < 10 {
            let next = page["next"].as_str().unwrap_or_default().to_owned();
            let result = query("media", 2, &next);
            valid &= result
                .as_ref()
                .is_some_and(|p| p["items"].as_array().is_none_or(|i| i.len() <= 2));
            page = result.unwrap_or_else(|| json!({"items": []}));
            all.extend(page["items"].as_array().cloned().unwrap_or_default());
            pages += 1;
        }
        evidence.check(
            &format!("{protocol}_cursor_paging_no_gaps_or_duplicates_during_live_insert"),
            valid && page["next"].as_str().is_none_or(str::is_empty) && ids(&all) == expected,
            "paging",
        );
        let mut geometry = true;
        for (index, record) in all.iter().enumerate() {
            let n = 6 - i64::try_from(index).unwrap_or(0);
            let sent = record["data"]["sent_at"]
                .as_str()
                .and_then(|text| text.parse::<jiff::Timestamp>().ok());
            geometry &= record["data"]["message_id"]
                == json!(alias("messages.message", &format!("media-message-{n}")))
                && sent.is_some_and(|sent| sent.as_second() == base + n);
        }
        evidence.check(
            &format!("{protocol}_owning_message_and_sent_at_projected"),
            geometry,
            "projection",
        );
        if protocol == "rest" {
            parity.clone_from(&all);
        } else {
            evidence.check(
                "rest_mcp_items_and_projection_identical",
                parity == all,
                "parity",
            );
        }
        let fresh = query("media", 1, "");
        evidence.check(
            &format!("{protocol}_live_insert_visible_at_head"),
            fresh.as_ref().is_some_and(|f| {
                f["items"].as_array().is_some_and(|i| i.len() == 1)
                    && f["items"][0]["data"]["filename"] == json!(live_key)
            }),
            "head",
        );
        evidence.check(
            &format!("{protocol}_cursor_bound_to_kind"),
            query("photos", 0, &first_cursor).is_none(),
            "cursor kind",
        );
        evidence.check(
            &format!("{protocol}_page_limit_enforced"),
            query("media", 201, "").is_none(),
            "limit",
        );
        let restricted = Client::new(world, "system:read")?;
        let hidden = call(
            &restricted,
            protocol,
            "messages.collection",
            &json!({"chat_id": chat_id, "kind": "media"}),
        );
        evidence.check(
            &format!("{protocol}_visibility_preserved"),
            hidden.is_some_and(|h| h["items"].as_array().is_none_or(Vec::is_empty)),
            "visibility",
        );
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        evidence.check(
            &format!("{protocol}_journey_under_one_second"),
            elapsed < 1000.0,
            &format!("{elapsed:.0} ms"),
        );
        let hidden = put(
            world,
            &[message(
                &live_key,
                &at(3600)?,
                &json!({"deleted_at": at(7200)?}),
            )],
        );
        evidence.check(
            &format!("{protocol}_live_fixture_hidden_before_parity_repeat"),
            hidden.is_ok(),
            "hide",
        );
    }
    Ok(())
}
pub fn readmodels(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    match evidence.scenario.as_str() {
        "WEB-RM-MEDIA-COLLECTION" => media_collection(world, evidence),
        "WEB-RM-SEARCH" => crate::search::search(world, evidence),
        "MSG-DECODE-BALLOON" | "MSG-DECODE-FORMATTING" => {
            crate::messages_a::decode::run(world, evidence)
        }
        "WEB-CON-LIST" => crate::contacts_list::readmodels(world, evidence),
        "WEB-RM-THREAD-ORIGINATOR" | "WEB-RM-THREAD-ORIGINATOR-PERF" => {
            crate::threads::originator(world, evidence)
        }
        "WEB-RM-REMINDERS-DEMO" => crate::reminders_demo::reminders_demo(world, evidence),
        other => Err(Error::Invalid(format!("readmodels not ported for {other}"))),
    }
}
