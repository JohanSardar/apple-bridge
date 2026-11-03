#![allow(clippy::too_many_lines)]
use crate::{
    fastclient::Fast,
    http::Client,
    readmodels::{PROTOCOLS, message, observation, offset, reference},
    rm_core::{SCOPE, add, alias, aliases, chat_row, clients, items, put_ok, text},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::time::Duration;

fn chat_with(display: &str, participants: &[Value]) -> Value {
    observation(
        "messages.chat",
        "rm-chat-000",
        &json!({"display_name": display, "participants": participants, "style": "group", "service": "iMessage"}),
    )
}
fn handles(world: &World, evidence: &mut Evidence) -> Result<()> {
    let digest = "a".repeat(64);
    add(
        evidence,
        "handle_avatar_fixture",
        world
            .control(
                "/external-change",
                &json!({"key": "rm-contact-0", "collection": "contacts.contact", "data": {"given_name": "Alpha", "emails": [{"value": "synthetic-0@example.invalid"}], "thumbnail_image": {"digest": digest, "state": "present"}}}),
            )
            .is_ok(),
    );
    add(
        evidence,
        "handle_avatar_capture",
        world.control("/flush", &json!({})).is_ok(),
    );
    let map = aliases(world);
    let id = alias(&map, "messages.handle", "rm-handle-0");
    let restricted = Client::new(world, "messages:read")?;
    for (protocol, client) in clients(world, SCOPE)? {
        let batched = client
            .call(
                protocol,
                "messages.handles",
                &json!({"ids": [id, id, "unknown"]}),
            )
            .is_some_and(|out| {
                let h = &out["handles"][0];
                items(&out["handles"]).len() == 1
                    && h["handle_id"] == json!(id)
                    && h["display_name"] == "Alpha"
                    && h["given_name"] == "Alpha"
                    && text(&h["family_name"]).is_empty()
                    && h["initials"] == "A"
                    && !h["contact_id"].is_null()
                    && h["avatar"]["digest"] == json!(digest)
                    && h["avatar"]["variant"] == "original"
            });
        add(
            evidence,
            &format!("{protocol}_batched_handle_name_avatar_dedup"),
            batched,
        );
        let hidden = Fast::of(&restricted)
            .call(protocol, "messages.handles", &json!({"ids": [id]}))
            .is_some_and(|out| {
                let h = &out["handles"][0];
                items(&out["handles"]).len() == 1
                    && h["display_name"] == "synthetic-0@example.invalid"
                    && h["contact_id"].is_null()
                    && h["avatar"].is_null()
                    && text(&h["given_name"]).is_empty()
                    && text(&h["family_name"]).is_empty()
            });
        add(
            evidence,
            &format!("{protocol}_handle_contact_visibility"),
            hidden,
        );
        let budget = client.call(protocol, "messages.handles", &json!({"ids": vec![""; 501]}));
        add(
            evidence,
            &format!("{protocol}_handles_input_budget"),
            budget.is_none(),
        );
    }
    Ok(())
}
fn known_sender(world: &World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let restricted = Client::new(world, "messages:read")?;
    let full = Client::new(world, SCOPE)?;
    for known in [true, false] {
        let mut records = vec![];
        let mut participants = vec![];
        for index in 0..4 {
            let key = format!("rm-known-handle-{index}");
            let address = if index == 3 && known {
                "synthetic-0@example.invalid".to_owned()
            } else {
                format!("unlinked-{index}@example.com")
            };
            records.push(observation(
                "messages.handle",
                &key,
                &json!({"address": address, "service": "iMessage"}),
            ));
            participants.push(reference("messages.handle", &key));
        }
        records.push(observation(
            "messages.chat",
            "rm-chat-000",
            &json!({"display_name": "[ab-test] Known sender", "participants": participants, "filter": "unknown", "style": "group", "service": "iMessage"}),
        ));
        add(
            evidence,
            &format!("known_sender_fixture_{known}"),
            put_ok(world, &records),
        );
        for protocol in PROTOCOLS {
            for hidden in [false, true] {
                let client = Fast::of(if hidden { &restricted } else { &full });
                let mut valid = chat_row(&client, protocol, chat_id).is_some_and(|row| {
                    row["known_sender"] == json!(known)
                        && items(&row["avatar_handles"]).len() == 3
                        && items(&row["avatar_handles"])
                            .iter()
                            .all(|h| h["contact_id"].is_null())
                });
                for filter in ["known", "unknown"] {
                    let found = client
                        .call(
                            protocol,
                            "messages.chats",
                            &json!({"filter": filter, "limit": 200}),
                        )
                        .map(|r| {
                            items(&r["chats"])
                                .iter()
                                .any(|c| c["chat"]["id"] == json!(chat_id))
                        });
                    valid &= found == Some(known == (filter == "known"));
                }
                add(
                    evidence,
                    &format!(
                        "{protocol}_known_sender_{known}_contacts_hidden_{hidden}_matches_server_filters"
                    ),
                    valid,
                );
            }
        }
    }
    Ok(())
}
pub fn names(world: &World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let mut records = vec![];
    let mut participants = vec![];
    for (index, name) in ["Alpha", "Beta", "Gamma", "Delta"].iter().enumerate() {
        let address = format!("synthetic-{index}@example.invalid");
        let stored = world.control(
            "/external-change",
            &json!({"key": format!("rm-contact-{index}"), "collection": "contacts.contact", "data": {"given_name": name, "emails": [{"value": address}]}}),
        );
        add(
            evidence,
            &format!("contact_name_fixture_{index}"),
            stored.is_ok(),
        );
        let key = format!("rm-handle-{index}");
        records.push(observation(
            "messages.handle",
            &key,
            &json!({"address": address, "service": "iMessage"}),
        ));
        participants.push(reference("messages.handle", &key));
    }
    add(
        evidence,
        "contact_fixtures_committed",
        world.control("/flush", &json!({})).is_ok(),
    );
    records.push(chat_with("", &participants));
    add(
        evidence,
        "handle_contact_title_capture",
        put_ok(world, &records),
    );
    handles(world, evidence)?;
    known_sender(world, evidence, chat_id)?;
    add(
        evidence,
        "known_sender_original_title_fixture_restored",
        put_ok(world, &[chat_with("", &participants)]),
    );
    let messages_only = Client::new(world, "messages:read")?;
    add(evidence, "messages_only_token", true);
    for (protocol, client) in clients(world, SCOPE)? {
        let row = chat_row(&client, protocol, chat_id);
        let valid = row.is_some_and(|row| {
            row["title"] == "Alpha, Beta & 2 others"
                && items(&row["avatar_handles"]).len() == 3
                && items(&row["avatar_handles"])
                    .iter()
                    .all(|h| !h["contact_id"].is_null())
        });
        add(
            evidence,
            &format!("{protocol}_contact_names_and_three_avatar_handles"),
            valid,
        );
        let row = chat_row(&Fast::of(&messages_only), protocol, chat_id);
        let valid = row.is_some_and(|row| {
            text(&row["title"]).contains("synthetic-0@example.invalid")
                && items(&row["avatar_handles"])
                    .iter()
                    .all(|h| h["contact_id"].is_null())
        });
        add(
            evidence,
            &format!("{protocol}_contact_visibility_falls_back_to_addresses"),
            valid,
        );
    }
    add(
        evidence,
        "display_name_override_capture",
        put_ok(world, &[chat_with("Bridge Test 000", &participants)]),
    );
    let gateway = clients(world, SCOPE)?.remove(0).1;
    let row = chat_row(&gateway, "rest", chat_id);
    add(
        evidence,
        "display_name_overrides_participant_names",
        row.is_some_and(|row| row["title"] == "Bridge Test 000"),
    );
    add(
        evidence,
        "restore_chat_fixture",
        put_ok(world, &[chat_with("Bridge Test 000", &[])]),
    );
    Ok(())
}
pub fn previews(world: &World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let base = "2026-05-02T00:00:00Z";
    for (index, kind) in ["text", "attachments", "group_event", "unsent"]
        .iter()
        .enumerate()
    {
        let mut extra = json!({"is_from_me": true, "sender_handle_id": "synthetic-sender"});
        if *kind == "group_event" {
            extra["kind"] = json!("group_event");
            extra["group_event"] = json!({"kind": "rename", "name": "Bridge Test"});
        }
        if *kind == "unsent" {
            extra["retracted_at"] = json!(base);
            extra["retracted_parts"] = json!([0]);
        }
        let seconds = i64::try_from(index).unwrap_or_default();
        let mut records = vec![message("rm-preview", &offset(base, seconds)?, &extra)];
        if *kind == "attachments" {
            for (position, mime) in ["image/png", "video/mp4"].iter().enumerate() {
                records.push(observation(
                    "messages.attachment",
                    &format!("rm-preview-attachment-{position}"),
                    &json!({"message_id": reference("messages.message", "rm-preview"), "mime_type": mime}),
                ));
            }
        }
        add(
            evidence,
            &format!("preview_{kind}_capture"),
            put_ok(world, &records),
        );
        for (protocol, client) in clients(world, SCOPE)? {
            let valid = chat_row(&client, protocol, chat_id).is_some_and(|row| {
                let last = &row["last_message"];
                let mut ok = !last.is_null() && last["preview"]["kind"] == json!(kind);
                if (*kind == "group_event" || *kind == "unsent") && ok {
                    ok = last["sender_handle_id"] == "synthetic-sender";
                    if *kind == "group_event" {
                        ok = ok && last["preview"]["group_event"]["kind"] == "rename";
                    }
                }
                if *kind == "attachments" && ok {
                    ok = last["preview"]["count"] == 2
                        && items(&last["preview"]["attachment_kinds"]).len() == 2;
                }
                ok
            });
            add(evidence, &format!("{protocol}_preview_{kind}"), valid);
        }
    }
    Ok(())
}
pub fn rich_filters(world: &World, evidence: &mut Evidence, chat_id: &str) -> Result<()> {
    let stamp = "2026-10-01T12:00:00Z";
    let mut records = vec![
        observation(
            "messages.chat",
            "rm-chat-000",
            &json!({"display_name": "Bridge Test known", "filter": "known", "muted_until": "2099-01-01T00:00:00Z", "pinned_index": 1}),
        ),
        observation(
            "messages.chat",
            "rm-chat-498",
            &json!({"display_name": "Bridge Test unknown", "filter": "unknown"}),
        ),
        observation(
            "messages.chat",
            "rm-chat-497",
            &json!({"display_name": "Bridge Test spam", "filter": "unknown"}),
        ),
        observation(
            "messages.chat",
            "rm-chat-496",
            &json!({"display_name": "Bridge Test deleted", "filter": "known", "deleted_at": stamp}),
        ),
    ];
    let fixtures = [
        ("rm-rich-unread", "rm-chat-498", json!({})),
        ("rm-rich-spam", "rm-chat-497", json!({"is_spam": true})),
        (
            "rm-rich-deleted",
            "rm-chat-496",
            json!({"deleted_at": stamp}),
        ),
        (
            "rm-rich-link",
            "rm-chat-000",
            json!({"has_unseen_mention": true, "balloon": {"kind": "link", "link": {"url": "https://example.invalid/preview", "title": "Synthetic preview"}}}),
        ),
        (
            "rm-rich-location",
            "rm-chat-000",
            json!({"balloon": {"kind": "find_my"}}),
        ),
    ];
    for (key, chat, extra) in fixtures {
        let mut data = json!({"kind": "message", "chat_id": reference("messages.chat", chat), "sent_at": stamp, "text": "[ab-test] synthetic rich"});
        for (name, value) in extra.as_object().into_iter().flatten() {
            data[name] = value.clone();
        }
        records.push(observation("messages.message", key, &data));
    }
    add(evidence, "rich_filter_fixtures", put_ok(world, &records));
    let map = aliases(world);
    let id = |key: &str| alias(&map, "messages.chat", key);
    let contains = |rows: &Value, id: &str| {
        items(&rows["chats"])
            .iter()
            .any(|c| c["chat"]["id"] == json!(id))
    };
    for (protocol, client) in clients(world, SCOPE)? {
        for filter in ["all", "known", "unknown", "unread", "deleted", "spam"] {
            let out = client.call(
                protocol,
                "messages.chats",
                &json!({"filter": filter, "limit": 200}),
            );
            let valid = out.is_some_and(|out| {
                let rows = items(&out["chats"]);
                match filter {
                    "known" => contains(&out, chat_id) && !contains(&out, &id("rm-chat-498")),
                    "unknown" => contains(&out, &id("rm-chat-498")) && !contains(&out, chat_id),
                    "all" | "unread" => {
                        contains(&out, &id("rm-chat-498")) && !contains(&out, &id("rm-chat-496"))
                    }
                    "deleted" => {
                        rows.len() == 1 && rows[0]["chat"]["id"] == json!(id("rm-chat-496"))
                    }
                    _ => rows.len() == 1 && rows[0]["chat"]["id"] == json!(id("rm-chat-497")),
                }
            });
            add(evidence, &format!("{protocol}_rich_filter_{filter}"), valid);
        }
        let summary = chat_row(&client, protocol, chat_id).is_some_and(|s| {
            s["has_unseen_mention"] == json!(true)
                && s["muted"] == json!(true)
                && s["pinned_index"] == 1
        });
        add(
            evidence,
            &format!("{protocol}_captured_unseen_mention"),
            summary,
        );
        for kind in ["links", "locations"] {
            let valid = client
                .call(
                    protocol,
                    "messages.collection",
                    &json!({"chat_id": chat_id, "kind": kind}),
                )
                .is_some_and(|out| {
                    items(&out["items"]).len() == 1
                        && out["items"][0]["collection"] == "messages.message"
                });
            add(
                evidence,
                &format!("{protocol}_balloon_collection_{kind}"),
                valid,
            );
        }
    }
    Ok(())
}
pub fn pin_preview(world: &World, evidence: &mut Evidence) -> Result<()> {
    let base = jiff::Timestamp::now().to_string();
    let incoming = |body: &str, read: &Value| {
        observation(
            "messages.message",
            "rm-pin-incoming",
            &json!({"chat_id": reference("messages.chat", "rm-pin-chat"), "kind": "message", "sent_at": base, "is_from_me": false, "read_at": read, "text": body, "parts": []}),
        )
    };
    let mut records = vec![
        observation(
            "messages.chat",
            "rm-pin-chat",
            &json!({"display_name": "[ab-test] Pin preview", "participants": [], "style": "direct", "pinned_index": 0}),
        ),
        incoming("[ab-test] Incoming pin message", &Value::Null),
    ];
    for index in 0..60_i64 {
        records.push(observation(
            "messages.message",
            &format!("rm-pin-outgoing-{index:02}"),
            &json!({"chat_id": reference("messages.chat", "rm-pin-chat"), "kind": "message", "sent_at": offset(&base, index + 1)?, "is_from_me": true, "text": "[ab-test] Later outgoing", "parts": []}),
        ));
    }
    records.push(observation(
        "messages.reaction",
        "rm-pin-removal",
        &json!({"message_id": reference("messages.message", "rm-pin-incoming"), "kind": "love", "removed": true, "sent_at": offset(&base, 61)?}),
    ));
    add(
        evidence,
        "pin_preview_60_outgoing_fixture",
        put_ok(world, &records),
    );
    let map = aliases(world);
    let chat_id = alias(&map, "messages.chat", "rm-pin-chat");
    let message_id = alias(&map, "messages.message", "rm-pin-incoming");
    for (protocol, client) in clients(world, SCOPE)? {
        let valid = chat_row(&client, protocol, &chat_id).is_some_and(|row| {
            let pin = &row["pin_preview"];
            row["unread_count"] == 1
                && pin["message_id"] == json!(message_id)
                && pin["text_preview"] == "[ab-test] Incoming pin message"
                && pin["at"] == json!(base)
        });
        add(
            evidence,
            &format!("{protocol}_pin_preview_latest_unread_incoming_not_outgoing_or_removal"),
            valid,
        );
    }
    add(
        evidence,
        "pin_preview_edit_capture",
        put_ok(
            world,
            &[incoming("[ab-test] Updated incoming", &Value::Null)],
        ),
    );
    for (protocol, client) in clients(world, SCOPE)? {
        let valid = chat_row(&client, protocol, &chat_id)
            .is_some_and(|row| row["pin_preview"]["text_preview"] == "[ab-test] Updated incoming");
        add(
            evidence,
            &format!("{protocol}_pin_preview_updates_with_read_model"),
            valid,
        );
    }
    let images = [
        incoming("", &Value::Null),
        observation(
            "messages.attachment",
            "rm-pin-photo-a",
            &json!({"message_id": reference("messages.message", "rm-pin-incoming"), "mime_type": "image/png"}),
        ),
        observation(
            "messages.attachment",
            "rm-pin-photo-b",
            &json!({"message_id": reference("messages.message", "rm-pin-incoming"), "mime_type": "image/jpeg"}),
        ),
    ];
    add(
        evidence,
        "pin_preview_images_capture",
        put_ok(world, &images),
    );
    for (protocol, client) in clients(world, SCOPE)? {
        let valid = chat_row(&client, protocol, &chat_id)
            .is_some_and(|row| row["pin_preview"]["text_preview"] == "Attachment: 2 Images");
        add(
            evidence,
            &format!("{protocol}_pin_preview_attachment_wording"),
            valid,
        );
    }
    add(
        evidence,
        "pin_preview_read_capture",
        put_ok(
            world,
            &[incoming(
                "[ab-test] Updated incoming",
                &json!(offset(&base, 120)?),
            )],
        ),
    );
    for (protocol, client) in clients(world, SCOPE)? {
        let valid = chat_row(&client, protocol, &chat_id)
            .is_some_and(|row| row["unread_count"] == 0 && row["pin_preview"].is_null());
        add(
            evidence,
            &format!("{protocol}_read_pin_preview_is_null"),
            valid,
        );
    }
    Ok(())
}
fn participant_chat(key: &str, members: &[&str]) -> Value {
    let participants: Vec<Value> = members
        .iter()
        .map(|member| reference("messages.handle", &format!("rm-exact-handle-{member}")))
        .collect();
    let style = if members.len() > 1 { "group" } else { "direct" };
    observation(
        "messages.chat",
        &format!("rm-exact-chat-{key}"),
        &json!({"display_name": format!("Bridge Test exact {key}"), "participants": participants, "style": style, "service": "iMessage"}),
    )
}
fn exact_ids(client: &Fast, protocol: &str, handles: &[String]) -> Option<(Vec<String>, String)> {
    let result = client.call(
        protocol,
        "messages.chats",
        &json!({"participants": handles, "limit": 1}),
    )?;
    let ids = items(&result["chats"])
        .iter()
        .map(|row| text(&row["chat"]["id"]).to_owned())
        .collect();
    Some((ids, text(&result["next"]).to_owned()))
}
pub fn exact_participants(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    let addresses: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|key| format!("participant.{key}@example.com"))
        .collect();
    let mut records = vec![];
    for (index, key) in ["a", "b", "c"].iter().enumerate() {
        records.push(observation(
            "messages.handle",
            &format!("rm-exact-handle-{key}"),
            &json!({"address": addresses[index], "service": "iMessage"}),
        ));
    }
    records.push(participant_chat("a", &["a"]));
    records.push(participant_chat("ab", &["a", "b"]));
    records.push(participant_chat("abc", &["a", "b", "c"]));
    if !put_ok(world, &records) {
        add(evidence, "exact_participant_fixture", false);
        return Ok(());
    }
    let map = aliases(world);
    add(evidence, "exact_participant_aliases", !map.is_empty());
    let chat_alias = |key: &str| alias(&map, "messages.chat", &format!("rm-exact-chat-{key}"));
    let queries: Vec<(&str, Vec<String>, &str)> = vec![
        ("single", addresses[..1].to_vec(), "a"),
        ("group", addresses[..2].to_vec(), "ab"),
        (
            "reordered_set",
            vec![
                addresses[1].to_uppercase(),
                addresses[0].clone(),
                addresses[1].clone(),
            ],
            "ab",
        ),
        ("superset", addresses.clone(), "abc"),
        (
            "different_set",
            vec![addresses[0].clone(), addresses[2].clone()],
            "",
        ),
        (
            "unknown",
            vec!["participant.unknown@example.com".to_owned()],
            "",
        ),
    ];
    for (name, handles, key) in &queries {
        let mut prior: Option<Vec<String>> = None;
        for (protocol, client) in clients(world, SCOPE)? {
            let reply = exact_ids(&client, protocol, handles);
            let want: Vec<String> = if key.is_empty() {
                vec![]
            } else {
                vec![chat_alias(key)]
            };
            let ids = reply.as_ref().map(|(ids, _)| ids.clone());
            add(
                evidence,
                &format!("{protocol}_exact_participants_{name}"),
                reply
                    .as_ref()
                    .is_some_and(|(ids, next)| *ids == want && next.is_empty()),
            );
            if protocol == "mcp" {
                add(
                    evidence,
                    &format!("exact_participants_{name}_parity"),
                    ids.is_some() && ids.as_deref() == prior.as_deref(),
                );
            }
            prior = ids;
        }
    }
    let denied = Fast::of(&Client::new(world, "system:read")?);
    let hidden = denied.call(
        "rest",
        "messages.chats",
        &json!({"participants": &addresses[..1]}),
    );
    add(
        evidence,
        "exact_participants_visibility",
        hidden.is_some_and(|r| items(&r["chats"]).is_empty()),
    );
    let check = |world: &World,
                 evidence: &mut Evidence,
                 name: &str,
                 participants: &[String],
                 key: &str|
     -> Result<()> {
        for (protocol, client) in clients(world, SCOPE)? {
            let reply = exact_ids(&client, protocol, participants);
            let valid = reply.is_some_and(|(ids, _)| {
                if key.is_empty() {
                    ids.is_empty()
                } else {
                    ids == [chat_alias(key)]
                }
            });
            add(
                evidence,
                &format!("{protocol}_exact_participants_{name}"),
                valid,
            );
        }
        Ok(())
    };
    add(
        evidence,
        "exact_participant_membership_change",
        put_ok(world, &[participant_chat("ab", &["a", "c"])]),
    );
    check(
        world,
        evidence,
        "old_membership_absent",
        &addresses[..2],
        "",
    )?;
    check(
        world,
        evidence,
        "new_membership",
        &[addresses[0].clone(), addresses[2].clone()],
        "ab",
    )?;
    add(
        evidence,
        "exact_participant_address_change",
        put_ok(
            world,
            &[observation(
                "messages.handle",
                "rm-exact-handle-a",
                &json!({"address": "participant.changed@example.com", "service": "iMessage"}),
            )],
        ),
    );
    check(world, evidence, "old_address_absent", &addresses[..1], "")?;
    check(
        world,
        evidence,
        "changed_address",
        &["PARTICIPANT.CHANGED@example.com".to_owned()],
        "a",
    )?;
    let gateway = clients(world, SCOPE)?.remove(0).1;
    for invalid in [json!([]), json!([""]), json!(vec![""; 33])] {
        let rejected = gateway.call("rest", "messages.chats", &json!({"participants": invalid}));
        add(
            evidence,
            "invalid_participant_set_rejected",
            rejected.is_none(),
        );
    }
    world.stop_gateway();
    let dropped = Connection::open(world.gateway_home.join("state.db")).and_then(|database| {
        database.busy_timeout(Duration::from_secs(5))?;
        database.execute_batch("DROP TABLE chat_participant; DROP TABLE handle_address; DELETE FROM meta WHERE key='chat_participants_version'")
    });
    add(
        evidence,
        "participant_index_migration_fixture",
        dropped.is_ok(),
    );
    add(
        evidence,
        "participant_index_migration_restart",
        world.restart_gateway().is_ok(),
    );
    check(
        world,
        evidence,
        "rebuilt_address",
        &["participant.changed@example.com".to_owned()],
        "a",
    )?;
    check(
        world,
        evidence,
        "rebuilt_membership",
        &[
            "participant.changed@example.com".to_owned(),
            addresses[2].clone(),
        ],
        "ab",
    )?;
    Ok(())
}
