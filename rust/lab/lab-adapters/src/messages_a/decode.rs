use crate::{
    assertions::gateway_aliases,
    http::Client,
    readmodels::{PROTOCOLS, call, observation, offset, put, reference},
};
use lab_runner::Evidence;
use lab_world::{Error, Result, World};
use serde_json::{Value, json};

pub const SCENARIOS: [&str; 2] = ["MSG-DECODE-BALLOON", "MSG-DECODE-FORMATTING"];
const SCOPE: &str = "messages:read,contacts:read,system:read";
const AT: &str = "2026-10-01T12:00:00Z";
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

fn alias_of(world: &World, collection: &str, native: &str) -> String {
    Client::new(world, SCOPE)
        .and_then(|client| gateway_aliases(&client))
        .ok()
        .and_then(|aliases| {
            aliases
                .get(&format!("messages/{collection}/{native}"))
                .cloned()
        })
        .unwrap_or_default()
}
fn chat_summary(client: &Client, protocol: &str, id: &str) -> Option<Value> {
    let result = call(client, protocol, "messages.chats", &json!({"limit": 200}))?;
    result["chats"]
        .as_array()?
        .iter()
        .find(|summary| summary["chat"]["id"] == id)
        .cloned()
}
fn items(value: Option<&Value>, key: &str) -> usize {
    value
        .and_then(|v| v[key].as_array())
        .map_or(usize::MAX, Vec::len)
}

