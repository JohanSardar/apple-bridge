use crate::{
    readmodels::{message, observation, put, reference},
    rm_core::{SCOPE, add, alias, aliases, chat_row, clients, items, put_ok, text},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::{collections::BTreeSet, thread};

pub fn paging(
    world: &World,
    evidence: &mut Evidence,
    chat_id: &str,
    expected: &[Value],
) -> Result<()> {
    let first = text(&expected[0]["id"]).to_owned();
    for (protocol, client) in clients(world, SCOPE)? {
        for direction in ["forward", "backward"] {
            let forward = direction == "forward";
            let mut seen = BTreeSet::new();
            let mut valid = true;
            let mut query = if forward {
                json!({"chat_id": chat_id, "anchor": first, "limit": 1})
            } else {
                json!({"chat_id": chat_id, "limit": 37})
            };
            let written = thread::scope(|scope| {
                let mut writer = None;
                for page in 0..40 {
                    let Some(result) = client.call(protocol, "messages.thread", &query) else {
                        valid = false;
                        break;
                    };
                    if page == 0 {
                        let mut records = vec![];
                        for index in 0..8_i64 {
                            let key = format!("concurrent-{protocol}-{direction}-{index}");
                            let sent = format!("2026-03-01T00:00:{index:02}Z");
                            records.push(message(&key, &sent, &json!({"is_from_me": true})));
                        }
                        writer = Some(scope.spawn(move || put(world, &records)));
                    }
                    for item in items(&result["items"]) {
                        valid &= seen.insert(text(&item["id"]).to_owned());
                    }
                    let cursor = text(&result[if forward { "after" } else { "before" }]).to_owned();
                    if cursor.is_empty() {
                        break;
                    }
                    query = json!({"chat_id": chat_id, "limit": 37});
                    query[if forward { "after" } else { "before" }] = json!(cursor);
                }
                writer.is_some_and(|handle| handle.join().is_ok_and(|result| result.is_ok()))
            });
            valid &= written;
            for record in expected {
                valid &= seen.contains(text(&record["id"]));
            }
            add(
                evidence,
                &format!("{protocol}_{direction}_concurrent_paging_no_gaps_or_duplicates"),
                valid,
            );
        }
    }
    Ok(())
}
pub fn placed_stickers(world: &World, evidence: &mut Evidence) -> Result<()> {
    let records = [
        observation(
            "messages.chat",
            "rm-sticker-chat",
            &json!({"display_name": "Bridge Test stickers", "participants": [], "filter": "known"}),
        ),
        observation(
            "messages.message",
            "rm-sticker-parent",
            &json!({"kind": "message", "text": "[ab-test] synthetic sticker target", "sent_at": "2026-10-01T12:00:00Z", "chat_id": reference("messages.chat", "rm-sticker-chat")}),
        ),
        observation(
            "messages.message",
            "rm-sticker-control",
            &json!({"kind": "system"}),
        ),
        observation(
            "messages.attachment",
            "rm-placed-sticker",
            &json!({"message_id": reference("messages.message", "rm-sticker-control"), "is_sticker": true, "mime_type": "image/png", "hidden": false, "sticker": {"target_message_id": reference("messages.message", "rm-sticker-parent"), "part_index": 0, "x": 0.25, "y": 0.5, "scale": 1.25, "rotation": 0.125}}),
        ),
    ];
    add(evidence, "placed_sticker_capture", put_ok(world, &records));
    let map = aliases(world);
    let chat = alias(&map, "messages.chat", "rm-sticker-chat");
    let target = alias(&map, "messages.message", "rm-sticker-parent");
    for (protocol, client) in clients(world, SCOPE)? {
        let on_target = client
            .call(protocol, "messages.thread", &json!({"chat_id": chat}))
            .is_some_and(|t| {
                items(&t["items"]).len() == 1
                    && t["items"][0]["id"] == json!(target)
                    && items(&t["items"][0]["attachments"]).len() == 1
            });
        add(
            evidence,
            &format!("{protocol}_sticker_on_target_bubble"),
            on_target,
        );
        let preview = chat_row(&client, protocol, &chat).is_some_and(|s| {
            let kinds = items(&s["last_message"]["preview"]["attachment_kinds"]);
            s["last_message"]["preview"]["kind"] == "attachments"
                && kinds.len() == 1
                && kinds[0] == "stickers"
        });
        add(
            evidence,
            &format!("{protocol}_sticker_target_preview"),
            preview,
        );
    }
    Ok(())
}
pub fn recoverable(world: &World, evidence: &mut Evidence) -> Result<()> {
    let deleted = "2026-10-01T12:00:00Z";
    let records = [
        observation(
            "messages.chat",
            "rich-junk-only",
            &json!({"display_name": "Bridge Test Junk", "participants": [], "filter": "unknown", "is_spam": true}),
        ),
        observation(
            "messages.chat",
            "rich-mixed-chat",
            &json!({"display_name": "Bridge Test mixed recovery", "participants": [], "filter": "known"}),
        ),
        observation(
            "messages.message",
            "rich-mixed-active",
            &json!({"chat_id": reference("messages.chat", "rich-mixed-chat"), "kind": "message", "text": "[ab-test] synthetic active", "sent_at": deleted}),
        ),
        observation(
            "messages.message",
            "rich-mixed-deleted",
            &json!({"chat_id": reference("messages.chat", "rich-mixed-chat"), "kind": "message", "text": "[ab-test] synthetic deleted", "sent_at": deleted, "deleted_at": deleted, "recoverable_until": null}),
        ),
        observation(
            "messages.attachment",
            "rich-mixed-image",
            &json!({"message_id": reference("messages.message", "rich-mixed-deleted"), "mime_type": "image/png"}),
        ),
    ];
    add(evidence, "mixed_deleted_capture", put_ok(world, &records));
    let map = aliases(world);
    let mixed = alias(&map, "messages.chat", "rich-mixed-chat");
    let junk = alias(&map, "messages.chat", "rich-junk-only");
    for (protocol, client) in clients(world, SCOPE)? {
        let listed = |filter: &str, id: &str| {
            client
                .call(protocol, "messages.chats", &json!({"filter": filter}))
                .is_some_and(|r| {
                    items(&r["chats"])
                        .iter()
                        .any(|c| c["chat"]["id"] == json!(id))
                })
        };
        add(
            evidence,
            &format!("{protocol}_deleted_filter_contains_mixed_chat"),
            listed("deleted", &mixed),
        );
        let unread = chat_row(&client, protocol, &mixed).is_some_and(|s| s["unread_count"] == 1);
        add(
            evidence,
            &format!("{protocol}_mixed_chat_retains_active_unread"),
            unread,
        );
        let excluded = client
            .call(
                protocol,
                "messages.collection",
                &json!({"chat_id": mixed, "kind": "photos"}),
            )
            .is_some_and(|r| items(&r["items"]).is_empty());
        add(
            evidence,
            &format!("{protocol}_recoverable_media_excluded"),
            excluded,
        );
        add(
            evidence,
            &format!("{protocol}_native_junk_without_messages"),
            listed("spam", &junk),
        );
    }
    Ok(())
}
