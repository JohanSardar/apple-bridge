#![allow(clippy::too_many_lines)]
use crate::{
    assertions::gateway_aliases,
    http::Client,
    readmodels::{PROTOCOLS, call, observation, offset, put, reference},
};
use lab_runner::Evidence;
use lab_world::{Result, World};
use serde_json::{Value, json};
use std::process::Command;

pub const SCENARIOS: [&str; 1] = ["MSG-READ-RICH-FIELDS"];
const SCOPE: &str = "messages:read,contacts:read,system:read";
const KINDS: [&str; 10] = [
    "link",
    "app",
    "digital_touch",
    "handwriting",
    "poll",
    "check_in",
    "apple_cash",
    "find_my",
    "chatbot",
    "unknown",
];

fn script(args: &[&str], script: &str) -> (bool, String) {
    let output = Command::new(script).args(args).output();
    match output {
        Ok(o) => (
            o.status.success(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        ),
        Err(error) => (false, error.to_string()),
    }
}

fn chat_summary(client: &Client, protocol: &str, id: &str) -> Option<Value> {
    let reply = call(client, protocol, "messages.chats", &json!({"limit": 200}))?;
    reply["chats"]
        .as_array()?
        .iter()
        .find(|chat| chat["chat"]["id"] == id)
        .cloned()
}

fn handle_checks(
    world: &World,
    protocol: &str,
    handle: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let gateway = Client::new(world, SCOPE)?;
    let handles = call(
        &gateway,
        protocol,
        "messages.handles",
        &json!({"ids": [handle, handle, "missing"]}),
    );
    let rows = handles
        .as_ref()
        .and_then(|reply| reply["handles"].as_array())
        .cloned()
        .unwrap_or_default();
    evidence.check(
        &format!("{protocol}_handles_names_initials_avatar_dedup"),
        rows.len() == 1
            && rows[0]["display_name"] == "Synthetic Sender"
            && rows[0]["initials"] == "SS"
            && !rows[0]["avatar"].is_null(),
        "",
    );
    let token = world
        .gateway_cli(&["token", "create", "--scope", "messages:read"])?
        .trim()
        .to_owned();
    let restricted = Client::with_token(world, &token)?;
    let private = call(
        &restricted,
        protocol,
        "messages.handles",
        &json!({"ids": [handle]}),
    );
    let rows = private
        .as_ref()
        .and_then(|reply| reply["handles"].as_array())
        .cloned()
        .unwrap_or_default();
    evidence.check(
        &format!("{protocol}_handles_respect_contact_visibility"),
        rows.len() == 1
            && rows[0]["contact_id"].is_null()
            && rows[0]["avatar"].is_null()
            && rows[0]["display_name"] == "reader@example.invalid",
        "",
    );
    evidence.check(
        &format!("{protocol}_handles_reject_over_budget"),
        call(
            &gateway,
            protocol,
            "messages.handles",
            &json!({"ids": vec![""; 501]}),
        )
        .is_none(),
        "",
    );
    Ok(())
}

fn deleted_messages(world: &World, evidence: &mut Evidence) -> Result<()> {
    let deleted = "2026-10-01T12:00:00Z";
    let chat = reference("messages.chat", "rich-mixed-chat");
    evidence.check(
        "mixed_deleted_capture",
        put(
            world,
            &[
                observation(
                    "messages.chat",
                    "rich-mixed-chat",
                    &json!({"display_name": "Bridge Test mixed recovery", "participants": [], "filter": "known"}),
                ),
                observation(
                    "messages.message",
                    "rich-mixed-active",
                    &json!({"chat_id": chat, "kind": "message", "text": "[ab-test] synthetic active", "sent_at": deleted}),
                ),
                observation(
                    "messages.message",
                    "rich-mixed-deleted",
                    &json!({"chat_id": chat, "kind": "message", "text": "[ab-test] synthetic deleted", "sent_at": deleted, "deleted_at": deleted, "recoverable_until": null}),
                ),
                observation(
                    "messages.attachment",
                    "rich-mixed-image",
                    &json!({"message_id": reference("messages.message", "rich-mixed-deleted"), "mime_type": "image/png"}),
                ),
            ],
        )
        .is_ok(),
        "",
    );
    let gateway = Client::new(world, SCOPE)?;
    let aliases = gateway_aliases(&gateway).unwrap_or_default();
    let id = aliases
        .get("messages/messages.chat/rich-mixed-chat")
        .cloned()
        .unwrap_or_default();
    for protocol in PROTOCOLS {
        let listed = call(
            &gateway,
            protocol,
            "messages.chats",
            &json!({"filter": "deleted"}),
        );
        let found = listed.as_ref().is_some_and(|reply| {
            reply["chats"]
                .as_array()
                .is_some_and(|chats| chats.iter().any(|chat| chat["chat"]["id"] == id.as_str()))
        });
        evidence.check(
            &format!("{protocol}_deleted_filter_contains_mixed_chat"),
            found,
            "",
        );
        let summary = chat_summary(&gateway, protocol, &id);
        evidence.check(
            &format!("{protocol}_mixed_chat_retains_active_unread"),
            summary.is_some_and(|s| s["unread_count"] == 1),
            "",
        );
        let photos = call(
            &gateway,
            protocol,
            "messages.collection",
            &json!({"chat_id": id, "kind": "photos"}),
        );
        evidence.check(
            &format!("{protocol}_recoverable_media_excluded"),
            photos.is_some_and(|reply| reply["items"].as_array().is_some_and(Vec::is_empty)),
            "",
        );
    }
    Ok(())
}

pub fn run(world: &mut World, _: &Value, evidence: &mut Evidence) -> Result<()> {
    crate::native_core::rust_tests(
        evidence,
        "native_rich_capture_and_live_mutation",
        "ab-native",
        &["--lib", "rich"],
    );
    let (truth, detail) = script(&[], "packaging/dev/check-messages-truth.sh");
    evidence.check(
        "independent_truth_synthetic_field_counts",
        truth,
        &if truth {
            String::new()
        } else {
            format!("native Truth fixture check: {detail}")
        },
    );
    let sent = |seconds: i64| offset("2026-10-01T12:00:00Z", seconds);
    let mut records = vec![];
    for filter in ["known", "unknown", "deleted", "spam"] {
        let mut data = json!({"display_name": format!("Bridge Test {filter}"), "participants": [], "filter": filter, "read_receipts": "default", "muted_until": null, "pinned_index": null});
        if filter == "spam" {
            data["filter"] = json!("unknown");
        }
        if filter == "deleted" {
            data["filter"] = json!("known");
            data["deleted_at"] = json!(sent(0)?);
            data["recoverable_until"] = json!(sent(30 * 24 * 3600)?);
        }
        records.push(observation(
            "messages.chat",
            &format!("rich-{filter}"),
            &data,
        ));
    }
    for (index, kind) in KINDS.iter().enumerate() {
        let seconds = i64::try_from(index).unwrap_or(0);
        let mut data = json!({"title": format!("[ab-test] rich-{kind}"), "text": format!("[ab-test] rich-{kind}"), "kind": "message", "chat_id": reference("messages.chat", "rich-known"), "sent_at": sent(seconds)?, "balloon": {"bundle_id": format!("synthetic.{kind}"), "kind": kind}, "has_unseen_mention": true});
        if *kind == "link" {
            data["subject"] = json!("Synthetic subject");
            data["audio"] = json!({"is_audio_message": true, "played_at": sent(0)?, "expirable": true, "kept": null, "transcription": "Synthetic transcript"});
            data["schedule"] = json!({"state": "native_3", "scheduled_for": sent(0)?});
            data["delivery"] = json!({"quietly": true, "notified_anyway": true});
            data["via_satellite"] = json!(true);
            data["is_sos"] = json!(true);
            data["formatting"] = json!([{"utf16_location": 0, "utf16_length": 2, "styles": ["bold", "italic"], "effect": "ripple"}]);
            data["parts"] = json!([{"index": 0, "kind": "genmoji", "attachment_id": reference("messages.attachment", "rich-glyph"), "description": "Synthetic glyph"}]);
            data["balloon"] = json!({"bundle_id": "com.apple.messages.URLBalloonProvider", "kind": "link", "link": {"url": "https://example.invalid/preview", "title": "Synthetic link", "image_attachment_id": reference("messages.attachment", "rich-image")}});
        }
        let key = if *kind == "unknown" {
            "rich-balloon-unknown".to_owned()
        } else {
            format!("rich-{kind}")
        };
        records.push(observation("messages.message", &key, &data));
    }
    records.push(observation(
        "messages.message",
        "rich-spam-message",
        &json!({"title": "[ab-test] spam", "kind": "message", "is_spam": true, "chat_id": reference("messages.chat", "rich-spam"), "sent_at": sent(0)?}),
    ));
    records.push(observation(
        "messages.message",
        "rich-sticker-control",
        &json!({"kind": "system", "text": "[ab-test] synthetic sticker control", "sent_at": sent(0)?}),
    ));
    records.push(observation(
        "messages.attachment",
        "rich-glyph",
        &json!({"message_id": reference("messages.message", "rich-sticker-control"), "filename": "synthetic.png", "mime_type": "image/png", "is_sticker": true, "hidden": true, "genmoji": true, "sticker": {"target_message_id": reference("messages.message", "rich-link"), "part_index": 0, "x": 0.25, "y": 0.5, "scale": 1.25, "rotation": 0.125}}),
    ));
    records.push(observation(
        "messages.attachment",
        "rich-image",
        &json!({"message_id": reference("messages.message", "rich-link"), "filename": "synthetic-preview.png", "mime_type": "image/png", "hidden": true}),
    ));
    records.push(observation(
        "messages.handle",
        "rich-handle",
        &json!({"address": "reader@example.invalid", "service": "iMessage"}),
    ));
    evidence.check(
        "contact_capture",
        world
            .control(
                "/external-change",
                &json!({"key": "rich-contact", "collection": "contacts.contact", "data": {"given_name": "Synthetic", "family_name": "Sender", "emails": [{"value": "reader@example.invalid"}], "thumbnail_image": {"digest": "a".repeat(64), "state": "present"}}}),
            )
            .is_ok(),
        "",
    );
    evidence.check(
        "contact_committed",
        world.control_for("/flush", &json!({}), 20).is_ok(),
        "",
    );
    evidence.check("rich_capture", put(world, &records).is_ok(), "");
    let gateway = Client::new(world, SCOPE)?;
    let aliases = gateway_aliases(&gateway).unwrap_or_default();
    let alias = |collection: &str, key: &str| {
        aliases
            .get(&format!("messages/{collection}/{key}"))
            .cloned()
            .unwrap_or_default()
    };
    let chat = alias("messages.chat", "rich-known");
    for protocol in PROTOCOLS {
        for filter in ["known", "unknown", "unread", "deleted", "spam"] {
            let want = if filter == "unknown" || filter == "unread" {
                2
            } else {
                1
            };
            let reply = call(
                &gateway,
                protocol,
                "messages.chats",
                &json!({"filter": filter}),
            );
            evidence.check(
                &format!("{protocol}_filter_{filter}"),
                reply.is_some_and(|r| r["chats"].as_array().is_some_and(|c| c.len() == want)),
                "",
            );
        }
        evidence.check(
            &format!("{protocol}_unseen_mention"),
            chat_summary(&gateway, protocol, &chat)
                .is_some_and(|s| s["has_unseen_mention"] == true),
            "",
        );
        for kind in ["links", "locations"] {
            let reply = call(
                &gateway,
                protocol,
                "messages.collection",
                &json!({"chat_id": chat, "kind": kind}),
            );
            evidence.check(
                &format!("{protocol}_collection_{kind}"),
                reply.is_some_and(|r| r["items"].as_array().is_some_and(|i| i.len() == 1)),
                "",
            );
        }
        let thread = call(
            &gateway,
            protocol,
            "messages.thread",
            &json!({"chat_id": chat}),
        );
        let items = thread
            .as_ref()
            .and_then(|reply| reply["items"].as_array())
            .cloned()
            .unwrap_or_default();
        evidence.check(
            &format!("{protocol}_every_balloon_kind_passes_through"),
            thread.is_some() && items.len() == 10,
            "",
        );
        for item in &items {
            let data = &item["data"];
            if data["subject"] == "Synthetic subject" {
                evidence.check(
                    &format!("{protocol}_rich_fields_and_attachment_refs"),
                    !data["audio"].is_null()
                        && !data["schedule"].is_null()
                        && !data["delivery"].is_null()
                        && !data["formatting"].is_null()
                        && !data["parts"].is_null()
                        && data["via_satellite"] == true
                        && data["is_sos"] == true
                        && item["attachments"].as_array().is_some_and(|a| a.len() == 2),
                    "",
                );
            }
        }
        handle_checks(
            world,
            protocol,
            &alias("messages.handle", "rich-handle"),
            evidence,
        )?;
    }
    let target = alias("messages.message", "rich-link");
    let thread = |gateway: &Client| {
        call(
            gateway,
            "rest",
            "messages.thread",
            &json!({"chat_id": chat, "anchor": target, "limit": 1}),
        )
    };
    let before = thread(&gateway);
    evidence.check(
        "link_reaction_capture",
        put(
            world,
            &[observation(
                "messages.reaction",
                "rich-reaction",
                &json!({"message_id": reference("messages.message", "rich-link"), "sender_handle_id": reference("messages.handle", "rich-handle"), "kind": "emoji", "emoji": "🧪", "part_index": 0, "sent_at": sent(3600)?}),
            )],
        )
        .is_ok(),
        "",
    );
    let after = thread(&gateway);
    let first = |reply: &Option<Value>| {
        reply
            .as_ref()
            .and_then(|r| r["items"].as_array())
            .filter(|items| items.len() == 1)
            .map(|items| items[0].clone())
    };
    let (before_item, after_item) = (first(&before), first(&after));
    evidence.check(
        "reaction_updates_link_thread_live",
        after_item.as_ref().is_some_and(|a| {
            a["reactions"].as_array().is_some_and(|r| r.len() == 1)
                && before_item
                    .as_ref()
                    .is_some_and(|b| a["data"]["balloon"] == b["data"]["balloon"])
        }),
        "",
    );
    if let Some(item) = after_item {
        let mut fields = item["data"].clone();
        fields["subject"] = json!("Synthetic revised subject");
        evidence.check(
            "live_rich_mutation_capture",
            put(
                world,
                &[observation("messages.message", "rich-link", &fields)],
            )
            .is_ok(),
            "",
        );
        let changed = first(&thread(&gateway));
        evidence.check(
            "live_rich_mutation_increments_record_version",
            changed.is_some_and(|c| {
                c["version"].as_u64() > item["version"].as_u64()
                    && c["data"]["subject"] == "Synthetic revised subject"
                    && c["data"]["balloon"] == fields["balloon"]
            }),
            "",
        );
    }
    deleted_messages(world, evidence)
}