#[allow(clippy::too_many_lines)]
pub fn decode(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    crate::native_core::rust_tests(
        evidence,
        "scrubbed_native_decoder_fixtures",
        "ab-native",
        &["--test", "decoders"],
    );
    if !evidence.assertions.iter().all(|a| a.passed) {
        return Ok(());
    }
    let at = |second: i64| offset(AT, second);
    let mut observations = vec![];
    for filter in ["known", "unknown", "deleted", "spam"] {
        let mut data = json!({"display_name": format!("Bridge Test {filter}"), "participants": [], "filter": filter, "read_receipts": "default", "muted_until": null, "pinned_index": null});
        if filter == "spam" {
            data["filter"] = json!("unknown");
        }
        if filter == "deleted" {
            data["filter"] = json!("known");
            data["deleted_at"] = json!(AT);
            data["recoverable_until"] = json!(offset(AT, 30 * 24 * 3600)?);
        }
        observations.push(observation(
            "messages.chat",
            &format!("rich-{filter}"),
            &data,
        ));
    }
    for (i, kind) in KINDS.iter().enumerate() {
        let mut data = json!({"title": format!("[ab-test] rich-{kind}"), "text": format!("[ab-test] rich-{kind}"), "kind": "message", "chat_id": reference("messages.chat", "rich-known"), "sent_at": at(i64::try_from(i).unwrap_or(0))?, "balloon": {"bundle_id": format!("synthetic.{kind}"), "kind": kind}, "has_unseen_mention": true});
        if *kind == "link" {
            data["subject"] = json!("Synthetic subject");
            data["audio"] = json!({"is_audio_message": true, "played_at": AT, "expirable": true, "kept": null, "transcription": "Synthetic transcript"});
            data["schedule"] = json!({"state": "native_3", "scheduled_for": AT});
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
        observations.push(observation("messages.message", &key, &data));
    }
    observations.push(observation(
        "messages.message",
        "rich-spam-message",
        &json!({"title": "[ab-test] spam", "kind": "message", "is_spam": true, "chat_id": reference("messages.chat", "rich-spam"), "sent_at": AT}),
    ));
    observations.push(observation(
        "messages.message",
        "rich-sticker-control",
        &json!({"kind": "system", "text": "[ab-test] synthetic sticker control", "sent_at": AT}),
    ));
    observations.push(observation(
        "messages.attachment",
        "rich-glyph",
        &json!({"message_id": reference("messages.message", "rich-sticker-control"), "filename": "synthetic.png", "mime_type": "image/png", "is_sticker": true, "hidden": true, "genmoji": true, "sticker": {"target_message_id": reference("messages.message", "rich-link"), "part_index": 0, "x": 0.25, "y": 0.5, "scale": 1.25, "rotation": 0.125}}),
    ));
    observations.push(observation(
        "messages.attachment",
        "rich-image",
        &json!({"message_id": reference("messages.message", "rich-link"), "filename": "synthetic-preview.png", "mime_type": "image/png", "hidden": true}),
    ));
    observations.push(observation(
        "messages.handle",
        "rich-handle",
        &json!({"address": "reader@example.invalid", "service": "iMessage"}),
    ));
    let contact = world
        .control(
            "/external-change",
            &json!({"key": "rich-contact", "collection": "contacts.contact", "data": {"given_name": "Synthetic", "family_name": "Sender", "emails": [{"value": "reader@example.invalid"}], "thumbnail_image": {"digest": "a".repeat(64), "state": "present"}}}),
        )
        .is_ok();
    evidence.check("contact_capture", contact, "contact captured");
    let flushed = world.control("/flush", &Value::Null).is_ok();
    evidence.check("contact_committed", flushed, "contact capture committed");
    evidence.check(
        "rich_capture",
        put(world, &observations).is_ok(),
        "rich capture committed",
    );
    let client = Client::new(world, SCOPE)?;
    let chat = alias_of(world, "messages.chat", "rich-known");
    for protocol in PROTOCOLS {
        for filter in ["known", "unknown", "unread", "deleted", "spam"] {
            let result = call(
                &client,
                protocol,
                "messages.chats",
                &json!({"filter": filter}),
            );
            let want = if filter == "unknown" || filter == "unread" {
                2
            } else {
                1
            };
            evidence.check(
                &format!("{protocol}_filter_{filter}"),
                result.is_some() && items(result.as_ref(), "chats") == want,
                "chat filter count",
            );
        }
        let summary = chat_summary(&client, protocol, &chat);
        evidence.check(
            &format!("{protocol}_unseen_mention"),
            summary.is_some_and(|s| s["has_unseen_mention"] == true),
            "unseen mention",
        );
        for kind in ["links", "locations"] {
            let result = call(
                &client,
                protocol,
                "messages.collection",
                &json!({"chat_id": chat, "kind": kind}),
            );
            evidence.check(
                &format!("{protocol}_collection_{kind}"),
                result.is_some() && items(result.as_ref(), "items") == 1,
                "collection items",
            );
        }
        let thread = call(
            &client,
            protocol,
            "messages.thread",
            &json!({"chat_id": chat}),
        );
        evidence.check(
            &format!("{protocol}_every_balloon_kind_passes_through"),
            thread.is_some() && items(thread.as_ref(), "items") == 10,
            "balloon kinds",
        );
        for item in thread
            .as_ref()
            .and_then(|t| t["items"].as_array())
            .into_iter()
            .flatten()
        {
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
                        && item["attachments"].as_array().map_or(0, Vec::len) == 2,
                    "rich fields",
                );
            }
        }
        let handle = alias_of(world, "messages.handle", "rich-handle");
        handle_checks(world, &client, protocol, &handle, evidence)?;
    }
    let target = alias_of(world, "messages.message", "rich-link");
    let anchored = json!({"chat_id": chat, "anchor": target, "limit": 1});
    let before = call(&client, "rest", "messages.thread", &anchored);
    let reaction = observation(
        "messages.reaction",
        "rich-reaction",
        &json!({"message_id": reference("messages.message", "rich-link"), "sender_handle_id": reference("messages.handle", "rich-handle"), "kind": "emoji", "emoji": "🧪", "part_index": 0, "sent_at": at(3600)?}),
    );
    evidence.check(
        "link_reaction_capture",
        put(world, &[reaction]).is_ok(),
        "reaction captured",
    );
    let after = call(&client, "rest", "messages.thread", &anchored);
    let first = |thread: &Option<Value>| {
        thread
            .as_ref()
            .and_then(|t| t["items"].as_array())
            .filter(|items| items.len() == 1)
            .map(|items| items[0].clone())
    };
    let (before_item, after_item) = (first(&before), first(&after));
    evidence.check(
        "reaction_updates_link_thread_live",
        after_item
            .as_ref()
            .is_some_and(|a| a["reactions"].as_array().map_or(0, Vec::len) == 1)
            && before_item
                .as_ref()
                .zip(after_item.as_ref())
                .is_some_and(|(b, a)| a["data"]["balloon"] == b["data"]["balloon"]),
        "reaction updates the link thread",
    );
    if let Some(after_item) = after_item {
        let mut fields = after_item["data"].clone();
        fields["subject"] = json!("Synthetic revised subject");
        let revised = observation("messages.message", "rich-link", &fields);
        evidence.check(
            "live_rich_mutation_capture",
            put(world, &[revised]).is_ok(),
            "mutation captured",
        );
        let changed = first(&call(&client, "rest", "messages.thread", &anchored));
        evidence.check(
            "live_rich_mutation_increments_record_version",
            changed.is_some_and(|c| {
                c["version"].as_u64() > after_item["version"].as_u64()
                    && c["data"]["subject"] == "Synthetic revised subject"
                    && c["data"]["balloon"] == fields["balloon"]
            }),
            "mutation bumps version",
        );
    }
    deleted_messages(world, &client, evidence);
    Ok(())
}

