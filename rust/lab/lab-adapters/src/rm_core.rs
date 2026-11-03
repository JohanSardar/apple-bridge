#![allow(clippy::too_many_lines)]
use crate::{
    assertions::{Aliases, Visibility, await_until, gateway_aliases, truth_records},
    fastclient::Fast,
    http::Client,
    readmodels::{PROTOCOLS, at, message, observation, put, reference},
    replicas, rm_chats, rm_thread,
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

pub const SCOPE: &str = "messages:read,contacts:read,system:read";
const SEED_SECONDS: u64 = 900;

pub(crate) fn add(evidence: &mut Evidence, name: &str, passed: bool) {
    evidence.check(name, passed, "");
}
#[must_use]
pub(crate) fn items(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}
#[must_use]
pub(crate) fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}
pub(crate) fn clients(world: &World, scope: &str) -> Result<Vec<(&'static str, Fast)>> {
    let client = Client::new(world, scope)?;
    Ok(PROTOCOLS
        .iter()
        .map(|protocol| (*protocol, Fast::of(&client)))
        .collect())
}
pub(crate) fn aliases(world: &World) -> Aliases {
    Client::new(world, SCOPE)
        .and_then(|client| gateway_aliases(&client))
        .unwrap_or_default()
}
#[must_use]
pub(crate) fn alias(aliases: &Aliases, collection: &str, key: &str) -> String {
    aliases
        .get(&format!("messages/{collection}/{key}"))
        .cloned()
        .unwrap_or_default()
}
pub(crate) fn chat_row(client: &Fast, protocol: &str, id: &str) -> Option<Value> {
    let result = client.call(protocol, "messages.chats", &json!({"limit": 200}))?;
    items(&result["chats"])
        .iter()
        .find(|row| row["chat"]["id"] == json!(id))
        .cloned()
}
pub(crate) fn put_ok(world: &World, records: &[Value]) -> bool {
    put(world, records).is_ok()
}
pub(crate) fn reaction(
    key: &str,
    kind: &str,
    removed: bool,
    second: i64,
    extra: &Value,
) -> Result<Value> {
    let mut data = json!({"message_id": reference("messages.message", "rm-root"), "sender_handle_id": "synthetic-sender", "part_index": 0, "kind": kind, "removed": removed, "sent_at": at(second)?});
    for (name, value) in extra.as_object().into_iter().flatten() {
        data[name] = value.clone();
    }
    Ok(observation("messages.reaction", key, &data))
}
fn sorted_expected(truth: &[Value], chat_id: &str) -> Vec<Value> {
    let mut expected: Vec<Value> = truth
        .iter()
        .filter(|r| r["collection"] == "messages.message" && r["data"]["chat_id"] == json!(chat_id))
        .cloned()
        .collect();
    expected.sort_by(|a, b| {
        text(&a["data"]["sent_at"])
            .cmp(text(&b["data"]["sent_at"]))
            .then_with(|| text(&a["id"]).cmp(text(&b["id"])))
    });
    expected
}
fn boundaries(world: &World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    for (protocol, client) in clients(world, SCOPE)? {
        let probes = [
            ("messages.chats", json!({"limit": 201})),
            ("messages.chats", json!({"after": "broken"})),
            (
                "messages.thread",
                json!({"chat_id": chat_id, "before": "broken"}),
            ),
            (
                "messages.thread",
                json!({"chat_id": chat_id, "anchor": "latest", "after": "broken"}),
            ),
            (
                "messages.collection",
                json!({"chat_id": chat_id, "kind": "unknown"}),
            ),
        ];
        for (index, (name, input)) in probes.iter().enumerate() {
            add(
                evidence,
                &format!("{protocol}_invalid_read_model_input_{index}"),
                client.call(protocol, name, input).is_none(),
            );
        }
    }
    Ok(())
}
fn live(world: &World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let root = message("rm-root", &at(0)?, &json!({"is_from_me": true}));
    let unread = message("rm-unread", &at(1)?, &json!({}));
    let reply = message(
        "rm-reply",
        &at(2)?,
        &json!({"is_from_me": true, "thread_originator_id": reference("messages.message", "rm-root")}),
    );
    let tail = message("rm-tail", &at(3)?, &json!({"is_from_me": true}));
    let mut records = vec![
        root,
        unread,
        reply,
        tail,
        reaction("rm-love", "love", false, 3, &json!({}))?,
        reaction("rm-like", "like", false, 4, &json!({}))?,
        reaction("rm-old-remove", "love", true, 5, &json!({}))?,
        reaction("rm-emoji", "emoji", false, 6, &json!({"emoji": "🧪"}))?,
        reaction(
            "rm-emoji-removed",
            "emoji",
            true,
            7,
            &json!({"emoji": "🧪"}),
        )?,
        reaction("rm-other-emoji", "emoji", false, 8, &json!({"emoji": "🎉"}))?,
        reaction(
            "rm-other-sender",
            "question",
            false,
            9,
            &json!({"sender_handle_id": "synthetic-other"}),
        )?,
        reaction(
            "rm-sticker-one",
            "sticker",
            false,
            6,
            &json!({"sticker_id": "one"}),
        )?,
        reaction(
            "rm-sticker-one-latest",
            "sticker",
            false,
            7,
            &json!({"sticker_id": "one"}),
        )?,
        reaction(
            "rm-sticker-two",
            "sticker",
            false,
            8,
            &json!({"sticker_id": "two"}),
        )?,
    ];
    for (index, mime) in [
        "image/png",
        "video/mp4",
        "audio/aac",
        "application/pdf",
        "image/png",
    ]
    .iter()
    .enumerate()
    {
        records.push(observation(
            "messages.attachment",
            &format!("rm-attachment-{index}"),
            &json!({"message_id": reference("messages.message", "rm-root"), "mime_type": mime, "filename": "synthetic", "is_sticker": index == 4, "derivatives": {"thumb": "synthetic-digest"}, "media": {"width": 64}}),
        ));
    }
    add(
        evidence,
        "inbound_reaction_attachment_reply_capture",
        put_ok(world, &records),
    );
    let map = aliases(world);
    let root_id = alias(&map, "messages.message", "rm-root");
    let unread_id = alias(&map, "messages.message", "rm-unread");
    for (protocol, client) in clients(world, SCOPE)? {
        let summary = chat_row(&client, protocol, chat_id);
        add(
            evidence,
            &format!("{protocol}_live_reaction_preview_and_unread"),
            summary.is_some_and(|s| {
                s["unread_count"] == 1
                    && s["last_message"]["preview"]["kind"] == "reaction"
                    && s["last_message"]["preview"]["reaction"]["target_excerpt"]
                        == "[ab-test] rm-root"
            }),
        );
        let centered = client
            .call(
                protocol,
                "messages.thread",
                &json!({"chat_id": chat_id, "anchor": "unread", "limit": 5}),
            )
            .is_some_and(|t| {
                t["unread_anchor"] == json!(unread_id)
                    && items(&t["items"]).len() == 5
                    && t["items"][2]["id"] == json!(unread_id)
            });
        add(
            evidence,
            &format!("{protocol}_unread_anchor_centered"),
            centered,
        );
        let folded = client
            .call(
                protocol,
                "messages.thread",
                &json!({"chat_id": chat_id, "anchor": root_id, "limit": 1}),
            )
            .is_some_and(|t| {
                let item = &t["items"][0];
                let mut valid = items(&t["items"]).len() == 1
                    && item["id"] == json!(root_id)
                    && items(&item["reactions"]).len() == 5
                    && item["reply_count"] == 1
                    && items(&item["attachments"]).len() == 5;
                if valid {
                    for attachment in items(&item["attachments"]) {
                        valid &= !attachment["data"]["derivatives"].is_null()
                            && !attachment["data"]["media"].is_null();
                    }
                    let mut kinds: Vec<&str> = items(&item["reactions"])
                        .iter()
                        .map(|r| text(&r["data"]["kind"]))
                        .collect();
                    kinds.sort_unstable();
                    valid &= kinds == ["emoji", "like", "question", "sticker", "sticker"];
                }
                valid
            });
        add(
            evidence,
            &format!("{protocol}_explicit_anchor_folded_reactions_full_attachments_reply_count"),
            folded,
        );
        for kind in [
            "photos",
            "videos",
            "audio",
            "documents",
            "links",
            "locations",
        ] {
            let want = usize::from(kind != "links" && kind != "locations");
            let valid = client
                .call(
                    protocol,
                    "messages.collection",
                    &json!({"chat_id": chat_id, "kind": kind}),
                )
                .is_some_and(|r| items(&r["items"]).len() == want);
            add(evidence, &format!("{protocol}_collection_{kind}"), valid);
        }
    }
    add(
        evidence,
        "classic_removal_capture",
        put_ok(
            world,
            &[
                reaction("rm-like-remove", "like", true, 10, &json!({}))?,
                reaction(
                    "rm-sticker-remove",
                    "sticker",
                    true,
                    11,
                    &json!({"sticker_id": "one"}),
                )?,
            ],
        ),
    );
    for (protocol, client) in clients(world, SCOPE)? {
        let valid = client
            .call(
                protocol,
                "messages.thread",
                &json!({"chat_id": chat_id, "anchor": root_id, "limit": 1}),
            )
            .is_some_and(|t| {
                items(&t["items"]).len() == 1 && items(&t["items"][0]["reactions"]).len() == 3
            });
        add(
            evidence,
            &format!("{protocol}_classic_and_sticker_removal_folded"),
            valid,
        );
    }
    let read = message("rm-unread", &at(1)?, &json!({"read_at": at(60)?}));
    let edited = message(
        "rm-reply",
        &at(20)?,
        &json!({"is_from_me": true, "edited_at": at(60)?, "text": "[ab-test] edited", "thread_originator_id": reference("messages.message", "rm-root")}),
    );
    add(
        evidence,
        "read_edit_capture",
        put_ok(world, &[read, edited]),
    );
    for (protocol, client) in clients(world, SCOPE)? {
        let summary = chat_row(&client, protocol, chat_id);
        add(
            evidence,
            &format!("{protocol}_read_edit_live_summary"),
            summary.is_some_and(|s| {
                s["unread_count"] == 0
                    && s["last_message"]["preview"]["kind"] == "edited"
                    && s["last_message"]["preview"]["text"] == "[ab-test] edited"
            }),
        );
        let valid = client
            .call(
                protocol,
                "messages.thread",
                &json!({"chat_id": chat_id, "anchor": "unread", "limit": 5}),
            )
            .is_some_and(|t| {
                text(&t["unread_anchor"]).is_empty()
                    && items(&t["items"]).len() == 5
                    && text(&t["after"]).is_empty()
            });
        add(
            evidence,
            &format!("{protocol}_unread_falls_back_to_latest"),
            valid,
        );
    }
    Ok(())
}
fn visibility(world: &World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let denied = Client::new(world, "system:read")?;
    add(evidence, "restricted_token", true);
    for protocol in PROTOCOLS {
        let client = Fast::of(&denied);
        let chats = client.call(protocol, "messages.chats", &json!({}));
        add(
            evidence,
            &format!("{protocol}_visibility_chats"),
            chats.is_some_and(|r| items(&r["chats"]).is_empty()),
        );
        let thread = client.call(protocol, "messages.thread", &json!({"chat_id": chat_id}));
        add(
            evidence,
            &format!("{protocol}_visibility_thread"),
            thread.is_some_and(|r| items(&r["items"]).is_empty()),
        );
        let collection = client.call(
            protocol,
            "messages.collection",
            &json!({"chat_id": chat_id, "kind": "photos"}),
        );
        add(
            evidence,
            &format!("{protocol}_visibility_collection"),
            collection.is_some_and(|r| items(&r["items"]).is_empty()),
        );
    }
    Ok(())
}
fn replica(world: &World, evidence: &mut Evidence) -> Result<()> {
    add(
        evidence,
        "replica_added_after_data_for_snapshot",
        replicas::start_with(world, 1, SCOPE, 0).is_ok(),
    );
    let Some(endpoint) = replicas::endpoints().into_iter().next() else {
        return Ok(());
    };
    let gateway = clients(world, SCOPE)?.remove(0).1;
    let mirror = Fast::at(&endpoint.address, &endpoint.token);
    let list = json!({"limit": 200});
    let mut want = gateway
        .call("rest", "messages.chats", &list)
        .unwrap_or(Value::Null);
    let mut latest = Value::Null;
    let matched = await_until(20, || {
        latest = mirror
            .call("rest", "messages.chats", &list)
            .unwrap_or(Value::Null);
        latest["chats"] == want["chats"]
    });
    evidence.check(
        "snapshot_rebuild_matches_gateway",
        matched,
        &format!(
            "gateway {} replica {} sequences {} {}",
            items(&want["chats"]).len(),
            items(&latest["chats"]).len(),
            want["cursor"]["sequence"],
            latest["cursor"]["sequence"]
        ),
    );
    let _ = put(
        world,
        &[message(
            "rm-live-replica",
            "2026-06-01T00:00:00Z",
            &json!({}),
        )],
    );
    want = gateway
        .call("rest", "messages.chats", &list)
        .unwrap_or(Value::Null);
    let matched = await_until(20, || {
        latest = mirror
            .call("mcp", "messages.chats", &list)
            .unwrap_or(Value::Null);
        latest["chats"] == want["chats"]
    });
    add(evidence, "replica_live_apply_matches_gateway", matched);
    Ok(())
}
fn migration(world: &mut World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let gateway = clients(world, SCOPE)?.remove(0).1;
    let Some(before) = chat_row(&gateway, "rest", chat_id) else {
        add(evidence, "migration_baseline", false);
        return Ok(());
    };
    world.stop_gateway();
    let dropped = Connection::open(world.gateway_home.join("state.db"))
        .and_then(|database| {
            database.busy_timeout(Duration::from_secs(5))?;
            database.execute_batch("DROP TRIGGER rm_msg_insert; DROP TRIGGER rm_msg_delete; DROP TRIGGER rm_activity_insert; DROP TRIGGER rm_activity_delete; DROP TABLE msg_by_chat; DROP TABLE reaction_by_message; DROP TABLE attachment_by_message; DROP TABLE activity_by_chat; DROP TABLE chat_summary; DELETE FROM meta WHERE key='readmodels_version'")
        })
        .is_ok();
    add(evidence, "existing_store_without_read_models", dropped);
    add(
        evidence,
        "existing_store_migration_restart",
        world.restart_gateway().is_ok(),
    );
    let after = clients(world, SCOPE)?.remove(0).1;
    let after = chat_row(&after, "rest", chat_id);
    add(
        evidence,
        "one_time_migration_rebuild_matches",
        after.is_some_and(|row| row == before),
    );
    Ok(())
}
fn latency(world: &World, evidence: &mut Evidence, chats: bool, chat_id: &str) -> Result<()> {
    for (protocol, client) in clients(world, SCOPE)? {
        let mut durations: Vec<f64> = vec![];
        let mut valid = true;
        for _ in 0..100 {
            if chats {
                std::thread::sleep(Duration::from_millis(10));
            }
            let start = Instant::now();
            let reply = if chats {
                client.call(protocol, "messages.chats", &json!({"limit": 50}))
            } else {
                client.call(
                    protocol,
                    "messages.thread",
                    &json!({"chat_id": chat_id, "limit": 100}),
                )
            };
            durations.push(start.elapsed().as_secs_f64() * 1000.0);
            let field = if chats { "chats" } else { "items" };
            let want = if chats { 50 } else { 100 };
            valid &= reply.is_some_and(|r| items(&r[field]).len() == want);
        }
        durations.sort_by(f64::total_cmp);
        let p95 = durations[(durations.len() * 95).div_ceil(100) - 1];
        let name = format!("{protocol}_{}", if chats { "chats" } else { "thread" });
        if chats {
            add(evidence, &format!("{name}_requests_succeed"), valid);
        }
        let budget = if chats { 15.0 } else { 10.0 };
        evidence.check(
            &format!("{name}_p95_budget"),
            valid && p95 <= budget,
            &format!(
                "p50 {:.2} p95 {p95:.2} max {:.2} wire {:.1} total {:.1} rate limited {}",
                durations[50],
                durations[99],
                client.wire_milliseconds(),
                durations.iter().sum::<f64>(),
                client.rate_limited()
            ),
        );
    }
    Ok(())
}
fn ordered_chats(client: &Fast, protocol: &str, chat_id: &str, last_id: &str) -> (bool, bool) {
    let first = client.call(protocol, "messages.chats", &json!({"limit": 200}));
    let summaries = first.as_ref().is_some_and(|chats| {
        let head = &chats["chats"][0];
        items(&chats["chats"]).len() == 200
            && head["chat"]["id"] == json!(chat_id)
            && head["unread_count"] == 0
            && head["last_message"]["id"] == json!(last_id)
            && head["muted"].is_null()
            && head["pinned_index"].is_null()
            && !head["has_unseen_mention"].as_bool().unwrap_or(false)
            && !items(&chats["coverage_notes"]).is_empty()
    });
    let mut all: Vec<Value> = vec![];
    let mut seen = BTreeSet::new();
    let mut valid = first.is_some();
    let mut page = first.unwrap_or(Value::Null);
    loop {
        for row in items(&page["chats"]) {
            valid &= seen.insert(text(&row["chat"]["id"]).to_owned());
            all.push(row.clone());
        }
        let next = text(&page["next"]).to_owned();
        if next.is_empty() {
            break;
        }
        let more = client.call(
            protocol,
            "messages.chats",
            &json!({"limit": 200, "after": next}),
        );
        valid &= more.is_some();
        let Some(more) = more else { break };
        page = more;
    }
    for pair in all.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let (left, right) = (text(&a["last_activity_at"]), text(&b["last_activity_at"]));
        valid &= left > right || (left == right && text(&a["chat"]["id"]) < text(&b["chat"]["id"]));
    }
    (summaries, valid && all.len() == 500)
}
pub fn readmodels(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let chats = evidence.scenario == "WEB-RM-CHATS";
    let seeded = world.control_for(
        "/readmodels",
        &json!({"action": "seed", "count": 1000}),
        SEED_SECONDS,
    );
    add(evidence, "synthetic_sim_seed_committed", seeded.is_ok());
    if seeded.is_err() {
        return Ok(());
    }
    let Ok(truth) = world.control("/inspect", &Value::Null) else {
        add(evidence, "independent_truth", false);
        return Ok(());
    };
    let map = aliases(world);
    if map.is_empty() {
        add(evidence, "bridge_aliases", false);
        return Ok(());
    }
    let Ok(truth) = truth_records(&truth, &map, &Visibility::from_scope("messages:read")) else {
        add(evidence, "independent_truth_resolved", false);
        return Ok(());
    };
    let chat_id = truth
        .iter()
        .find(|r| {
            r["collection"] == "messages.chat" && r["data"]["display_name"] == "Bridge Test 000"
        })
        .map(|r| text(&r["id"]).to_owned())
        .unwrap_or_default();
    let expected = sorted_expected(&truth, &chat_id);
    add(
        evidence,
        "truth_has_500_chats_and_501_thread_messages",
        !chat_id.is_empty() && expected.len() == 501,
    );
    if expected.len() != 501 {
        return Ok(());
    }
    let last_id = text(&expected[expected.len() - 1]["id"]).to_owned();
    for (protocol, client) in clients(world, SCOPE)? {
        let (summaries, stable) = ordered_chats(&client, protocol, &chat_id, &last_id);
        add(evidence, &format!("{protocol}_chat_summaries"), summaries);
        add(
            evidence,
            &format!("{protocol}_all_chats_stably_ordered"),
            stable,
        );
        let latest = client.call(
            protocol,
            "messages.thread",
            &json!({"chat_id": chat_id, "limit": 100}),
        );
        add(
            evidence,
            &format!("{protocol}_latest_thread_matches_truth"),
            latest.is_some_and(|t| {
                items(&t["items"]).len() == 100
                    && t["items"][99]["id"] == json!(last_id)
                    && !text(&t["before"]).is_empty()
                    && text(&t["after"]).is_empty()
            }),
        );
    }
    visibility(world, evidence, &chat_id)?;
    if chats {
        rm_chats::names(world, evidence, &chat_id)?;
    } else {
        rm_thread::paging(world, evidence, &chat_id, &expected)?;
        rm_thread::placed_stickers(world, evidence)?;
        rm_thread::recoverable(world, evidence)?;
    }
    live(world, evidence, &chat_id)?;
    boundaries(world, evidence, &chat_id)?;
    if chats {
        rm_chats::previews(world, evidence, &chat_id)?;
        rm_chats::rich_filters(world, evidence, &chat_id)?;
        rm_chats::pin_preview(world, evidence)?;
        replica(world, evidence)?;
        migration(world, evidence, &chat_id)?;
    } else {
        let seeded = world.control_for(
            "/readmodels",
            &json!({"action": "seed", "count": 170_000}),
            SEED_SECONDS,
        );
        add(
            evidence,
            "170000_message_sim_seed_committed",
            seeded.is_ok(),
        );
        if seeded.is_err() {
            return Ok(());
        }
    }
    latency(world, evidence, chats, &chat_id)?;
    if chats {
        rm_chats::exact_participants(world, evidence)?;
    }
    Ok(())
}