fn handle_checks(
    world: &World,
    client: &Client,
    protocol: &str,
    handle: &str,
    evidence: &mut Evidence,
) -> Result<()> {
    let handles = call(
        client,
        protocol,
        "messages.handles",
        &json!({"ids": [handle, handle, "missing"]}),
    );
    evidence.check(
        &format!("{protocol}_handles_names_initials_avatar_dedup"),
        handles.is_some_and(|h| {
            h["handles"].as_array().is_some_and(|list| {
                list.len() == 1
                    && list[0]["display_name"] == "Synthetic Sender"
                    && list[0]["initials"] == "SS"
                    && !list[0]["avatar"].is_null()
            })
        }),
        "handle names",
    );
    let restricted = Client::new(world, "messages:read")?;
    let private = call(
        &restricted,
        protocol,
        "messages.handles",
        &json!({"ids": [handle]}),
    );
    evidence.check(
        &format!("{protocol}_handles_respect_contact_visibility"),
        private.is_some_and(|h| {
            h["handles"].as_array().is_some_and(|list| {
                list.len() == 1
                    && list[0]["contact_id"].is_null()
                    && list[0]["avatar"].is_null()
                    && list[0]["display_name"] == "reader@example.invalid"
            })
        }),
        "handle visibility",
    );
    let over = call(
        client,
        protocol,
        "messages.handles",
        &json!({"ids": vec![""; 501]}),
    );
    evidence.check(
        &format!("{protocol}_handles_reject_over_budget"),
        over.is_none(),
        "over budget rejected",
    );
    Ok(())
}

fn deleted_messages(world: &World, client: &Client, evidence: &mut Evidence) {
    let put_ok = put(
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
                &json!({"chat_id": reference("messages.chat", "rich-mixed-chat"), "kind": "message", "text": "[ab-test] synthetic active", "sent_at": AT}),
            ),
            observation(
                "messages.message",
                "rich-mixed-deleted",
                &json!({"chat_id": reference("messages.chat", "rich-mixed-chat"), "kind": "message", "text": "[ab-test] synthetic deleted", "sent_at": AT, "deleted_at": AT, "recoverable_until": null}),
            ),
            observation(
                "messages.attachment",
                "rich-mixed-image",
                &json!({"message_id": reference("messages.message", "rich-mixed-deleted"), "mime_type": "image/png"}),
            ),
        ],
    )
    .is_ok();
    evidence.check(
        "mixed_deleted_capture",
        put_ok,
        "mixed deleted chat captured",
    );
    let id = alias_of(world, "messages.chat", "rich-mixed-chat");
    for protocol in PROTOCOLS {
        let result = call(
            client,
            protocol,
            "messages.chats",
            &json!({"filter": "deleted"}),
        );
        let found = result.as_ref().is_some_and(|r| {
            r["chats"]
                .as_array()
                .is_some_and(|chats| chats.iter().any(|c| c["chat"]["id"] == id.as_str()))
        });
        evidence.check(
            &format!("{protocol}_deleted_filter_contains_mixed_chat"),
            result.is_some() && found,
            "deleted filter",
        );
        let summary = chat_summary(client, protocol, &id);
        evidence.check(
            &format!("{protocol}_mixed_chat_retains_active_unread"),
            summary.is_some_and(|s| s["unread_count"] == 1),
            "active unread retained",
        );
        let photos = call(
            client,
            protocol,
            "messages.collection",
            &json!({"chat_id": id, "kind": "photos"}),
        );
        evidence.check(
            &format!("{protocol}_recoverable_media_excluded"),
            photos.is_some() && items(photos.as_ref(), "items") == 0,
            "recoverable media excluded",
        );
    }
}

pub fn run(world: &mut World, evidence: &mut Evidence) -> Result<()> {
    if !SCENARIOS.contains(&evidence.scenario.as_str()) {
        return Err(Error::Invalid(format!(
            "decode not ported for {}",
            evidence.scenario
        )));
    }
    decode(world, evidence)
}
